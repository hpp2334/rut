//! The `Slice<T>` sequence contract (RFC 0005): `s.len()`, `s[i]`,
//! `s[i] = v`, and `for (x of s)` lower through the receiver's `Slice`
//! impl. `Array<T>` has a builtin (native) impl that emits the fused
//! `arrget`/`arrset`/`arrlen` ops (RFC 0032 §1.1 R2). `Vec<T>` is
//! std-lib rut code: its impl lives in `rut/std-collection/vec.rut`
//! (`impl Slice<T> for Vec<T>`), and its one-line accessors are inlined
//! here under the receiver's concrete class instantiation — no class name
//! or field-shape is hardcoded in the compiler.
//!
//! `string`/`bytes` indexing and iteration are language primitives
//! (`strcharat`/`bytesget`), not `Slice` impls.

use super::*;

#[derive(Clone)]
pub(crate) enum SliceSource {
    /// builtin `Array<T>` — fused element ops
    Array,
    /// `impl Slice<..> for Class<..>`: accessor bodies inlined at the use
    /// site under `self_ty`/`subst`
    Impl {
        impl_idx: usize,
        self_ty: TypeId,
        subst: Vec<(IdentId, TypeId)>,
    },
}

#[derive(Clone)]
pub(crate) struct SliceInfo {
    pub source: SliceSource,
    pub elem: TypeId,
}

impl<'a, 'b> FnCompiler<'a, 'b> {
    /// Resolve the `Slice<T>` impl for `ty`, if any.
    pub(crate) fn slice_info(&self, ty: TypeId) -> Option<SliceInfo> {
        match self.ctx.types.kind(ty).clone() {
            TyKind::Array { elem } => Some(SliceInfo { source: SliceSource::Array, elem }),
            TyKind::Data { .. } => {
                let (dname, args) = self.ctx.inst_data.get(&ty).cloned()?;
                // a source `impl Slice<..> for dname<..>`
                let mut found = None;
                let slice_trait = self.ctx.slice_trait;
                for (i, im) in self.ctx.impls.iter().enumerate() {
                    let Some((d, params)) = &im.target_data else { continue };
                    if *d != dname || Some(im.trait_id) != slice_trait || params.len() != args.len() {
                        continue;
                    }
                    let Some(elem_name) = im.trait_args.first().copied() else { continue };
                    let subst: Vec<(IdentId, TypeId)> =
                        params.iter().cloned().zip(args.iter().cloned()).collect();
                    let Some(elem) = subst.iter().find(|(n, _)| *n == elem_name).map(|(_, t)| *t) else { continue };
                    found = Some(SliceInfo {
                        source: SliceSource::Impl { impl_idx: i, self_ty: ty, subst },
                        elem,
                    });
                    break;
                }
                found
            }
            _ => None,
        }
    }

    /// `s.len()` — the live length.
    pub(crate) fn emit_slice_len(&mut self, recv: u16, info: &SliceInfo, sp: u32) -> TcResult<u16> {
        match &info.source {
            SliceSource::Array => {
                let dst = self.new_reg(TY_I32);
                self.emit(Op::CallNat { nat: Nat::ArrLen, recv: Some(recv), args: vec![], dst: Some(dst) }, sp);
                Ok(dst)
            }
            SliceSource::Impl { .. } => {
                let r = self.inline_slice_accessor(info, "len", recv, &[], Some(TY_I32), sp)?;
                Ok(r.expect("Slice::len returns a value"))
            }
        }
    }

    /// `s[i]` — element read; the VM bounds-traps.
    pub(crate) fn emit_slice_get(&mut self, recv: u16, idx: u16, info: &SliceInfo, sp: u32) -> TcResult<u16> {
        match &info.source {
            SliceSource::Array => {
                let repr = self.ctx.types.repr_of(info.elem);
                let dst = self.new_reg(info.elem);
                self.emit(Op::ArrGet { dst, arr: recv, idx, repr }, sp);
                Ok(dst)
            }
            SliceSource::Impl { .. } => {
                let r = self.inline_slice_accessor(info, "get", recv, &[idx], Some(info.elem), sp)?;
                Ok(r.expect("Slice::get returns a value"))
            }
        }
    }

    /// `s[i] = val` — element write.
    pub(crate) fn emit_slice_set(&mut self, recv: u16, idx: u16, val: u16, info: &SliceInfo, sp: u32) -> TcResult<()> {
        match &info.source {
            SliceSource::Array => {
                let repr = self.ctx.types.repr_of(info.elem);
                self.emit(Op::ArrSet { arr: recv, idx, val, repr }, sp);
                Ok(())
            }
            SliceSource::Impl { .. } => {
                self.inline_slice_accessor(info, "set", recv, &[idx, val], None, sp)?;
                Ok(())
            }
        }
    }

