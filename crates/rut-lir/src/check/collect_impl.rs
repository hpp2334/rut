//! The pass-2 impl-block walk (`collect_impl`, RFC 0012): `impl T { .. }` inherent / trait-impl registration with coverage checks, plus `TraitReq` — what a trait requires of an implementing method.

use super::*;

/// What a trait requires of an implementing method (RFC 0012 §2):
/// name, `async`, receiver form, and the resolved signature.
struct TraitReq {
    name: IdentId,
    is_async: bool,
    /// `Some(is_mut)` — a `self`/`mut self` receiver; `None` — no `self`
    self_form: Option<bool>,
    /// parameter types (after `self` when there is one)
    ptys: Vec<TypeId>,
    ret: TypeId,
}

impl<'a> Ctx<'a> {


    /// Pass 2 — an `impl` block (RFC 0012): `impl T { .. }` attaches
    /// inherent methods to the target (the type's module only — a local
    /// struct/class, or a `builtin class` this module owns through its
    /// surface); `impl I for T { .. }` registers a trait impl (any
    /// module) after coverage checks, and its methods enter the
    /// monomorphization queue eagerly so vtables carry real ids.
    pub(crate) fn collect_impl(
        &mut self,
        node: NodeId,
        trait_ref: Option<NodeHandle<AnyTy>>,
        target: NodeHandle<AnyTy>,
        methods: &[NodeHandle<MethodDeclNode>],
    ) {
        let sp = self.ast.span(node);
        // ---- the target (both forms): a local struct/class, a
        // module-owned `builtin class` resolved through the native-type
        // table (the `LaunchedTask<T>` pattern), or — trait impls only —
        // a type this module USES (RFC 0012 §2: `impl ForeignTrait for
        // ForeignType` is legal; the pair's uniqueness is a link check).
        // A generic target (`impl .. for Vec<T>`) is a template: its
        // methods monomorphize per instantiation through `target_data`.
        // A bare primitive name (`impl T for i32`) is a trait-impl
        // target too — trait impls only: a primitive's inherent surface
        // stays core's `builtin impl` (RFC 0012 §2 / RFC 0032 §1.1).
        let (target_ty, target_data, is_local, is_used, is_prim, spell) = match self.ast.ty(target) {
            TypeKind::TyPath { segs, .. } if segs.len() == 1 => {
                let name = segs[0].name;
                let generics = segs[0].generics.clone();
                match self.ty_path_impl_target(sp, name, generics) {
                    Some(arm) => arm,
                    None => return,
                }
            }
            TypeKind::TyArray { elem } => {
                // `impl [T] { .. }` — an inherent impl over the array
                // type (RFC 0012 §2's `LaunchedTask<T>` pattern):
                // generic through the element parameter, a template
                // id for duplicate detection and inst keys
                let Some(params) = self.ty_generic_idents(std::slice::from_ref(elem)) else {
                    self.err(sp, "a generic impl target must name its type parameters (e.g. `[T]`)");
                    return;
                };
                if params.len() != 1 {
                    self.err(sp, "`[T]` takes one type parameter");
                    return;
                }
                let ph = self.types.intern(RutType {
                    name: sym::ARRAY,
                    kind: TyKind::Data { fields: vec![] },
                });
                (ph, Some((sym::ARRAY, params)), false, false, false, sym::ARRAY)
            }
            TypeKind::TyOpt { inner } => {
                // `impl I for ?T` — the nullable impl target (the rut-json
                // batch phase 1's sanctioned checker gap 1): mirrors the
                // TyArray arm exactly — generic through the element
                // parameter, a template id for duplicate detection and
                // inst keys; the methods monomorphize per instantiation.
                let Some(params) = self.ty_generic_idents(std::slice::from_ref(inner)) else {
                    self.err(sp, "a generic impl target must name its type parameters (e.g. `?T`)");
                    return;
                };
                if params.len() != 1 {
                    self.err(sp, "`?T` takes one type parameter");
                    return;
                }
                let ph = self.types.intern(RutType {
                    name: sym::OPT,
                    kind: TyKind::Data { fields: vec![] },
                });
                (ph, Some((sym::OPT, params)), false, false, false, sym::OPT)
            }
            _ => {
                self.err(
                    sp,
                    "impl target must be a struct or class of this module — a `builtin class` takes impls only in its own module (RFC 0012 §2)",
                );
                return;
            }
        };
        let mut mths: Vec<(IdentId, NodeHandle<MethodDeclNode>)> = Vec::new();
        for m in methods {
            mths.push((self.ast.method_decl(*m).name, *m));
        }
        // the orphan gate's target-side classification (RFC 0012 §2a) —
        // every unresolvable shape already returned above
        let (ty_display, ty_origin) = self.classify_target_origin(target);
        // the impl block's own splice origin — the instantiation ledger's
        // owner anchor for its monomorphized methods
        let origin = self.origin_of(sp.lo).to_string();
        match trait_ref {
            None => {
                if is_prim {
                    let pname = self.type_name(target_ty).to_string();
                    self.err(
                        sp,
                        format!(
                            "a primitive takes trait impls only — `{pname}`'s inherent surface is core's `builtin impl` (RFC 0032 §1.1)"
                        ),
                    );
                    return;
                }
                if is_used {
                    self.err(
                        sp,
                        "inherent impls live in the type's module — only `impl Trait for UsedType` may name a used type (RFC 0012 §2)",
                    );
                    return;
                }
                self.collect_impl_inherent(target, spell, target_ty, target_data, is_local, mths, origin)
            }
            Some(tr) => self.collect_impl_trait(sp, tr, target_ty, target_data, ty_display, ty_origin, mths, origin),
        }
    }
    /// The TyPath impl target, resolved (RFC 0012 §2 plus the
    /// hashmap-surface batch's seam (c)): the ROW form re-targets
    /// through the family's row first — `impl I for HashMap<K, i64>`
    /// registers on the row TARGET's class template, the same
    /// (trait, type) pair and substitution the direct spelling
    /// produces; a PLAIN alias expands in impl-target position
    /// (probe D1's lift — the alias spells the target's type); then
    /// the ordinary chain: a local struct/class, a module-owned
    /// `builtin class`, a used type (trait impls only), a primitive
    /// (trait impls only). Returns the target's type id, its generic
    /// template when generic, locality/used/prim flags, and the decl
    /// name the inherent path attaches under (`spell` — the expansion's
    /// decl, not the alias spelling). `None` = diagnosed, abort.
    #[allow(clippy::type_complexity)]
    fn ty_path_impl_target(
        &mut self,
        sp: rut_lexer::span::Span,
        name: IdentId,
        generics: Vec<NodeHandle<AnyTy>>,
    ) -> Option<(TypeId, Option<(IdentId, Vec<IdentId>)>, bool, bool, bool, IdentId)> {
        // (seam c, D1's lift) a PLAIN alias expands in impl-target
        // position: `impl Paint for B2` where `type B2 = Box2;` compiles
        // exactly as if `Box2` were spelled.
        if generics.is_empty() {
            if let Some(t) = self.plain_alias_target(name) {
                if let Some((dname, d)) = self.datas.iter().find(|(_, d)| d.ty == t).cloned().map(|(n, d)| (n, d)) {
                    return Some((d.ty, None, true, false, false, dname));
                }
            }
        }
        if let Some(d) = self.find_data(name).cloned() {
            if d.generics.is_empty() {
                Some((d.ty, None, true, false, false, name))
            } else {
                let Some(params) = self.ty_generic_idents(&generics) else {
                    self.err(sp, "a generic impl target must name its type parameters (e.g. `Vec<T>`)");
                    return None;
                };
                if params.len() != d.generics.len() {
                    self.err(sp, format!(
                        "`{}<..>` takes {} type parameter(s), {} given",
                        self.name(name), d.generics.len(), params.len()
                    ));
                    return None;
                }
                Some((d.ty, Some((name, params)), true, false, false, name))
            }
        } else if let Some(kind) = self.extern_native_types.get(&name).copied() {
            match (kind, generics.as_slice()) {
                (rut_core::binary::NativeTy::Opaque, []) => Some((TY_OPAQUE, None, false, false, false, name)),
                (rut_core::binary::NativeTy::Opaque, _) => {
                    self.err(sp, "`opaque` takes no type parameters");
                    None
                }
                // the trace snapshot takes no user impls: its
                // members are engine-builtins, the contract is
                // closed (RFC 0025's `builtin class` row)
                (rut_core::binary::NativeTy::StackTrace, _) => {
                    self.err(sp, "`StackTrace` takes no impl blocks — its members are engine builtins (`len`/`name(i)`/`line(i)`/`col(i)`/`render`)");
                    None
                }
                // the builder likewise: closed engine contract
                // (json-perf phase 2)
                (rut_core::binary::NativeTy::StrBuf, _) => {
                    self.err(sp, "`StrBuf` takes no impl blocks — its members are engine builtins (`push(s)`/`push_code(c)`/`len()`/`finish()`)");
                    None
                }
                // the weak box likewise: closed engine contract
                // (RFC 0017 v1) — its one member is the upgrade native
                (rut_core::binary::NativeTy::Weak, _) => {
                    self.err(sp, "`Weak` takes no impl blocks — its member is engine builtin (`upgrade()`)");
                    None
                }
                // the disposal context likewise: engine-minted, no
                // member surface — there is nothing to impl onto it
                (rut_core::binary::NativeTy::DisposalContext, _) => {
                    self.err(sp, "`DisposalContext` takes no impl blocks — the engine mints it per `dispose` call");
                    None
                }
            }
        } else if generics.is_empty() && self.extern_types.contains_key(&name) {
            // a USED type (RFC 0035 §1): legal as a TRAIT-impl
            // target only — inherent impls stay in the type's
            // module (RFC 0012 §2). The id is the exporter's
            // scope-qualified one; link rebases it.
            Some((self.extern_types[&name], None, false, true, false, name))
        } else if let Some(prim) = sym::primitive_ty(name) {
            // a primitive (integers, bool, str, bytes, …): trait
            // impls only, any module (RFC 0012 §2's
            // `impl ForeignTrait for ForeignType` pattern) — the
            // pair's uniqueness is a link check. Boot ids are
            // global, no rebase needed.
            if !generics.is_empty() {
                self.err(sp, format!("`{}` takes no generic arguments", self.name(name)));
                return None;
            }
            Some((prim, None, false, false, true, name))
        } else {
            self.err(
                sp,
                "impl target must be a struct or class of this module — a `builtin class` takes impls only in its own module (RFC 0012 §2)",
            );
            None
        }
    }

