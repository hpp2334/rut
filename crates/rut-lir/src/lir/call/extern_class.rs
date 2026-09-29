//! Used-class calls (the linkable-classes phase): a consumer's
//! `Logger.new(..)` / `b.append(..)` against a class that compiled in
//! another package. A PLAIN class's method binds the exporter's
//! scope-qualified fn directly (the surface row carries it, link
//! rebases it — the extern-fn law). A GENERIC class's method mints a
//! MIRROR instantiation (`Ctx::mirror_inst`): the bodyless stub's
//! ledger row names the declaring package as owner, so link redirects
//! the call onto the owner's compiled copy — instantiation is owned by
//! the declaring package, consumers request.

use crate::check::TcResult;
use crate::lir::*;
use rut_core::sym;
use std::collections::HashMap;

impl<'a, 'b> FnCompiler<'a, 'b> {

    /// The signature of an extern inherent method under the receiver's
    /// instantiation: the row's carried types spell the exporter's
    /// table — placeholder leaves (`#<param>`, and `#Self` for a
    /// generic class's `Self`) substitute through the class's concrete
    /// arguments (the instantiated self row for `#Self`), everything
    /// else resolves as carried (the `use_types` blocks registered the
    /// exporter's rows here).
    fn extern_sig(
        &mut self,
        ih: usize,
        midx: usize,
        subst: &[(IdentId, TypeId)],
        self_ty: Option<TypeId>,
    ) -> (Vec<TypeId>, TypeId) {
        let mut env: HashMap<String, TypeId> = subst
            .iter()
            .map(|(g, t)| (format!("#{}", self.ctx.name(*g)), *t))
            .collect();
        if let Some(s) = self_ty {
            env.insert("#Self".to_string(), s);
        }
        let (_n, params, ret, _f, _has_self) = self.ctx.extern_inherents[ih].methods[midx].clone();
        let ptys = params
            .iter()
            .map(|&p| self.ctx.subst_template_ty(p, &env))
            .collect();
        let ret = self.ctx.subst_template_ty(ret, &env);
        (ptys, ret)
    }

