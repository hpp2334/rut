//! Interface statics: the bound-parameter static (a monomorphized
//! body's `T.decode(r)` — the member lives on the concrete type's own
//! inherent surface) and the itable-slot call finisher.

use crate::check::TcResult;
use crate::lir::*;

impl<'a, 'b> FnCompiler<'a, 'b> {

    /// A STATIC member call on a bound type parameter's concrete type —
    /// `T.decode(r)` inside a monomorphized body (`decodeJson<T>`'s
    /// wrapper story): the body compiles per instantiation, so the
    /// parameter names its substituted concrete type here, and the
    /// member is the type's OWN inherent surface (no-self only — a
    /// receiver member is spelled through a value, `t.encode(w)`).
    /// Satisfaction was proved at the instantiation's bound admission;
    /// a miss here is a diagnosed residue.
    pub(crate) fn compile_bound_param_static_call(
        &mut self,
        concrete: TypeId,
        iface_id: Option<Vec<u32>>,
        name: IdentId,
        args: Vec<NodeHandle<AnyExpr>>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        // the carried-member law first: a REQUESTER-CARRIED concrete
        // type (an owner-anchored mirror body over a consumer's type)
        // binds through the bound's descriptor — the mirror's ledger
        // row names the type's HOME unit, link unifies it with the
        // real body
        // the owner's OWN instantiation may dedup onto a seed-carried
        // row (the attach's structural dedup) before any value use
        // registered it — force the inst row first, so the carried test
        // never misfires on the unit's own class
        self.ctx.ensure_local_inst_row(concrete);
        if self.ctx.type_is_carried(concrete) {
            let Some(ifaces) = iface_id else {
                self.ctx.err(sp, format!(
                    "`{}` is carried from another unit and names no interface bound here",
                    self.ctx.type_name(concrete)
                ));
                return Err(());
            };
            let (ptys, ret_ty, iface_id) = self.iface_member_shape(concrete, &ifaces, name, sp)?;
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
            let fid = self.mirror_carried(concrete, iface_id, name);
            let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
            let _ = expected;
            { let (argv_off, argc) = self.pool_args(&(aregs)); self.emit(Op::Call { func: fid, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
            return Ok(ret_ty);
        }
        let Some(member) = self.ctx.find_inherent_member(concrete, name) else {
            self.ctx.err(sp, format!(
                "`{}` has no static member `{}`",
                self.ctx.type_name(concrete),
                self.ctx.name(name)
            ));
            return Err(());
        };
        match member {
            crate::check::MemberSrc::Local { node, env, data } => {
                let saved_subst = std::mem::replace(&mut self.subst, env.clone());
                let saved_self = self.self_ty;
                self.self_ty = Some(concrete);
                let md = self.ctx.ast.method_decl(node).clone();
                let mut ptys = Vec::new();
                for p in md.params.iter() {
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
                    if !self.widens(t, ptys[i]) {
                        self.ctx.err(self.ctx.ast.span(a.id()), format!(
                            "argument {} is `{}`, `{}` expected",
                            i + 1, self.ctx.type_name(t), self.ctx.type_name(ptys[i])
                        ));
                    }
                    aregs.push(self.last_reg);
                }
                let inst = crate::check::Inst {
                    key: crate::check::FnKey::Method { data, name },
                    subst: env,
                    iface_origins: vec![],
                };
                let fid = self.ctx.ensure_inst(inst);
                let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
                let _ = expected;
                { let (argv_off, argc) = self.pool_args(&(aregs)); self.emit(Op::Call { func: fid, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
                Ok(ret_ty)
            }
            crate::check::MemberSrc::Extern { subst, data, .. } => {
                // the exporter's compiled fn, mirrored: the stub's ledger
                // row names the declaring package, link redirects it
                let inst = crate::check::Inst {
                    key: crate::check::FnKey::Method { data, name },
                    subst,
                    iface_origins: vec![],
                };
                let fid = self.ctx.mirror_inst(inst);
                let md_sig = self
                    .ctx
                    .extern_inherents
                    .iter()
                    .find_map(|ih| ih.methods.iter().find(|m| m.name == name))
                    .cloned();
                let (ptys, ret_ty) = match md_sig {
                    Some(m) => (m.params, m.ret),
                    None => (vec![], TY_NIL),
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
                            i + 1, self.ctx.type_name(t), self.ctx.type_name(ptys[i])
                        ));
                    }
                    aregs.push(self.last_reg);
                }
                let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
                { let (argv_off, argc) = self.pool_args(&(aregs)); self.emit(Op::Call { func: fid, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
                Ok(ret_ty)
            }
        }
    }


    /// The descriptor-typed shape of one interface member, respelled
    /// for the carried concrete (`Self` leaves become the concrete).
    pub(crate) fn iface_member_shape(&mut self, concrete: TypeId, ifaces: &[u32], name: IdentId, sp: rut_lexer::span::Span) -> TcResult<(Vec<TypeId>, TypeId, u32)> {
        // the bound spells the capabilities; the member picks its own
        // descriptor (spelled, finite — never a search)
        let mut found: Option<(rut_core::binary::IfaceMethod, u32)> = None;
        for tid in ifaces {
            let desc = self.ctx.iface_by_id(*tid).clone();
            if let Some(tm) = desc.methods.iter().find(|m| m.name == name).cloned() {
                found = Some((tm, *tid));
                break;
            }
        }
        let Some((tm, iface_id)) = found else {
            let names: Vec<String> = ifaces.iter().map(|t| self.ctx.iface_base_name(*t)).collect();
            self.ctx.err(sp, format!(
                "`{}` is not a member of the bound's interfaces ({})",
                self.ctx.name(name),
                names.join(", ")
            ));
            return Err(());
        };
        let ptys = tm.params.iter().map(|&t| self.respell_self(t, iface_id, concrete)).collect();
        let ret = self.respell_self(tm.ret, iface_id, concrete);
        Ok((ptys, ret, iface_id))
    }

    /// `Self` leaves (this interface's object type) become the carried
    /// concrete; everything else passes through. The leaf may spell a
    /// FOREIGN table's row (a used interface's descriptor rides its
    /// exporter's ids), so the match bridges by the row's base name —
    /// the same bridge the dispatch law uses for carried descriptors.
    fn respell_self(&mut self, t: TypeId, iface_id: u32, concrete: TypeId) -> TypeId {
        match self.ctx.types.kind(t).clone() {
            TyKind::IfaceObj { iface_id: tid } => {
                if tid == iface_id || self.ctx.iface_base_name(tid) == self.ctx.iface_base_name(iface_id) {
                    return concrete;
                }
                t
            }
            // `Self` at any structural depth — `(?Self, ?E)` respells
            // the leaf under the shape
            TyKind::Opt { elem } => {
                let e = self.respell_self(elem, iface_id, concrete);
                if e == elem { t } else { self.ctx.mk_opt(e) }
            }
            TyKind::Array { elem } => {
                let e = self.respell_self(elem, iface_id, concrete);
                if e == elem { t } else { self.ctx.mk_array(e) }
            }
            TyKind::Data { fields } => {
                let out: Vec<Option<TypeId>> = fields
                    .iter()
                    .map(|f| {
                        let r = self.respell_self(f.ty, iface_id, concrete);
                        if r == f.ty { None } else { Some(r) }
                    })
                    .collect();
                if out.iter().all(|o| o.is_none()) {
                    return t;
                }
                let fields = fields
                    .iter()
                    .zip(out.iter())
                    .map(|(f, o)| rut_core::types::FieldInfo { name: f.name, ty: o.unwrap_or(f.ty) })
                    .collect();
                self.ctx.types.intern(rut_core::types::RutType {
                    name: self.ctx.types.type_at(t).name,
                    kind: TyKind::Data { fields },
                })
            }
            _ => t,
        }
    }

    /// The mirror fn for a carried type's member: the ledger row names
    /// the type's HOME unit (the requester), so link unifies the stub
    /// with the real body.
    pub(crate) fn mirror_carried(&mut self, concrete: TypeId, iface_id: u32, name: IdentId) -> u32 {
        if let Some((d, args)) = self.ctx.inst_data.get(&concrete).cloned() {
            // a carried instantiation mirrors at its substitution
            let inst = crate::check::Inst {
                key: crate::check::FnKey::Method { data: d, name },
                // the mirror never compiles a body — the substitution's
                // ident half is filler; the ledger key spells the TYPES
                subst: args.iter().map(|&a| (d, a)).collect(),
                iface_origins: vec![],
            };
            return self.ctx.mirror_inst(inst);
        }
        let data = self.ctx.types.type_at(concrete).name;
        let inst = crate::check::Inst {
            key: crate::check::FnKey::Method { data, name },
            subst: vec![],
            iface_origins: vec![],
        };
        let _ = iface_id;
        // the ledger row names the type's HOME unit — link unifies the
        // stub with the real body there (the claim law runs dep-first,
        // so a bodyless stub must spell the winner's key, never win)
        self.ctx.pending_mirror_owner = self.ctx.carried_home(concrete);
        let fid = self.ctx.mirror_inst(inst);
        self.ctx.pending_mirror_owner = None;
        fid
    }

    /// The itable-slot call: the vtable form is final — origins
    /// multiple (or unknown), the call consults the value's concrete
    /// type through the itable (the fill was proved at the boxing
    /// site). The receiver register holds the cell handle (only
    /// ref-repr values satisfy interfaces); the args widen to the
    /// interface's declared signature.
    pub(crate) fn finish_iface_call(
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
            aregs.push(self.last_reg);
        }
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        { let (argv_off, argc) = self.pool_recv_args(rreg, &(aregs)); self.emit(Op::CallI { slot: slot, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
        Ok(ret_ty)
    }

}
