//! Structural satisfaction and bounds: the member-set checker that
//! replaced the (trait, target) registry, the inline-bound admission,
//! and the union-bound capability probe. Nothing registers —
//! satisfaction is boundary CHECKING (member-set match), never a
//! resolution search (one job per mechanism: the interface quadrant
//! observes, the inherent quadrant provides).

use super::*;

/// What this unit can see of a type's member surface (the
/// member-visibility law's three answers).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SurfaceKind {
    /// the declaring decl or a used class's carried rows — the
    /// member-set check answers here
    Visible,
    /// a bare carried row (a requester's seed-block type in an
    /// owner-anchored mirror body) — no surface; the requester's own
    /// boundary proved satisfaction
    Carried,
    /// this unit's own (or a boot) type with no surface — a real miss
    Own,
}

/// One interface member's requirement, as the satisfaction checker
/// reads it: the descriptor's shape (receiver excluded), plus — when
/// the interface's AST is local — the receiver form and asyncness the
/// member must match.
pub(crate) struct MemberReq {
    pub name: IdentId,
    pub self_form: Option<bool>,
    pub is_async: Option<bool>,
    pub ptys: Vec<TypeId>,
    pub ret: TypeId,
}

/// Where a concrete type's inherent member was found: a local decl
/// (its AST node resolves under the class substitution) or a used
/// class's surface row (its types arrive carried, substituted per
/// instantiation).
pub(crate) enum MemberSrc {
    Local {
        node: NodeHandle<MethodDeclNode>,
        env: Vec<(IdentId, TypeId)>,
        /// the decl name the member's Inst keys under (`Vec` for a
        /// `Vec<i64>` member)
        data: IdentId,
    },
    Extern {
        row: rut_core::binary::SurfaceMethod,
        subst: Vec<(IdentId, TypeId)>,
        data: IdentId,
    },
}

impl<'a> Ctx<'a> {

    // ---- the satisfaction checker ----

    /// Does `concrete` satisfy interface `iface_id` — every member
    /// present on the type's own inherent surface with a compatible
    /// signature? Checking only: whether the site also demands the
    /// itable fill (a boxing site does; an admission check does not)
    /// is the caller's business. `Err` names the first missing or
    /// mismatched member (the Go-shape diagnostic).
    pub(crate) fn check_satisfies(&mut self, concrete: TypeId, iface_id: u32) -> Result<(), String> {
        let tname = self.iface_base_name(iface_id);
        // the member-visibility law: satisfaction is checked where the
        // members are VISIBLE. A requester-carried type row (an
        // owner-anchored mirror body sees the consumer's seed block —
        // its type rows, never its impls) was PROVED at the
        // requester's own boxing site; the owner's re-admission defers
        // to that proof. A type with a visible surface (local decl,
        // used class, used generic) answers for real.
        self.ensure_local_inst_row(concrete);
        // the member-visibility law (the early-out): a Carried concrete
        // defers to the requester's own boundary proof
        if self.member_surface(concrete) == SurfaceKind::Carried {
            return Ok(());
        }
        let reqs = self.iface_member_reqs(iface_id);
        for req in &reqs {
            let Some(got) = self.find_inherent_member(concrete, req.name) else {
                return Err(format!("no member `{}`", self.name(req.name)));
            };
            if !self.member_sig_matches(concrete, &got, req, iface_id) {
                return Err(format!(
                    "member `{}`'s signature differs from the interface's",
                    self.name(req.name)
                ));
            }
        }
        Ok(())
    }

    /// Record the itable fill for a proven (concrete × interface) pair —
    /// the boxing-site demand. Idempotent per pair.
    pub(crate) fn demand_iface_fill(&mut self, concrete: TypeId, iface_id: u32) {
        if self.iface_fill_set.insert((concrete, iface_id)) {
            self.iface_fills.push((concrete, iface_id));
        }
    }

