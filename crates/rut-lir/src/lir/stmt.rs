//! Statement compilation: blocks with scoped locals, if/while/for-c/for-of,
//! when statements (pattern-test chains), exhaustiveness, and the
//! expression-depth budget.

use crate::check::{float_suffix_ty, int_suffix_ty, TcResult};
use rut_core::ops::*;
use rut_core::sym;
use rut_core::types::*;
use super::*;

/// Where a for-of's designated `[iterable]` member binds (v20): the
/// receiver's own class/enum decl (the method monomorphizes here,
/// keyed as the ordinary inherent method) or another module's
/// inherent-method row (the body mints in its owner — the mirror
/// request the linkable-classes machinery already speaks).
pub(crate) enum IterSource {
    /// `(decl name, marked member name, substitution env)`
    Local { data: IdentId, method: IdentId, env: Vec<(IdentId, TypeId)> },
    /// the mirror: same key shape, the bodies live in the owner —
    /// `(decl name, marked member name, substitution, owner, concrete target)`
    Extern { data: IdentId, method: IdentId, subst: Vec<(IdentId, TypeId)>, owner: String, concrete: TypeId },
}

impl<'a, 'b> FnCompiler<'a, 'b> {
    // ---- statements ----

    pub(crate) fn compile_block(&mut self, h: impl Into<NodeId>) -> TcResult<()> {
        let stmts = match self.ctx.ast.kind(h.into()) {
            Kind::Expr(ExprKind::Block { stmts }) => stmts,
            _ => return Ok(()),
        };
        let stmts = stmts.clone();
        let base = self.locals.len();
        for s in stmts {
            self.compile_stmt(s)?;
        }
        self.locals.truncate(base);
        Ok(())
    }

