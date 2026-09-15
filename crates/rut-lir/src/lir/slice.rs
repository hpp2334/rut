//! The `Iter` sequence contract (RFC 0012): `s.len()`, `s[i]`, and
//! `for (x of s)` lower through the receiver's `Iter` impl. The contract
//! is read-only — `type Target`, `len`, `get`.
//!
//! Implementations:
//! - `Array<T>` — the builtin (native) impl, emitting the fused
//!   `arrget`/`arrlen` ops (RFC 0032 §1.1 R2).
//! - `str` / `bytes` — the builtin (native) impls over the primitive
//!   cells, emitting `strcharat`/`strlen` and `bytesget`/`byteslen`.
//! - `Vec<T>` — std-lib rut code: `impl Iter for Vec<T> { type Target = T; .. }`
//!   in `rut/std-collection/vec.rut`; the one-line accessors are inlined
//!   here under the receiver's concrete class instantiation — no class name
//!   or field-shape is hardcoded in the compiler.
//!
//! `s[i] = v` is not part of `Iter` (which is read-only): mutable element
//! write stays on the concrete `Array`/`Vec` fused `arrset` path.

use super::*;

#[derive(Clone)]
pub(crate) enum SliceSource {
    /// builtin `Array<T>` — fused element ops
    Array,
    /// builtin `str` — `char` elements
    Str,
    /// builtin `bytes` — `u8` elements
    Bytes,
    /// `impl Iter for Class<..>`: accessor bodies inlined at the use
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

impl SliceInfo {
    /// The length is loop-invariant: `str`/`bytes` are immutable and
    /// `Array` is fixed-size, so a `for..of` need only read it once. An
    /// `impl Iter` accessor may be arbitrary, so its `len` stays live.
    pub(crate) fn fixed_len(&self) -> bool {
        !matches!(self.source, SliceSource::Impl { .. })
    }
}

impl<'a, 'b> FnCompiler<'a, 'b> {
    /// Resolve the `Iter` impl for `ty`, if any.
    pub(crate) fn slice_info(&mut self, ty: TypeId) -> Option<SliceInfo> {
        match self.ctx.types.kind(ty).clone() {
            TyKind::Array { elem } => Some(SliceInfo { source: SliceSource::Array, elem }),
            TyKind::Str => Some(SliceInfo { source: SliceSource::Str, elem: TY_STR }),
            TyKind::Bytes => Some(SliceInfo { source: SliceSource::Bytes, elem: TY_U8 }),
            TyKind::Data { .. } => {
                let seq_trait = self.ctx.seq_trait;
                // a source `impl Iter for ..` (concrete target or a generic
                // `Class<..>` template with the receiver's args substituted)
                let mut found: Option<(usize, Vec<(IdentId, TypeId)>, NodeHandle<AnyTy>)> = None;
                for (i, im) in self.ctx.impls.iter().enumerate() {
                    if Some(im.trait_id) != seq_trait {
                        continue;
                    }
                    let subst: Vec<(IdentId, TypeId)> = match (&im.target_data, self.ctx.inst_data.get(&ty)) {
                        (Some((d, params)), Some((dname, args))) if d == dname && params.len() == args.len() => {
                            params.iter().cloned().zip(args.iter().cloned()).collect()
                        }
                        (None, _) if im.target == ty => Vec::new(),
                        _ => continue,
                    };
                    // the interface ref's first type argument is the element
                    let Some(ty_node) = im.trait_arg_nodes.first().copied() else { continue };
                    found = Some((i, subst, ty_node));
                    break;
                }
                let (impl_idx, subst, ty_node) = found?;
                let elem = self.ctx.resolve_type(ty_node, &subst);
                Some(SliceInfo { source: SliceSource::Impl { impl_idx, self_ty: ty, subst }, elem })
            }
            _ => None,
        }
    }

    /// Resolve the `Iterator` impl for `ty`, if any: `(impl index, Item,
    /// target subst)`. The subst resolves the `next` body and its return
    /// type under the receiver's instantiation.
    pub(crate) fn iterator_info(&mut self, ty: TypeId) -> Option<(usize, TypeId, Vec<(IdentId, TypeId)>)> {
        let iter_trait = self.ctx.iter_trait?;
        for (i, im) in self.ctx.impls.iter().enumerate() {
            if Some(im.trait_id) != Some(iter_trait) {
                continue;
            }
            let subst: Vec<(IdentId, TypeId)> = match (&im.target_data, self.ctx.inst_data.get(&ty)) {
                (Some((d, params)), Some((dname, args))) if d == dname && params.len() == args.len() => {
                    params.iter().cloned().zip(args.iter().cloned()).collect()
                }
                (None, _) if im.target == ty => Vec::new(),
                _ => continue,
            };
            let Some(ty_node) = im.trait_arg_nodes.first().copied() else { continue };
            let item = self.ctx.resolve_type(ty_node, &subst);
            return Some((i, item, subst));
        }
        None
    }