    /// What this unit can SEE of `concrete`'s member surface: the
    /// declaring decl or a used class's rows (`Visible`), a type this
    /// unit carries as a bare row with no surface at all (`Carried` —
    /// a requester's seed-block row in an owner-anchored mirror body).
    pub(crate) fn member_surface(&self, concrete: TypeId) -> SurfaceKind {
        if let Some((dname, _)) = self.inst_data.get(&concrete).cloned() {
            if self.find_data(dname).is_some() || self.extern_generics.contains_key(&dname) {
                return SurfaceKind::Visible;
            }
        }
        if self.datas.iter().any(|(_, d)| self.types.dense(d.ty) == self.types.dense(concrete))
            || self.enums.iter().any(|(_, e)| self.types.dense(e.ty) == self.types.dense(concrete))
        {
            return SurfaceKind::Visible;
        }
        if self.extern_inherents.iter().any(|ih| ih.target == concrete) {
            return SurfaceKind::Visible;
        }
        if self.extern_generics.iter().any(|(_, g)| g.template == concrete) {
            return SurfaceKind::Visible;
        }
        // a head-local instantiation whose LOCAL mint carries the members:
        // the local decl's own methods answer (the inst_data entry may sit
        // on the locally minted twin of this carried row)
        {
            let name = self.types.type_at(concrete).name;
            let text = self.interner.name(name);
            if let Some((head, args_text)) = text.split_once('<') {
                if let Some(hid) = self.interner.lookup(head) {
                    if let Some(d) = self.find_data(hid) {
                        if !d.generics.is_empty() && args_text.ends_with('>') {
                            return SurfaceKind::Visible;
                        }
                    }
                }
            }
        }
        // an interface OBJECT row is never Carried: the box is not a
        // requester's seed row, it is this unit's own spelling of a
        // trait-typed value — and a trait object's member surface is
        // EMPTY (only the boxed interface's own members reach it, and
        // those are dispatches, not satisfaction proofs). Reading the
        // head-law fallback here classified `[interface] Readable<f64>`
        // as Carried, so a boxed value "satisfied" every other
        // interface (the Writable-pass hole): the widening recorded a
        // fill keyed on the BOX row — a row with no inherent members,
        // which build_vtables can never land — while the dispatch read
        // the PAYLOAD row. The honest answer: Own, an empty member
        // set — `check_satisfies` refuses, and the origin-aware
        // widening (the call sites) re-binds through the value's
        // carried origin where a proof actually exists.
        if matches!(self.types.kind(concrete), TyKind::IfaceObj { .. }) {
            return SurfaceKind::Own;
        }
        // no surface here. The HEAD law decides: a row whose head names
        // a type this unit declares is the unit's OWN instantiation —
        // the real body compiles here (owner-anchored;
        // `ensure_local_inst_row` registers the row on the call path).
        // A foreign-headed row (a requester's type in an owner-anchored
        // mirror body) is CARRIED — its member set was proved at the
        // requester's own boundary.
        let name = self.types.type_at(concrete).name;
        let text = self.interner.name(name);
        let head = text.split('<').next().unwrap_or(text);
        if let Some(hid) = self.interner.lookup(head) {
            if self.find_data(hid).is_some()
                || self.find_enum(hid).is_some()
                || sym::primitive_ty(hid).is_some()
            {
                return SurfaceKind::Own;
            }
        }
        SurfaceKind::Carried
    }

    /// The interface's member requirements: the AST's signatures when
    /// the declaration is local (receiver form + asyncness included),
    /// the instantiated descriptor's otherwise (a used interface or a
    /// generic interface's per-argument-list row).
    fn iface_member_reqs(&mut self, iface_id: u32) -> Vec<MemberReq> {
        // the DECL row (non-generic local interface): the AST is here
        if let Some((_, info)) = self.iface_decls.iter().find(|(_, i)| i.id == iface_id).cloned() {
            let obj = self.mk_iface_obj(iface_id);
            let methods = match self.ast.item(rut_ast::ast::NodeHandle::new(info.node)) {
                ItemKind::Interface { methods, .. } => methods.clone(),
                _ => Vec::new(),
            };
            let mut out = Vec::new();
            for m in &methods {
                let md = self.ast.method_decl(*m);
                let mut ptys = Vec::new();
                for (i, p) in md.params.iter().enumerate() {
                    match self.ast.param(*p) {
                        MemberKind::SelfParam(sd) if i == 0 => {}
                        MemberKind::SelfParam(_) => {}
                        MemberKind::Param(ParamData { ty: Some(t), .. }) => {
                            ptys.push(self.resolve_sig_ty(*t, &[], Some(obj)));
                        }
                        _ => ptys.push(TY_I32),
                    }
                }
                let ret = md.ret.map(|r| self.resolve_sig_ty(r, &[], Some(obj))).unwrap_or(TY_NIL);
                out.push(MemberReq {
                    name: md.name,
                    self_form: md.params.first().map(|p| match self.ast.param(*p) {
                        MemberKind::SelfParam(sd) => Some(sd.is_mut),
                        _ => None,
                    }).flatten(),
                    is_async: Some(md.is_async),
                    ptys,
                    ret,
                });
            }
            return out;
        }
        // the descriptor row (extern, or a generic interface's minted
        // instantiation — its signatures are already substituted)
        let desc = self.iface_by_id(iface_id).clone();
        desc.methods.iter().map(|m| MemberReq {
            name: m.name,
            self_form: None,
            is_async: None,
            ptys: m.params.clone(),
            ret: m.ret,
        }).collect()
    }