    pub(crate) fn compile_stmt(&mut self, node: NodeHandle<AnyStmt>) -> TcResult<()> {
        let sp = self.ctx.ast.span(node.id());
        self.span = sp.lo;
        match self.ctx.ast.stmt(node).clone() {
            StmtKind::LetStmt { is_mut, name, destructure, ty, init } => {
                let ty_node = ty; // the annotation node (shadowed below)
                let expected = ty.map(|t| self.resolve_type_now(t));
                let t = self.compile_expr(init, expected)?;
                if let Some(e) = expected {
                    if !self.widens(t, e) {
                        self.ctx.err(sp, format!(
                            "let `{}` is `{}` but the initializer is `{}`",
                            self.ctx.name(name), self.ctx.type_name(e), self.ctx.type_name(t)
                        ));
                    }
                    // concrete → slot widening boxes scalars (the slot
                    // ABI): the binding always holds a cell
                    self.widen_to_slot(t, e, sp.lo);
                }
                let ty = expected.unwrap_or(t);
                // origin counting: a trait-typed binding
                // remembers its concrete origins — a widening let names
                // the origin; a copy of another trait-typed binding takes
                // its origins; anything else stays unknown (vtable)
                let mut origins: Vec<TypeId> = Vec::new();
                if matches!(self.ctx.types.kind(ty), TyKind::TraitObj { .. }) {
                    origins = if t != ty && !matches!(self.ctx.types.kind(t), TyKind::TraitObj { .. }) {
                        // a widening let: the initializer's concrete type
                        // is the single origin
                        vec![t]
                    } else if let ExprKind::Path { segs } = self.ctx.ast.expr(init) {
                        if segs.len() == 1 { self.origins_of(segs[0].name) } else { Vec::new() }
                    } else {
                        Vec::new()
                    };
                }
                match destructure {
                    None => {
                        // the sharing law: the binding takes the
                        // initializer's cell handle — a share, never a copy
                        let reg = self.last_reg;
                        self.bind_local(name, reg, ty, is_mut, false, origins, sp.lo);
                        // union provenance (native-fastpath
                        // phase 1): an annotation spelling a union-bounded
                        // generic carries it; a copy takes its
                        // initializer's; anything else clears
                        if let Some(tn) = ty_node {
                            self.note_union_binding(name, Some(tn));
                        } else {
                            let copied = match self.ctx.ast.expr(init) {
                                ExprKind::Path { segs } if segs.len() == 1 => self
                                    .union_syms
                                    .get(&segs[0].name)
                                    .copied(),
                                _ => self.union_provenance(init).map(|(g, _)| g),
                            };
                            match copied {
                                Some(g) => {
                                    self.union_syms.insert(name, g);
                                }
                                None => {
                                    self.union_syms.remove(&name);
                                }
                            }
                        }
                    }
                    Some(names) => {
                        // `let (a, b) = ..`: each binding takes
                        // the matching tuple field
                        let TyKind::Data { fields } = self.ctx.types.kind(ty).clone() else {
                            self.ctx.err(sp, format!("destructuring needs a tuple —got `{}`", self.ctx.type_name(ty)));
                            return Err(());
                        };
                        if fields.len() != names.len() {
                            self.ctx.err(sp, format!(
                                "the pattern binds {} names but the tuple has {} fields",
                                names.len(), fields.len()
                            ));
                            return Err(());
                        }
                        let tuple_reg = self.last_reg;
                        for (i, n) in names.iter().enumerate() {
                            let fty = fields[i].ty;
                            let reg = self.new_reg(fty);
                            self.emit(
                                Op::GetF {
                                    dst: reg,
                                    obj: tuple_reg,
                                    field: i as u32,
                                    repr: self.ctx.types.repr_of(fty),
                                },
                                sp.lo,
                            );
                            self.bind_local(*n, reg, fty, is_mut, false, Vec::new(), sp.lo);
                            // a fresh binding from a tuple field carries no
                            // union provenance
                            self.union_syms.remove(n);
                        }
                    }
                }
                Ok(())
            }
            StmtKind::If { cond, then, els } => {
                self.compile_expr(cond, Some(TY_BOOL))?;
                let cond_reg = self.last_reg;
                let l_then = self.new_label();
                let l_end = self.new_label();
                let has_else = els.is_some();
                let else_label = if has_else { self.new_label() } else { l_end };
                self.br(cond_reg, l_then, else_label);
                self.bind(l_then);
                self.compile_block(then)?;
                self.jmp(l_end);
                if let Some(e) = els {
                    self.bind(else_label);
                    match e {
                        ElseBranch::If(h) => self.compile_stmt(h.into())?,
                        ElseBranch::Block(b) => {
                            self.compile_block(b)?;
                        }
                    }
                }
                self.bind(l_end);
                Ok(())
            }
            StmtKind::While { cond, body } => {
                let l_head = self.new_label();
                let l_body = self.new_label();
                let l_end = self.new_label();
                self.bind(l_head);
                self.emit(Op::LoopHead, sp.lo);
                self.compile_expr(cond, Some(TY_BOOL))?;
                let cond_reg = self.last_reg;
                self.br(cond_reg, l_body, l_end);
                self.bind(l_body);
                self.loops.push((l_head, l_end));
                self.compile_block(body)?;
                self.loops.pop();
                self.jmp(l_head);
                self.bind(l_end);
                Ok(())
            }
            StmtKind::ForOf { var, iter, body } => self.compile_for_of(node.id(), var, iter, body, sp),
            StmtKind::ForC { var, init, cond, update, body } => {
                // induction var is loop-owned; bind directly
                // to the initializer's register
                let t = self.compile_expr(init, None)?;
                let reg = self.last_reg;
                self.bind_local(var, reg, t, true, true, Vec::new(), sp.lo);
                let l_head = self.new_label();
                let l_body = self.new_label();
                let l_end = self.new_label();
                self.bind(l_head);
                self.emit(Op::LoopHead, sp.lo);
                self.compile_expr(cond, Some(TY_BOOL))?;
                let cond_reg = self.last_reg;
                self.br(cond_reg, l_body, l_end);
                self.bind(l_body);
                let l_cont = self.new_label();
                self.loops.push((l_cont, l_end));
                self.compile_block(body)?;
                self.loops.pop();
                self.bind(l_cont);
                self.compile_expr(update, None)?;
                self.jmp(l_head);
                self.bind(l_end);
                Ok(())
            }
            StmtKind::Return { value } => {
                // inlined method body: `return` targets the inliner's result
                if let Some((dst, l_end)) = self.inline_ret {
                    if let Some(v) = value {
                        let t = self.compile_expr(v, Some(self.ret_ty))?;
                        if !self.widens(t, self.ret_ty) {
                            self.ctx.err(sp, format!(
                                "return type mismatch: `{}` ({:?}) expected, `{}` ({:?}) returned",
                                self.ctx.type_name(self.ret_ty), self.ret_ty, self.ctx.type_name(t), t
                            ));
                        }
                        self.widen_to_slot(t, self.ret_ty, sp.lo);
                        let src = self.last_reg;
                        if self.ctx.types.is_ref(self.ret_ty) {
                            self.emit(Op::MovRef { dst, src }, sp.lo);
                        } else {
                            self.emit(Op::Mov { dst, src }, sp.lo);
                        }
                    }
                    self.jmp(l_end);
                    return Ok(());
                }
                // the async weave: the return value lands in the frame's
                // ANSWER lane (the awaiting frame's resume arm reads it),
                // then the frame retires exactly like the body-end
                // completion — answer BEFORE state, or the woken awaiter
                // reads a null (the value half)
                if let Some(f) = self.async_frame.clone() {
                    let ans_ty = match self.ctx.types.kind(f.frame_ty) {
                        TyKind::Data { fields } => fields
                            .get(rut_core::async_frame::ANSWER_FIELD as usize)
                            .map(|fi| fi.ty)
                            .unwrap_or(TY_NIL),
                        _ => TY_NIL,
                    };
                    match value {
                        Some(v) => {
                            let t = self.compile_expr(v, Some(ans_ty))?;
                            // the async block's infer pass (v20): the
                            // found type is RECORDED (the call site
                            // unifies it into the answer and re-weaves);
                            // the final weave checks as usual
                            if self.async_infer {
                                self.async_founds.push(t);
                            } else if !self.widens(t, ans_ty) {
                                self.ctx.err(sp, format!(
                                    "return type mismatch: `{}` expected, `{}` returned",
                                    self.ctx.type_name(ans_ty), self.ctx.type_name(t)
                                ));
                            }
                            self.widen_to_slot(t, ans_ty, sp.lo);
                            let src = self.last_reg;
                            let repr = self.ctx.types.repr_of(ans_ty);
                            self.emit(
                                Op::SetF {
                                    obj: f.frame_reg,
                                    field: rut_core::async_frame::ANSWER_FIELD,
                                    val: src,
                                    repr,
                                },
                                sp.lo,
                            );
                        }
                        None => {
                            if self.ctx.types.is_ref(ans_ty) {
                                let null = self.emit_null(sp.lo);
                                let repr = self.ctx.types.repr_of(ans_ty);
                                self.emit(
                                    Op::SetF {
                                        obj: f.frame_reg,
                                        field: rut_core::async_frame::ANSWER_FIELD,
                                        val: null,
                                        repr,
                                    },
                                    sp.lo,
                                );
                            }
                        }
                    }
                    let null = self.emit_null(sp.lo);
                    self.emit(
                        Op::SetF {
                            obj: f.frame_reg,
                            field: rut_core::async_frame::STATE_FIELD,
                            val: null,
                            repr: Repr::Ref,
                        },
                        sp.lo,
                    );
                    self.emit(Op::Ret { val: None }, sp.lo);
                    return Ok(());
                }
                match value {
                    Some(v) => {
                        let t = self.compile_expr(v, Some(self.ret_ty))?;
                        // implicit widening at the return:
                        // a concrete value coerces to a trait-typed return
                        if !self.widens(t, self.ret_ty) {
                            self.ctx.err(sp, format!(
                                "return type mismatch: `{}` ({:?}) expected, `{}` ({:?}) returned",
                                self.ctx.type_name(self.ret_ty), self.ret_ty, self.ctx.type_name(t), t
                            ));
                        }
                        self.widen_to_slot(t, self.ret_ty, sp.lo);
                        self.emit(Op::Ret { val: Some(self.last_reg) }, sp.lo);
                    }
                    None => {
                        if self.ret_ty != TY_NIL {
                            self.ctx.err(sp, "missing return value");
                        }
                        self.emit(Op::Ret { val: None }, sp.lo);
                    }
                }
                Ok(())
            }
            StmtKind::WhenStmt { scrut, arms } => {
                self.compile_when(scrut, &arms, None, sp)?;
                Ok(())
            }
            StmtKind::Break => {
                // inside a desugared `for..of` emit closure,
                // stopping the iteration IS returning `false`
                if self.emit_closure {
                    let f = self.new_reg(TY_BOOL);
                    self.emit(Op::ConstRaw { dst: f, bits: 0 }, sp.lo);
                    self.emit(Op::Ret { val: Some(f) }, sp.lo);
                    return Ok(());
                }
                let Some((_, brk)) = self.loops.last().copied() else {
                    self.ctx.err(sp, "`break` outside a loop");
                    return Ok(());
                };
                self.jmp(brk);
                Ok(())
            }
            StmtKind::Continue => {
                // skipping to the next element IS returning `true`
                if self.emit_closure {
                    let t = self.new_reg(TY_BOOL);
                    self.emit(Op::ConstRaw { dst: t, bits: 1 }, sp.lo);
                    self.emit(Op::Ret { val: Some(t) }, sp.lo);
                    return Ok(());
                }
                let Some((cont, _)) = self.loops.last().copied() else {
                    self.ctx.err(sp, "`continue` outside a loop");
                    return Ok(());
                };
                self.jmp(cont);
                Ok(())
            }
            StmtKind::ExprStmt(e) => {
                self.compile_expr(e, None)?;
                Ok(())
            }
        }
    }

