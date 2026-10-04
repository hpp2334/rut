//! Free-function calls and direct method emission.

use crate::check::TcResult;
use crate::lir::*;

impl<'a, 'b> FnCompiler<'a, 'b> {

    pub(crate) fn compile_free_fn_call(
        &mut self,
        name: IdentId,
        generics: Vec<NodeHandle<AnyTy>>,
        args: Vec<NodeHandle<AnyExpr>>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let Some(fnode) = self.ctx.fn_nodes.iter().find(|(n, _)| *n == name).map(|(_, n)| *n) else {
            self.ctx.err(sp, format!("unknown function `{}`", self.ctx.name(name)));
            return Err(());
        };
        let fd = self.ctx.ast.fn_decl(fnode).clone();
        // the async landing: an async fn's call site mints
        // the frame — the cx is engine-minted like `self`, the call's
        // value IS the frame cell (widening to `Future<T>` through the
        // registered impl)
        if fd.is_async {
            if !generics.is_empty() {
                self.ctx.err(sp, "generic async fns are not woven in this build");
                return Err(());
            }
            return crate::lir::asyncfn::compile_async_call(self, name, &fd, &args, expected, sp);
        }
        let (decl_generics, params, ret) = (fd.generics, fd.params, fd.ret);
        if generics.len() > decl_generics.len() {
            self.ctx.err(sp, format!(
                "`{}` takes {} generic arguments, {} given",
                self.ctx.name(name), decl_generics.len(), generics.len()
            ));
            return Err(());
        }
        // substitution: explicit args first, then inference from arguments
        let mut subst: Vec<(IdentId, TypeId)> = Vec::new();
        for (g, node) in decl_generics.iter().zip(generics.iter()) {
            let t = self.resolve_type_now(*node);
            subst.push((*g, t));
        }
        // compile args under best-effort expected types; unify generics
        let param_nodes: Vec<Option<NodeHandle<AnyTy>>> = params
            .iter()
            .map(|p| match self.ctx.ast.param(*p) {
                MemberKind::Param(ParamData { ty: Some(t), .. }) => Some(*t),
                _ => None,
            })
            .collect();
        if args.len() != params.len() {
            self.ctx.err(sp, format!("call arity: {} args for {} params", args.len(), params.len()));
            return Err(());
        }
        let mut arg_tys: Vec<TypeId> = Vec::new();
        let mut aregs: Vec<u16> = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let expected = match param_nodes[i] {
                Some(tn) if self.free_generics(tn, &decl_generics, &subst).is_empty() => {
                    Some(self.ctx.resolve_type(tn, &subst))
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
        for (i, _) in args.iter().enumerate() {
            if let Some(tn) = param_nodes[i] {
                self.unify_generic(tn, arg_tys[i], &decl_generics, &mut subst, sp)?;
            }
        }
        for g in &decl_generics {
            if !subst.iter().any(|(n, _)| n == g) {
                self.ctx.err(sp, format!(
                    "cannot infer generic parameter `{}` — annotate the call: `{}<..>(..)`",
                    self.ctx.name(*g), self.ctx.name(name)
                ));
                return Err(());
            }
        }
        // inline bounds gate the completed substitution (admission-only)
        self.ctx.admit_bounds(&fd.bounds, &subst, sp);
        // final param types under the completed substitution
        let ptys: Vec<TypeId> = params
            .iter()
            .map(|p| match self.ctx.ast.param(*p) {
                MemberKind::Param(ParamData { ty: Some(t), .. }) => self.ctx.resolve_type(*t, &subst),
                _ => TY_I32,
            })
            .collect();
        for (i, a) in args.iter().enumerate() {
            if !self.widens(arg_tys[i], ptys[i]) {
                self.ctx.err(self.ctx.ast.span(a.id()), format!(
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
        let ret_ty = ret.map(|r| self.ctx.resolve_type(r, &subst)).unwrap_or(TY_NIL);
        // a small, non-recursive body inlines at the call site (P1.3):
        // `mix64`-shaped helpers flatten into the caller, every hash pays
        // no frame. Runs after all checks, so diagnostics stay put.
        if self.try_inline_free_fn(name, fnode, &subst, &aregs, &ptys, ret_ty, sp) {
            return Ok(ret_ty);
        }
        let inst = crate::check::Inst {
            key: crate::check::FnKey::Free(name),
            subst,
            iface_origins,
        };
        let fid = self.ctx.ensure_inst(inst);
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        { let (argv_off, argc) = self.pool_args(&(aregs)); self.emit(Op::Call { func: fid, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
        Ok(ret_ty)
    }


    /// class-method call through the type name (`Circle.new(..)`)
    pub(crate) fn compile_direct_method(
        &mut self,
        dname: IdentId,
        class_args: Vec<TypeId>,
        self_ty: TypeId,
        mnode: NodeHandle<MethodDeclNode>,
        args: Vec<NodeHandle<AnyExpr>>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        // the decl name may be a class (generics substitute) or an
        // enum (concrete — the empty substitution)
        let class_subst: Vec<(IdentId, TypeId)> = match self.ctx.find_data(dname) {
            Some(d) => d.generics.iter().cloned().zip(class_args.iter().cloned()).collect(),
            None => vec![],
        };
        let md = self.ctx.ast.method_decl(mnode).clone();
        let (params, ret, mname) = (md.params, md.ret, md.name);
        // no-self first param (or no params at all) = class method
        if matches!(
            params.first().map(|p| self.ctx.ast.param(*p)),
            Some(MemberKind::SelfParam(_))
        ) {
            self.ctx.err(sp, "instance methods are called on a value, not the class");
            return Err(());
        }
        // inline bounds gate the completed substitution
        self.ctx.admit_bounds(&md.bounds, &class_subst, sp);
        let mut ptys = Vec::new();
        // the callee's signature may spell `Self`/`T` — resolve under the
        // CALLEE's class instantiation (the caller's context is irrelevant)
        let saved_self = self.self_ty;
        let saved_subst = std::mem::replace(&mut self.subst, class_subst.clone());
        self.self_ty = Some(self_ty);
        for p in &params {
            match self.ctx.ast.param(*p) {
                MemberKind::Param(ParamData { ty: Some(t), .. }) => ptys.push(self.resolve_type_now(*t)),
                _ => ptys.push(TY_I32),
            }
        }
        let ret_ty = ret.map(|r| self.resolve_type_now(r)).unwrap_or(TY_NIL);
        self.self_ty = saved_self;
        self.subst = saved_subst;
        if args.len() != ptys.len() {
            self.ctx.err(sp, format!(
                "call arity: `{}.{}` takes {} parameter{}, {} given",
                self.ctx.name(dname),
                self.ctx.name(mname),
                ptys.len(),
                if ptys.len() == 1 { "" } else { "s" },
                args.len()
            ));
            return Err(());
        }
        let mut aregs = Vec::new();
        let mut arg_tys = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let t = self.compile_expr(*a, Some(ptys[i]))?;
            if !self.widens(t, ptys[i]) {
                self.ctx.err(self.ctx.ast.span(a.id()), format!(
                    "argument {} is `{}`, `{}` expected",
                    i + 1, self.ctx.type_name(t), self.ctx.type_name(ptys[i])
                ));
            }
            aregs.push(self.last_reg);
            arg_tys.push(t);
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
        let inst = crate::check::Inst {
            key: crate::check::FnKey::Method { data: dname, name: mname },
            subst: class_subst,
            iface_origins,
        };
        let fid = self.ctx.ensure_inst(inst);
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        // class method: no receiver —plain Call
        { let (argv_off, argc) = self.pool_args(&(aregs)); self.emit(Op::Call { func: fid, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
        Ok(ret_ty)
    }

}