    /// The concrete type's own inherent member named `name` — the
    /// declaring decl's methods (a generic instantiation reads its
    /// class's decl under the instantiation substitution) or a used
    /// class's surface rows.
    pub(crate) fn find_inherent_member(&self, concrete: TypeId, name: IdentId) -> Option<MemberSrc> {
        // the class substitution (a generic instantiation) and its decl
        let inst = self.inst_data.get(&concrete).cloned();
        if let Some((dname, cargs)) = &inst {
            if let Some(d) = self.find_data(*dname) {
                if let Some((_, node)) = d.methods.iter().find(|(n, _)| *n == name) {
                    let env = d.generics.iter().cloned().zip(cargs.iter().cloned()).collect();
                    return Some(MemberSrc::Local { node: *node, env, data: *dname });
                }
                return None;
            }
            // a used generic class: the surface rows carry the
            // placeholder-spelled signatures
            if let Some(g) = self.extern_generics.get(dname) {
                if let Some(ih) = self.extern_inherents.iter().find(|ih| ih.target == g.template) {
                    if let Some(row) = ih.methods.iter().find(|m| m.name == name) {
                        let subst = g.params.iter().cloned().zip(cargs.iter().cloned()).collect();
                        return Some(MemberSrc::Extern { row: row.clone(), subst, data: *dname });
                    }
                }
                return None;
            }
        }
        if let Some((dname, d)) = self.datas.iter().find(|(_, d)| self.types.dense(d.ty) == self.types.dense(concrete)).cloned() {
            if let Some((_, node)) = d.methods.iter().find(|(n, _)| *n == name) {
                return Some(MemberSrc::Local { node: *node, env: vec![], data: dname });
            }
            return None;
        }
        if let Some((ename, e)) = self.enums.iter().find(|(_, e)| self.types.dense(e.ty) == self.types.dense(concrete)).cloned() {
            if let Some((_, node)) = e.methods.iter().find(|(n, _)| *n == name) {
                return Some(MemberSrc::Local { node: *node, env: vec![], data: ename });
            }
            return None;
        }
        // a used plain class: its carried rows are the surface. The
        // mirror's ledger key spells the TYPE's name (the owner's own
        // row claims under it) — never a placeholder
        if let Some(ih) = self.extern_inherents.iter().find(|ih| ih.target == concrete) {
            if let Some(row) = ih.methods.iter().find(|m| m.name == name) {
                let data = self.types.type_at(ih.target).name;
                return Some(MemberSrc::Extern { row: row.clone(), subst: vec![], data });
            }
        }
        None
    }