    pub(crate) fn compile_for_of(&mut self, node: NodeId, var: IdentId, iter: NodeHandle<AnyExpr>, body: NodeHandle<BlockNode>, sp: rut_lexer::span::Span) -> TcResult<()> {
        let it = self.compile_expr(iter, None)?;
        let iter_reg = self.last_reg;
        // `for (v of p)` auto-derefs a pointer
        let (it, iter_reg) = self.deref_for_use(it, iter_reg, sp.lo);
        // the builtin sequences (Vec, Array, str, bytes) keep their fused
        // loops; a user type iterates through its registered
        // a `[iterable]`-marked member (the designated slot)
        let info = match self.slice_info(it) {
            Some(info) => info,
            None => {
                if let Some((src, elem_ty)) = self.iterate_impl(it) {
                    if std::env::var("RUT_PROBE_BT").is_ok() {
                        eprintln!("PROBE for-of marked member found, elem={}", self.ctx.type_name(elem_ty));
                    }
                    return self.compile_for_of_iterate(var, iter_reg, src, elem_ty, body, sp);
                }
                if std::env::var("RUT_PROBE_BT").is_ok() {
                    eprintln!("PROBE for-of NO marked member on {}", self.ctx.type_name(it));
                }
                self.ctx.err(sp, format!(
                    "`for (let .. of ..)` needs a sequence — `{}` is not one and marks no `[iterable]` member",
                    self.ctx.type_name(it)
                ));
                return Err(());
            }
        };
        let elem_ty = info.elem;
        let idx = self.new_reg(TY_I32);
        self.emit(Op::ConstRaw { dst: idx, bits: 0 }, sp.lo);
        // the sugar law: the loop var is ONE variable reassigned per
        // iteration. When a closure site captures it, the binding is
        // cell-backed: the hidden cell mints ONCE here (null-seeded —
        // the per-iteration store precedes any read), every iteration
        // stores into it, reads go through the accessor, and captures
        // pool the cell — the fused form behaves exactly like the
        // desugared one.
        let mut var_cell: Option<TypeId> = None;
        let mut cell_reg = NOREG;
        if self.captured.contains(&var) {
            let cname = self.ctx.intern(&format!(
                "#cell@{}@{}",
                self.ctx.name(var),
                self.cell_counter
            ));
            self.cell_counter += 1;
            let cty = self.ctx.types.intern(RutType {
                name: cname,
                kind: TyKind::Data { fields: vec![FieldInfo { name: var, ty: elem_ty }] },
            });
            let crec = self.new_reg(cty);
            let null = self.new_reg(elem_ty);
            self.emit(Op::ConstRaw { dst: null, bits: 0 }, sp.lo);
            { let (argv_off, argc) = self.pool_args(&[null]); self.emit(Op::MakeRecord { dst: crec, ty: cty, argv_off, argc }, sp.lo); }
            var_cell = Some(cty);
            cell_reg = crec;
        }
        // read a fixed source's length once, before the back-edge (`str`/
        // `bytes` are immutable, `Array` is fixed-size); an `impl Iter`
        // accessor stays inside the loop
        let hoisted_len = if info.fixed_len() {
            Some(self.emit_slice_len(iter_reg, &info, sp.lo)?)
        } else {
            None
        };
        let l_head = self.new_label();
        let l_body = self.new_label();
        let l_end = self.new_label();
        let l_cont = self.new_label();
        self.bind(l_head);
        self.emit(Op::LoopHead, sp.lo);
        let len_reg = match hoisted_len {
            Some(r) => r,
            None => self.emit_slice_len(iter_reg, &info, sp.lo)?,
        };
        let cond_reg = self.new_reg(TY_BOOL);
        self.emit(cmpop(CmpOp::Lt, PrimTy::I32, cond_reg, idx, len_reg), sp.lo);
        self.br(cond_reg, l_body, l_end);
        self.bind(l_body);
        // var = iter[idx]; the dst register is the loop variable (one reg
        // reused every iteration, overwritten/released by the element op).
        // The shared element yields as-is: a `?T` element
        // (`Vec<T>`'s `[?T]` backing, `[?T]` arrays) binds its handle and
        // every use auto-derefs; scalars copy their slot.
        let var_reg = self.emit_slice_get(iter_reg, idx, &info, sp.lo)?;
        match var_cell {
            Some(cty) => {
                // the element store: into the shared cell (the binding's
                // register holds the cell, stable across iterations)
                let repr = self.ctx.types.repr_of(elem_ty);
                self.emit(Op::SetF { obj: cell_reg, field: 0, val: var_reg, repr }, sp.lo);
                self.locals.push(Local { name: var, reg: cell_reg, ty: elem_ty, is_mut: false, loop_var: true, origins: Vec::new(), field: NO_FIELD, cell: Some(cty) });
            }
            None => {
                self.locals.push(Local { name: var, reg: var_reg, ty: elem_ty, is_mut: false, loop_var: true, origins: Vec::new(), field: NO_FIELD, cell: None });
            }
        }
        self.loops.push((l_cont, l_end));
        self.compile_block(body)?;
        self.loops.pop();
        self.locals.pop();
        self.bind(l_cont);
        let one = self.new_reg(TY_I32);
        self.emit(Op::ConstRaw { dst: one, bits: 1 }, sp.lo);
        let next = self.new_reg(TY_I32);
        self.emit(arith(ArithOp::Add, PrimTy::I32, next, idx, one), sp.lo);
        self.emit(Op::Mov { dst: idx, src: next }, sp.lo);
        self.jmp(l_head);
        self.bind(l_end);
        let _ = node;
        Ok(())
    }