    /// The plain-alias expansion of an impl-target name (seam c):
    /// `Some(target TypeId)` when `name` is a declared alias whose
    /// single target resolved to a type.
    fn plain_alias_target(&mut self, name: IdentId) -> Option<TypeId> {
        if self.find_alias(name).is_none() {
            return None;
        }
        self.validate_alias(name);
        let resolved = self.find_alias(name).and_then(|a| a.resolved);
        match resolved {
            Some(AliasTarget::Ty(t)) => Some(t),
            _ => None,
        }
    }

    /// The bare single-segment ident a type node spells, when it does.
    fn node_head_ident(&self, node: NodeHandle<AnyTy>) -> Option<IdentId> {
        match self.ast.ty(node) {
            TypeKind::TyPath { segs, .. } if segs.len() == 1 && segs[0].generics.is_empty() => {
                Some(segs[0].name)
            }
            _ => None,
        }
    }

    /// Is the node's head name a KNOWN type (primitive, declared,
    /// used)? The impl-row matcher's parameter test — an unknown bare
    /// name is the impl's own generic.
    fn node_is_known_type(&self, node: NodeHandle<AnyTy>) -> bool {
        let Some(n) = self.node_head_ident(node) else { return true };
        sym::primitive_ty(n).is_some()
            || self.find_enum(n).is_some()
            || self.find_data(n).is_some()
            || self.find_trait(n).is_some()
            || self.find_alias(n).is_some()
            || self.extern_native_types.contains_key(&n)
            || self.extern_types.contains_key(&n)
    }