    /// Does the found member's signature ACCEPT the requirement's —
    /// the interface's declared `Self` (this interface's object type,
    /// at any structural depth) accepts the concrete spelling at the
    /// leaf; everything else matches structurally. Receiver form and
    /// asyncness compare only when both sides spell them (local AST).
    fn member_sig_matches(&mut self, concrete: TypeId, got: &MemberSrc, req: &MemberReq, iface_id: u32) -> bool {
        let (self_form, is_async, got_ptys, got_ret) = match got {
            MemberSrc::Local { node, env, .. } => {
                let md = self.ast.method_decl(*node);
                let mut ptys = Vec::new();
                for (i, p) in md.params.iter().enumerate() {
                    match self.ast.param(*p) {
                        MemberKind::SelfParam(_) if i == 0 => {}
                        MemberKind::SelfParam(_) => {}
                        MemberKind::Param(ParamData { ty: Some(t), .. }) => {
                            ptys.push(self.resolve_sig_ty(*t, env, Some(concrete)));
                        }
                        _ => ptys.push(TY_I32),
                    }
                }
                let ret = md.ret.map(|r| self.resolve_sig_ty(r, env, Some(concrete))).unwrap_or(TY_NIL);
                let sf = md.params.first().map(|p| match self.ast.param(*p) {
                    MemberKind::SelfParam(sd) => Some(sd.is_mut),
                    _ => None,
                }).flatten();
                (sf, Some(md.is_async), ptys, ret)
            }
            MemberSrc::Extern { row, subst, .. } => {
                // placeholder-spelled rows substitute per instantiation;
                // a plain class's rows are concrete
                let ptys: Vec<TypeId> = row.params.iter().map(|&t| self.subst_placeholder(t, subst)).collect();
                (None, None, ptys, self.subst_placeholder(row.ret, subst))
            }
        };
        if let (Some(want), Some(got)) = (req.self_form, self_form) {
            if want != got {
                return false;
            }
        }
        if let (Some(want), Some(got)) = (req.is_async, is_async) {
            if want != got {
                return false;
            }
        }
        if got_ptys.len() != req.ptys.len() {
            return false;
        }
        for (a, b) in req.ptys.iter().zip(got_ptys.iter()) {
            if !self.sig_leaf_matches(*a, *b, iface_id) {
                return false;
            }
        }
        self.sig_leaf_matches(req.ret, got_ret, iface_id)
    }

    /// Substitute an extern row's `#<param>` placeholder rows under the
    /// class instantiation's substitution (a plain class's rows pass
    /// through — placeholders never occur).
    fn subst_placeholder(&mut self, t: TypeId, subst: &[(IdentId, TypeId)]) -> TypeId {
        let name = self.types.types.get(self.types.dense(t) as usize).map(|d| d.name);
        let Some(name) = name else { return t };
        let text = self.interner.name(name).to_string();
        let Some(param) = text.strip_prefix('#').map(|p| p.to_string()) else { return t };
        let pid = self.intern(&param);
        subst.iter().find(|(n, _)| *n == pid).map(|(_, ty)| *ty).unwrap_or(t)
    }

    /// The descriptor-derived sig type vs the member's concrete sig
    /// type: the interface's declared `Self` (this interface's object
    /// type, nested in the shape — `(?Self, ?E)`) accepts the
    /// concrete spelling at the leaf; everything else matches
    /// structurally (same kind, same members).
    fn sig_leaf_matches(&self, a: TypeId, b: TypeId, iface_id: u32) -> bool {
        if a == b {
            return true;
        }
        let self_obj = |t: TypeId| -> bool {
            matches!(self.types.kind(t), TyKind::IfaceObj { iface_id: tid } if *tid == iface_id)
        };
        if self_obj(a) {
            return true;
        }
        match (self.types.kind(a).clone(), self.types.kind(b).clone()) {
            (TyKind::Opt { elem: ae }, TyKind::Opt { elem: be }) => self.sig_leaf_matches(ae, be, iface_id),
            (TyKind::Array { elem: ae }, TyKind::Array { elem: be }) => self.sig_leaf_matches(ae, be, iface_id),
            (TyKind::Data { fields: fa }, TyKind::Data { fields: fb }) => {
                fa.len() == fb.len()
                    && fa.iter().zip(fb.iter()).all(|(x, y)| {
                        x.ty == y.ty || self.sig_leaf_matches(x.ty, y.ty, iface_id)
                    })
            }
            _ => false,
        }
    }

    /// Is `concrete` a REQUESTER-CARRIED row (no member surface here)?
    pub(crate) fn type_is_carried(&self, concrete: TypeId) -> bool {
        self.member_surface(concrete) == SurfaceKind::Carried
    }

    /// The carried type's HOME unit spec (the mirror ledger's owner
    /// anchor): recorded per carried row name at the seed attach.
    pub(crate) fn carried_home(&self, concrete: TypeId) -> Option<String> {
        let name = self.types.type_at(concrete).name;
        self.carried_names.get(&self.interner.name(name).to_string()).cloned()
    }