    /// The receiver's designated `[iterable]` member (v20, the marker's
    /// dispatch): where the weave's `xs.<member>(emit)` binds, and the
    /// element type — read off the MARKED MEMBER's own emit parameter
    /// (`fn(E) -> bool`) under the target substitution. Built-in
    /// sequences never reach here (their fused loops lower first).
    /// LOCAL first — this unit's own class/enum decls; then the
    /// cross-package inherent rows (the linkable-classes registry):
    /// another module's marked member, a template row whose per-
    /// instantiation bodies live in its owner.
    fn iterate_impl(&mut self, ty: TypeId) -> Option<(IterSource, TypeId)> {
        if matches!(self.ctx.types.kind(ty), TyKind::TraitObj { .. }) {
            // trait objects dispatch through their vtable — a concrete
            // iterable is needed at the call site in this build
            return None;
        }
        let iterable_marker = sym::ITERABLE_MARKER;
        // the receiver names the class: either directly (a plain
        // record) or through its instantiation's decl (`inst_data`)
        let (dname, args): (IdentId, Vec<TypeId>) = match self.ctx.inst_data.get(&ty).cloned() {
            Some((d, a)) => (d, a),
            None => (self.ctx.types.type_at(ty).name, vec![]),
        };
        // LOCAL: the class's or enum's own marked member (methods attach
        // to the decl — the ordinary inherent machinery)
        let mut locals: Vec<(IdentId, Vec<(IdentId, NodeHandle<MethodDeclNode>)>, Vec<IdentId>)> = Vec::new();
        if let Some((n, d)) = self.ctx.datas.iter().find(|(n, _)| *n == dname) {
            locals.push((*n, d.methods.clone(), d.generics.clone()));
        }
        if let Some((n, e)) = self.ctx.enums.iter().find(|(n, _)| *n == dname) {
            locals.push((*n, e.methods.clone(), Vec::new()));
        }
        for (n, methods, generics) in locals {
            for (mname, mnode) in &methods {
                let md = self.ctx.ast.method_decl(*mnode);
                if md.marker != Some(iterable_marker) {
                    continue;
                }
                let env: Vec<(IdentId, TypeId)> = generics
                    .iter()
                    .cloned()
                    .zip(args.iter().cloned())
                    .collect();
                let Some(elem) = self.marker_emit_elem(&md.params, &env) else {
                    self.ctx.err(
                        self.ctx.ast.span(mnode.id()),
                        format!(
                            "`[iterable] {}` needs an emit parameter — `fn {}(self, emit: fn(E) -> bool)` (the element type falls out of the marked member's signature)",
                            self.ctx.name(*mname),
                            self.ctx.name(*mname)
                        ),
                    );
                    return None;
                };
                return Some((IterSource::Local { data: n, method: *mname, env }, elem));
            }
        }
        self.iterate_impl_extern(ty, dname, args)
    }