    /// The orphan classification of the impl TARGET as written (RFC 0012
    /// §2a): the head's display text and the pkg whose source declares it
    /// — `None` for a builtin, in no pkg. The doors mirror
    /// `collect_impl`'s target match, which has already rejected every
    /// unresolvable shape. Locality is of the HEAD: a generic head's type
    /// parameters never satisfy it, and a `?T`/`[T]` head peels to "no
    /// pkg" — only a local trait may be implemented for a builtin.
    fn classify_target_origin(&self, target: NodeHandle<AnyTy>) -> (String, Option<String>) {
        match self.ast.ty(target) {
            TypeKind::TyPath { segs, .. } if segs.len() == 1 => {
                let name = segs[0].name;
                let display = self.name(name).to_string();
                if let Some(d) = self.find_data(name) {
                    // a decl of THIS unit — own source or a spliced leaf;
                    // the origin map tells the two apart
                    let lo = self.ast.span(d.node.id()).lo;
                    (display, Some(self.origin_of(lo).to_string()))
                } else if let Some(AliasTarget::Ty(t)) = self
                    .find_alias(name)
                    .and_then(|a| a.resolved)
                {
                    // (seam c) a plain alias expands: the origin is the
                    // TARGET's decl (already validated — collect_impl's
                    // target resolution ran first)
                    match self.datas.iter().find(|(_, d)| d.ty == t) {
                        Some((_, d)) => {
                            let lo = self.ast.span(d.node.id()).lo;
                            (display, Some(self.origin_of(lo).to_string()))
                        }
                        None => (display, None),
                    }
                } else if self.extern_native_types.contains_key(&name) {
                    // `opaque` (the closed `StackTrace` shape never gets
                    // here) — a builtin, in no pkg
                    (display, None)
                } else if self.extern_types.contains_key(&name) {
                    // a USED type: the exporter's spec rides the binding
                    let origin = self
                        .extern_origins
                        .get(&name)
                        .cloned()
                        .unwrap_or_else(|| self.own_spec.clone());
                    (display, Some(origin))
                } else {
                    // a primitive — in no pkg
                    (display, None)
                }
            }
            TypeKind::TyArray { .. } => ("[T]".to_string(), None),
            TypeKind::TyOpt { .. } => ("?T".to_string(), None),
            _ => (String::new(), None), // unreachable — collect_impl returned
        }
    }

