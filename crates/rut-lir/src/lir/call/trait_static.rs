//! Static trait dispatch: calls through registered impls, generic-param and extern-trait impls, native statics, and the vtable-slot call finisher.

use crate::check::TcResult;
use crate::lir::*;

impl<'a, 'b> FnCompiler<'a, 'b> {

    /// Static dispatch of a trait method through a registered impl: the
    /// receiver's concrete type names the impl, so the call binds to the
    /// impl method directly (`CallM`) — no vtable hop (RFC 0012 §5).
    /// Arguments type against the trait's declared signature (the impl's
    /// was checked to match at collection).
    ///
    /// The `slot_abi` switch picks the callee variant (P1.1/P1.2): a
    /// trait-object receiver (box already materialized) calls the SLOT
    /// variant — the trait's declared signature crosses, scalars arrive
    /// boxed and the prologue unboxes; a BARE concrete receiver calls the
    /// CONCRETE variant — the receiver stays raw, `Self`-spelled params
    /// cross as the concrete type, no box is minted. At concrete sites a
    /// small body inlines at the call site (P1.3).    #[allow(clippy::too_many_arguments)]
    pub(crate) fn compile_trait_static_call(
        &mut self,
        impl_idx: usize,
        midx: usize,
        concrete: TypeId,
        rreg: u16,
        args: Vec<NodeHandle<AnyExpr>>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
        slot_abi: bool,
    ) -> TcResult<TypeId> {
        let im = self.ctx.impls[impl_idx].clone();
        let tdesc = self.ctx.trait_by_id(im.trait_id).clone();
        let tm = tdesc.methods[midx].clone();
        // generic target: this instantiation's substitution
        let subst: Vec<(IdentId, TypeId)> = match &im.target_data {
            Some((_, params)) => match self.ctx.inst_data.get(&concrete).cloned() {
                Some((_, cargs)) => params.iter().cloned().zip(cargs.into_iter()).collect(),
                // structural template targets: the element rides the
                // receiver's own shape (`?elem` / `[elem]` — the rut-json
                // batch phase 1's `impl I for ?T` / `[T]` dispatch)
                None => match (self.ctx.types.kind(concrete), params.len()) {
                    (TyKind::Opt { elem }, 1) => vec![(params[0], *elem)],
                    (TyKind::Array { elem }, 1) => vec![(params[0], *elem)],
                    _ => vec![],
                },
            },
            None => vec![],
        };
        // inline bounds gate the completed substitution (RFC 0043):
        // the impl method's own bounds under the target substitution
        if let Some((_, mnode)) = im.methods.iter().find(|(n, _)| *n == tm.name) {
            let bounds = self.ctx.ast.method_decl(*mnode).bounds.clone();
            self.ctx.admit_bounds(&bounds, &subst, sp);
        }
        // the impl method's own AST (present for local impls)
        let mnode = im.methods.iter().find(|(n, _)| *n == tm.name).map(|(_, n)| *n);
        // param/ret types for the chosen ABI: the slot ABI crosses the
        // trait's declared signature; the concrete ABI crosses the impl
        // method's own signature resolved under the target substitution
        // (`Self` spells the concrete target)
        let (ptys, ret_ty) = if slot_abi {
            (tm.params.clone(), tm.ret)
        } else {
            match mnode {
                Some(mn) => {
                    let md = self.ctx.ast.method_decl(mn).clone();
                    let mut ps = Vec::new();
                    for p in md.params.iter().skip(1) {
                        match self.ctx.ast.param(*p) {
                            MemberKind::Param(ParamData { ty: Some(t), .. }) => {
                                ps.push(self.ctx.resolve_sig_ty(*t, &subst, Some(concrete)))
                            }
                            _ => ps.push(TY_I32),
                        }
                    }
                    let ret = md.ret.map(|r| self.ctx.resolve_sig_ty(r, &subst, Some(concrete))).unwrap_or(TY_NIL);
                    (ps, ret)
                }
                None => (tm.params.clone(), tm.ret),
            }
        };
        if args.len() != ptys.len() {
            self.ctx.err(sp, format!("call arity: {} args for {} params", args.len(), ptys.len()));
            return Err(());
        }
        let mut aregs = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let t = self.compile_expr(*a, Some(ptys[i]))?;
            // a `Self`-typed trait parameter accepts the concrete
            // receiver (nominal widening, RFC 0012 §4); under the
            // concrete ABI the types already match, so the check is
            // exact and NO box is emitted
            if !self.widens(t, ptys[i]) {
                self.ctx.err(self.ctx.ast.span(a.id()), format!(
                    "argument {} is `{}`, `{}` expected",
                    i + 1, self.ctx.type_name(t), self.ctx.type_name(ptys[i])
                ));
            }
            if slot_abi {
                self.widen_to_slot(t, ptys[i], sp.lo);
            }
            aregs.push(self.last_reg);
        }
        // tiny concrete-ABI bodies inline at the site; box sites fall
        // back to the slot-variant CallM (P1.3)
        if !slot_abi {
            if let Some(mn) = mnode {
                // the enclosing class for `Self`-ish statics in the body
                let cname = self
                    .ctx
                    .inst_data
                    .get(&concrete)
                    .map(|(d, _)| *d)
                    .or_else(|| self.ctx.datas.iter().find(|(_, d)| d.ty == concrete).map(|(n, _)| *n));
                if self.try_inline_impl_call(
                    impl_idx, mn, tm.name, concrete, &subst, cname, rreg, &aregs, &ptys, ret_ty, sp,
                ) {
                    return Ok(ret_ty);
                }
            }
        }
        let key = self.ctx.impl_method_key(impl_idx, tm.name, slot_abi);
        let inst = crate::check::Inst {
            key,
            subst,
            trait_origins: vec![],
        };
        let fid = self.ctx.ensure_inst(inst);
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        { let (argv_off, argc) = self.pool_recv_args(rreg, &(aregs)); self.emit(Op::CallM { func: fid, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
        Ok(ret_ty)
    }

    /// A trait method called through a TYPE PARAMETER's name —
    /// `T.decode(r)` inside `fn decodeJson<T requires JsonDeserialize>`
    /// (the rut-json batch phase 1's sanctioned checker gap 2; RFC 0012's
    /// no-self trait method, the trait's Self param spelled by name).
    /// Generic bodies compile per instantiation, so `param` is already
    /// the substituted concrete type and the `(trait, type)` impl on it
    /// answers. NO-SELF methods only: the callee's parameter list has no
    /// receiver slot, so this lowers to a plain `Call` — a method with a
    /// receiver is a caller-side error, never a silent misalign.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn compile_trait_param_static_call(
        &mut self,
        param: TypeId,
        name: IdentId,
        args: Vec<NodeHandle<AnyExpr>>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        // the local registry first (the impl and the generic body usually
        // share a module — json's entries and its impls do), then another
        // module's registration (RFC 0012 §2/§5)
        if let Some((idx, midx)) = self.find_trait_impl_method(param, name) {
            let im = self.ctx.impls[idx].clone();
            let tdesc = self.ctx.trait_by_id(im.trait_id).clone();
            let tm = tdesc.methods[midx].clone();
            // the trait method must be no-self: check the impl's own
            // method node — its first param is the receiver slot when
            // present, and a receiver would eat this call's first argument
            let mnode = im.methods.iter().find(|(n, _)| *n == tm.name).map(|(_, n)| *n);
            let has_recv = mnode.map_or(false, |mn| {
                matches!(
                    self.ctx.ast.method_decl(mn).params.first().map(|p| self.ctx.ast.param(*p)),
                    Some(MemberKind::SelfParam(_))
                )
            });
            if has_recv {
                let t = self.ctx.name(tdesc.name);
                let m = self.ctx.name(tm.name);
                self.ctx.err(sp, format!(
                    "`{m}` takes a receiver — `{t}` methods with `self` are called on a value, not the type parameter's name"
                ));
                return Err(());
            }
            // the impl method's own signature resolved under the target
            // substitution (the same law as compile_trait_static_call's
            // concrete ABI; `Self` spells the concrete target)
            let subst: Vec<(IdentId, TypeId)> = match &im.target_data {
                Some((_, params)) => match self.ctx.inst_data.get(&param).cloned() {
                    Some((_, cargs)) => params.iter().cloned().zip(cargs.into_iter()).collect(),
                    // structural template targets: the element rides the
                    // receiver's own shape (`?elem` / `[elem]`)
                    None => match (self.ctx.types.kind(param), params.len()) {
                        (TyKind::Opt { elem }, 1) => vec![(params[0], *elem)],
                        (TyKind::Array { elem }, 1) => vec![(params[0], *elem)],
                        _ => vec![],
                    },
                },
                None => vec![],
            };
            let (ptys, ret_ty) = match mnode {
                Some(mn) => {
                    let md = self.ctx.ast.method_decl(mn).clone();
                    let mut ps = Vec::new();
                    for p in md.params.iter() {
                        match self.ctx.ast.param(*p) {
                            MemberKind::Param(ParamData { ty: Some(t), .. }) => {
                                ps.push(self.ctx.resolve_sig_ty(*t, &subst, Some(param)))
                            }
                            _ => ps.push(TY_I32),
                        }
                    }
                    let ret = md.ret.map(|r| self.ctx.resolve_sig_ty(r, &subst, Some(param))).unwrap_or(TY_NIL);
                    (ps, ret)
                }
                None => (tm.params.clone(), tm.ret),
            };
            if args.len() != ptys.len() {
                self.ctx.err(sp, format!("call arity: {} args for {} params", args.len(), ptys.len()));
                return Err(());
            }
            // the enclosing class for `Self`-ish statics in the body —
            // the same probe compile_trait_static_call's inline path runs
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
            // no inline here (P1.3 stays a receiver-call optimization):
            // a no-self static body binds through the compiled Inst below
            let key = self.ctx.impl_method_key(idx, tm.name, false);
            let inst = crate::check::Inst {
                key,
                subst,
                trait_origins: vec![],
            };
            let fid = self.ctx.ensure_inst(inst);
            let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
            { let (argv_off, argc) = self.pool_args(&(aregs)); self.emit(Op::Call { func: fid, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
            return Ok(ret_ty);
        }
        if let Some((eidx, midx)) = self.find_extern_trait_impl_method(param, name) {
            let im = &self.ctx.extern_impls[eidx];
            let tdesc = self.ctx.trait_by_id(im.trait_id).clone();
            let tm = tdesc.methods[midx].clone();
            let list = im.methods_concrete.iter().find(|(n, _)| *n == tm.name);
            let (fid, ptys, ret_ty) = match list {
                Some(&(_, f)) => (f, tm.params.clone(), tm.ret),
                None => match im.methods.iter().find(|(n, _)| *n == tm.name) {
                    Some(&(_, f)) => (f, tm.params.clone(), tm.ret),
                    None => {
                        let t = self.ctx.name(tdesc.name);
                        self.ctx.err(sp, format!("impl `{t}` is missing `{}`", self.ctx.name(tm.name)));
                        return Err(());
                    }
                },
            };
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
            let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
            { let (argv_off, argc) = self.pool_args(&(aregs)); self.emit(Op::Call { func: fid, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
            return Ok(ret_ty);
        }
        self.ctx.err(sp, format!(
            "`{}` has no trait impl providing `{}` in this instantiation — the bound `requires` clause names a trait whose impl is missing",
            self.ctx.type_name(param),
            self.ctx.name(name)
        ));
        Err(())
    }

    /// Another module's registration of the `(trait, type)` impl
    /// (RFC 0012 §2/§5): the method is already compiled in the exporter,
    /// so the call binds to its scope-qualified fn id directly (`CallM`)
    /// — link rebases it. The signature comes from the trait's
    /// descriptor as registered from the surface.
    ///
    /// `slot_abi` picks the bound variant (P1.2): the slot id for a
    /// trait-object receiver, the concrete id for a bare receiver
    /// (`Self`-spelled params cross as the concrete target — the trait
    /// descriptor spells them as trait objects, so the concrete ABI maps
    /// this trait's trait-object params onto `concrete`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn compile_extern_trait_static_call(
        &mut self,
        ext_idx: usize,
        midx: usize,
        concrete: TypeId,
        rreg: u16,
        args: Vec<NodeHandle<AnyExpr>>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
        slot_abi: bool,
    ) -> TcResult<TypeId> {
        // a used GENERIC-target impl (the row's target is a registered
        // template): the per-instantiation bodies live in the impl's
        // owner — mint the mirror request instead of binding the
        // placeholder surface fn ids
        let im_target = self.ctx.extern_impls[ext_idx].target;
        let is_template = self
            .ctx
            .extern_generics
            .values()
            .any(|g| g.template == im_target)
            || self.ctx.impl_target_is_structural_template(im_target);
        if is_template {
            let subst = self
                .ctx
                .inst_data
                .get(&concrete)
                .cloned()
                .map(|(_, a)| a)
                .unwrap_or_else(|| match self.ctx.types.kind(concrete).clone() {
                    TyKind::Opt { elem } | TyKind::Array { elem } => vec![elem],
                    _ => vec![],
                });
            return self.compile_extern_impl_template_call(ext_idx, midx, concrete, subst, rreg, args, expected, sp);
        }
        let (fid, tm, trait_id) = {
            let im = &self.ctx.extern_impls[ext_idx];
            let tdesc = self.ctx.trait_by_id(im.trait_id);
            let tm = tdesc.methods[midx].clone();
            let list = if slot_abi {
                None
            } else {
                im.methods_concrete.iter().find(|(n, _)| *n == tm.name)
            };
            match list {
                Some(&(_, f)) => (f, tm, im.trait_id),
                // no concrete twin registered (single-ABI exporter):
                // the slot variant IS the concrete one there
                None => match im.methods.iter().find(|(n, _)| *n == tm.name) {
                    Some(&(_, f)) => (f, tm, im.trait_id),
                    None => {
                        let t = self.ctx.name(tdesc.name);
                        self.ctx.err(sp, format!("impl `{t}` is missing `{}`", self.ctx.name(tm.name)));
                        return Err(());
                    }
                },
            }
        };
        // param types for the chosen ABI
        let ptys: Vec<TypeId> = if slot_abi {
            tm.params.clone()
        } else {
            tm.params
                .iter()
                .map(|&p| match self.ctx.types.kind(p) {
                    TyKind::TraitObj { trait_id: t } if *t == trait_id => concrete,
                    _ => p,
                })
                .collect()
        };
        let ret_ty = tm.ret;
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
            if slot_abi {
                self.widen_to_slot(t, ptys[i], sp.lo);
            }
            aregs.push(self.last_reg);
        }
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        { let (argv_off, argc) = self.pool_recv_args(rreg, &(aregs)); self.emit(Op::CallM { func: fid, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
        Ok(ret_ty)
    }

    /// The extern registration of `(trait, type)` whose method set
    /// contains `name` — `(extern impl index, trait method index)`. The
    /// use-both gate lives here: an impl whose trait's name was never
    /// used does not dispatch (RFC 0012 §6); `no_method_error` still
    /// sees it for the "use `I` .." diagnostic.
    pub(crate) fn find_extern_trait_impl_method(&self, rt: TypeId, name: IdentId) -> Option<(usize, usize)> {
        for (eidx, im) in self.ctx.extern_impls.iter().enumerate() {
            // the use-both gate (RFC 0012 §6): the trait's name must be
            // callable here — a bound foreign trait, or the owner's own
            // declaration (a seed impl re-registers a caller's impl of
            // the owner's own trait; the caller used the trait to spell
            // the call)
            if !self.ctx.extern_trait_decls.contains_key(&im.trait_name)
                && self.ctx.find_trait(im.trait_name).is_none()
            {
                continue;
            }
            let target_hit = im.target == rt
                || self
                    .ctx
                    .impl_target_is_template_for(im.target, rt)
                || self
                    .ctx
                    .impl_target_is_structural_template_for(im.target, rt);
            if !target_hit || !im.methods.iter().any(|(n, _)| *n == name) {
                continue;
            }
            let tdesc = self.ctx.trait_by_id(im.trait_id);
            if let Some(midx) = tdesc.methods.iter().position(|m| m.name == name) {
                return Some((eidx, midx));
            }
        }
        None
    }

    /// Static dispatch of a native builtin class's inherent method
    /// (`impl Array<T> { .. }`): the native shape supplies the target
    /// substitution; `params` is the `target_data` binding.
    pub(crate) fn compile_native_static_call(
        &mut self,
        impl_idx: usize,
        mname: IdentId,
        subst: Vec<(IdentId, TypeId)>,
        rreg: u16,
        args: Vec<NodeHandle<AnyExpr>>,
        _expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let im = self.ctx.impls[impl_idx].clone();
        let (_, mnode) = im.methods.iter().find(|(n, _)| *n == mname).cloned().ok_or(())?;
        let md = self.ctx.ast.method_decl(mnode).clone();
        let saved_subst = std::mem::replace(&mut self.subst, subst.clone());
        let saved_self = self.self_ty;
        self.self_ty = None;
        let mut ptys = Vec::new();
        for p in md.params.iter().skip(1) {
            match self.ctx.ast.param(*p) {
                MemberKind::Param(ParamData { ty: Some(t), .. }) => ptys.push(self.resolve_type_now(*t)),
                _ => ptys.push(TY_I32),
            }
        }
        let ret_ty = md.ret.map(|r| self.resolve_type_now(r)).unwrap_or(TY_NIL);
        self.subst = saved_subst;
        self.self_ty = saved_self;
        if args.len() != ptys.len() {
            self.ctx.err(sp, format!("call arity: {} args for {} params", args.len(), ptys.len()));
            return Err(());
        }
        let mut aregs = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let t = self.compile_expr(*a, Some(ptys[i]))?;
            if t != ptys[i] {
                self.ctx.err(self.ctx.ast.span(a.id()), format!(
                    "argument {} is `{}`, `{}` expected",
                    i + 1, self.ctx.type_name(t), self.ctx.type_name(ptys[i])
                ));
            }
            aregs.push(self.last_reg);
        }
        let inst = crate::check::Inst {
            key: self.ctx.impl_method_key(impl_idx, mname, false),
            subst,
            trait_origins: vec![],
        };
        let fid = self.ctx.ensure_inst(inst);
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        { let (argv_off, argc) = self.pool_recv_args(rreg, &(aregs)); self.emit(Op::CallM { func: fid, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
        Ok(ret_ty)
    }


    pub(crate) fn finish_trait_call(
        &mut self,
        slot: u32,
        param_tys: Vec<TypeId>,
        ret_ty: TypeId,
        rreg: u16,
        args: Vec<NodeHandle<AnyExpr>>,
        _expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        if args.len() != param_tys.len() {
            self.ctx.err(sp, format!("call arity: {} args for {} params", args.len(), param_tys.len()));
            return Err(());
        }
        let mut aregs = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let t = self.compile_expr(*a, Some(param_tys[i]))?;
            if !self.widens(t, param_tys[i]) {
                self.ctx.err(self.ctx.ast.span(a.id()), format!(
                    "argument {} is `{}`, `{}` expected",
                    i + 1, self.ctx.type_name(t), self.ctx.type_name(param_tys[i])
                ));
            }
            self.widen_to_slot(t, param_tys[i], sp.lo);
            aregs.push(self.last_reg);
        }
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        // the vtable form is final — origins multiple, the call consults
        // the descriptor (RFC 0012 §1, the two-rule dispatch law)
        { let (argv_off, argc) = self.pool_recv_args(rreg, &(aregs)); self.emit(Op::CallI { slot: slot, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
        Ok(ret_ty)
    }

}