    /// `s.len()` — the element count.
    pub(crate) fn emit_slice_len(&mut self, recv: u16, info: &SliceInfo, sp: u32) -> TcResult<u16> {
        match &info.source {
            SliceSource::Array => {
                let dst = self.new_reg(TY_I32);
                self.emit(Op::CallNat { nat: Nat::ArrLen, recv: Some(recv), args: vec![], dst: Some(dst) }, sp);
                Ok(dst)
            }
            SliceSource::Str => {
                let dst = self.new_reg(TY_I32);
                self.emit(Op::CallNat { nat: Nat::StrLen, recv: Some(recv), args: vec![], dst: Some(dst) }, sp);
                Ok(dst)
            }
            SliceSource::Bytes => {
                let dst = self.new_reg(TY_I32);
                self.emit(Op::CallNat { nat: Nat::ArrLen, recv: Some(recv), args: vec![], dst: Some(dst) }, sp);
                Ok(dst)
            }
            SliceSource::Impl { .. } => {
                let r = self.inline_slice_accessor(info, "len", recv, &[], Some(TY_I32), sp)?;
                Ok(r.expect("Iter::len returns a value"))
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
            SliceSource::Str => {
                // StrCharAt yields a char slot; convert to a 1-codepoint str
                let creg = self.new_reg(TY_CHAR);
                self.emit(Op::StrCharAt { dst: creg, s: recv, idx }, sp);
                let dst = self.new_reg(TY_STR);
                self.emit(Op::CallNat { nat: Nat::Str, recv: None, args: vec![creg], dst: Some(dst) }, sp);
                Ok(dst)
            }
            SliceSource::Bytes => {
                let repr = self.ctx.types.repr_of(TY_U8);
                let dst = self.new_reg(TY_U8);
                self.emit(Op::ArrGet { dst, arr: recv, idx, repr }, sp);
                Ok(dst)
            }
            SliceSource::Impl { .. } => {
                let r = self.inline_slice_accessor(info, "get", recv, &[idx], Some(info.elem), sp)?;
                Ok(r.expect("Iter::get returns a value"))
            }
        }
    }

    /// `s[i] = val` — element write. Only the concrete `Array`/`Vec` path
    /// has a mutable element; `str`/`bytes` and the read-only `Iter`
    /// contract do not.
    pub(crate) fn emit_slice_set(&mut self, recv: u16, idx: u16, val: u16, info: &SliceInfo, sp: u32) -> TcResult<()> {
        match &info.source {
            SliceSource::Array => {
                let repr = self.ctx.types.repr_of(info.elem);
                let val = self.clone_arg(val, info.elem, sp);
                self.emit(Op::ArrSet { arr: recv, idx, val, repr }, sp);
                Ok(())
            }
            SliceSource::Str | SliceSource::Bytes => {
                self.ctx.err(Span::new(sp, sp + 1), "`str`/`bytes` are immutable — element assignment is not allowed");
                Err(())
            }
            SliceSource::Impl { impl_idx, self_ty, .. } => {
                // the impl itself provides `set` (the class is mutable)
                if !self.ctx.impls[*impl_idx].methods.iter().any(|(n, _)| self.ctx.name(*n) == "set") {
                    self.ctx.err(Span::new(sp, sp + 1), format!("`{}` is not mutably indexable", self.ctx.types.name(*self_ty)));
                    return Err(());
                }
                self.inline_slice_accessor(info, "set", recv, &[idx, val], None, sp)?;
                Ok(())
            }
        }
    }

    /// Compile a `Iter` impl accessor body inline at the use site. The body
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
            self.ctx.err(Span::new(sp, sp + 1), format!("the `{mname}` Iter impl is missing on `{}`", self.ctx.types.name(self_ty)));
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
            self.ctx.err(Span::new(sp, sp + 1), format!("Iter::{mname}: {} args for {} params", args.len(), ptys.len()));
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
            self.ctx.err(Span::new(sp, sp + 1), "an `Iter` impl method body must be a single `return expr;` or `expr;`");
            Err(())
        } else {
            match self.ctx.ast.stmt(stmts[0]).clone() {
                StmtKind::Return { value: Some(e) } => {
                    self.compile_expr(e, want.or(Some(ret_ty))).map(|_| Some(self.last_reg))
                }
                StmtKind::ExprStmt(e) => self.compile_expr(e, None).map(|_| None),
                _ => {
                    self.ctx.err(Span::new(sp, sp + 1), "an `Iter` impl method body must be a single `return expr;` or `expr;`");
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
