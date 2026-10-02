//! Used-class calls (the linkable-classes phase): a consumer's
//! `Logger.new(..)` / `b.append(..)` against a class that compiled in
//! another package. A PLAIN class's method binds the exporter's
//! scope-qualified fn directly (the surface row carries it, link
//! rebases it — the extern-fn law). A GENERIC class's method mints a
//! MIRROR instantiation (`Ctx::mirror_inst`): the bodyless stub's
//! ledger row names the declaring package as owner, so link redirects
//! the call onto the owner's compiled copy — instantiation is owned by
//! the declaring package, consumers request.

use crate::check::{collect::split_top_commas, TcResult};
use crate::lir::*;
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
        let m = self.ctx.extern_inherents[ih].methods[midx].clone();
        let (params, ret) = (m.params, m.ret);
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
    #[allow(clippy::too_many_arguments)]
    /// The template substitution env for an extern template impl's
    /// descriptor: the method signature's `#<name>` leaves are the
    /// trait's own generics — the k-th DISTINCT leaf (first-appearance
    /// order in the signature's display text) binds to the target's
    /// k-th class argument (the v1 template law: the trait arguments
    /// ARE the target's parameters — `impl<E> IntoFlow<E> for Vec<E>`
    /// over `Vec<i32>` spells `#E := i32`).
    pub(crate) fn descriptor_leaf_env(
        &self,
        tm: &rut_core::binary::TraitMethod,
        trait_id: u32,
        class_args: &[TypeId],
    ) -> HashMap<String, TypeId> {
        let mut sig = String::new();
        for &p in &tm.params {
            // a TraitObj param of THIS trait is the receiver slot
            if let TyKind::TraitObj { trait_id: t } = self.ctx.types.kind(p) {
                if *t == trait_id {
                    continue;
                }
            }
            sig.push(' ');
            sig.push_str(&self.ctx.type_name(p));
        }
        sig.push_str(" -> ");
        sig.push_str(&self.ctx.type_name(tm.ret));
        let mut leaves: Vec<String> = Vec::new();
        for piece in sig.split(|c: char| !c.is_alphanumeric() && c != '#') {
            let piece = piece.trim();
            if piece.len() > 1 && piece.starts_with('#') && !leaves.iter().any(|l| l == piece) {
                leaves.push(piece.to_string());
            }
        }
        let mut env: HashMap<String, TypeId> = HashMap::new();
        for (i, leaf) in leaves.iter().enumerate() {
            if let Some(&ca) = class_args.get(i) {
                env.insert(leaf.clone(), ca);
            }
        }
        env
    }

    /// Force the template re-spell of a crossed descriptor row whose
    /// name spells `Base<#leaf, ..>` (or a bare `#leaf`): the shared
    /// `subst_template_ty` declines rows with empty field lists (a
    /// template instantiation carried without its layout), but the
    /// SIGNATURE the call sites need is the name-text shape — re-mint
    /// from the base's extern-generic entry with the env-bound leaves.
    fn respell_template_row(&mut self, id: TypeId, env: &HashMap<String, TypeId>) -> TypeId {
        let text = {
            let t = self.ctx.types.type_at(id);
            self.ctx.interner.name(t.name).to_string()
        };
        if !text.contains('#') {
            return id;
        }
        if let Some(&arg) = env.get(&text) {
            return arg;
        }
        // (the `[trait] Iterable<#leaf>` respell lane is GONE with the
        // trait — v20's `[iterable]` marker crosses on the inherent
        // method row, and no parameter names the contract anymore)
        let Some((base_text, rest)) = text.split_once('<') else {
            return id;
        };
        let Some(args_text) = rest.strip_suffix('>') else {
            return id;
        };
        let base_id = self.ctx.intern(base_text);
        let Some(g) = self.ctx.extern_generics.get(&base_id).cloned() else {
            return id;
        };
        let arg_texts = crate::check::collect::split_top_commas(args_text);
        if arg_texts.len() != g.params.len() {
            return id;
        }
        let mut args = Vec::with_capacity(arg_texts.len());
        for a in &arg_texts {
            if let Some(&t) = env.get(a.as_str()) {
                args.push(t);
                continue;
            }
            let aid = self.ctx.intern(a);
            match self.ctx.types.dense_id_of_name(aid) {
                Some(t) => args.push(t),
                None => return id,
            }
        }
        self.ctx.mk_extern_data_inst(base_id, &g, args)
    }

    pub(crate) fn compile_extern_method_call(
        &mut self,
        ih: usize,
        midx: usize,
        subst: &[(IdentId, TypeId)],
        dname: Option<IdentId>,
        rt: TypeId,
        rreg: u16,
        generics: Vec<NodeHandle<AnyTy>>,
        args: Vec<NodeHandle<AnyExpr>>,
        _expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let m = self.ctx.extern_inherents[ih].methods[midx].clone();
        let (name, func, has_self, mgen) = (m.name, m.local, m.has_self, m.generics);
        debug_assert!(has_self, "instance call routed a class method");
        // a generic METHOD's own parameters join the class's
        // substitution: explicit site arguments first (`source<i64>`),
        // else structural unification against the placeholder signature
        let mut full_subst = subst.to_vec();
        if !mgen.is_empty() && !generics.is_empty() {
            if generics.len() != mgen.len() {
                self.ctx.err(sp, format!(
                    "`{}` takes {} type argument(s), {} given",
                    self.ctx.name(name),
                    mgen.len(),
                    generics.len()
                ));
                return Err(());
            }
            for (g, node) in mgen.iter().zip(generics.iter()) {
                full_subst.push((*g, self.resolve_type_now(*node)));
            }
        }
        if !mgen.is_empty() && generics.is_empty() {
            // the unannotated-lambda diagnosis (v1): a fully unannotated
            // lambda cannot drive the placeholder signature's unification
            // (its shape hint's `#leaf` placeholders would leak into the
            // checks) — diagnose with the fix, mirroring the local inline
            // path's gate
            let mparams = self.ctx.extern_inherents[ih].methods[midx].params.clone();
            for (i, a) in args.iter().enumerate() {
                let mentions_leaf = mparams.get(i).map_or(false, |&p| {
                    self.ctx.type_name(p).to_string().contains('#')
                });
                if !mentions_leaf {
                    continue;
                }
                if let ExprKind::Lambda { params: lps, ret: lret, .. } = self.ctx.ast.expr(*a) {
                    let fully_unannotated = lps.iter().all(|p| {
                        matches!(
                            self.ctx.ast.param(*p),
                            MemberKind::Param(ParamData { ty: None, .. })
                        )
                    }) && lret.is_none();
                    if fully_unannotated {
                        let free_names = mgen
                            .iter()
                            .map(|g| self.ctx.name(*g).to_string())
                            .collect::<Vec<_>>()
                            .join(", ");
                        let recv = dname
                            .map(|d| self.ctx.name(d).to_string())
                            .unwrap_or_else(|| self.ctx.type_name(rt).to_string());
                        self.ctx.err(sp, format!(
                            "cannot infer `{}` of `{}.{}` from an unannotated lambda — annotate its parameters (`fn(x: T) ..`) or spell the type argument (`.{}<{}>(..)`)",
                            free_names, recv, self.ctx.name(name), self.ctx.name(name), free_names,
                        ));
                        return Err(());
                    }
                }
            }
            // inference: compile args against the placeholder signature,
            // binding the method's generics from the arguments
            let env0: HashMap<String, TypeId> = full_subst
                .iter()
                .map(|(g, t)| (format!("#{}", self.ctx.name(*g)), *t))
                .collect();
            let mparams = self.ctx.extern_inherents[ih].methods[midx].params.clone();
            let ptys0: Vec<TypeId> = mparams
                .iter()
                .map(|&p| self.ctx.subst_template_ty(p, &env0))
                .collect();
            let mut arg_tys = Vec::new();
            for (i, a) in args.iter().enumerate() {
                let hint = if self.ctx.template_leaf(ptys0[i], &mgen) { None } else { Some(ptys0[i]) };
                let t = self.compile_expr(*a, hint)?;
                arg_tys.push(t);
            }
            for (i, a) in args.iter().enumerate() {
                let mut env: HashMap<String, TypeId> = full_subst
                    .iter()
                    .map(|(g, t)| (format!("#{}", self.ctx.name(*g)), *t))
                    .collect();
                self.infer_named_placeholders(ptys0[i], arg_tys[i], &mgen, &mut env, sp)?;
                // the merge walks the METHOD's parameter list, not the
                // env's hash order — the subst vec's argument ORDER is
                // positional downstream (a mirror request's args read
                // it index-for-index), so it must be deterministic
                for g in &mgen {
                    let key = format!("#{}", self.ctx.name(*g));
                    if let Some(&t) = env.get(&key) {
                        if !full_subst.iter().any(|(n, _)| n == g) {
                            full_subst.push((*g, t));
                        }
                    }
                }
            }
            for g in &mgen {
                if !full_subst.iter().any(|(n, _)| n == g) {
                    self.ctx.err(sp, format!(
                        "cannot infer generic parameter `{}` of `{}` — annotate the call",
                        self.ctx.name(*g),
                        self.ctx.name(name)
                    ));
                    return Err(());
                }
            }
        }
        // the receiver IS the instantiation: `#Self` substitutes to it;
        // the method's own generics substitute through the same env
        let (ptys, ret_ty) = self.extern_sig(ih, midx, &full_subst, Some(rt));
        if args.len() != ptys.len() {
            self.ctx.err(sp, format!("call arity: {} args for {} params", args.len(), ptys.len()));
            return Err(());
        }
        // the method's own generics are in scope for nested calls
        // (`Vec.new()` inside `source<Vec<Todo>>(Vec.new())` sees
        // T → Vec<Todo> through self.subst)
        let saved_subst = std::mem::replace(&mut self.subst, full_subst.clone());
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
        self.subst = saved_subst;
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        // the mirror request: a generic class's method OR a generic
        // method on a plain class — the bodies live in the owner
        let needs_mirror = dname.is_some() || !mgen.is_empty();
        let fid = if needs_mirror {
            let data = dname.unwrap_or_else(|| self.ctx.types.type_at(rt).name);
            let inst = crate::check::Inst {
                key: crate::check::FnKey::Method { data, name },
                subst: full_subst.iter().map(|(g, t)| (*g, *t)).collect(),
                trait_origins: vec![],
            };
            self.ctx.mirror_inst(inst)
        } else {
            func
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
            // the hint respects an EXPLICIT type argument: a placeholder
            // the site already bound is concrete here, so the argument
            // compiles at that type (an integer literal inside `[..]`
            // must not fall back to the i32 default and overwrite the
            // explicit binding at inference)
            let hinted = self.ctx.subst_template_ty(gf.args[i], &env);
            let hint = if is_placeholder(self, hinted) { None } else { Some(hinted) };
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
        let names: Vec<IdentId> = gf.params.clone();
        self.infer_named_placeholders(param, arg, &names, env, sp)
    }

    /// [`Self::infer_placeholders`] against a method's OWN generic
    /// parameter names (the extern method rows carry no
    /// [`crate::check::ExternGenericFn`]). `#X` binds `X` to the
    /// argument; matching wrappers recurse; ground types require
    /// equality (checked at the widening above). An explicit binding
    /// already in `env` wins: inference only fills what the site left
    /// open.
    fn infer_named_placeholders(
        &mut self,
        param: TypeId,
        arg: TypeId,
        names: &[IdentId],
        env: &mut HashMap<String, TypeId>,
        _sp: rut_lexer::span::Span,
    ) -> TcResult<()> {
        let ptext = self.ctx.types.type_at(param).name;
        let ptext = self.ctx.interner.name(ptext).to_string();
        if names.iter().any(|p| format!("#{}", self.ctx.name(*p)) == ptext) {
            if !env.contains_key(&ptext) {
                env.insert(ptext, arg);
            }
            return Ok(());
        }
        match (self.ctx.types.kind(param).clone(), self.ctx.types.kind(arg).clone()) {
            (TyKind::Opt { elem: pe }, TyKind::Opt { elem: ae }) => {
                self.infer_named_placeholders(pe, ae, names, env, _sp)
            }
            (TyKind::Array { elem: pe }, TyKind::Array { elem: ae }) => {
                self.infer_named_placeholders(pe, ae, names, env, _sp)
            }
            (TyKind::Weak { elem: pe }, TyKind::Weak { elem: ae }) => {
                self.infer_named_placeholders(pe, ae, names, env, _sp)
            }
            // the closed `Future<T>` handle (v20): the element binds
            // element-wise — `launch_future<T>(f: Future<T>)` over a
            // `Future<nil>` argument binds `T := nil`
            (TyKind::Future { elem: pe }, TyKind::Future { elem: ae }) => {
                self.infer_named_placeholders(pe, ae, names, env, _sp)
            }
            (
                TyKind::Fn { params: pp, ret: pr },
                TyKind::Fn { params: ap, ret: ar },
            ) => {
                if pp.len() == ap.len() {
                    for (p, a) in pp.iter().zip(ap.iter()) {
                        self.infer_named_placeholders(*p, *a, names, env, _sp)?;
                    }
                    self.infer_named_placeholders(pr, ar, names, env, _sp)
                } else {
                    Ok(())
                }
            }
            (TyKind::TraitObj { trait_id: _ }, TyKind::Data { .. }) => {
                // a trait-parameter slot (`a: Readable<T>`) against a
                // concrete record: the TypeObj row's own name spells the
                // trait and its arguments ("[trait] Readable<#T>") — the
                // only carrier across a binding (the trait-table index
                // is unit-local and may not resolve in the consumer).
                // The args unify against the target's registered
                // parameterized impl's trait arguments — the impl may
                // live in the OWNING package, crossing as an extern row
                let row_text =
                    self.ctx.name(self.ctx.types.type_at(param).name).to_string();
                let inner = row_text.strip_prefix("[trait] ").unwrap_or(&row_text[..]);
                let Some((tbase, targs_text)) = inner.split_once('<') else {
                    return Ok(());
                };
                let Some(targs_text) = targs_text.strip_suffix('>') else {
                    return Ok(());
                };
                let tname = self.ctx.intern(tbase);
                let arity = split_top_commas(targs_text).len();
                let local_args = self.ctx.template_trait_args(tname, arity, arg);
                let concrete_args = local_args.or_else(|| {
                    // the impl row's carried trait args spell the
                    // TARGET's own generic parameters (`impl
                    // Writable<A, R> for Source<A>` carries `#A, #A`);
                    // each maps through the target's instantiation to
                    // the concrete value (`Source<Vec<Req>>` →
                    // `Writable<Vec<Req>, Vec<Req>>`)
                    let Some((d, cargs)) = self.ctx.inst_data.get(&arg).cloned() else {
                        return None;
                    };
                    let (g_params, im_targs) = {
                        let Some(g) = self.ctx.extern_generics.get(&d) else {
                            return None;
                        };
                        let Some(im) = self.ctx.extern_impls.iter().find(|im| {
                            self.ctx.name(im.trait_name) == tbase && g.template == im.target
                        }) else {
                            return None;
                        };
                        (g.params.clone(), im.trait_args.clone())
                    };
                    if im_targs.len() != arity {
                        return None;
                    }
                    let mut concrete = cargs.clone();
                    concrete.resize(arity, concrete[0]);
                    for (i, &ta) in im_targs.iter().enumerate() {
                        let ta_text =
                            self.ctx.name(self.ctx.types.type_at(ta).name).to_string();
                        let Some(tparam) = ta_text.strip_prefix('#') else { continue };
                        let tid = self.ctx.intern(tparam);
                        if let Some(pos) = g_params.iter().position(|&p| p == tid) {
                            if pos < cargs.len() {
                                concrete[i] = cargs[pos];
                            }
                        }
                    }
                    Some(concrete)
                });
                let Some(concrete_args) = concrete_args else {
                    return Ok(());
                };
                let param_args = split_top_commas(targs_text);
                if param_args.len() != concrete_args.len() {
                    return Ok(());
                }
                for (pa, ca) in param_args.iter().zip(concrete_args.iter()) {
                    let Some(prow) = self.ctx.resolve_elem_text(pa) else {
                        continue;
                    };
                    self.infer_named_placeholders(prow, *ca, names, env, _sp)?;
                }
                Ok(())
            }
            (TyKind::TraitObj { .. }, TyKind::TraitObj { .. }) => {
                // the Future protocol's element rides the trait-inst
                // name (its only carrier — the kind holds a unit-local
                // trait-table index): decode both sides and recurse
                if let (Some(pe), Some(ae)) =
                    (self.ctx.future_elem(param), self.ctx.future_elem(arg))
                {
                    return self.infer_named_placeholders(pe, ae, names, env, _sp);
                }
                Ok(())
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
        let m = self.ctx.extern_inherents[ih].methods[midx].clone();
        let (name, func, has_self) = (m.name, m.local, m.has_self);
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
    /// reference-repr target's ABIs coincide).
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
        // trait-param arm instead. The binary desc's params EXCLUDE the
        // receiver (the compiler passes self as arg0) — a `Self`-spelled
        // remaining parameter maps to the concrete target. The
        // descriptor's OTHER leaves spell the impl head's trait
        // arguments as `#<param>` rows (`into_flow(self) -> Flow<#E>`)
        // — positionally the target's own parameters (the v1 template
        // law), so the target substitution re-spells them (`Flow<i32>`).
        // the descriptor's leaves spell the impl head's trait arguments
        // as `#<param>` rows (`into_flow(self) -> Flow<#E>`) —
        // positionally the target's own parameters (the v1 template
        // law), so the target substitution re-spells them (`Flow<i32>`)
        let class_args = self
            .ctx
            .inst_data
            .get(&concrete)
            .cloned()
            .map(|(_, a)| a)
            .unwrap_or_else(|| match self.ctx.types.kind(concrete).clone() {
                TyKind::Opt { elem } | TyKind::Array { elem } => vec![elem],
                _ => vec![],
            });
        // the descriptor's leaves spell the trait's own generics as
        // `#<name>` rows (`into_flow(self) -> Flow<#E>`) — the k-th
        // DISTINCT leaf, in first-appearance order, is the trait's k-th
        // generic, and the v1 template law makes that the target's k-th
        // class argument (`impl<E> IntoFlow<E> for Vec<E>` over
        // `Vec<i32>`: `#E := i32`)
        let env = self.descriptor_leaf_env(&tm, im.trait_id, &class_args);
        let (ptys, ret_ty): (Vec<TypeId>, TypeId) = {
            let mut ps = Vec::new();
            for p in tm.params.iter() {
                ps.push(match self.ctx.types.kind(*p) {
                    TyKind::TraitObj { trait_id: t } if *t == im.trait_id => concrete,
                    _ => {
                        let sub = self.ctx.subst_template_ty(*p, &env);
                        self.respell_template_row(sub, &env)
                    }
                });
            }
            let sub = self.ctx.subst_template_ty(tm.ret, &env);
            let ret = self.respell_template_row(sub, &env);
            (ps, ret)
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
        // the mirror: owner = the ROW's mint anchor when it carries one
        // (the consumer-registered foreign-trait rows — the impl's home
        // compiles the bodies), else the trait's declaring pkg (the
        // impl is its to compile), else this unit
        let owner = im
            .origin
            .clone()
            .or_else(|| self.ctx.extern_origins.get(&im.trait_name).cloned())
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

    /// The no-self twin of `compile_extern_impl_template_call`: a trait
    /// impl's no-self static called through the TYPE name across a
    /// package boundary — `Vec.from_flow(it)` against flow's mounted
    /// group. The mirror request is the same (the owner mints the
    /// template impl at the concrete target); the ABI carries no
    /// receiver slot, so the call is a plain `Call` over the args.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn compile_extern_impl_template_static(
        &mut self,
        eidx: usize,
        midx: usize,
        concrete: TypeId,
        subst: Vec<TypeId>,
        args: Vec<NodeHandle<AnyExpr>>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let im = self.ctx.extern_impls[eidx].clone();
        let tdesc = self.ctx.trait_by_id(im.trait_id).clone();
        let tm = tdesc.methods[midx].clone();
        // the trait method must be no-self here (the receiver-shaped
        // form routes through the template call above); the
        // descriptor's `#<param>` leaves re-spell under the target
        // substitution (`from_flow(it: Iterable<#T>) -> Vec<#T>` over
        // `Vec<i32>` answers `Iterable<i32>` / `Vec<i32>`)
        let class_args = self
            .ctx
            .inst_data
            .get(&concrete)
            .cloned()
            .map(|(_, a)| a)
            .unwrap_or_else(|| match self.ctx.types.kind(concrete).clone() {
                TyKind::Opt { elem } | TyKind::Array { elem } => vec![elem],
                _ => vec![],
            });
        // the descriptor leaves → the target's class arguments (the
        // same law the receiver route runs)
        let env = self.descriptor_leaf_env(&tm, im.trait_id, &class_args);
        let (ptys, ret_ty): (Vec<TypeId>, TypeId) = {
            let mut ps = Vec::new();
            for p in tm.params.iter() {
                ps.push(match self.ctx.types.kind(*p) {
                    TyKind::TraitObj { trait_id: t } if *t == im.trait_id => concrete,
                    _ => {
                        let sub = self.ctx.subst_template_ty(*p, &env);
                        self.respell_template_row(sub, &env)
                    }
                });
            }
            let sub = self.ctx.subst_template_ty(tm.ret, &env);
            // `fn from_flow(..) -> Self` — the descriptor spells `Self`
            // as this trait's object; the impl's concrete target answers it
            let ret = match self.ctx.types.kind(sub) {
                TyKind::TraitObj { trait_id: t } if *t == im.trait_id => concrete,
                _ => self.respell_template_row(sub, &env),
            };
            (ps, ret)
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
                    i + 1,
                    self.ctx.type_name(t),
                    self.ctx.type_name(ptys[i])
                ));
            }
            aregs.push(self.last_reg);
        }
        // the same owner law the receiver route runs: the row's mint
        // anchor first (the consumer-registered rows), the trait's
        // declaring pkg otherwise
        let owner = im
            .origin
            .clone()
            .or_else(|| self.ctx.extern_origins.get(&im.trait_name).cloned())
            .unwrap_or_else(|| self.ctx.own_spec.clone());
        let fid = self
            .ctx
            .mirror_impl_method(owner.clone(), im.trait_name, concrete, tm.name, subst);
        self.ctx.request_inst_impl_method(owner, im.trait_name, concrete, tm.name);
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        { let (argv_off, argc) = self.pool_args(&(aregs)); self.emit(Op::Call { func: fid, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
        Ok(ret_ty)
    }
}
