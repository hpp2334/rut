//! The pass-2 impl-block walk (`collect_impl`): `impl T { .. }` inherent / trait-impl registration with coverage checks, plus `TraitReq` — what a trait requires of an implementing method.

use super::*;

/// What a trait requires of an implementing method:
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


    /// Pass 2 — an `impl` block: `impl T { .. }` attaches
    /// inherent methods to the target (the type's module only — a local
    /// struct/class, or a `builtin class` this module owns through its
    /// surface); `impl I for T { .. }` registers a trait impl (any
    /// module) after coverage checks, and its methods enter the
    /// monomorphization queue eagerly so vtables carry real ids.
    pub(crate) fn collect_impl(
        &mut self,
        node: NodeId,
        declared: &[IdentId],
        target: NodeHandle<AnyTy>,
        methods: &[NodeHandle<MethodDeclNode>],
    ) {
        let sp = self.ast.span(node);
        // ---- the target: a local struct/class or a local enum (a
        // plain alias expands to its target's decl). Observed
        // capability is satisfied by HAVING the members — nothing
        // registers, so there is no foreign-target form and no
        // orphan gate. A generic target (`impl<T> Vec<T>`) is a
        // template: its methods monomorphize per instantiation
        // through `target_data`.
        //
        // Generic binders are DECLARED after `impl` — the head's bare
        // parameter names are USES; each must resolve against the
        // declared list (the undeclared-name diagnostic names the fix).
        let (target_ty, target_data, spell) = match self.ast.ty(target) {
            TypeKind::TyPath { segs, .. } if segs.len() == 1 => {
                let name = segs[0].name;
                let generics = segs[0].generics.clone();
                match self.ty_path_impl_target(sp, declared, name, generics) {
                    Some(arm) => arm,
                    None => return,
                }
            }
            _ => {
                self.err(
                    sp,
                    "impl target must be a struct, class, or enum of this module — satisfaction attaches members to the type's own decl",
                );
                return;
            }
        };
        let mut mths: Vec<(IdentId, NodeHandle<MethodDeclNode>)> = Vec::new();
        for m in methods {
            mths.push((self.ast.method_decl(*m).name, *m));
        }
        // the placement law (phase 3): an inherent impl lives in the
        // type's module — literally, by path now. A flat package has
        // one module, so every impl is where the type is, exactly as
        // before.
        let type_home = self
            .datas
            .iter()
            .find(|(n, _)| *n == spell)
            .map(|(_, d)| self.mod_of(d.node.id()).to_string())
            .or_else(|| {
                self.enums
                    .iter()
                    .find(|(n, _)| *n == spell)
                    .map(|(_, e)| self.mod_of(e.node).to_string())
            });
        if let Some(home) = type_home {
            if home != self.cur_mod {
                self.err(
                    sp,
                    format!(
                        "an inherent impl for `{}` must live in `{}`'s module ({}) — this impl is in {}",
                        self.name(spell),
                        self.name(spell),
                        ModInputs::display(&home),
                        ModInputs::display(&self.cur_mod),
                    ),
                );
                return;
            }
        }
        // the impl block's owner — the instantiation ledger's owner
        // anchor for its monomorphized methods. Every impl in the unit
        // is the unit's own (no splicing), so this is the unit's spec.
        let origin = self.own_spec.clone();
        self.collect_impl_inherent(spell, target_ty, target_data, mths, origin)
    }
    /// The TyPath impl target, resolved: a plain alias expands to its
    /// target's decl; then the ordinary chain — a local struct/class
    /// (generic ⇒ template row) or a local enum. Everything else is
    /// diagnosed: satisfaction is having the type's own members, so
    /// used types, primitives, and engine classes take no impl blocks
    /// here. Returns the target's type id, its generic template when
    /// generic, and the decl name the inherent path attaches under
    /// (`spell` — the expansion's decl, not the alias spelling).
    /// `None` = diagnosed, abort.
    fn ty_path_impl_target(
        &mut self,
        sp: rut_lexer::span::Span,
        declared: &[IdentId],
        name: IdentId,
        generics: Vec<NodeHandle<AnyTy>>,
    ) -> Option<(TypeId, Option<(IdentId, Vec<IdentId>)>, IdentId)> {
        // a PLAIN alias expands in impl-target position: the alias
        // spells the target's type.
        if generics.is_empty() {
            if let Some(t) = self.plain_alias_target(name) {
                if let Some((dname, d)) = self.datas.iter().find(|(_, d)| d.ty == t).cloned().map(|(n, d)| (n, d)) {
                    return Some((d.ty, None, dname));
                }
            }
        }
        if let Some(d) = self.find_data(name).cloned() {
            if d.generics.is_empty() {
                Some((d.ty, None, name))
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
                if let Some(bad) = self.undeclared_binder(declared, &params) {
                    self.err(sp, format!(
                        "undeclared type parameter `{}` — declare it: `impl<{}> ..`",
                        self.name(bad),
                        self.name(bad),
                    ));
                    return None;
                }
                Some((d.ty, Some((name, params)), name))
            }
        } else if let Some(e) = self.find_enum(name).cloned() {
            // a local enum: concrete and non-generic — inherent
            // methods attach exactly as a struct's
            if !generics.is_empty() {
                self.err(sp, format!("`{}` takes no generic arguments", self.name(name)));
                return None;
            }
            Some((e.ty, None, name))
        } else if self.extern_native_types.contains_key(&name) {
            self.err(sp, "a builtin class takes no impl blocks — its members are the engine's closed contract (markers and impls live on user struct and class decls)");
            None
        } else if self.extern_types.contains_key(&name) || self.extern_generics.contains_key(&name) {
            self.err(sp, "inherent impls live in the type's module — a used type is satisfied where it was declared");
            None
        } else if sym::primitive_ty(name).is_some() {
            self.err(sp, "a primitive takes no impl blocks — capability on a primitive is manufactured by spelling a wrapper (a newtype class) whose inherent impl carries the members");
            None
        } else {
            self.err(
                sp,
                "impl target must be a struct, class, or enum of this module",
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
    pub fn node_is_known_type(&self, node: NodeHandle<AnyTy>) -> bool {
        let Some(n) = self.node_head_ident(node) else { return true };
        sym::primitive_ty(n).is_some()
            || self.find_enum(n).is_some()
            || self.find_data(n).is_some()
            || self.find_iface(n).is_some()
            || self.find_alias(n).is_some()
            || self.extern_native_types.contains_key(&n)
            || self.extern_types.contains_key(&n)
    }


    /// `impl T { .. }` — inherent methods. Local targets attach into the
    /// `DataDecl`; native builtin-class targets register an inherent
    /// `ImplDecl` (dispatch resolves through the native shape). Both
    /// dispatch statically — the receiver's concrete type names the impl.
    fn collect_impl_inherent(
        &mut self,
        spell: IdentId,
        target_ty: TypeId,
        target_data: Option<(IdentId, Vec<IdentId>)>,
        mths: Vec<(IdentId, NodeHandle<MethodDeclNode>)>,
        _origin: String,
    ) {
        // `spell` is the target's DECL name (the expansion's decl when
        // the impl was spelled through an alias), resolved by
        // `ty_path_impl_target`
        let tname = spell;
        // local struct/class: methods attach to the decl, where the
        // ordinary inherent-call machinery finds them
        if let Some(idx) = self.datas.iter().position(|(n, _)| *n == tname) {
            // the MARKER validation (v20): the closed set, one per
            // contract per class, signature per contract, concrete
            // record targets only — the engine's contracts live here
            // since the `builtin trait` rows died
            let vsp = mths.first().map(|(_, m)| self.ast.span(m.id())).unwrap_or(self.ast.span(self.ast.root.id()));
            self.validate_markers(tname, &mths, target_data, target_ty, vsp);
            for (n, mnode) in &mths {
                if self.datas[idx].1.methods.iter().any(|(pn, _)| pn == n) {
                    self.err(
                        self.ast.span(mnode.id()),
                        format!("duplicate method `{}` on `{}`", self.name(*n), self.name(tname)),
                    );
                }
            }
            // the engine calls a `[disposal]` member with NO call site:
            // its body queues eagerly, so the per-type disposal row
            // finds a compiled fn (a `[iterable]` member binds at its
            // for-of sites like any method)
            let disposal = sym::DISPOSAL_MARKER;
            let marked: Vec<IdentId> = mths
                .iter()
                .filter(|(_, m)| self.ast.method_decl(*m).marker == Some(disposal))
                .map(|(n, _)| *n)
                .collect();
            if std::env::var("RUT_DEBUG_SATISFY").is_ok() {
                eprintln!("DBG attach: {} methods {:?} at collect", self.name(tname), mths.iter().map(|(n, _)| self.name(*n).to_string()).collect::<Vec<_>>());
            }
            // the `[constructor]` designation registers here — the
            // 1-seg call's fallback reads it (`Point(..)` lowers exactly
            // like `Point.<member>(..)`); a call-site consumer, so no
            // eager queue and no engine row
            let constructor = sym::CONSTRUCTOR_MARKER;
            if let Some((cn, _)) = mths
                .iter()
                .find(|(_, m)| self.ast.method_decl(*m).marker == Some(constructor))
            {
                self.class_ctors.insert(tname, *cn);
            }
            self.datas[idx].1.methods.extend(mths);
            for n in marked {
                self.ensure_inst(Inst {
                    key: FnKey::Method { data: tname, name: n },
                    subst: vec![],
                    iface_origins: vec![],
                });
            }
            return;
        }
        // a local enum: the same attach on the enum's own decl slot —
        // statics and self methods alike, the struct rule
        if let Some(idx) = self.enums.iter().position(|(n, _)| *n == tname) {
            // the marker walk for enums: `[iterable]` is legal (an
            // enum VALUE iterates like a class's — for-of drives the
            // designated member), but `[disposal]` is not — an enum's
            // members are immortal singletons, never released, and an
            // unknown word dies here too (the closed set's other gate)
            for (_n, mnode) in &mths {
                let md = self.ast.method_decl(*mnode);
                let Some(word) = md.marker else { continue };
                if word == sym::DISPOSAL_MARKER {
                    self.err(self.ast.span(mnode.id()), format!(
                        "`{}` cannot carry `[disposal]` — only a struct or class can (the engine disposes a record's cell; an enum's members are immortal singletons)",
                        self.name(tname)
                    ));
                } else if word == sym::CONSTRUCTOR_MARKER {
                    self.err(self.ast.span(mnode.id()), format!(
                        "`{}` cannot carry `[constructor]` — only a class can (an enum's members are its own immortal singletons; there is no call form to designate)",
                        self.name(tname)
                    ));
                } else if word != sym::ITERABLE_MARKER {
                    self.err(self.ast.span(mnode.id()), format!(
                        "`[{}]` is not a designated surface — the closed marker set is `[disposal]`, `[iterable]`, and `[constructor]`",
                        self.name(word)
                    ));
                }
            }
            for (n, mnode) in &mths {
                if self.enums[idx].1.methods.iter().any(|(pn, _)| pn == n) {
                    self.err(
                        self.ast.span(mnode.id()),
                        format!("duplicate method `{}` on `{}`", self.name(*n), self.name(tname)),
                    );
                }
            }
            self.enums[idx].1.methods.extend(mths);
        }
    }

    /// The bracket markers' CHECKER half (v20): the parser accepts any
    /// `[word]`, this validates the DESIGNATED SURFACES' closed set and
    /// each surface's law.
    ///
    /// - the set: `[disposal]`, `[iterable]`, `[constructor]` — the
    ///   three designated surfaces, nothing else marks;
    /// - at most one per surface per class (a second `[disposal]`
    ///   member has no slot to fill; a second `[constructor]` leaves
    ///   `Type(..)` ambiguous — the duplicate names both members);
    /// - inherent-members-only (the parser already rejects the trait /
    ///   trait-impl spellings; the builtin-class targets never reach
    ///   here — they diagnosed above);
    /// - the signature per surface: `[disposal] fn <free>(mut self,
    ///   cx: DisposalContext)` — the name is FREE, the bracket
    ///   designates; the engine calls it at refcount zero, so the
    ///   target must be a CONCRETE struct/class (the row keys the
    ///   cell's type id; a generic target has no static row);
    ///   `[iterable] fn <free>(self, emit: fn(E) -> bool)` — the
    ///   element type falls out of the marked member's own signature;
    ///   `[constructor] fn <free>(..) -> Self | ?Self` — class targets
    ///   only, no receiver: the call form `Type(..)` lowers exactly
    ///   like `Type.<member>(..)` (a call-site consumer, not an engine
    ///   row), so the visibility seal rides the member's `pub` and a
    ///   `?Self` return makes the call a try-construction. A newtype
    ///   never carries it — the newtype's `Name(v)` surface already IS
    ///   the construction.
    fn validate_markers(
        &mut self,
        tname: IdentId,
        mths: &[(IdentId, NodeHandle<MethodDeclNode>)],
        target_data: Option<(IdentId, Vec<IdentId>)>,
        target_ty: TypeId,
        sp: rut_lexer::span::Span,
    ) {
        let disposal = sym::DISPOSAL_MARKER;
        let iterable = sym::ITERABLE_MARKER;
        let constructor = sym::CONSTRUCTOR_MARKER;
        // the target's own type parameters bind as template placeholders
        // for the signature checks — a `[iterable]` member on a generic
        // class (`impl<E> Flow<E>`) spells the element type with the
        // class's binder (`emit: fn(E) -> bool`), and the concrete type
        // falls out at instantiation, exactly like any template method's
        let param_env: Vec<(IdentId, TypeId)> = match target_data.as_ref() {
            Some((_, params)) => {
                params.iter().map(|p| (*p, self.param_placeholder(*p))).collect()
            }
            None => vec![],
        };
        for (n, mnode) in mths {
            let md = self.ast.method_decl(*mnode);
            let Some(word) = md.marker else { continue };
            if word != disposal && word != iterable && word != constructor {
                self.err(
                    self.ast.span(mnode.id()),
                    format!(
                        "`[{}]` is not a designated surface — the closed marker set is `[disposal]`, `[iterable]`, and `[constructor]`",
                        self.name(word)
                    ),
                );
                continue;
            }
            if md.is_async {
                self.err(
                    self.ast.span(mnode.id()),
                    format!("`[{}]` marks a synchronous contract — `async` is not allowed here", self.name(word)),
                );
            }
            // at most one per surface per class — the decl's existing
            // methods count too (an earlier impl block may have marked)
            let already = self
                .find_data(tname)
                .map(|d| {
                    d.methods
                        .iter()
                        .any(|(pn, pm)| *pn != *n && self.ast.method_decl(*pm).marker == Some(word))
                })
                .unwrap_or(false)
                || mths.iter().any(|(pn, pm)| {
                    pn != n && self.ast.method_decl(*pm).marker == Some(word)
                });
            if already {
                if word == constructor {
                    // the duplicate names BOTH member spellings — the
                    // call form `Type(..)` has to pick one
                    let other = self
                        .find_data(tname)
                        .and_then(|d| {
                            d.methods
                                .iter()
                                .find(|(pn, pm)| *pn != *n && self.ast.method_decl(*pm).marker == Some(word))
                                .map(|(pn, _)| *pn)
                        })
                        .or_else(|| {
                            mths.iter()
                                .find(|(pn, pm)| pn != n && self.ast.method_decl(*pm).marker == Some(word))
                                .map(|(pn, _)| *pn)
                        });
                    match other {
                        Some(o) => self.err(
                            self.ast.span(mnode.id()),
                            format!(
                                "`{}` already carries a `[constructor]` member — `{}` and `{}` both designate the construction surface; at most one per class (`{}(..)` must lower to one member)",
                                self.name(tname),
                                self.name(o),
                                self.name(*n),
                                self.name(tname)
                            ),
                        ),
                        None => self.err(
                            self.ast.span(mnode.id()),
                            format!(
                                "`{}` already carries a `[constructor]` member — at most one per class (`{}(..)` must lower to one member)",
                                self.name(tname),
                                self.name(tname)
                            ),
                        ),
                    }
                } else {
                    self.err(
                        self.ast.span(mnode.id()),
                        format!(
                            "`{}` already carries a `[{}]` member — at most one per contract per class (the engine's slot is singular)",
                            self.name(tname),
                            self.name(word)
                        ),
                    );
                }
            }
            match word {
                w if w == disposal => {
                    if target_data.is_some() {
                        self.err(
                            sp,
                            format!(
                                "`{}` cannot carry `[disposal]` through a generic target — implement it for the concrete struct or class (the engine's row keys the cell's type id)",
                                self.name(tname)
                            ),
                        );
                        continue;
                    }
                    if !matches!(self.types.kind(target_ty), TyKind::Data { .. }) {
                        self.err(
                            sp,
                            format!(
                                "`{}` cannot carry `[disposal]` — only a struct or class can (the engine calls the member on the record's cell at refcount zero)",
                                self.name(tname)
                            ),
                        );
                        continue;
                    }
                    // signature: `(mut self, cx: DisposalContext)` -> nil
                    let mut params = md.params.iter();
                    let self_form = match params.next().map(|p| self.ast.param(*p)) {
                        Some(MemberKind::SelfParam(sd)) => Some(sd.is_mut),
                        _ => None,
                    };
                    if self_form != Some(true) {
                        self.err(
                            self.ast.span(mnode.id()),
                            "a `[disposal]` member takes `mut self` first — the engine calls it on the pinned cell",
                        );
                    }
                    let cx_ok = match params.next().map(|p| self.ast.param(*p)) {
                        Some(MemberKind::Param(ParamData { ty: Some(t), .. })) => {
                            self.resolve_sig_ty(*t, &param_env, None) == TY_DISPOSAL_CONTEXT
                        }
                        _ => false,
                    };
                    if !cx_ok {
                        self.err(
                            self.ast.span(mnode.id()),
                            "a `[disposal]` member's second parameter is `cx: DisposalContext` — the engine mints it per call",
                        );
                    }
                    if params.next().is_some() {
                        self.err(
                            self.ast.span(mnode.id()),
                            "a `[disposal]` member takes exactly `(mut self, cx: DisposalContext)`",
                        );
                    }
                    if md.ret.map(|r| self.resolve_sig_ty(r, &param_env, None)).unwrap_or(TY_NIL) != TY_NIL {
                        self.err(
                            self.ast.span(mnode.id()),
                            "a `[disposal]` member returns nil — the engine ignores any answer",
                        );
                    }
                    // (the engine calls the member with NO call site;
                    // the eager queue happened in collect_impl_inherent)
                }
                w if w == iterable => {
                    // signature: `(self, emit: fn(E) -> bool)` -> nil —
                    // the element type falls out of the emit parameter
                    let mut params = md.params.iter();
                    let has_self = matches!(params.next().map(|p| self.ast.param(*p)), Some(MemberKind::SelfParam(_)));
                    if !has_self {
                        self.err(
                            self.ast.span(mnode.id()),
                            "an `[iterable]` member takes a receiver — `fn <name>(self, emit: fn(E) -> bool)`",
                        );
                    }
                    let emit_ok = match params.next().map(|p| self.ast.param(*p)) {
                        Some(MemberKind::Param(ParamData { ty: Some(t), .. })) => {
                            {
                                let et = self.resolve_sig_ty(*t, &param_env, None);
                                // a fn type names the element directly; a
                                // target parameter (`emit: fn(E) -> bool`
                                // under `impl<E>`) falls out at
                                // instantiation
                                matches!(self.types.kind(et), TyKind::Fn { .. })
                                    || param_env.iter().any(|(_, ph)| *ph == et)
                            }
                        }
                        _ => false,
                    };
                    if !emit_ok {
                        self.err(
                            self.ast.span(mnode.id()),
                            "an `[iterable]` member's second parameter is the emit callback — `emit: fn(E) -> bool` (`false` stops the iteration; the element type falls out of the marked member's signature)",
                        );
                    }
                    if params.next().is_some() {
                        self.err(
                            self.ast.span(mnode.id()),
                            "an `[iterable]` member takes exactly `(self, emit: fn(E) -> bool)`",
                        );
                    }
                    if md.ret.map(|r| self.resolve_sig_ty(r, &param_env, None)).unwrap_or(TY_NIL) != TY_NIL {
                        self.err(
                            self.ast.span(mnode.id()),
                            "an `[iterable]` member returns nil — the drive consumes through `emit`",
                        );
                    }
                }
                w if w == constructor => {
                    // NEWTYPES NEVER CARRY THE MARKER (Law 2a): the
                    // newtype's call surface (`Name(v)`) already IS the
                    // construction — the compile_newtype_ctor mint — and
                    // nothing designates it. Cover the extern-newtype
                    // spelling too (a used type takes no impl block, so
                    // the ordinary path never gets here; the gate keeps
                    // the law in one place).
                    let is_newtype = self
                        .find_data(tname)
                        .map(|d| d.newtype)
                        .unwrap_or(false)
                        || self.extern_newtypes.contains(&tname);
                    if is_newtype {
                        self.err(
                            sp,
                            format!(
                                "`{}` cannot carry `[constructor]` — the newtype's call surface (`{0}(v)`) already IS the construction; there is nothing to designate",
                                self.name(tname)
                            ),
                        );
                        continue;
                    }
                    // CLASS targets only (Law 4): a struct constructs by
                    // literal, an enum's members are its own singletons
                    match self.find_data(tname).map(|d| d.kind) {
                        Some(crate::check::DataKind::Class) => {}
                        Some(crate::check::DataKind::Struct) => {
                            self.err(
                                sp,
                                format!(
                                    "`{}` cannot carry `[constructor]` — structs construct by literal (`{0} {{ .. }}`); the marker designates a class's call form",
                                    self.name(tname)
                                ),
                            );
                            continue;
                        }
                        None => {
                            self.err(
                                sp,
                                format!(
                                    "`{}` cannot carry `[constructor]` — only a class can (the marker designates the `{}(..)` call form)",
                                    self.name(tname),
                                    self.name(tname)
                                ),
                            );
                            continue;
                        }
                    }
                    // signature: NO receiver (the class itself is the
                    // call's subject) and the return is `Self` or `?Self`
                    if matches!(
                        md.params.first().map(|p| self.ast.param(*p)),
                        Some(MemberKind::SelfParam(_))
                    ) {
                        self.err(
                            self.ast.span(mnode.id()),
                            "a `[constructor]` member takes no receiver — `fn <free>(..) -> Self` (the call `Type(..)` spells the class, not a value)",
                        );
                    }
                    // the return resolves against the target's own
                    // binders as placeholders — `Self` binds the target
                    // type, `?Self` its nullable (the try-construction)
                    let ret_ok = md
                        .ret
                        .map(|r| {
                            let t = self.resolve_sig_ty(r, &param_env, Some(target_ty));
                            t == target_ty || t == self.mk_opt(target_ty)
                        })
                        .unwrap_or(false);
                    if !ret_ok {
                        self.err(
                            self.ast.span(mnode.id()),
                            "a `[constructor]` member returns `Self` (or `?Self` — the try-construction: `Type(..)` then yields `?Type`)",
                        );
                    }
                    // (the call sites lower the member like any class
                    // method — `Type(..)` IS `Type.<member>(..)`; the
                    // designation registry rides collect_impl_inherent
                    // below, no eager queue, no engine row)
                }
                _ => {}
            }
        }
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

    /// The first head parameter name that is neither in the impl's
    /// declared binder list nor a known type spelling — the
    /// explicit-impl-generics law's check. A KNOWN type name (`i32`,
    /// `str`, a decl in scope) is a concrete argument, not a binder;
    /// an unknown bare name is the undeclared-binder diagnostic.
    /// `None` when the head resolves.
    fn undeclared_binder(&self, declared: &[IdentId], params: &[IdentId]) -> Option<IdentId> {
        params
            .iter()
            .copied()
            .find(|p| !declared.contains(p) && !self.name_is_known_type(*p))
    }

    /// Is this bare name a known type in scope? The impl-row matcher's
    /// own law, factored for the binder check.
    fn name_is_known_type(&self, n: IdentId) -> bool {
        sym::primitive_ty(n).is_some()
            || self.find_enum(n).is_some()
            || self.find_data(n).is_some()
            || self.find_iface(n).is_some()
            || self.find_alias(n).is_some()
            || self.extern_native_types.contains_key(&n)
            || self.extern_types.contains_key(&n)
            || self.extern_generics.contains_key(&n)
    }
}
