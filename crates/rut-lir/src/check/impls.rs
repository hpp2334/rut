//! Impl search and minting: the (trait, target) registry lookups (local + cross-module, RFC 0012), the parameterized-template dispatch half (RFC 0043 §A5), inline-bound admission, and impl-method ABI keying.

use super::*;

impl<'a> Ctx<'a> {

    // ---- impl-method ABI variants (P1, mapset perf plan) ----

    /// True when the impl's methods compile in TWO ABI variants: a
    /// prim-target trait impl (the prologue unboxes; a bare receiver
    /// wants the concrete variant). Ref-repr targets (`str`/`bytes`/
    /// records) keep one — the ABIs coincide.
    pub fn impl_is_dual_abi(&self, idx: usize) -> bool {
        let im = &self.impls[idx];
        !im.inherent && matches!(self.types.kind(im.target), TyKind::Prim(_))
    }

    /// The [`FnKey`] for an impl method under the requested ABI. `slot`
    /// collapses to the concrete variant for single-ABI impls — the
    /// vtable binding and a bare-receiver call then intern one fn.
    pub fn impl_method_key(&self, idx: usize, name: IdentId, slot: bool) -> FnKey {
        FnKey::ImplMethod {
            idx,
            name,
            slot_abi: slot && self.impl_is_dual_abi(idx),
        }
    }


    pub fn find_impl(&self, trait_id: u32, target: TypeId) -> Option<usize> {
        self.impls
            .iter()
            .position(|i| !i.inherent && i.trait_id == trait_id && i.target == target)
    }

    /// The impl satisfying `(trait, target)` INCLUDING the structural
    /// template targets (the rut-json batch phase 1): a generic class
    /// instantiation binds its class's template impl (`Vec<i64>` →
    /// `impl I for Vec<T>`), and a nullable/array receiver binds its
    /// element-generic impl (`?i64` → `impl I for ?T`, `[i64]` →
    /// `impl I for [T]`). Admission (RFC 0043 bounds) and dispatch read
    /// through this one law.
    pub fn find_impl_for(&self, trait_id: u32, target: TypeId) -> Option<usize> {
        self.impls
            .iter()
            .position(|i| {
                if i.inherent || i.trait_id != trait_id {
                    return false;
                }
                if i.target == target {
                    return true;
                }
                if let Some((d, _)) = self.inst_data.get(&target) {
                    if let Some(td) = self.find_data(*d) {
                        if td.ty == i.target {
                            return true;
                        }
                    }
                }
                match (self.types.kind(target), &i.target_data) {
                    (TyKind::Opt { .. }, Some((d, params))) if *d == sym::OPT && params.len() == 1 => true,
                    (TyKind::Array { .. }, Some((d, params))) if *d == sym::ARRAY && params.len() == 1 => true,
                    _ => false,
                }
            })
    }

    /// The impl satisfying `(trait, target)` wherever it lives: a local
    /// impl block, or another module's surface registration (RFC 0012
    /// §2/§5). Extern impls are gated on the trait's name having been
    /// used — an unused trait's impl is invisible to dispatch.
    pub fn find_impl_ex(&self, trait_id: u32, target: TypeId) -> Option<ImplHit> {
        if let Some(idx) = self.find_impl_for(trait_id, target) {
            return Some(ImplHit::Local(idx));
        }
        self.extern_impls.iter().position(|im| {
            im.trait_id == trait_id
                && im.target == target
                && self.extern_trait_decls.contains_key(&im.trait_name)
        }).map(ImplHit::Extern)
    }

    // ---- parameterized trait impls: the dispatch half (phase 2) ----

