//! Inherent-impl calls and the inline-at-call-site fast paths (free fns and impl methods).

use crate::check::TcResult;
use crate::lir::*;

impl<'a, 'b> FnCompiler<'a, 'b> {

    /// Instance-method call on a concrete receiver. The method may be
    /// GENERIC in its own right (`store.get<T>(a)`, the two-pkg store's
    /// whole surface): its type arguments — spelled at the site or
    /// inferred from the arguments through the same unification the
    /// free-fn path runs (`ctx.get(items$)` binds `T` through the
    /// `Readable<T>` template) — join the CLASS instantiation to form the
    /// completed substitution. That full substitution resolves the
    /// callee's signature AND keys the Inst, so a method's parameter
    /// registers are typed per instantiation in the monomorphized frame —
    /// v1's class-instantiation-only typing is what left a generic
    /// method's `Readable<T>` parameter unresolved (the stale-wrong-value
    /// frames the spike0 e-cluster died of).
    pub(crate) fn compile_inherent_call(
        &mut self,
        dname: IdentId,
        class_args: Vec<TypeId>,
        self_ty: TypeId,
        mnode: NodeHandle<MethodDeclNode>,
        rreg: u16,
        generics: Vec<NodeHandle<AnyTy>>,
        args: Vec<NodeHandle<AnyExpr>>,
        _expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let d = self.ctx.find_data(dname).cloned().unwrap();
        let class_subst: Vec<(IdentId, TypeId)> =
            d.generics.iter().cloned().zip(class_args.iter().cloned()).collect();
        let md = self.ctx.ast.method_decl(mnode).clone();
        let mut_self = matches!(
            md.params.first().map(|p| self.ctx.ast.param(*p)),
            Some(MemberKind::SelfParam(SelfParamData { is_mut: true }))
        );
        let (params, ret, mname) = (md.params, md.ret, md.name);
        let _ = mut_self;
        // the method's own generics: explicit site arguments first
        let decl_generics: Vec<IdentId> = md.generics.clone();
        if generics.len() > decl_generics.len() {
            self.ctx.err(sp, format!(
                "`{}.{}` takes {} generic argument(s), {} given",
                self.ctx.name(dname), self.ctx.name(mname), decl_generics.len(), generics.len()
            ));
            return Err(());
        }
        let mut subst = class_subst.clone();
        for (g, node) in decl_generics.iter().zip(generics.iter()) {
            let t = self.resolve_type_now(*node);
            subst.push((*g, t));
        }
        if args.len() != params.len() - 1 {
            self.ctx.err(sp, format!("call arity: {} args for {} params", args.len(), params.len() - 1));
            return Err(());
        }
        // the callee's signature may spell `Self`/`T` — resolve under the
        // CALLEE's instantiation (the caller's context is irrelevant)
        let saved_self = self.self_ty;
        let saved_subst = std::mem::replace(&mut self.subst, subst.clone());
        self.self_ty = Some(self_ty);
        // compile args under best-effort expected types; a parameter node
        // still spelling an unbound generic (`a: Readable<T>`) gets no
        // hint — unification binds the generic from the argument
        let param_nodes: Vec<Option<NodeHandle<AnyTy>>> = params
            .iter()
            .skip(1)
            .map(|p| match self.ctx.ast.param(*p) {
                MemberKind::Param(ParamData { ty: Some(t), .. }) => Some(*t),
                _ => None,
            })
            .collect();
        let mut arg_tys: Vec<TypeId> = Vec::new();
        let mut aregs: Vec<u16> = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let expected = match param_nodes[i] {
                Some(tn) if self.free_generics(tn, &decl_generics, &subst).is_empty() => {
                    Some(self.resolve_type_now(tn))
                }
                // the shape hint (the phase-2 placeholder env): a
                // best-effort expected type whose unbound generics ride
                // fresh placeholder types — a spelled lambda argument
                // (`fn (ctx) -> str { .. }`) takes its parameter
                // annotations from the BOUND part (`ctx: DeriveCtx`);
                // unification below binds the real values
                Some(tn) => Some(self.hint_with_placeholders(tn, &decl_generics, &subst)),
                _ => None,
            };
            let t = self.compile_expr(*a, expected)?;
            if let Some(e) = expected {
                self.widen_to_slot(t, e, sp.lo);
            }
            arg_tys.push(t);
            aregs.push(self.last_reg);
        }
        // structural unification binds the method's remaining generics
        for (i, tn) in param_nodes.iter().enumerate() {
            if let Some(tn) = tn {
                self.unify_generic(*tn, arg_tys[i], &decl_generics, &mut subst, sp)?;
            }
        }
        for g in &decl_generics {
            if !subst.iter().any(|(n, _)| n == g) {
                self.ctx.err(sp, format!(
                    "cannot infer generic parameter `{}` of `{}.{}` — annotate the call: `.{}<..>(..)`",
                    self.ctx.name(*g), self.ctx.name(dname), self.ctx.name(mname), self.ctx.name(mname)
                ));
                return Err(());
            }
        }
        // inline bounds gate the completed substitution (RFC 0043)
        self.ctx.admit_bounds(&md.bounds, &subst, sp);
        // final param/ret types under the completed substitution —
        // through the FnCompiler resolver, so `Self` in the method's
        // own signature binds to the receiver (the `new`/`bump` law)
        self.subst = subst.clone();
        let mut ptys = Vec::new();
        for tn in param_nodes.iter() {
            match tn {
                Some(t) => ptys.push(self.resolve_type_now(*t)),
                None => ptys.push(TY_I32),
            }
        }
        let ret_ty = ret.map(|r| self.resolve_type_now(r)).unwrap_or(TY_NIL);
        self.self_ty = saved_self;
        self.subst = saved_subst;
        for (i, _) in args.iter().enumerate() {
            if !self.widens(arg_tys[i], ptys[i]) {
                self.ctx.err(self.ctx.ast.span(args[i].id()), format!(
                    "argument {} is `{}`, `{}` expected",
                    i + 1, self.ctx.type_name(arg_tys[i]), self.ctx.type_name(ptys[i])
                ));
            }
            // a trait-typed parameter whose expected type was unknown
            // during the argument's compile (it spells a free generic —
            // `a: Readable<T>`) widens HERE, after unification resolved
            // `ptys[i]` (the concrete-ABI scalar boxes, ref values
            // cross as their handles)
            let before = self.last_reg;
            self.widen_to_slot(arg_tys[i], ptys[i], sp.lo);
            if self.last_reg != before {
                aregs[i] = self.last_reg;
            }
        }
        // trait-typed parameters specialize per concrete argument (RFC
        // 0012 §5): the Inst carries one origin per trait-obj param
        let mut trait_origins = Vec::new();
        for (i, _) in args.iter().enumerate() {
            if matches!(self.ctx.types.kind(ptys[i]), TyKind::TraitObj { .. })
                && !matches!(self.ctx.types.kind(arg_tys[i]), TyKind::TraitObj { .. })
            {
                trait_origins.push(arg_tys[i]);
            }
        }
        // small instance methods inline at the call site: the class's
        // `push`/`pop`/`freeze` are rut code (RFC 0005), so an interpreted
        // frame per call is the cost of the design; inlining removes it
        // (generic methods never inline — the generic-method gate inside)
        if self.try_inline_method(
            dname, &subst, self_ty, mnode, mname, mut_self, rreg, &aregs, &ptys, ret_ty, sp,
        ) {
            return Ok(ret_ty);
        }
        let inst = crate::check::Inst {
            key: crate::check::FnKey::Method { data: dname, name: mname },
            subst,
            trait_origins,
        };
        let fid = self.ctx.ensure_inst(inst);
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        { let (argv_off, argc) = self.pool_recv_args(rreg, &(aregs)); self.emit(Op::CallM { func: fid, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
        Ok(ret_ty)
    }

    /// Inline a small, non-recursive instance method body at the call site.
    /// Returns `true` when it compiled the body; `false` to emit `CallM`.
    ///
    /// SUSPENDED (the two-pkg store batch): the splice re-interns the
    /// body's pooled operand lists into the caller's pool, and a body
    /// reaching a host fn with `any` params (the nmap `hv` lanes) then
    /// reads its site types from the caller's register file through a
    /// pool the peephole has since re-rewritten — the site types scramble
    /// (`store.kv.put(id, box)` decoded its KEY as the table). Until the
    /// rewriter tracks the register-type table in lockstep, methods stay
    /// real calls: `CallM` to the compiled Inst keeps every argv span and
    /// its site types exact. (P1.3's perf stays for FREE fns, whose
    /// bodies never cross the host lanes in the corpus.)
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_inline_method(
        &mut self,
        dname: IdentId,
        class_subst: &[(IdentId, TypeId)],
        self_ty: TypeId,
        mnode: NodeHandle<MethodDeclNode>,
        mname: IdentId,
        mut_self: bool,
        recv: u16,
        aregs: &[u16],
        ptys: &[TypeId],
        ret_ty: TypeId,
        sp: rut_lexer::span::Span,
    ) -> bool {
        let _ = (dname, class_subst, self_ty, mnode, mname, mut_self, recv, aregs, ptys, ret_ty, sp);
        return false;
    }
    #[allow(dead_code, clippy::too_many_arguments)]
    pub(crate) fn try_inline_method_orig(
        &mut self,
        dname: IdentId,
        class_subst: &[(IdentId, TypeId)],
        self_ty: TypeId,
        mnode: NodeHandle<MethodDeclNode>,
        mname: IdentId,
        mut_self: bool,
        recv: u16,
        aregs: &[u16],
        ptys: &[TypeId],
        ret_ty: TypeId,
        sp: rut_lexer::span::Span,
    ) -> bool {
        const MAX_STMTS: usize = 24;
        const MAX_DEPTH: usize = 4;
        // nested splices (a method body that itself calls inlined methods)
        // are where the pooled host-val call sites lose their register
        // identity — v1 keeps those bodies as real calls
        if !self.inline_stack.is_empty() {
            return false;
        }
        if self.inline_stack.len() >= MAX_DEPTH || self.inline_stack.contains(&(dname, mname)) {
            return false;
        }
        let md = self.ctx.ast.method_decl(mnode).clone();
        if md.is_async {
            return false; // `async` is diagnosed when the body is compiled
        }
        let Some(body) = md.body else { return false };
        let stmts = match self.ctx.ast.kind(body.id()) {
            Kind::Expr(ExprKind::Block { stmts }) => stmts.clone(),
            _ => return false,
        };
        if stmts.len() > MAX_STMTS {
            return false;
        }
        let saved_self_ty = self.self_ty;
        let saved_subst = std::mem::replace(&mut self.subst, class_subst.to_vec());
        let saved_class = self.current_class;
        let saved_ret = self.ret_ty;
        let saved_inline_ret = self.inline_ret;
        let saved_inline_self = self.inline_self;
        let saved_ub = std::mem::take(&mut self.union_bounds);
        let saved_us = std::mem::take(&mut self.union_syms);
        self.arm_union_bounds(&md.bounds, Some(dname));
        self.self_ty = Some(self_ty);
        self.current_class = Some(dname);
        self.ret_ty = ret_ty;
        let base = self.locals.len();
        // bind `self` (well-known symbol) for the inlined body
        self.locals.push(Local { name: sym::SELF, reg: recv, ty: self_ty, is_mut: mut_self, loop_var: false, origins: Vec::new(), field: NO_FIELD });
        self.inline_self = Some((sym::SELF, recv));
        let params: Vec<NodeHandle<AnyParam>> = md.params.clone();
        let mut ai = 0usize;
        for p in &params {
            if let MemberKind::Param(ParamData { name, is_mut, ty, .. }) = self.ctx.ast.param(*p) {
                self.locals.push(Local { name: *name, reg: aregs[ai], ty: ptys[ai], is_mut: *is_mut, loop_var: false, origins: Vec::new(), field: NO_FIELD });
                self.note_union_binding(*name, *ty);
                ai += 1;
            }
        }
        let res = self.new_reg(ret_ty);
        let l_end = self.new_label();
        self.inline_ret = Some((res, l_end));
        self.inline_stack.push((dname, mname));
        let ok = self.compile_block(body.id()).is_ok();
        self.inline_stack.pop();
        self.bind(l_end);
        self.locals.truncate(base);
        self.self_ty = saved_self_ty;
        self.subst = saved_subst;
        self.current_class = saved_class;
        self.ret_ty = saved_ret;
        self.inline_ret = saved_inline_ret;
        self.inline_self = saved_inline_self;
        self.union_bounds = saved_ub;
        self.union_syms = saved_us;
        let _ = sp;
        if !ok {
            return false;
        }
        self.last_reg = res;
        true
    }

    /// Inline a small, non-recursive free-fn body at the call site.
    /// Returns `true` when it compiled the body; `false` to emit `Call`.
    /// Params bind to the argument registers directly — `Op::Call` copies
    /// slots without deep-copying value args, so the inline keeps the
    /// exact aliasing behavior of the call it replaces. Trait-obj params
    /// bind without origins (vtable dispatch inside the body —
    /// conservative, like [`Self::try_inline_method`]'s params).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_inline_free_fn(
        &mut self,
        name: IdentId,
        fnode: NodeHandle<FnNode>,
        subst: &[(IdentId, TypeId)],
        aregs: &[u16],
        ptys: &[TypeId],
        ret_ty: TypeId,
        sp: rut_lexer::span::Span,
    ) -> bool {
        const MAX_STMTS: usize = 24;
        const MAX_DEPTH: usize = 4;
        // the inline stack keys (owner, method) — free fns have no owner
        if self.inline_stack.len() >= MAX_DEPTH || self.inline_stack.contains(&(name, name)) {
            return false;
        }
        // a trait-obj parameter loses its per-call origin specialization
        // in an inline (RFC 0012 §5: one clone per concrete argument,
        // each binding statically) — that boundary can't splice, so the
        // call stays a call
        if ptys.iter().any(|&t| matches!(self.ctx.types.kind(t), TyKind::TraitObj { .. })) {
            return false;
        }
        let fd = self.ctx.ast.fn_decl(fnode).clone();
        if fd.is_async {
            return false; // `async` is diagnosed when the body is compiled
        }
        let body = fd.body;
        let stmts = match self.ctx.ast.kind(body.id()) {
            Kind::Expr(ExprKind::Block { stmts }) => stmts.clone(),
            _ => return false,
        };
        if stmts.len() > MAX_STMTS {
            return false;
        }
        let saved_self_ty = self.self_ty;
        let saved_subst = std::mem::replace(&mut self.subst, subst.to_vec());
        let saved_class = self.current_class;
        let saved_ret = self.ret_ty;
        let saved_inline_ret = self.inline_ret;
        let saved_inline_self = self.inline_self;
        let saved_ub = std::mem::take(&mut self.union_bounds);
        let saved_us = std::mem::take(&mut self.union_syms);
        // the callee's union-spelled bounds gate the spliced body (RFC
        // 0043 §3) — a free fn has no class
        self.arm_union_bounds(&fd.bounds, None);
        self.self_ty = None;
        self.current_class = None;
        self.ret_ty = ret_ty;
        let base = self.locals.len();
        let params: Vec<NodeHandle<AnyParam>> = fd.params.clone();
        let mut ai = 0usize;
        for p in &params {
            if let MemberKind::Param(ParamData { name: pname, is_mut, ty, .. }) = self.ctx.ast.param(*p) {
                self.locals.push(Local { name: *pname, reg: aregs[ai], ty: ptys[ai], is_mut: *is_mut, loop_var: false, origins: Vec::new(), field: NO_FIELD });
                self.note_union_binding(*pname, *ty);
                ai += 1;
            }
        }
        let res = self.new_reg(ret_ty);
        let l_end = self.new_label();
        self.inline_ret = Some((res, l_end));
        self.inline_stack.push((name, name));
        let ok = self.compile_block(body.id()).is_ok();
        self.inline_stack.pop();
        self.bind(l_end);
        self.locals.truncate(base);
        self.self_ty = saved_self_ty;
        self.subst = saved_subst;
        self.current_class = saved_class;
        self.ret_ty = saved_ret;
        self.inline_ret = saved_inline_ret;
        self.inline_self = saved_inline_self;
        self.union_bounds = saved_ub;
        self.union_syms = saved_us;
        let _ = sp;
        if !ok {
            return false;
        }
        self.last_reg = res;
        true
    }

    /// Inline a small, non-recursive trait-impl method body at a
    /// bare-receiver static call site (P1.3). Mirrors
    /// [`Self::try_inline_method`]: `self` (and the `Self`-spelled
    /// params) bind to the concrete registers the call carries — the
    /// receiver register is already bare concrete at these sites.
    /// Returns `true` when it compiled the body; `false` to emit the
    /// concrete-variant `CallM`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_inline_impl_call(
        &mut self,
        impl_idx: usize,
        mnode: NodeHandle<MethodDeclNode>,
        mname: IdentId,
        self_ty: TypeId,
        subst: &[(IdentId, TypeId)],
        current_class: Option<IdentId>,
        recv: u16,
        aregs: &[u16],
        ptys: &[TypeId],
        ret_ty: TypeId,
        sp: rut_lexer::span::Span,
    ) -> bool {
        const MAX_STMTS: usize = 24;
        const MAX_DEPTH: usize = 4;
        // the inline stack keys (owner, method) — the trait names the
        // impl body (one impl per (trait, type) pair; two impls of one
        // trait share the key and just decline the second inline)
        let trait_name = self.ctx.impls[impl_idx].trait_name;
        if self.inline_stack.len() >= MAX_DEPTH || self.inline_stack.contains(&(trait_name, mname)) {
            return false;
        }
        // a trait-obj parameter loses its per-call origin specialization
        // in an inline (RFC 0012 §5) — that boundary can't splice
        if ptys.iter().any(|&t| matches!(self.ctx.types.kind(t), TyKind::TraitObj { .. })) {
            return false;
        }
        let md = self.ctx.ast.method_decl(mnode).clone();
        if md.is_async {
            return false; // `async` is diagnosed when the body is compiled
        }
        // a GENERIC method's body stays a call (v1): the spliced body's
        // nested host-val calls (the nmap `any` lanes) read their site
        // types from the caller's register file, and the splice's
        // parameter bindings lose the substitution's register types —
        // dispatch through the compiled Inst keeps every site type exact
        if !md.generics.is_empty() {
            return false;
        }
        let Some(body) = md.body else { return false };
        let stmts = match self.ctx.ast.kind(body.id()) {
            Kind::Expr(ExprKind::Block { stmts }) => stmts.clone(),
            _ => return false,
        };
        if stmts.len() > MAX_STMTS {
            return false;
        }
        let mut_self = matches!(
            md.params.first().map(|p| self.ctx.ast.param(*p)),
            Some(MemberKind::SelfParam(SelfParamData { is_mut: true }))
        );
        let saved_self_ty = self.self_ty;
        let saved_subst = std::mem::replace(&mut self.subst, subst.to_vec());
        let saved_class = self.current_class;
        let saved_ret = self.ret_ty;
        let saved_inline_ret = self.inline_ret;
        let saved_inline_self = self.inline_self;
        let saved_ub = std::mem::take(&mut self.union_bounds);
        let saved_us = std::mem::take(&mut self.union_syms);
        self.arm_union_bounds(&md.bounds, current_class);
        self.self_ty = Some(self_ty);
        self.current_class = current_class;
        self.ret_ty = ret_ty;
        let base = self.locals.len();
        // bind `self` (well-known symbol) for the inlined body — reads
        // alias the receiver register (no copy, RFC 0005 accessor rule)
        self.locals.push(Local { name: sym::SELF, reg: recv, ty: self_ty, is_mut: mut_self, loop_var: false, origins: Vec::new(), field: NO_FIELD });
        self.inline_self = Some((sym::SELF, recv));
        let params: Vec<NodeHandle<AnyParam>> = md.params.clone();
        let mut ai = 0usize;
        for p in &params {
            if let MemberKind::Param(ParamData { name: pname, is_mut, ty, .. }) = self.ctx.ast.param(*p) {
                self.locals.push(Local { name: *pname, reg: aregs[ai], ty: ptys[ai], is_mut: *is_mut, loop_var: false, origins: Vec::new(), field: NO_FIELD });
                self.note_union_binding(*pname, *ty);
                ai += 1;
            }
        }
        let res = self.new_reg(ret_ty);
        let l_end = self.new_label();
        self.inline_ret = Some((res, l_end));
        self.inline_stack.push((trait_name, mname));
        let ok = self.compile_block(body.id()).is_ok();
        self.inline_stack.pop();
        self.bind(l_end);
        self.locals.truncate(base);
        self.self_ty = saved_self_ty;
        self.subst = saved_subst;
        self.current_class = saved_class;
        self.ret_ty = saved_ret;
        self.inline_ret = saved_inline_ret;
        self.inline_self = saved_inline_self;
        self.union_bounds = saved_ub;
        self.union_syms = saved_us;
        let _ = sp;
        if !ok {
            return false;
        }
        self.last_reg = res;
        true
    }

}