    /// The interface row's BASE name text: the row itself for a
    /// declaration, the spelled instantiation's head (`Readable<#T>` →
    /// `Readable`) for a per-argument-list descriptor row.
    pub fn iface_base_name(&self, iface_id: u32) -> String {
        let row = self.iface_by_id(iface_id);
        let text = self.interner.name(row.name);
        text.split('<').next().unwrap_or(text).to_string()
    }

    // ---- inline generic bounds (admission-only) ----

    /// Resolve one bound member — a `TyUnion` fans out; an alias expands
    /// (validated in pass 1b) before interface-vs-concrete detection.
    pub(crate) fn resolve_bound_members(
        &mut self,
        node: NodeHandle<AnyTy>,
        subst: &[(IdentId, TypeId)],
    ) -> Vec<BoundMember> {
        self.resolve_bound_members_in(node, subst, false)
    }

    /// The recursive resolver. `in_union` marks members resolved THROUGH a
    /// union spelling — a `TyUnion` bound node, or a union alias in bound
    /// position. Type-union bounds:
    /// a union bound takes type NAMES only — an interface member
    /// diagnoses and drops out. An interface bound must stand alone
    /// (`K requires Enc`), where satisfaction is the structural
    /// member-set fact; inside a union it would quietly turn capability
    /// resolution ("every named member has the method") into an
    /// any-member-suffices fact.
    fn resolve_bound_members_in(
        &mut self,
        node: NodeHandle<AnyTy>,
        subst: &[(IdentId, TypeId)],
        in_union: bool,
    ) -> Vec<BoundMember> {
        match self.ast.ty(node) {
            TypeKind::TyUnion { elems } => {
                let mut out = Vec::new();
                for e in elems.clone() {
                    out.extend(self.resolve_bound_members_in(e, subst, true));
                }
                out
            }
            TypeKind::TyPath { segs, .. }
                if segs.len() == 1 && segs[0].generics.is_empty()
                    && self.find_alias(segs[0].name).is_some() =>
            {
                let name = segs[0].name;
                self.validate_alias(name);
                let idx = self.aliases.iter().position(|a| a.name == name).unwrap();
                match self.aliases[idx].resolved {
                    Some(AliasTarget::Union) => {
                        let target = self.aliases[idx].target;
                        self.resolve_bound_members_in(target, subst, true)
                    }
                    _ => self.one_bound_member(node, subst, in_union),
                }
            }
            _ => self.one_bound_member(node, subst, in_union),
        }
    }

    /// One bound member at a leaf type node. An interface member is
    /// legal inside a union: `T requires JsonSerialize |
    /// JsonDeserialize` admits either capability (any-member-suffices,
    /// the union law), and each member call binds through the
    /// descriptor that declares it.
    fn one_bound_member(
        &mut self,
        node: NodeHandle<AnyTy>,
        subst: &[(IdentId, TypeId)],
        _in_union: bool,
    ) -> Vec<BoundMember> {
        let t = self.resolve_type(node, subst);
        vec![self.bound_member_of(t)]
    }

    fn bound_member_of(&self, t: TypeId) -> BoundMember {
        match self.types.kind(t) {
            TyKind::IfaceObj { iface_id } => BoundMember::Trait(*iface_id),
            _ => BoundMember::Concrete(t),
        }
    }

    /// Does the bound node SPELL a union — a `TyUnion`, or an alias
    /// resolving to a union target? pure shape question; alias
    /// validation is the admission path's business.
    pub(crate) fn bound_is_union(&self, node: NodeHandle<AnyTy>) -> bool {
        match self.ast.ty(node) {
            TypeKind::TyUnion { .. } => true,
            TypeKind::TyPath { segs, .. } if segs.len() == 1 && segs[0].generics.is_empty() => {
                self.find_alias(segs[0].name)
                    .map_or(false, |a| matches!(a.resolved, Some(AliasTarget::Union)))
            }
            _ => false,
        }
    }