    /// The registered param-template impl answering (trait name, trait
    /// arity) over `target`'s class instantiation, if any. A template is a
    /// generic-target impl whose trait arguments mention the target's own
    /// parameters (`impl Readable<T> for Source<T>`); the match is on the
    /// class and its arity — the per-instantiation substitution happens at
    /// the caller.
    pub(crate) fn template_impl_for(
        &self,
        trait_name: IdentId,
        trait_arity: usize,
        target: TypeId,
    ) -> Option<usize> {
        let (dname, cargs) = self.inst_data.get(&target)?;
        if cargs.is_empty() {
            return None; // a non-generic record's impls register concretely
        }
        self.impls.iter().position(|im| {
            if im.inherent || im.trait_name != trait_name {
                return false;
            }
            match &im.target_data {
                Some((d, params)) => {
                    *d == *dname
                        && params.len() == cargs.len()
                        && im.trait_arg_nodes.len() == trait_arity
                }
                None => false,
            }
        })
    }

    /// The CONCRETE trait arguments a widening `(trait, target)` carries:
    /// the template impl's trait-arg nodes re-resolved under the target
    /// instantiation's substitution — `Readable<T>` over `Source<str>`
    /// becomes `Readable<str>`. Repeated params (`Writable<T, T>`) resolve
    /// positionally, every node independently. `None` when no template
    /// matches the shape.
    pub(crate) fn template_trait_args(
        &mut self,
        trait_name: IdentId,
        trait_arity: usize,
        target: TypeId,
    ) -> Option<Vec<TypeId>> {
        let idx = self.template_impl_for(trait_name, trait_arity, target)?;
        let im = self.impls[idx].clone();
        let (_, params) = im.target_data?;
        let (_, cargs) = self.inst_data.get(&target).cloned()?;
        let env: Vec<(IdentId, TypeId)> = params
            .iter()
            .cloned()
            .zip(cargs.iter().cloned())
            .collect();
        let args: Vec<TypeId> = im
            .trait_arg_nodes
            .iter()
            .map(|g| self.resolve_type(*g, &env))
            .collect();
        Some(args)
    }

    /// The exact-match door for trait dispatch (RFC 0012 §4/§5): a
    /// CONCRETE impl wins unchanged (`find_impl` — also the mint cache);
    /// on a miss, a parameterized trait impl whose unification PRODUCES
    /// the requested trait instantiation is minted for the concrete
    /// `(trait inst, target inst)` pair and its method bodies enter the
    /// monomorphization queue under the instantiation's substitution.
    /// Minting happens once — the minted registration IS the cache, every
    /// later lookup hits `find_impl` directly. v1 policy: the mint arm
    /// only runs on a miss, so a hand-written concrete impl
    /// (`impl Readable<i64> for Source<i64>`) always shadows the
    /// template's instantiation of the same pair.
    pub fn find_or_mint_impl(&mut self, trait_id: u32, target: TypeId) -> Option<usize> {
        if let Some(idx) = self.find_impl(trait_id, target) {
            return Some(idx);
        }
        // the requested trait's (name, args) — a generic trait's
        // instantiation (non-generic traits have exact impls or none)
        let (tname, targs) = self
            .trait_inst
            .iter()
            .find(|(_, &id)| id == trait_id)
            .map(|(k, _)| k.clone())?;
        let idx = self.template_impl_for(tname, targs.len(), target)?;
        let im = self.impls[idx].clone();
        let resolved = self.template_trait_args(tname, targs.len(), target)?;
        // the unification must PRODUCE the requested instantiation —
        // `Readable<T>` over `Source<str>` yields `Readable<str>`; any
        // other resolution is a different impl, not this one
        if resolved != targs || self.mk_trait_inst(tname, resolved.clone()) != trait_id {
            return None;
        }
        let Some((_, params)) = im.target_data.clone() else {
            return None;
        };
        let (_, cargs) = self.inst_data.get(&target).cloned()?;
        let env: Vec<(IdentId, TypeId)> = params
            .iter()
            .cloned()
            .zip(cargs.iter().cloned())
            .collect();
        // MINT: the concrete pair joins the registry as the template's
        // clone — `target_data` (and the template flag) stay so the
        // substitution machinery and the vtable fills keep working
        // unchanged
        self.impls.push(ImplDecl {
            trait_id,
            trait_name: im.trait_name,
            target,
            target_data: im.target_data.clone(),
            trait_arg_nodes: im.trait_arg_nodes.clone(),
            is_template: true,
            inherent: false,
            methods: im.methods.clone(),
            origin: im.origin.clone(),
        });
        let minted = self.impls.len() - 1;
        // the instantiated method bodies (the vtable rows' callees)
        for (mname, _) in &im.methods {
            self.ensure_inst(Inst {
                key: self.impl_method_key(minted, *mname, true),
                subst: env.clone(),
                trait_origins: vec![],
            });
        }
        Some(minted)
    }