    /// The emit parameter's element: the marked member's one non-self
    /// parameter must be `fn(E) -> bool` — `E` resolved under `env`.
    fn marker_emit_elem(&mut self, params: &[NodeHandle<AnyParam>], env: &[(IdentId, TypeId)]) -> Option<TypeId> {
        for p in params {
            if let MemberKind::Param(ParamData { ty: Some(t), .. }) = self.ctx.ast.param(*p) {
                let ty = self.ctx.resolve_sig_ty(*t, env, None);
                if let TyKind::Fn { params: fps, ret } = self.ctx.types.kind(ty).clone() {
                    if fps.len() == 1 && ret == TY_BOOL {
                        return Some(fps[0]);
                    }
                }
                return None;
            }
        }
        None
    }

    /// The cross-package half: another module's marked member on a used
    /// class — a TEMPLATE inherent row (the receiver instantiates the
    /// row's target), the element read off the placeholder emit
    /// parameter under the class substitution, the mint owner the row's
    /// exporter. The concrete member body need not exist here at all:
    /// the weave lowers to the mirror request and link binds the
    /// owner's compiled body.
    fn iterate_impl_extern(&mut self, ty: TypeId, dname: IdentId, args: Vec<TypeId>) -> Option<(IterSource, TypeId)> {
        let rows = self.ctx.extern_inherents.clone();
        for (_ih, row) in rows.iter().enumerate() {
            let hits_template = self
                .ctx
                .extern_generics
                .get(&dname)
                .map_or(false, |g| g.template == row.target);
            if !hits_template && row.target != ty {
                continue;
            }
            for (_midx, m) in row.methods.iter().enumerate() {
                if m.marker != rut_core::binary::MARKER_ITERABLE {
                    continue;
                }
                let _ = m;
                // the element: the emit parameter's `#leaf` re-spelled
                // under the target substitution (the row's signatures
                // spell the template's `#<param>` placeholders)
                let subst: Vec<(IdentId, TypeId)> = match self.ctx.extern_generics.get(&dname) {
                    Some(g) => g.params.iter().cloned().zip(args.iter().cloned()).collect(),
                    None => vec![],
                };
                let Some(elem) = self.extern_marker_emit_elem(m.params.first().copied(), &subst) else {
                    continue;
                };
                let owner = self.ctx.owner_of_data(dname);
                let method = m.name;
                return Some((IterSource::Extern { data: dname, method, subst, owner, concrete: ty }, elem));
            }
        }
        None
    }

    /// The extern row's emit-parameter element: the placeholder fn row
    /// (`fn(#E) -> bool`) resolves its leaf text through the target
    /// substitution — the leaf spells the parameter, the substitution
    /// keys the bare name.
    fn extern_marker_emit_elem(&mut self, param: Option<TypeId>, subst: &[(IdentId, TypeId)]) -> Option<TypeId> {
        let p = param?;
        let TyKind::Fn { params: fps, ret } = self.ctx.types.kind(p).clone() else {
            return None;
        };
        if fps.len() != 1 || ret != TY_BOOL {
            return None;
        }
        let leaf = self.ctx.type_name(fps[0]).to_string();
        let bare = leaf.trim_start_matches('#');
        let bid = self.ctx.lookup_name(bare);
        if let Some((_, t)) = bid.and_then(|b| subst.iter().find(|(n, _)| *n == b)) {
            return Some(*t);
        }
        // a CONCRETE element (a non-generic row): the leaf is the
        // element's own spelling
        bid.and_then(|b| self.ctx.types.dense_id_of_name(b))
    }