    /// The orphan classification of the impl's TRAIT name (RFC 0012 §2a):
    /// the pkg whose source declares it. A builtin trait (`Iterator`,
    /// `Index`, `Disposal`) is core's decl like every prelude name (RFC
    /// 0012 §2) — its origin is `core`, never "no pkg". Every shape that
    /// reaches the orphan gate resolved, so the own-spec fallback never
    /// fires.
    fn trait_origin(&self, name: IdentId) -> String {
        if let Some(info) = self.find_trait(name) {
            let lo = self.ast.span(info.node).lo;
            return self.origin_of(lo).to_string();
        }
        if let Some(spec) = self.extern_origins.get(&name) {
            return spec.clone();
        }
        if self.extern_traits.contains_key(&name) {
            return "core".to_string();
        }
        self.own_spec.clone()
    }

    /// `impl T { .. }` — inherent methods. Local targets attach into the
    /// `DataDecl`; native builtin-class targets register an inherent
    /// `ImplDecl` (dispatch resolves through the native shape). Both
    /// dispatch statically — the receiver's concrete type names the impl.
    fn collect_impl_inherent(
        &mut self,
        target: NodeHandle<AnyTy>,
        spell: IdentId,
        target_ty: TypeId,
        target_data: Option<(IdentId, Vec<IdentId>)>,
        is_local: bool,
        mths: Vec<(IdentId, NodeHandle<MethodDeclNode>)>,
        origin: String,
    ) {
        // `spell` is the target's DECL name (the expansion's decl when
        // the impl was spelled through an alias or a row — seam (c)),
        // resolved by `ty_path_impl_target`
        let tname = spell;
        if !is_local {
            // a `builtin class` inherent impl — registered for static
            // dispatch through the native shape
            let prev_names: Vec<IdentId> = self
                .impls
                .iter()
                .find(|im| im.inherent && im.target == target_ty)
                .map(|im| im.methods.iter().map(|(n, _)| *n).collect())
                .unwrap_or_default();
            for (n, mnode) in &mths {
                if prev_names.contains(n) {
                    self.err(
                        self.ast.span(mnode.id()),
                        format!("duplicate method `{}` on `{}`", self.name(*n), self.name(tname)),
                    );
                }
            }
            let generic = target_data.is_some();
            self.impls.push(ImplDecl {
                trait_id: u32::MAX,
                trait_name: tname,
                target: target_ty,
                target_data,
                trait_arg_nodes: vec![],
                is_template: false,
                inherent: true,
                methods: mths.clone(),
                origin,
            });
            // concrete targets' methods are eagerly queued (no call site
            // may exist); generic targets monomorphize at their call sites
            if !generic {
                let idx = self.impls.len() - 1;
                for (n, _) in &mths {
                    self.ensure_inst(Inst {
                        key: self.impl_method_key(idx, *n, false),
                        subst: vec![],
                        trait_origins: vec![],
                    });
                }
            }
            return;
        }
        // local struct/class: methods attach to the decl, where the
        // ordinary inherent-call machinery finds them
        if let Some(kind) = self.find_data(tname).map(|d| d.kind) {
            if kind == DataKind::Dataclass {
                for (_, mnode) in &mths {
                    if self.ast.method_decl(*mnode).vis.is_some() {
                        self.err(
                            self.ast.span(mnode.id()),
                            "dataclasses have no member visibility —all members are public (RFC 0009)",
                        );
                        break;
                    }
                }
            }
        }
        let Some(idx) = self.datas.iter().position(|(n, _)| *n == tname) else {
            return;
        };
        for (n, mnode) in &mths {
            if self.datas[idx].1.methods.iter().any(|(pn, _)| pn == n) {
                self.err(
                    self.ast.span(mnode.id()),
                    format!("duplicate method `{}` on `{}`", self.name(*n), self.name(tname)),
                );
            }
        }
        self.datas[idx].1.methods.extend(mths);
    }