    // ---- inline generic bounds (RFC 0043, admission-only) ----

    /// Resolve one bound member — a `TyUnion` fans out; an alias expands
    /// (validated in pass 1b) before trait-vs-concrete detection.
    pub(crate) fn resolve_bound_members(
        &mut self,
        node: NodeHandle<AnyTy>,
        subst: &[(IdentId, TypeId)],
    ) -> Vec<BoundMember> {
        self.resolve_bound_members_in(node, subst, false)
    }

    /// The recursive resolver. `in_union` marks members resolved THROUGH a
    /// union spelling — a `TyUnion` bound node, or a union alias in bound
    /// position. Type-union bounds (RFC 0043 §3, native-fastpath phase 1):
    /// a union bound takes type NAMES only — a trait member diagnoses and
    /// drops out. A trait bound must stand alone (`K requires Enc`), where
    /// satisfaction is the one-trait fact the RFC 0012 registry answers;
    /// inside a union it would quietly turn capability resolution
    /// ("every named member has the method") into an any-impl fact.
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

    /// One bound member at a leaf type node; under a union spelling a
    /// trait member diagnoses and is dropped (the bound stays malformed —
    /// the diagnostics speak; admission sees only what survived).
    fn one_bound_member(
        &mut self,
        node: NodeHandle<AnyTy>,
        subst: &[(IdentId, TypeId)],
        in_union: bool,
    ) -> Vec<BoundMember> {
        let t = self.resolve_type(node, subst);
        let m = self.bound_member_of(t);
        if let (true, BoundMember::Trait(tid)) = (in_union, m) {
            let tname = self.name(self.traits[tid as usize].name).to_string();
            self.err(
                self.ast.span(node.id()),
                format!(
                    "`{tname}` is a trait — a union bound takes type names only; a trait bound must stand alone (RFC 0043)"
                ),
            );
            return vec![];
        }
        vec![m]
    }