    /// The capability probe behind union bounds (native-fastpath
    /// phase 1): does `ty` provide `name` as a method?
    /// Surfaces scanned: the str/bytes members, prim `builtin impl`s,
    /// the declaring decls' inherent methods, and used classes' carried
    /// rows. Arity/signature stay the
    /// call's business: this answers "a member can be called this way".
    pub(crate) fn member_has_method(&self, ty: TypeId, name: IdentId) -> bool {
        match self.types.kind(ty) {
            TyKind::Str => {
                matches!(name, sym::LEN | sym::SLICE | sym::CODE | sym::ENCODE
                    | sym::CODE_AT | sym::SCAN | sym::STARTS_WITH)
            }
            TyKind::Bytes => {
                matches!(name, sym::LEN | sym::DECODE | sym::CLONE)
            }
            TyKind::Prim(_) => self.builtin_impl(name, ty).is_some(),
            TyKind::Data { .. } => {
                // inherent: the declaring class/struct's methods — or a
                // used class's surface rows (the linkable-classes phase)
                let inherent = match self.inst_data.get(&ty).cloned() {
                    Some((dname, _)) => self
                        .find_data(dname)
                        .map_or(false, |d| d.methods.iter().any(|(n, _)| *n == name)),
                    None => self
                        .datas
                        .iter()
                        .find(|(_, d)| matches!(self.types.kind(d.ty), TyKind::Data { .. }) && self.types.dense(d.ty) == self.types.dense(ty))
                        .map_or(false, |(_, d)| d.methods.iter().any(|(n, _)| *n == name)),
                };
                inherent || self.has_extern_method(ty, name)
            }
            TyKind::Enum { .. } => {
                // inherent: the enum's `impl` block — or a used enum's
                // surface rows (the row target is the enum's type id)
                let inherent = self
                    .enums
                    .iter()
                    .find(|(_, e)| self.types.dense(e.ty) == self.types.dense(ty))
                    .map_or(false, |(_, e)| e.methods.iter().any(|(n, _)| *n == name));
                inherent || self.has_extern_method(ty, name)
            }
            _ => false,
        }
    }

    /// Gate a completed substitution against the item's inline bounds
    ///: the concrete type must satisfy ANY union member —
    /// a concrete member by `TypeId` equality, an interface member by
    /// the structural member-set check. Interface-typed instantiations
    /// satisfy nothing. Admission-only: no IR, no dispatch change.
    pub(crate) fn admit_bounds(
        &mut self,
        bounds: &[(IdentId, NodeHandle<AnyTy>)],
        subst: &[(IdentId, TypeId)],
        sp: Span,
    ) {
        if bounds.is_empty() {
            return;
        }
        for (g, bnode) in bounds {
            let Some(&concrete) = subst.iter().find(|(n, _)| n == g).map(|(_, t)| t) else {
                continue; // the generic was never substituted — earlier errors
            };
            // the TEMPLATE law: a `#<param>` placeholder instantiation
            // (a generic wrapper's own field laying out its class
            // template) defers admission — the real argument's check
            // fires at the CONSUMER's instantiation, where the
            // placeholder is spelled
            if self.ty_is_placeholder(concrete) {
                continue;
            }
            let members = self.resolve_bound_members(*bnode, subst);
            if members.is_empty() {
                continue; // unresolvable bound — already diagnosed
            }
            let satisfiable = !matches!(self.types.kind(concrete), TyKind::IfaceObj { .. });
            let mut why: Option<String> = None;
            let ok = satisfiable
                && members.iter().any(|m| match *m {
                    BoundMember::Trait(tid) => match self.check_satisfies(concrete, tid) {
                        Ok(()) => true,
                        Err(detail) => {
                            why = Some(detail);
                            false
                        }
                    },
                    BoundMember::Concrete(t) => t == concrete,
                });
            if !ok {
                // the single-interface bound names the missing member —
                // the Go-shape diagnostic; unions keep the generic wording
                let msg = match members.as_slice() {
                    [BoundMember::Trait(tid)] => {
                        let tname = self.iface_base_name(*tid);
                        let ty = self.type_name(concrete).to_string();
                        let detail = why.unwrap_or_else(|| "no satisfying member set".to_string());
                        format!(
                            "`{ty}` does not satisfy `{iface}`: {detail}",
                            iface = tname,
                        )
                    }
                    _ => format!(
                        "`{}` does not satisfy `{}` requires `{}` — no matching type in the bound",
                        self.type_name(concrete),
                        self.name(*g),
                        bound_ty_str(self, *bnode),
                    ),
                };
                self.err(sp, msg);
            }
        }
    }
}