    /// `impl I for T { .. }` — a trait impl (one of the pair local, RFC
    /// 0012 §2a): the orphan gate, then duplicate (trait, type) pair,
    /// coverage (every trait method implemented; signature match incl.
    /// `is_async` and receiver form), no extras, then registration and
    /// eager monomorphization. A parameterized trait impl (`impl
    /// Readable<T> for Source<T>`, v1) registers as a TEMPLATE: the
    /// trait ref's bare-parameter arguments bind the target's own
    /// parameters to placeholder types (the v1 shape guard inside), the
    /// registration reuses the generic-target shape (`target_data`),
    /// and per-instantiation substitution is the phase-2 dispatch half.
    fn collect_impl_trait(
        &mut self,
        sp: rut_lexer::span::Span,
        trait_ref: NodeHandle<AnyTy>,
        target_ty: TypeId,
        target_data: Option<(IdentId, Vec<IdentId>)>,
        ty_display: String,
        ty_origin: Option<String>,
        mths: Vec<(IdentId, NodeHandle<MethodDeclNode>)>,
        origin: String,
    ) {
        // the trait ref's head, read before resolution: the v1 shape
        // guard below needs the raw argument nodes (a non-path head
        // stays `None` — resolve_trait_ref_env diagnoses the shape)
        let head = match self.ast.ty(trait_ref) {
            TypeKind::TyPath { segs, .. } if segs.len() == 1 => {
                Some((segs[0].name, segs[0].generics.clone()))
            }
            _ => None,
        };
        // ---- the v1 shape guard for parameterized trait impls (the
        // `impl Readable<T> for Source<T>` form): each trait argument is
        // EITHER a concrete type (resolved as always) OR a bare
        // single-segment ident naming one of the TARGET's own type
        // parameters — bound to a template placeholder type so
        // resolution and the coverage check proceed (the impl registers
        // as a template; per-instantiation substitution is the phase-2
        // dispatch half). Repeats are legal (`Writable<T, T>` — the same
        // parameter in two slots). A parameter NESTED in a type
        // (`Readable<Vec<T>>`) is a loud error naming the v1 rule, and
        // so is a bare name that is neither a target parameter nor a
        // type in scope. The guard runs BEFORE resolution so its
        // diagnostic is the only one.
        let param_env: Vec<(IdentId, TypeId)> = match (&target_data, &head) {
            (Some((_, params)), Some((_, args))) => {
                let mut env: Vec<(IdentId, TypeId)> = Vec::new();
                for g in args {
                    match self.ast.ty(*g) {
                        TypeKind::TyPath { segs, .. }
                            if segs.len() == 1 && segs[0].generics.is_empty() =>
                        {
                            let n = segs[0].name;
                            if self.node_is_known_type(*g) {
                                // a type in scope wins even when the name
                                // doubles as a parameter (the impl-row
                                // matcher's own law) — concrete, as always
                            } else if params.contains(&n) {
                                env.push((n, self.param_placeholder(n)));
                            } else {
                                self.err(
                                    self.ast.span(g.id()),
                                    format!(
                                        "`{}` is neither a type in scope nor a type parameter of the impl target — a parameterized trait impl (v1) takes only concrete types or the target's own type parameters as trait arguments",
                                        self.name(n),
                                    ),
                                );
                                return;
                            }
                        }
                        _ => {
                            if self.ty_mentions_any(*g, params) {
                                self.err(
                                    self.ast.span(g.id()),
                                    format!(
                                        "`{}`: a parameterized trait impl (v1) takes only a concrete type or a bare type parameter of the target as a trait argument — a parameter nested inside a type is not supported yet",
                                        bound_ty_str(self, *g),
                                    ),
                                );
                                return;
                            }
                            // fully concrete — resolved as always
                        }
                    }
                }
                env
            }
            _ => vec![],
        };
        let Some(trait_id) = self.resolve_trait_ref_env(trait_ref, &param_env) else {
            return;
        };
        let Some((trait_name, trait_arg_nodes)) = head else {
            return;
        };
        // ---- the orphan gate (RFC 0012 §2a): at least one of the pair
        // is defined in the pkg whose source DECLARED the block. The
        // block's origin — its span's leaf on the origin map — is the
        // law's "current pkg": the origin, not the compiling unit,
        // decides, so a pkg's own impls stay legal in every unit that
        // splices them while a consumer's hand-written cross-pkg pair
        // errs. A bound name carries its exporter's spec; a builtin
        // (`?T`, `[T]`, a primitive, `opaque`) is in no pkg — only a
        // local trait may be implemented for one. Placement precedes
        // registration: an orphan never reaches the duplicate check or
        // the impl table.
        let own = self.origin_of(sp.lo);
        let trait_origin = self.trait_origin(trait_name);
        if trait_origin != own && ty_origin.as_deref() != Some(own) {
            let trait_side = format!("`{}` is {}'s", self.name(trait_name), trait_origin);
            let type_side = match &ty_origin {
                Some(pkg) => format!("`{}` is {}'s", ty_display, pkg),
                None => format!("`{}` is a builtin, in no pkg", ty_display),
            };
            let tail = match ty_origin {
                Some(_) => {
                    "an `impl Trait for Type` needs at least one of the pair declared in its own pkg (RFC 0012 §2a)"
                }
                None => "only a trait of this pkg may be implemented for a builtin (RFC 0012 §2a)",
            };
            self.err(
                sp,
                format!(
                    "orphan impl: neither `{}` nor `{}` is defined in this pkg — {}, {}; {}",
                    self.name(trait_name),
                    ty_display,
                    trait_side,
                    type_side,
                    tail,
                ),
            );
            return;
        }
        if let Some(_prev) = self.find_impl(trait_id, target_ty) {
            self.err(sp, "duplicate impl for the same (trait, type) pair (RFC 0012 §2)");
            return;
        }
        // the disposal contract's target gate: a CONCRETE user record
        // type — the engine's release path keys the row by the cell's
        // type id at refcount zero, so primitives and engine shapes
        // refuse, and a generic target has no static row to fill
        if trait_name == sym::DISPOSAL {
            if target_data.is_some() {
                self.err(
                    sp,
                    format!(
                        "`{ty_display}` cannot implement Disposal — implement it for the concrete struct or class"
                    ),
                );
                return;
            }
            if !matches!(self.types.kind(target_ty), TyKind::Data { .. }) {
                self.err(
                    sp,
                    format!(
                        "`{ty_display}` cannot implement Disposal — only a struct or class can (the engine calls `dispose` on the record's cell)"
                    ),
                );
                return;
            }
        }
        // the trait's required signatures: from the trait's own AST when
        // it is declared here (async + receiver form live only there),
        // resolved under the impl's trait-argument substitution; from the
        // instantiated descriptor otherwise (engine-named contracts)
        let trait_env: Vec<(IdentId, TypeId)> = match self.find_trait(trait_name).cloned() {
            Some(info) if !info.generics.is_empty() => {
                let args: Vec<TypeId> = trait_arg_nodes
                    .iter()
                    .map(|g| self.resolve_type(*g, &param_env))
                    .collect();
                info.generics.iter().cloned().zip(args.into_iter()).collect()
            }
            _ => vec![],
        };
        let has_ast = self
            .find_trait(trait_name)
            .map(|info| matches!(self.ast.item(rut_ast::ast::NodeHandle::new(info.node)), ItemKind::Trait { .. }))
            .unwrap_or(false);
        let reqs: Vec<TraitReq> = if has_ast {
            let info = self.find_trait(trait_name).cloned().unwrap();
            let methods = match self.ast.item(rut_ast::ast::NodeHandle::new(info.node)) {
                ItemKind::Trait { methods, .. } => methods.clone(),
                _ => Vec::new(),
            };
            let mut out = Vec::new();
            for m in &methods {
                let md = self.ast.method_decl(*m);
                // the trait side resolves `Self` against the impl's target,
                // so `fn eq(self, other: Self)` matches `other: Circle`
                let (self_form, ptys) = self.impl_sig_params(&md.params, trait_env.clone(), Some(target_ty));
                let ret = md.ret.map(|r| self.resolve_sig_ty(r, &trait_env, Some(target_ty))).unwrap_or(TY_NIL);
                out.push(TraitReq {
                    name: md.name,
                    is_async: md.is_async,
                    self_form,
                    ptys,
                    ret,
                });
            }
            out
        } else {
            // engine-named contract (e.g. `Iterator<E>`): signatures from
            // the instantiated descriptor
            let tdesc = self.traits[trait_id as usize].clone();
            let mut out = Vec::new();
            for tm in &tdesc.methods {
                out.push(TraitReq {
                    name: tm.name,
                    is_async: false,
                    self_form: Some(false),
                    ptys: tm.params.clone(),
                    ret: tm.ret,
                });
            }
            out
        };
        // coverage: every trait method implemented — signature, `is_async`
        // and receiver form matching — and nothing extra. Generic targets
        // (`impl .. for Vec<T>`) stay structural: their parameters only
        // become types at instantiation.
        let dispose_name = self.intern("dispose");
        for req in &reqs {
            let Some((_, mnode)) = mths.iter().find(|(n, _)| *n == req.name) else {
                let tname = self.name(self.trait_by_id(trait_id).name);
                self.err(sp, format!("impl is missing `{}` from {}", self.name(req.name), tname));
                continue;
            };
            let md = self.ast.method_decl(*mnode);
            // the impl side resolves under the target's parameter
            // placeholders too (empty for a concrete target — identical
            // to the old empty env): a parameter-spelled signature
            // (`fn get(self, k: T)`) must not die as an unknown type
            // here; the (skipped-for-generic-targets) equality then
            // compares placeholder against placeholder
            let (self_form, ptys) = self.impl_sig_params(
                &md.params,
                param_env.clone(),
                Some(target_ty),
            );
            let ret = md
                .ret
                .map(|r| self.resolve_sig_ty(r, &param_env, Some(target_ty)))
                .unwrap_or(TY_NIL);
            if md.is_async != req.is_async {
                self.err(
                    self.ast.span(mnode.id()),
                    format!(
                        "`{}` must match the trait's signature — `async` {}",
                        self.name(req.name),
                        if req.is_async { "is required here" } else { "is not allowed here" }
                    ),
                );
            }
            // a descriptor-derived contract (engine/extern trait) spells
            // no receiver form — the impl's own spelling stands (RFC 0012 §2)
            if has_ast && self_form != req.self_form {
                let spell = |f: Option<bool>| match f {
                    Some(true) => "`mut self`".to_string(),
                    Some(false) => "`self`".to_string(),
                    None => "no `self`".to_string(),
                };
                self.err(
                    self.ast.span(mnode.id()),
                    format!(
                        "`{}` must match the trait's receiver —the trait spells {}, the impl spells {}",
                        self.name(req.name),
                        spell(req.self_form),
                        spell(self_form)
                    ),
                );
            }
            // the disposal contract pins its receiver: the engine calls
            // `dispose` on the pinned cell with `mut self` — a `self`
            // (by-value) or receiver-less spelling refuses
            if trait_name == sym::DISPOSAL && req.name == dispose_name && self_form != Some(true) {
                self.err(
                    self.ast.span(mnode.id()),
                    "`dispose` must take `mut self` — the engine calls it on the cell at refcount zero",
                );
            }
            if target_data.is_none() && (ptys != req.ptys || ret != req.ret) {
                // `Self`-spelled trait parameters reach the descriptor as
                // this trait's object type — the impl spells the concrete
                // receiver, and any concrete type satisfies it there
                // (RFC 0012 §4). Extern traits' descriptors are the only
                // source of such reqs (local traits resolve `Self` to the
                // target at the AST).
                let self_obj = |t: &TypeId| {
                    matches!(self.types.kind(*t), TyKind::TraitObj { trait_id: tid } if *tid == trait_id)
                };
                let ptys_match = ptys.len() == req.ptys.len()
                    && ptys.iter().zip(req.ptys.iter()).all(|(a, b)| a == b || self_obj(b));
                if !ptys_match || ret != req.ret {
                    let fmt = |tys: &[TypeId]| tys.iter().map(|t| self.type_name(*t).to_string()).collect::<Vec<_>>().join(", ");
                    self.err(
                        self.ast.span(mnode.id()),
                        format!(
                            "`{}` does not match the trait's signature — trait: ({}) -> {}, impl: ({}) -> {}",
                            self.name(req.name),
                            fmt(&req.ptys),
                            self.type_name(req.ret),
                            fmt(&ptys),
                            self.type_name(ret)
                        ),
                    );
                }
            }
        }
        for (n, mnode) in &mths {
            if !reqs.iter().any(|r| r.name == *n) {
                self.err(
                    self.ast.span(mnode.id()),
                    format!(
                        "`{}` is not a member of {} — inherent methods go in an `impl {} {{ .. }}` block (RFC 0012 §2)",
                        self.name(*n),
                        self.name(self.trait_by_id(trait_id).name),
                        self.type_name(target_ty)
                    ),
                );
            }
        }
        let is_generic = target_data.is_some();
        self.impls.push(ImplDecl {
            trait_id,
            trait_name,
            target: target_ty,
            target_data,
            trait_arg_nodes,
            // a parameterized trait impl (`impl Readable<T> for
            // Source<T>`) — some trait argument bound to a target
            // parameter's placeholder — is the template the dispatch
            // half instantiates per (trait inst, target inst)
            is_template: !param_env.is_empty(),
            inherent: false,
            methods: mths.clone(),
            origin,
        });
        // every impl method enters the monomorphization queue — vtables
        // need their bodies (RFC 0015 §6). Generic-target impls
        // monomorphize per instantiation at their call sites instead.
        if is_generic {
            return;
        }
        let idx = self.impls.len() - 1;
        for (mname, _) in &mths {
            let inst = Inst {
                key: self.impl_method_key(idx, *mname, true),
                subst: vec![],
                trait_origins: vec![],
            };
            self.ensure_inst(inst);
        }
    }