    fn bound_member_of(&self, t: TypeId) -> BoundMember {
        match self.types.kind(t) {
            TyKind::TraitObj { trait_id } => BoundMember::Trait(*trait_id),
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

    /// The capability probe behind union bounds (RFC 0043 §3,
    /// native-fastpath phase 1): does `ty` provide `name` as a method?
    /// Surfaces scanned: the str/bytes members, prim `builtin impl`s,
    /// inherent data methods (declared or `impl T` blocks), the `[T]`
    /// and `opaque` inherent impls, and trait impls — local and other
    /// modules' registrations (RFC 0012 §2). Arity/signature stay the
    /// call's business: this answers "a member can be called this way".
    pub(crate) fn member_has_method(&self, ty: TypeId, name: IdentId) -> bool {
        match self.types.kind(ty) {
            TyKind::Str => {
                matches!(name, sym::LEN | sym::SLICE | sym::CODE | sym::ENCODE
                    | sym::CODE_AT | sym::SCAN | sym::STARTS_WITH)
                    || self.has_trait_impl_method(ty, name)
            }
            TyKind::Bytes => {
                matches!(name, sym::LEN | sym::DECODE | sym::CLONE)
                    || self.has_trait_impl_method(ty, name)
            }
            TyKind::Prim(_) => self.builtin_impl(name, ty).is_some() || self.has_trait_impl_method(ty, name),
            TyKind::Data { .. } => {
                // inherent: the declaring class/struct's inline methods or
                // an `impl T { .. }` block
                let inherent = match self.inst_data.get(&ty).cloned() {
                    Some((dname, _)) => self
                        .find_data(dname)
                        .map_or(false, |d| d.methods.iter().any(|(n, _)| *n == name)),
                    None => self
                        .datas
                        .iter()
                        .find(|(_, d)| matches!(self.types.kind(d.ty), TyKind::Data { .. }) && self.types.dense(d.ty) == self.types.dense(ty))
                        .map_or(false, |(_, d)| d.methods.iter().any(|(n, _)| *n == name)),
                } || self
                    .impls
                    .iter()
                    .any(|im| im.inherent && self.impl_target_is(im, ty) && im.methods.iter().any(|(n, _)| *n == name));
                inherent || self.has_trait_impl_method(ty, name)
            }
            TyKind::Array { .. } => self
                .impls
                .iter()
                .any(|im| {
                    im.inherent
                        && matches!(&im.target_data, Some((d, _)) if *d == sym::ARRAY)
                        && im.methods.iter().any(|(n, _)| *n == name)
                })
                || self.has_trait_impl_method(ty, name),
            TyKind::Opaque => self
                .impls
                .iter()
                .any(|im| im.inherent && im.target == TY_OPAQUE && im.methods.iter().any(|(n, _)| *n == name)),
            _ => false,
        }
    }

    /// Is the impl registered for `target` (exactly, or by the target's
    /// generic class name when `target` is an instantiation)?
    fn impl_target_is(&self, im: &ImplDecl, target: TypeId) -> bool {
        im.target == target
            || matches!(&im.target_data, Some((d, _)) if self
                .inst_data
                .get(&target)
                .map_or(false, |(rd, _)| rd == d))
    }

    /// A trait impl (local, or another module's registration — RFC 0012 §2)
    /// providing `name` on `target`.
    pub(crate) fn has_trait_impl_method(&self, target: TypeId, name: IdentId) -> bool {
        self.impls.iter().any(|im| {
            !im.inherent
                && self.impl_target_is(im, target)
                && im.methods.iter().any(|(n, _)| *n == name)
                && self.trait_by_id(im.trait_id).methods.iter().any(|m| m.name == name)
        }) || self
            .extern_impls
            .iter()
            .any(|im| im.target == target && im.methods.iter().any(|(n, _)| *n == name))
    }

    /// Gate a completed substitution against the item's inline bounds
    /// (RFC 0043): the concrete type must satisfy ANY union member —
    /// a concrete member by `TypeId` equality, a trait member via the
    /// impl registry. Trait-object instantiations satisfy nothing
    /// (RFC 0013 §2). Admission-only: no IR, no dispatch change.
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
            let members = self.resolve_bound_members(*bnode, subst);
            if members.is_empty() {
                continue; // unresolvable bound — already diagnosed
            }
            let satisfiable = !matches!(self.types.kind(concrete), TyKind::TraitObj { .. });
            let ok = satisfiable
                && members.iter().any(|m| match *m {
                    BoundMember::Trait(tid) => self.find_impl_ex(tid, concrete).is_some(),
                    BoundMember::Concrete(t) => t == concrete,
                });
            if !ok {
                // the single-trait bound names the missing impl — the
                // class/fn shape the plan diagnoses ("no impl `Enc`
                // for `Foo`"); unions keep the generic wording
                let msg = match members.as_slice() {
                    [BoundMember::Trait(tid)] => {
                        let tname = self.name(self.traits[*tid as usize].name).to_string();
                        let ty = self.type_name(concrete).to_string();
                        format!(
                            "`{ty}` does not satisfy `{g}` requires `{tname}` — no impl `{tname}` for `{ty}` is registered (RFC 0043)",
                            g = self.name(*g),
                        )
                    }
                    _ => format!(
                        "`{}` does not satisfy `{}` requires `{}` — no matching type or impl is registered (RFC 0043)",
                        self.type_name(concrete),
                        self.name(*g),
                        bound_ty_str(self, *bnode),
                    ),
                };
                self.err(sp, msg);
            }
        }
    }

    pub fn impls_of(&self, target: TypeId) -> Vec<usize> {
        self.impls
            .iter()
            .enumerate()
            .filter(|(_, i)| !i.inherent && i.target == target)
            .map(|(k, _)| k)
            .collect()
    }

}