    /// `for (v of xs)` over a user iterable — desugars to
    /// `xs.iterate(emit)` on the registered impl, where `emit` is a
    /// synthetic closure carrying the loop body: `break` returns `false`,
    /// `continue` and the fall-through return `true`.
    ///
    /// The capture law: plain captures copy their slot (ref-headed
    /// handles share the cell, primitives copy); a PROMOTED capture
    /// (captured ∧ reassigned) pools the binding's hidden cell instead,
    /// so both frames stay linked. The loop var itself, when captured,
    /// is ONE variable reassigned per iteration (the sugar law): the
    /// outer frame mints its shared cell, the emit closure binds it
    /// cell-backed and stores the incoming element at frame entry.
    fn compile_for_of_iterate(
        &mut self,
        var: IdentId,
        rreg: u16,
        src: IterSource,
        elem_ty: TypeId,
        body: NodeHandle<BlockNode>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<()> {
        // captures: the body's enclosing locals; the capture inherits
        // the binding's mutability (writes through a captured `let mut`
        // stay legal)
        let mut referenced = Vec::new();
        self.scan_names(body.id(), &mut referenced);
        let mut caps: Vec<Capture> = Vec::new();
        let mut cap_regs: Vec<u16> = Vec::new();
        for n in referenced {
            if n == var {
                continue;
            }
            if let Some(l) = self.lookup(n).cloned() {
                if l.cell.is_some() {
                    // promoted: pool the SHARED CELL (not the value) —
                    // reads and writes on either side route through it
                    caps.push(Capture { name: n, ty: l.ty, is_mut: l.is_mut, cell: l.cell });
                    cap_regs.push(l.reg);
                } else {
                    // copy the CURRENT value into a capture register
                    // (the immediate-slot copy: primitives copy, ref
                    // handles share their cell)
                    let cap_reg = self.read_local(&l, sp.lo);
                    caps.push(Capture { name: n, ty: l.ty, is_mut: l.is_mut, cell: None });
                    cap_regs.push(cap_reg);
                }
            }
        }
        // the loop var captured: mint its shared cell in THIS frame
        // (null-seeded — the emit closure stores the element before the
        // body can ever read it), and pool it as the emit closure's
        // first capture
        let mut var_cell: Option<TypeId> = None;
        let mut var_cell_reg: Option<u16> = None;
        if self.captured.contains(&var) {
            let cname = self.ctx.intern(&format!(
                "#cell@{}@{}",
                self.ctx.name(var),
                self.cell_counter
            ));
            self.cell_counter += 1;
            let cty = self.ctx.types.intern(RutType {
                name: cname,
                kind: TyKind::Data { fields: vec![FieldInfo { name: var, ty: elem_ty }] },
            });
            let crec = self.new_reg(cty);
            let null = self.new_reg(elem_ty);
            self.emit(Op::ConstRaw { dst: null, bits: 0 }, sp.lo);
            { let (argv_off, argc) = self.pool_args(&[null]); self.emit(Op::MakeRecord { dst: crec, ty: cty, argv_off, argc }, sp.lo); }
            var_cell = Some(cty);
            var_cell_reg = Some(crec);
        }
        self.ctx
            .for_of_sigs
            .insert(body.id().0, (elem_ty, caps.clone(), var_cell));
        let emit = crate::check::Inst {
            key: crate::check::FnKey::ForOfEmit { body: body.id(), var },
            subst: vec![],
            trait_origins: vec![],
        };
        let fid = self.ctx.ensure_inst(emit);
        // the surface fn type spells the element parameter only — the
        // capture tail (and a promoted loop var's cell) is ABI
        let fty = self.ctx.mk_fn_ty(vec![elem_ty], TY_BOOL);
        let clo = self.new_reg(fty);
        {
            let mut argv: Vec<u16> = var_cell_reg.into_iter().collect();
            argv.extend(cap_regs.iter().copied());
            let (argv_off, argc) = self.pool_args(&argv);
            self.emit(Op::MakeClosure { dst: clo, func: fid, argv_off, argc }, sp.lo);
        }
        // `xs.iterate(emit)` — the impl's method, statically bound to
        // this impl (nominal registry); a cross-package row binds the
        // mirror stub instead — the owner mints the template impl at
        // the concrete target and compiles the body (link canonicalizes
        // the ledger keys, the call lands on the owner's fn)
        let mfid = match src {
            // the marked member IS an inherent method: the ordinary
            // class-method key (the same one a plain `xs.<member>(..)`
            // call binds), monomorphized under the target substitution
            IterSource::Local { data, method, env } => self
                .ctx
                .ensure_inst(crate::check::Inst {
                    key: crate::check::FnKey::Method { data, name: method },
                    subst: env,
                    trait_origins: vec![],
                }),
            IterSource::Extern { data, method, subst, owner, concrete } => {
                // the mirror claims under the class decl + method +
                // substitution: the OWNER's compiled bodies ledger under
                // that key, so link binds the call to the real body —
                // and `mirror_inst` files the owner-side body request
                // itself (the concrete instantiation is what the owner
                // mints at).
                let _ = (owner, concrete);
                self.ctx.mirror_inst(crate::check::Inst {
                    key: crate::check::FnKey::Method { data, name: method },
                    subst,
                    trait_origins: vec![],
                })
            }
        };
        { let (argv_off, argc) = self.pool_recv_args(rreg, &(vec![clo])); self.emit(Op::CallM { func: mfid, argv_off, argc, dst: NOREG }, sp.lo); }
        if std::env::var("RUT_PROBE_BT").is_ok() {
            eprintln!("PROBE for-of iterate call emitted: mfid={mfid} recv_reg={rreg}");
        }
        Ok(())
    }

    // ---- `when` ----

    pub(crate) fn compile_when(
        &mut self,
        scrut: NodeHandle<AnyExpr>,
        arms: &[NodeHandle<AnyArm>],
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let st = self.compile_expr(scrut, None)?;
        let scrut_reg = self.last_reg;
        // arm type unification (statement-when: nil)
        let result_ty = if let Some(e) = expected {
            e
        } else {
            // first expression-arm's type
            let mut t = TY_NIL;
            for a in arms {
                if let ArmKind::WhenArm { body, .. } = self.ctx.ast.arm(*a) {
                    t = self.expr_type_hint(*body);
                    break;
                }
            }
            t
        };
        let result_reg = if result_ty != TY_NIL {
            Some(self.new_reg(result_ty))
        } else {
            None
        };
        let l_end = self.new_label();
        // exhaustiveness + duplicates
        self.check_when_exhaustive(st, arms, sp)?;
        for arm in arms {
            let arm = *arm;
            let ArmKind::WhenArm { pats, body } = self.ctx.ast.arm(arm) else { continue };
            let l_arm = self.new_label();
            let l_next_arm = self.new_label();
            // alternatives chain within the arm: `1, 2, 3 -> ..`
            for (pi, p) in pats.iter().enumerate() {
                let matched = self.compile_pattern_test(*p, st, scrut_reg, sp)?;
                let last = pi == pats.len() - 1;
                if last {
                    self.br(matched, l_arm, l_next_arm);
                } else {
                    let l_next_pat = self.new_label();
                    self.br(matched, l_arm, l_next_pat);
                    self.bind(l_next_pat);
                }
            }
            self.bind(l_arm);
            // nil arms may be statement blocks (`-> { a(); b(); }`)
            // — blocks aren't value expressions in this
            // build, so compile them as scoped statement blocks
            let body_is_block = matches!(self.ctx.ast.expr(*body), ExprKind::Block { .. });
            let bt = if body_is_block && result_ty == TY_NIL {
                self.compile_block(*body)?;
                TY_NIL
            } else {
                self.compile_expr(*body, if result_ty != TY_NIL { Some(result_ty) } else { None })?
            };
            if result_ty != TY_NIL {
                if bt != result_ty {
                    self.ctx.err(self.ctx.ast.span(body.id()), format!(
                        "`when` arms must agree: `{}` vs `{}`",
                        self.ctx.type_name(result_ty), self.ctx.type_name(bt)
                    ));
                }
                let rr = result_reg.unwrap();
                if self.ctx.types.is_ref(result_ty) {
                    self.emit(Op::MovRef { dst: rr, src: self.last_reg }, sp.lo);
                } else {
                    self.emit(Op::Mov { dst: rr, src: self.last_reg }, sp.lo);
                }
            }
            self.jmp(l_end);
            self.bind(l_next_arm);
        }
        self.bind(l_end);
        if let Some(rr) = result_reg {
            // the when's value moves into a fresh register so the "value in
            // last_reg" convention holds for the ENCLOSING expression
            let out = self.new_reg(result_ty);
            if self.ctx.types.is_ref(result_ty) {
                self.emit(Op::MovRef { dst: out, src: rr }, sp.lo);
            } else {
                self.emit(Op::Mov { dst: out, src: rr }, sp.lo);
            }
            Ok(result_ty)
        } else {
            Ok(TY_NIL)
        }
    }

    pub(crate) fn expr_type_hint(&mut self, node: NodeHandle<AnyExpr>) -> TypeId {
        // best-effort type for when-arm unification without full inference:
        // literals only; otherwise first arm decides later via compile
        match self.ctx.ast.expr(node) {
            ExprKind::Lit(Lit::Int(_, s)) => s.map(int_suffix_ty).unwrap_or(TY_I32),
            ExprKind::Lit(Lit::Float(_, s)) => s.map(float_suffix_ty).unwrap_or(TY_F32),
            ExprKind::Lit(Lit::Str(_) | Lit::RawStr(_)) => TY_STR,
            ExprKind::Lit(Lit::Bool(_)) => TY_BOOL,
            ExprKind::Block { .. } => TY_NIL,
            ExprKind::Struct { ty, .. } => self.resolve_type_now(*ty),
            _ => TY_NIL,
        }
    }

    pub(crate) fn check_when_exhaustive(&mut self, scrut_ty: TypeId, arms: &[NodeHandle<AnyArm>], sp: rut_lexer::span::Span) -> TcResult<()> {
        let mut has_else = false;
        let mut seen: Vec<String> = Vec::new();
        for a in arms {
            if let ArmKind::WhenArm { pats, .. } = self.ctx.ast.arm(*a) {
                for p in pats {
                    let key = match self.ctx.ast.pat(*p) {
                        PatKind::PatElse => {
                            has_else = true;
                            "else".to_string()
                        }
                        PatKind::PatLit(l) => format!("{:?}", l),
                        PatKind::PatPath { segs } => segs
                            .iter()
                            .map(|s| self.ctx.name(s.name).to_string())
                            .collect::<Vec<_>>()
                            .join("."),
                        PatKind::PatCtor { segs, .. } => segs
                            .iter()
                            .map(|s| self.ctx.name(s.name).to_string())
                            .collect::<Vec<_>>()
                            .join("."),
                        PatKind::PatWild => "_".to_string(),
                    };
                    if seen.contains(&key) && key != "else" {
                        self.ctx.err(self.ctx.ast.span(p.id()), format!("duplicate pattern `{key}`"));
                    }
                    seen.push(key);
                }
            }
        }
        if !has_else {
            if let TyKind::Enum { members } = self.ctx.types.kind(scrut_ty).clone() {
                for (m, _v) in &members {
                    let mtext = self.ctx.name(*m);
                    if !seen.iter().any(|s| s.ends_with(&format!(".{mtext}")) || s == mtext) {
                        self.ctx.err(
                            sp,
                            format!(
                                "`when` over an enum must be exhaustive —`{mtext}` is not covered and there is no `else` arm"
                            ),
                        );
                        return Err(());
                    }
                }
            } else if scrut_ty != TY_BOOL {
                self.ctx.err(
                    sp,
                    format!(
                        "`when` over `{}` needs an `else` arm —only enums are enumerable",
                        self.ctx.type_name(scrut_ty)
                    ),
                );
                return Err(());
            } else {
                // bool: covered iff true and false appear
                if !(seen.contains(&"Bool(true)".to_string()) && seen.contains(&"Bool(false)".to_string())) {
                    self.ctx.err(sp, "`when` over `bool` must cover `true` and `false`");
                    return Err(());
                }
            }
        }
        Ok(())
    }

    /// Emit the match test for one pattern; returns the bool reg.
    pub(crate) fn compile_pattern_test(&mut self, p: NodeHandle<AnyPat>, scrut_ty: TypeId, scrut_reg: u16, sp: rut_lexer::span::Span) -> TcResult<u16> {
        match self.ctx.ast.pat(p).clone() {
            PatKind::PatElse | PatKind::PatWild => {
                let r = self.new_reg(TY_BOOL);
                self.emit(Op::ConstRaw { dst: r, bits: 1 }, sp.lo);
                Ok(r)
            }
            PatKind::PatLit(lit_node) => {
                // the pattern holds a literal expression node
                let lit = match self.ctx.ast.expr(lit_node) {
                    ExprKind::Lit(l) => l.clone(),
                    _ => {
                        self.ctx.err(sp, "pattern literal expected");
                        return Err(());
                    }
                };
                let (_lty, lreg) = self.load_lit(lit, Some(scrut_ty), sp)?;
                let r = self.new_reg(TY_BOOL);
                match self.ctx.types.kind(scrut_ty).clone() {
                    TyKind::Prim(p) => {
                        self.emit(cmpop(CmpOp::Eq, p, r, scrut_reg, lreg), sp.lo);
                    }
                    TyKind::Str => {
                        self.emit(Op::StrCmp { eq: true, dst: r, a: scrut_reg, b: lreg }, sp.lo);
                    }
                    _ => {
                        self.ctx.err(sp, "this literal pattern cannot match the scrutinee type");
                    }
                }
                Ok(r)
            }
            PatKind::PatPath { segs } => {
                // enum member
                if segs.len() == 2 {
                    let ename = segs[0].name;
                    let mname = segs[1].name;
                    // a LOCAL enum's decl or a USED enum's binding — the
                    // same members the member-path expression reads
                    let enum_hit: Option<(TypeId, Vec<(IdentId, i64)>)> =
                        if let Some(e) = self.ctx.find_enum(ename).cloned() {
                            let members: Vec<(IdentId, i64)> = match self.ctx.types.kind(e.ty) {
                                TyKind::Enum { members } => members.clone(),
                                _ => vec![],
                            };
                            Some((e.ty, members))
                        } else {
                            self.ctx.extern_enum(ename)
                        };
                    if let Some((ety, members)) = enum_hit {
                        if ety == scrut_ty {
                            let midx = members.iter().position(|&(m, _)| m == mname);
                            match midx {
                                Some(i) => {
                                    let mreg = self.new_reg(scrut_ty);
                                    self.emit(Op::EnumNew { dst: mreg, ty: scrut_ty, member: i as u32 }, sp.lo);
                                    let r = self.new_reg(TY_BOOL);
                                    self.emit(Op::RefEq { eq: true, dst: r, a: scrut_reg, b: mreg }, sp.lo);
                                    return Ok(r);
                                }
                                None => {
                                    self.ctx.err(sp, format!("`{}` is not a member of {}", self.ctx.name(mname), self.ctx.name(ename)));
                                }
                            }
                        } else {
                            self.ctx.err(sp, format!(
                                "`when` pattern `{}` does not match the scrutinee type `{}`",
                                self.ctx.name(ename), self.ctx.type_name(scrut_ty)
                            ));
                        }
                    }
                }
                if segs.len() == 1 {
                    // bare member of the scrutinee's own enum
                    if let TyKind::Enum { members } = self.ctx.types.kind(scrut_ty).clone() {
                        let mname = segs[0].name;
                        if let Some(i) = members.iter().position(|(m, _)| *m == mname) {
                            let mreg = self.new_reg(scrut_ty);
                            self.emit(Op::EnumNew { dst: mreg, ty: scrut_ty, member: i as u32 }, sp.lo);
                            let r = self.new_reg(TY_BOOL);
                            self.emit(Op::RefEq { eq: true, dst: r, a: scrut_reg, b: mreg }, sp.lo);
                            return Ok(r);
                        }
                    }
                    self.ctx.err(sp, "unknown pattern");
                }
                self.ctx.err(sp, "patterns in this build: enum members, literals, and `else`");
                Err(())
            }
            PatKind::PatCtor { .. } => {
                self.ctx.err(sp, "constructor patterns (Ok(x), Some(y)) are not supported in this build");
                Err(())
            }
        }
    }

}