    /// An impl method's receiver form and parameter types (after `self`),
    /// resolved under `env` with `self_ty` spelling `Self`.
    fn impl_sig_params(
        &mut self,
        params: &[NodeHandle<AnyParam>],
        env: Vec<(IdentId, TypeId)>,
        self_ty: Option<TypeId>,
    ) -> (Option<bool>, Vec<TypeId>) {
        let mut self_form: Option<bool> = None;
        let mut ptys = Vec::new();
        for (i, p) in params.iter().enumerate() {
            match self.ast.param(*p) {
                MemberKind::SelfParam(sd) if i == 0 => self_form = Some(sd.is_mut),
                MemberKind::SelfParam(_) => {}
                MemberKind::Param(ParamData { ty: Some(t), .. }) => {
                    ptys.push(self.resolve_sig_ty(*t, &env, self_ty))
                }
                MemberKind::Param(ParamData { ty: None, .. }) => ptys.push(TY_I32),
                _ => ptys.push(TY_I32),
            }
        }
        (self_form, ptys)
    }

    /// The identifier list of a generic argument list whose entries are all
    /// bare type-parameter names (`<T>`, `<T, U>`); `None` otherwise.
    fn ty_generic_idents(&self, generics: &[NodeHandle<AnyTy>]) -> Option<Vec<IdentId>> {
        let mut out = Vec::with_capacity(generics.len());
        for g in generics {
            match self.ast.ty(*g) {
                TypeKind::TyPath { segs, .. } if segs.len() == 1 && segs[0].generics.is_empty() => {
                    out.push(segs[0].name);
                }
                _ => return None,
            }
        }
        Some(out)
    }
}