    /// Instance-method call into a used class: the receiver register is
    /// compiled and bare; `subst` pairs the class's generic parameters
    /// with the receiver's concrete arguments. A plain class calls the
    /// exporter's fn; a generic class mints the mirror.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn compile_extern_method_call(
        &mut self,
        ih: usize,
        midx: usize,
        subst: &[(IdentId, TypeId)],
        dname: Option<IdentId>,
        rt: TypeId,
        rreg: u16,
        args: Vec<NodeHandle<AnyExpr>>,
        _expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let (name, _params, _ret, func, has_self) =
            self.ctx.extern_inherents[ih].methods[midx].clone();
        debug_assert!(has_self, "instance call routed a class method");
        // the receiver IS the instantiation: `#Self` substitutes to it
        let (ptys, ret_ty) = self.extern_sig(ih, midx, subst, Some(rt));
        if args.len() != ptys.len() {
            self.ctx.err(sp, format!("call arity: {} args for {} params", args.len(), ptys.len()));
            return Err(());
        }
        let mut aregs = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let t = self.compile_expr(*a, Some(ptys[i]))?;
            if !self.widens(t, ptys[i]) {
                self.ctx.err(self.ctx.ast.span(a.id()), format!(
                    "argument {} is `{}`, `{}` expected",
                    i + 1, self.ctx.type_name(t), self.ctx.type_name(ptys[i])
                ));
            }
            aregs.push(self.last_reg);
        }
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        let fid = match dname {
            // a plain class: the compiled fn crosses the surface
            None => func,
            // a generic class: the bodies live in the owner — mirror
            Some(d) => {
                let inst = crate::check::Inst {
                    key: crate::check::FnKey::Method { data: d, name },
                    subst: subst.to_vec(),
                    trait_origins: vec![],
                };
                self.ctx.mirror_inst(inst)
            }
        };
        { let (argv_off, argc) = self.pool_recv_args(rreg, &(aregs)); self.emit(Op::CallM { func: fid, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
        Ok(ret_ty)
    }

    /// A used GENERIC fn (`decodeJson<i64>(s)`): the type arguments
    /// resolve from the site's explicit list, else unify structurally
    /// against the placeholder signature (`#T` leaves bind; wrappers
    /// recurse); the mirror instantiation's ledger row names the owner,
    /// so link binds the call to the owner's monomorphized body.
    pub(crate) fn compile_extern_generic_fn_call(
        &mut self,
        name: IdentId,
        gf: &crate::check::ExternGenericFn,
        generics: Vec<NodeHandle<AnyTy>>,
        args: Vec<NodeHandle<AnyExpr>>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        if generics.len() > gf.params.len() {
            self.ctx.err(sp, format!(
                "`{}` takes {} type argument(s), {} given",
                self.ctx.name(name),
                gf.params.len(),
                generics.len()
            ));
            return Err(());
        }
        let mut env: HashMap<String, TypeId> = HashMap::new();
        for (p, g) in gf.params.iter().zip(generics.iter()) {
            env.insert(format!("#{}", self.ctx.name(*p)), self.resolve_type_now(*g));
        }
        // argument types under the (partial) substitution: bare
        // placeholder leaves take any concrete argument (the owner's
        // monomorphization re-checks the bounds); wrapper shapes
        // rebuild per element
        let ptys: Vec<TypeId> = gf
            .args
            .iter()
            .map(|&a| self.ctx.subst_template_ty(a, &env))
            .collect();
        if args.len() != ptys.len() {
            self.ctx.err(sp, format!("call arity: {} args for {} params", args.len(), ptys.len()));
            return Err(());
        }
        let is_placeholder = |c: &mut Self, p: TypeId| -> bool {
            let t = c.ctx.types.type_at(p).name;
            let t = c.ctx.interner.name(t).to_string();
            gf.params.iter().any(|g| format!("#{}", c.ctx.name(*g)) == t)
        };
        let mut aregs = Vec::new();
        let mut arg_tys = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let hint = if is_placeholder(self, gf.args[i]) { None } else { Some(ptys[i]) };
            let t = self.compile_expr(*a, hint)?;
            aregs.push(self.last_reg);
            arg_tys.push(t);
        }
        // structural inference binds the parameters: a bare `#X`
        // parameter binds its argument; a wrapper-shaped parameter
        // (`?T`, `[T]`) matches the argument's shape element-wise
        for (i, &param) in gf.args.iter().enumerate() {
            self.infer_placeholders(param, arg_tys[i], gf, &mut env, sp)?;
        }
        for p in &gf.params {
            let key = format!("#{}", self.ctx.name(*p));
            if !env.contains_key(&key) {
                self.ctx.err(sp, format!(
                    "cannot infer type parameter `{}` of `{}` — annotate the call: `{}<..>(..)`",
                    self.ctx.name(*p),
                    self.ctx.name(name),
                    self.ctx.name(name)
                ));
                return Err(());
            }
        }
        // final param types under the completed substitution
        let env2 = &env;
        let ptys: Vec<TypeId> = gf
            .args
            .iter()
            .map(|&a| self.ctx.subst_template_ty(a, env2))
            .collect();
        for (i, a) in args.iter().enumerate() {
            if !self.widens(arg_tys[i], ptys[i]) {
                self.ctx.err(self.ctx.ast.span(a.id()), format!(
                    "argument {} is `{}`, `{}` expected",
                    i + 1, self.ctx.type_name(arg_tys[i]), self.ctx.type_name(ptys[i])
                ));
            }
        }
        let ret = {
            let ret0 = gf.ret;
            self.ctx.subst_template_ty(ret0, &env)
        };
        let _ = expected;
        let subst: Vec<(IdentId, TypeId)> = gf
            .params
            .iter()
            .map(|p| (*p, env[&format!("#{}", self.ctx.name(*p))]))
            .collect();
        let inst = crate::check::Inst {
            key: crate::check::FnKey::Free(name),
            subst,
            trait_origins: vec![],
        };
        let fid = self.ctx.mirror_inst(inst);
        let dst = if ret == TY_NIL { None } else { Some(self.new_reg(ret)) };
        { let (argv_off, argc) = self.pool_args(&(aregs)); self.emit(Op::Call { func: fid, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
        Ok(ret)
    }

    /// One parameter's structural placeholder inference: `#X` binds
    /// `X` to the argument; matching wrappers recurse; ground types
    /// require equality (checked at the widening above).
    fn infer_placeholders(
        &mut self,
        param: TypeId,
        arg: TypeId,
        gf: &crate::check::ExternGenericFn,
        env: &mut HashMap<String, TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<()> {
        let ptext = self.ctx.types.type_at(param).name;
        let ptext = self.ctx.interner.name(ptext).to_string();
        if gf.params.iter().any(|p| format!("#{}", self.ctx.name(*p)) == ptext) {
            if let Some(_old) = env.insert(ptext, arg) {
                // already bound — the widening above checked agreement
            }
            return Ok(());
        }
        match (self.ctx.types.kind(param).clone(), self.ctx.types.kind(arg).clone()) {
            (TyKind::Opt { elem: pe }, TyKind::Opt { elem: ae }) => {
                self.infer_placeholders(pe, ae, gf, env, sp)
            }
            (TyKind::Array { elem: pe }, TyKind::Array { elem: ae }) => {
                self.infer_placeholders(pe, ae, gf, env, sp)
            }
            (TyKind::Weak { elem: pe }, TyKind::Weak { elem: ae }) => {
                self.infer_placeholders(pe, ae, gf, env, sp)
            }
            _ => Ok(()),
        }
    }

    /// Class-method call through a used class's name
    /// (`Logger.new(..)`, `Vec<T>.new()`): the same law as
    /// [`Self::compile_extern_method_call`], no receiver — a plain
    /// `Call`. `class_args` carries the generic class's resolved
    /// arguments (empty for a plain class).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn compile_extern_class_method_call(
        &mut self,
        ih: usize,
        midx: usize,
        dname: Option<IdentId>,
        class_args: Vec<TypeId>,
        self_ty: Option<TypeId>,
        args: Vec<NodeHandle<AnyExpr>>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let (name, _params, _ret, func, has_self) =
            self.ctx.extern_inherents[ih].methods[midx].clone();
        debug_assert!(!has_self, "class call routed an instance method");
        let subst: Vec<(IdentId, TypeId)> = match dname {
            Some(d) => match self.ctx.extern_generics.get(&d) {
                Some(g) => g.params.iter().cloned().zip(class_args.iter().cloned()).collect(),
                None => vec![],
            },
            None => vec![],
        };
        let (ptys, ret_ty) = self.extern_sig(ih, midx, &subst, self_ty);
        if args.len() != ptys.len() {
            self.ctx.err(sp, format!("call arity: {} args for {} params", args.len(), ptys.len()));
            return Err(());
        }
        let mut aregs = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let t = self.compile_expr(*a, Some(ptys[i]))?;
            if !self.widens(t, ptys[i]) {
                self.ctx.err(self.ctx.ast.span(a.id()), format!(
                    "argument {} is `{}`, `{}` expected",
                    i + 1, self.ctx.type_name(t), self.ctx.type_name(ptys[i])
                ));
            }
            aregs.push(self.last_reg);
        }
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        let fid = match dname {
            None => func,
            Some(d) => {
                let inst = crate::check::Inst {
                    key: crate::check::FnKey::Method { data: d, name },
                    subst,
                    trait_origins: vec![],
                };
                self.ctx.mirror_inst(inst)
            }
        };
        { let (argv_off, argc) = self.pool_args(&(aregs)); self.emit(Op::Call { func: fid, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
        Ok(ret_ty)
    }

    /// A used generic-target trait impl's method
    /// (`impl JsonSerialize for Vec<T>`, compiled in json): the
    /// concrete instantiation's body lives in the impl's owner, so the
    /// site mints the mirror impl-method request — the stub's ledger
    /// row keys the instantiation canonically (the same spelling the
    /// owner's compiled row carries), and link binds the call to the
    /// owner's fn. The trait's declared signature (registered from the
    /// surface) types the call; the receiver crosses bare (a
    /// reference-repr target's ABIs coincide, RFC 0012 §5).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn compile_extern_impl_template_call(
        &mut self,
        eidx: usize,
        midx: usize,
        concrete: TypeId,
        subst: Vec<TypeId>,
        rreg: u16,
        args: Vec<NodeHandle<AnyExpr>>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let im = self.ctx.extern_impls[eidx].clone();
        let tdesc = self.ctx.trait_by_id(im.trait_id).clone();
        let tm = tdesc.methods[midx].clone();
        // the trait method must be instance-shaped here: a template
        // impl's no-self method is a type-parameter call, routed by the
        // trait-param arm instead
        let (ptys, ret_ty): (Vec<TypeId>, TypeId) = {
            let mut ps = Vec::new();
            for p in tm.params.iter().skip(1) {
                ps.push(match self.ctx.types.kind(*p) {
                    TyKind::TraitObj { trait_id: t } if *t == im.trait_id => concrete,
                    _ => *p,
                });
            }
            (ps, tm.ret)
        };
        let _ = expected;
        if args.len() != ptys.len() {
            self.ctx.err(sp, format!("call arity: {} args for {} params", args.len(), ptys.len()));
            return Err(());
        }
        let mut aregs = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let t = self.compile_expr(*a, Some(ptys[i]))?;
            if !self.widens(t, ptys[i]) {
                self.ctx.err(self.ctx.ast.span(a.id()), format!(
                    "argument {} is `{}`, `{}` expected",
                    i + 1, self.ctx.type_name(t), self.ctx.type_name(ptys[i])
                ));
            }
            aregs.push(self.last_reg);
        }
        // the mirror: owner = the trait's declaring pkg (the impl is
        // its to compile), target = the receiver's mirror row (its
        // ledger row keys the instantiation), subst = the class's
        // concrete arguments
        let owner = self
            .ctx
            .extern_origins
            .get(&im.trait_name)
            .cloned()
            .unwrap_or_else(|| self.ctx.own_spec.clone());
        let fid = self
            .ctx
            .mirror_impl_method(owner.clone(), im.trait_name, concrete, tm.name, subst);
        // the body request: the owner's unit mints the template impl at
        // this concrete target and compiles the method
        self.ctx.request_inst_impl_method(owner, im.trait_name, concrete, tm.name);
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        { let (argv_off, argc) = self.pool_recv_args(rreg, &(aregs)); self.emit(Op::CallM { func: fid, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
        Ok(ret_ty)
    }
}
