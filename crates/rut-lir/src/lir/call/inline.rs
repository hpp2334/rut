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
        // the decl name may be a class (generics substitute) or an
        // enum (concrete — the empty substitution)
        let class_subst: Vec<(IdentId, TypeId)> = match self.ctx.find_data(dname) {
            Some(d) => d.generics.iter().cloned().zip(class_args.iter().cloned()).collect(),
            None => vec![],
        };
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
        let param_nodes: Vec<Option<NodeHandle<AnyTy>>> = params
            .iter()
            .skip(1)
            .map(|p| match self.ctx.ast.param(*p) {
                MemberKind::Param(ParamData { ty: Some(t), .. }) => Some(*t),
                _ => None,
            })
            .collect();
        // the unannotated-lambda diagnosis (v1): a lambda whose
        // parameters spell no types cannot drive the method's inference
        // — its body types never reach unification (the shape hint's
        // placeholders would leak into the result). Diagnose with the
        // fix instead of cascading mismatch errors.
        for (i, a) in args.iter().enumerate() {
            let Some(tn) = param_nodes[i] else { continue };
            let free = self.free_generics(tn, &decl_generics, &subst);
            if free.is_empty() {
                continue;
            }
            if let ExprKind::Lambda { params: lps, ret: lret, .. } = self.ctx.ast.expr(*a) {
                // FULLY unannotated: no parameter types and no return
                // type — nothing in the lambda's shape can bind the
                // generic (a partially annotated one — the store's
                // `lift(fn (c) -> i64 ..)` — still binds through the
                // annotated half)
                let fully_unannotated = lps.iter().all(|p| {
                    matches!(
                        self.ctx.ast.param(*p),
                        MemberKind::Param(ParamData { ty: None, .. })
                    )
                }) && lret.is_none();
                if fully_unannotated {
                    let free_names = free
                        .iter()
                        .map(|g| self.ctx.name(*g).to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.ctx.err(sp, format!(
                        "cannot infer `{}` of `{}.{}` from an unannotated lambda — annotate its parameters (`fn(x: T) ..`) or spell the type argument (`.{}<{}>(..)`)",
                        free_names,
                        self.ctx.name(dname),
                        self.ctx.name(mname),
                        self.ctx.name(mname),
                        free_names,
                    ));
                    return Err(());
                }
            }
        }
        // the ARGUMENTS' expected types (best-effort): a parameter node
        // still spelling an unbound generic (`a: Readable<T>`) gets no
        // hint — unification binds the generic from the argument;
        // otherwise the hint resolves under the CALLEE's instantiation
        // (the swap above) — and the shape hint (the phase-2 placeholder
        // env) rides fresh placeholder types for the unbound rest
        let hints: Vec<Option<TypeId>> = param_nodes
            .iter()
            .map(|tn| match tn {
                Some(t) if self.free_generics(*t, &decl_generics, &subst).is_empty() => {
                    Some(self.resolve_type_now(*t))
                }
                Some(t) => Some(self.hint_with_placeholders(*t, &decl_generics, &subst)),
                None => None,
            })
            .collect();
        // the arguments compile under the CALLER's substitution — a
        // lambda argument may spell the CALLER's generics
        // (`Vec.from_flow`'s `fn (x: T) ..` inside the sink's own
        // generic `T`), and the swap above must not leak into their
        // annotation resolution. The hints are type ids (callee-space);
        // no env rides them.
        self.subst = saved_subst.clone();
        let mut arg_tys: Vec<TypeId> = Vec::new();
        let mut aregs: Vec<u16> = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let expected = hints[i];
            let t = self.compile_expr(*a, expected)?;
            if let Some(e) = expected {
                self.widen_to_slot(t, e, sp.lo);
            }
            arg_tys.push(t);
            aregs.push(self.last_reg);
        }
        // re-arm the callee's instantiation for the unification and the
        // signature resolution below
        self.subst = subst.clone();
        // structural unification binds the method's remaining generics
        for (i, tn) in param_nodes.iter().enumerate() {
            if let Some(tn) = tn {
                self.unify_generic_val(*tn, Some(args[i]), arg_tys[i], &decl_generics, &mut subst, sp)?;
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
        // inline bounds gate the completed substitution
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
            if !self.widens_val(args[i], arg_tys[i], ptys[i]) {
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
        let mut iface_origins = Vec::new();
        for (i, _) in args.iter().enumerate() {
            if matches!(self.ctx.types.kind(ptys[i]), TyKind::IfaceObj { .. })
                && !matches!(self.ctx.types.kind(arg_tys[i]), TyKind::IfaceObj { .. })
            {
                iface_origins.push(arg_tys[i]);
            }
        }
        // small instance methods inline at the call site: the class's
        // `push`/`pop`/`freeze` are rut code, so an interpreted
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
            iface_origins,
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
        // in an inline (one clone per concrete argument,
        // each binding statically) — that boundary can't splice, so the
        // call stays a call
        if ptys.iter().any(|&t| matches!(self.ctx.types.kind(t), TyKind::IfaceObj { .. })) {
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
                self.locals.push(Local { name: *pname, reg: aregs[ai], ty: ptys[ai], is_mut: *is_mut, loop_var: false, origins: Vec::new(), field: NO_FIELD, cell: None });
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



}