    /// Compile a `Slice` impl accessor body inline at the use site. The body
    /// must be a single statement — `return <expr>;` (read) or `<expr>;`
    /// (write) — which is the contract's accessor shape. `self` binds to
    /// `recv` and the declared params to the pre-compiled `args`.
    fn inline_slice_accessor(
        &mut self,
        info: &SliceInfo,
        mname: &str,
        recv: u16,
        args: &[u16],
        want: Option<TypeId>,
        sp: u32,
    ) -> TcResult<Option<u16>> {
        let SliceSource::Impl { impl_idx, self_ty, subst } = &info.source else {
            unreachable!("inline_slice_accessor on a non-impl Slice source")
        };
        let (impl_idx, self_ty, subst) = (*impl_idx, *self_ty, subst.clone());
        let imp = self.ctx.impls[impl_idx].clone();
        let Some((_, mnode)) = imp.methods.iter().find(|(n, _)| self.ctx.name(*n) == mname).cloned() else {
            self.ctx.err(Span::new(sp, sp + 1), format!("the `{}` Slice impl is missing `{mname}`", self.ctx.types.name(self_ty)));
            return Err(());
        };
        let md = self.ctx.ast.method_decl(mnode).clone();
        let saved_self_ty = self.self_ty;
        let saved_subst = std::mem::replace(&mut self.subst, subst);
        let saved_class = self.current_class;
        let saved_inline_self = self.inline_self;
        self.self_ty = Some(self_ty);
        self.current_class = self.ctx.inst_data.get(&self_ty).map(|(d, _)| *d);
        // `self` resolves to the receiver register with no copy
        self.inline_self = self.ctx.lookup_name("self").map(|sid| (sid, recv));
        // resolve declared parameter/return types under the impl substitution
        let mut ptys: Vec<TypeId> = Vec::new();
        let mut self_mut = false;
        for p in &md.params {
            match self.ctx.ast.param(*p) {
                MemberKind::SelfParam(SelfParamData { is_mut }) => self_mut = *is_mut,
                MemberKind::Param(ParamData { ty: Some(t), .. }) => ptys.push(self.resolve_type_now(*t)),
                _ => ptys.push(TY_I32),
            }
        }
        let ret_ty = md.ret.map(|r| self.resolve_type_now(r)).unwrap_or(TY_UNIT);
        if ptys.len() != args.len() {
            self.ctx.err(Span::new(sp, sp + 1), format!("Slice::{mname}: {} args for {} params", args.len(), ptys.len()));
            self.self_ty = saved_self_ty;
            self.subst = saved_subst;
            self.current_class = saved_class;
            self.inline_self = saved_inline_self;
            return Err(());
        }
        // bind `self` + declared params to the already-evaluated registers
        let base = self.locals.len();
        if let Some(sid) = self.ctx.lookup_name("self") {
            self.locals.push(Local { name: sid, reg: recv, ty: self_ty, is_mut: self_mut, loop_var: false });
        }
        let mut ai = 0usize;
        for p in &md.params {
            if let MemberKind::Param(ParamData { name, is_mut, .. }) = self.ctx.ast.param(*p) {
                self.locals.push(Local { name: *name, reg: args[ai], ty: ptys[ai], is_mut: *is_mut, loop_var: false });
                ai += 1;
            }
        }
        // body: exactly one statement
        let stmts = match md.body.map(|b| b.id()) {
            Some(b) => match self.ctx.ast.kind(b) {
                Kind::Expr(ExprKind::Block { stmts }) => stmts.clone(),
                _ => Vec::new(),
            },
            None => Vec::new(),
        };
        let result: TcResult<Option<u16>> = if stmts.len() != 1 {
            self.ctx.err(Span::new(sp, sp + 1), "a `Slice` impl method body must be a single `return expr;` or `expr;`");
            Err(())
        } else {
            match self.ctx.ast.stmt(stmts[0]).clone() {
                StmtKind::Return { value: Some(e) } => {
                    self.compile_expr(e, want.or(Some(ret_ty))).map(|_| Some(self.last_reg))
                }
                StmtKind::ExprStmt(e) => self.compile_expr(e, None).map(|_| None),
                _ => {
                    self.ctx.err(Span::new(sp, sp + 1), "a `Slice` impl method body must be a single `return expr;` or `expr;`");
                    Err(())
                }
            }
        };
        self.locals.truncate(base);
        self.self_ty = saved_self_ty;
        self.subst = saved_subst;
        self.current_class = saved_class;
        self.inline_self = saved_inline_self;
        result
    }
}
