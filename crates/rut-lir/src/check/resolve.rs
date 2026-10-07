//! Type resolution: primitives, builtins (Array/Option/Result), user types
//! (including `pouch`'s `Vec` class), interface objects, fn types;
//! naming-position resolution.

use super::*;

impl<'a> Ctx<'a> {

    /// Mint (or fetch) the CONCRETE interface instantiation of an
    /// impl's member set under a target substitution — the template
    /// re-resolution law's mint half (the dispatch half that
    /// per-instantiation consumers run: the for-of weave, the vtable
    /// fills). A local interface instantiates from its AST. The
    /// `iface_inst` cache is shared, so an instantiation the ordinary
    /// interface resolution minted earlier is fetched, never duplicated. (The native contracts are
    /// gone — the markers are designated member slots, not ifaces — so
    /// there is no `Iterable` arm anymore.)
    pub fn mint_impl_iface_inst(&mut self, name: IdentId, args: Vec<TypeId>) -> u32 {
        // a used module's exported GENERIC trait: the carried descriptor
        // mints the instantiation (no local AST exists)
        if self.extern_trait(name).is_some() {
            return self.mint_extern_iface_inst(name, args);
        }
        self.mk_iface_inst(name, args)
    }

    /// The FOREIGN generic trait's mint half: one descriptor per
    /// type-argument list, built out of the CARRIED placeholder
    /// descriptor (the declaration crossed the used module's surface
    /// with its signatures spelling the trait's parameters as `#<param>`
    /// placeholder rows). The substitution maps the placeholder leaves
    /// to the arguments by the crossing template law — the k-th DISTINCT
    /// leaf in signature walk order is the trait's k-th generic, the
    /// same first-appearance reading the dispatch sites run
    /// (`descriptor_leaf_env`) — and the trait's own object leaves
    /// (`?Self`-spelled parameters) re-spell to the NEW instantiation's
    /// object. Cached in `iface_inst` alongside the local mints, so a
    /// spelling and an impl head land on one descriptor.
    pub fn mint_extern_iface_inst(&mut self, name: IdentId, args: Vec<TypeId>) -> u32 {
        if let Some(&id) = self.iface_inst.get(&(name, args.clone())) {
            return id;
        }
        let base_id = self
            .extern_trait(name)
            .map(|e| e.id)
            .unwrap_or(u32::MAX);
        let base_text = self.interner.name(name).to_string();
        self.mint_iface_inst_from_desc(base_id, &base_text, args, Some(name))
    }

    /// The mint core, keyed by the BASE DESCRIPTOR's table row (not a
    /// name): the use-both gate leaves interfaces the consumer never
    /// NAMED registered namelessly (`add_extern_iface_decl`'s `None`
    /// binding) — a signature's interface-typed param still carries the
    /// descriptor id, and the substitution re-mints from it directly.
    /// `base_text` formats the applied name; `cache_name` (when the
    /// name IS bound) keys the `iface_inst` dedup cache.
    pub(crate) fn mint_iface_inst_from_desc(
        &mut self,
        base_id: u32,
        base_text: &str,
        args: Vec<TypeId>,
        cache_name: Option<IdentId>,
    ) -> u32 {
        let id = self.ifaces.len() as u32;
        let tname = if args.is_empty() {
            base_text.to_string()
        } else {
            format!(
                "{}<{}>",
                base_text,
                args.iter().map(|a| self.type_name(*a).to_string()).collect::<Vec<_>>().join(", ")
            )
        };
        // the applied-NAME dedup runs FIRST for EVERY caller: the
        // nameless lane (the use-both gate — the consumer never bound
        // the interface's name) and the name-keyed lane must land on
        // ONE descriptor, or a fill demanded through one spelling
        // dispatches empty through the other
        {
            let wanted = &tname;
            if let Some(existing) = self
                .ifaces
                .iter()
                .enumerate()
                .find(|(_, d)| self.interner.name(d.name) == wanted)
                .map(|(i, _)| i as u32)
            {
                return existing;
            }
        }
        if let Some(name) = cache_name {
            if let Some(&id) = self.iface_inst.get(&(name, args.clone())) {
                return id;
            }
        }
        let tname = self.intern(&tname);
        self.ifaces.push(IfaceDesc { name: tname, methods: vec![] });
        if let Some(name) = cache_name {
            self.iface_inst.insert((name, args.clone()), id);
        }
        let Some(base) = (base_id != u32::MAX)
            .then(|| self.ifaces.get(base_id as usize).cloned())
            .flatten()
        else {
            return id;
        };
        let leaves = self.carried_trait_leaves(&base);
        let mut env: std::collections::HashMap<String, TypeId> = std::collections::HashMap::new();
        for (i, leaf) in leaves.iter().enumerate() {
            if let Some(&a) = args.get(i) {
                env.insert(leaf.clone(), a);
            }
        }
        let mut methods = Vec::with_capacity(base.methods.len());
        for tm in &base.methods {
            let params = tm
                .params
                .iter()
                .map(|&p| self.subst_carried_sig(p, base_id, id, &env))
                .collect();
            let ret = self.subst_carried_sig(tm.ret, base_id, id, &env);
            methods.push(rut_core::binary::IfaceMethod { name: tm.name, params, ret });
        }
        self.ifaces[id as usize].methods = methods;
        id
    }

    /// One carried signature under the mint's substitution: the
    /// placeholder leaves rebuild structurally (`subst_template_ty` —
    /// the shared crossing substitute), and a `?Self` leaf (the BASE
    /// placeholder descriptor's object type, at any depth) re-spells to
    /// the minted instantiation's object — the impl-side law's mint
    /// twin (`resolve_iface_sig_ty` spells `Self` the same way for a
    /// local trait).
    fn subst_carried_sig(
        &mut self,
        t: TypeId,
        base_id: u32,
        minted: u32,
        env: &std::collections::HashMap<String, TypeId>,
    ) -> TypeId {
        let s = self.subst_template_ty(t, env);
        if matches!(self.types.kind(s), TyKind::IfaceObj { iface_id } if *iface_id == base_id) {
            return self.mk_iface_obj(minted);
        }
        s
    }

    /// The element spelling inside a trait-inst NAME: boot optionals'
    /// rows ride the shell convention (the `?bytes` row is named
    /// "bytes"), and the name is the element's only carrier across a
    /// binding — an optional element must spell the `?` or the
    /// optionality is lost.
    fn elem_spelling(&self, arg: TypeId) -> String {
        let n = self.type_name(arg).to_string();
        if matches!(self.types.kind(arg), TyKind::Opt { .. }) && !n.starts_with('?') {
            return format!("?{n}");
        }
        n
    }

    /// The `Future<T>` protocol contract: one
    /// trait per type-argument list, its single member `yield(cx)`. The
    /// engine weaves impls for async fn frames; user impls register
    /// through the same trait (launcher-drivable). Mirrors
    /// `mk_iterator_inst` — the descriptor's params exclude the
    /// receiver, which the vtable ABI always supplies as argv[0].
    ///
    /// v20: NOT a user-visible trait anymore — the row is the internal
    /// designated-slot allocation for the closed `Future<T>` class (the
    /// "class layout binding"): every `Future<elem>` instantiation owns
    /// ONE global yield slot, and every engine-minted frame over that
    /// elem fills it (`extra_vtable_fills`). No surface row exists;
    /// nothing in source can name or implement it.
    pub fn mk_future_inst(&mut self, name: IdentId, arg: TypeId) -> u32 {
        if let Some(&id) = self.iface_inst.get(&(name, vec![arg])) {
            return id;
        }
        let id = self.ifaces.len() as u32;
        let tname = self.intern(&format!("Future<{}>", self.elem_spelling(arg)));
        let cx = self.run_context_ty();
        self.ifaces.push(IfaceDesc {
            name: tname,
            methods: vec![rut_core::binary::IfaceMethod {
                name: sym::YIELD,
                params: vec![cx],
                ret: TY_NIL,
            }],
        });
        self.iface_inst.insert((name, vec![arg]), id);
        id
    }

    /// The `Future<T>` CLOSED builtin class: the ONE surface spelling of
    /// every async producer's answer (`async fn` calls, `async { }`
    /// blocks, `sleep`, the select/completer mints). Interned per
    /// instantiation like `mk_weak`; the slot row above is its layout
    /// binding.
    pub fn mk_future(&mut self, elem: TypeId) -> TypeId {
        // the designated slot row exists FIRST — awaiting this type
        // aims its `CallI` through it, and `mk_future_inst` dedups
        self.mk_future_inst(sym::FUTURE, elem);
        let name = self.intern(&format!("Future<{}>", self.elem_spelling(elem)));
        self.types.intern(RutType {
            name,
            kind: TyKind::Future { elem },
        })
    }

    /// The impl-head / naming-position diagnostic for a spelled engine
    /// contract: the `builtin trait` row kind is gone, and each of the
    /// four names has exactly one replacement.
    pub fn engine_contract_head_error(&self, name: IdentId) -> Option<String> {
        if name == sym::ITERABLE {
            return Some(
                "`Iterable` is gone — mark the member: `impl T { [iterable] fn iterate(self, emit: fn(E) -> bool) { .. } }` (the element type falls out of the marked member's signature)".to_string(),
            );
        }
        if name == sym::DISPOSAL {
            return Some(
                "`Disposal` is gone — mark the member: `impl T { [disposal] fn dispose_db(mut self, cx: DisposalContext) { .. } }` (the engine calls it at refcount zero; the name is free, the bracket designates)".to_string(),
            );
        }
        if name == sym::FUTURE {
            return Some(
                "`Future` is a closed builtin class — futures are engine-minted (an `async fn` call, an `async { }` block, `sleep`, the select/completer mints); a user type cannot be one".to_string(),
            );
        }
        if name == sym::RUN_CONTEXT {
            return Some(
                "`RunContext` is a closed builtin class — the weave injects it as `cx` into every async body; a user type cannot be one".to_string(),
            );
        }
        None
    }


    /// The engine-minted `RunContext` cx record: one field —
    /// the frame edge — laid out per `rut_core::async_frame`. The NAME
    /// is the surface spelling (v20: the closed `builtin class` row),
    /// so the weave's injected `cx` binding and the class's members all
    /// land on this one type; its members
    /// inline as field ops in the weave and in user bodies alike.
    pub fn run_context_ty(&mut self) -> TypeId {
        if let Some(t) = self.run_context_ty {
            return t;
        }
        let name = self.intern(rut_core::async_frame::RUN_CONTEXT_TYPE);
        let fname = self.intern(rut_core::async_frame::RUN_CONTEXT_FRAME_FIELD);
        // the cx singleton is THIS unit's own engine row — interned into
        // the own block, never deduplicating against a used block's
        // carried copy (a linked pkg's surface carries its own minting;
        // the crossing re-spells at the binding instead)
        let ty = self.types.intern_own(RutType {
            name,
            kind: TyKind::Data { fields: vec![FieldInfo { name: fname, ty: TY_OPAQUE }] },
        });
        self.run_context_ty = Some(ty);
        ty
    }

    /// A type in a NAMING position (impl heads, requires lists, is RHS) —
    /// bare trait name → the trait's object type here means the TYPE;
    /// for `is` RHS we keep trait vs concrete distinction in the compiler.
    pub fn resolve_naming_type(&mut self, node: NodeHandle<AnyTy>) -> TypeId {
        self.resolve_type(node, &[])
    }

    /// Resolve a type AST node into the type table. `env` binds generic
    /// params of the enclosing instantiation.
    pub fn resolve_type(&mut self, node: NodeHandle<AnyTy>, env: &[(IdentId, TypeId)]) -> TypeId {
        let sp = self.ast.span(node.id());
        match self.ast.ty(node) {
            TypeKind::TyFn { params, ret } => {
                let mut ptys = Vec::new();
                for p in params {
                    ptys.push(self.resolve_type(*p, env));
                }
                let rty = self.resolve_type(*ret, env);
                self.mk_fn_ty(ptys, rty)
            }
            TypeKind::TyOpt { inner } => {
                let elem = self.resolve_type(*inner, env);
                self.mk_opt(elem)
            }
            TypeKind::TyArray { elem } => {
                // `[T]` — the array type: grammar-spelled,
                // resolved directly, no surface name behind it
                let elem = self.resolve_type(*elem, env);
                self.mk_array(elem)
            }
            TypeKind::TyTuple { elems } => {
                let mut etys = Vec::new();
                for e in elems {
                    etys.push(self.resolve_type(*e, env));
                }
                self.mk_tuple(etys)
            }
            TypeKind::TyConst(_) => {
                self.err(sp, "a const expression is not a type here");
                TY_I32
            }
            TypeKind::TyUnion { .. } => {
                // a union in a value position (bound-only)
                self.err(
                    sp,
                    "a union type is bound-only — unions are legal only in `requires` bounds and union aliases",
                );
                TY_I32
            }
            TypeKind::TyPath { segs, .. } => {
                if segs.len() > 1 {
                    self.err(sp, format!("unknown type `{}`", seg_str(self, segs)));
                    return TY_I32;
                }
                let seg = &segs[0];
                let name = seg.name;
                // generic param?
                if seg.generics.is_empty() {
                    if let Some((_, t)) = env.iter().find(|(p, _)| *p == name) {
                        return *t;
                    }
                }
                // primitives & builtins — names compare as symbols
                let prim = sym::primitive_ty(name);
                if let Some(p) = prim {
                    if !seg.generics.is_empty() {
                        self.err(sp, format!("`{}` takes no generic arguments", self.name(name)));
                    }
                    return p;
                }
                // `Weak<T>` resolves like every builtin now —
                // the pre-weak M5 stub ("not supported in this build") that
                // pre-reserved the name is gone; the gated native types
                // (`Weak`, `DisposalContext`) reach the `core_ty` match
                // below the same way every row here does: only once the
                // unit's `use` bound the name (the ambient rows —
                // `Opaque`, `StackTrace` — bind in every unit without it).
                // a used core builtin container: the
                // prelude is used, never ambient — `Opaque`
                // resolves only when the name was bound from the core
                // surface (`[T]` never reaches here — it is `TyArray`)
                let core_ty = self.extern_native_types.get(&name).copied();
                // a declared or used type shadows a builtin name (RFC
                // 0005: `pouch`'s `Vec` is an ordinary class, so it
                // never reaches the builtin table)
                let shadow = name == sym::OPAQUE
                    && (self.find_data(name).is_some() || self.extern_types.contains_key(&name));
                if let Some(kind) = core_ty.filter(|_| !shadow) {
                    let generics = seg.generics.clone();
                    return match (kind, generics.as_slice()) {
                        (rut_core::binary::NativeTy::Opaque, []) => TY_OPAQUE,
                        (rut_core::binary::NativeTy::Opaque, _) => {
                            self.err(sp, "`opaque` takes no generic arguments");
                            TY_I32
                        }
                        (rut_core::binary::NativeTy::StackTrace, []) => TY_STACK_TRACE,
                        (rut_core::binary::NativeTy::StackTrace, _) => {
                            self.err(sp, "`StackTrace` takes no generic arguments");
                            TY_I32
                        }
                        // the disposal drain's context cell: the engine mints
                        // it — the name resolves in type position (a
                        // `[disposal]` body's `cx` parameter), nothing
                        // constructs it
                        (rut_core::binary::NativeTy::DisposalContext, []) => TY_DISPOSAL_CONTEXT,
                        (rut_core::binary::NativeTy::DisposalContext, _) => {
                            self.err(sp, "`DisposalContext` takes no generic arguments");
                            TY_I32
                        }
                        // the TWO generic builtins — `Weak<T>` and (v20)
                        // the closed `Future<T>` — intern per
                        // instantiation (`mk_weak` / `mk_future`, the
                        // `mk_array` law). Exactly one parameter each.
                        (rut_core::binary::NativeTy::Weak, [g]) => {
                            let elem = self.resolve_type(*g, env);
                            self.mk_weak(elem)
                        }
                        (rut_core::binary::NativeTy::Weak, _) => {
                            self.err(sp, "`Weak<T>` takes exactly one type parameter — the weak-referenced type");
                            TY_I32
                        }
                        (rut_core::binary::NativeTy::Future, [g]) => {
                            let elem = self.resolve_type(*g, env);
                            self.mk_future(elem)
                        }
                        (rut_core::binary::NativeTy::Future, _) => {
                            self.err(sp, "`Future<T>` takes exactly one type parameter — the answer type");
                            TY_I32
                        }
                        // the cx class: the surface name IS the
                        // engine-minted record — the weave injects it as
                        // `cx`; nothing constructs it
                        (rut_core::binary::NativeTy::RunContext, []) => self.run_context_ty(),
                        (rut_core::binary::NativeTy::RunContext, _) => {
                            self.err(sp, "`RunContext` takes no generic arguments");
                            TY_I32
                        }
                    };
                }
                {
                    // user types
                        if let Some(e) = self.find_enum(name).cloned() {
                            if !seg.generics.is_empty() {
                                self.err(sp, format!("enum `{}` takes no generic arguments", self.name(name)));
                            }
                            return e.ty;
                        }
                        if let Some(d) = self.find_data(name).cloned() {
                            if d.generics.is_empty() {
                                if !seg.generics.is_empty() {
                                    self.err(sp, format!("`{}` takes no generic arguments", self.name(name)));
                                }
                                return d.ty;
                            }
                            if seg.generics.len() != d.generics.len() {
                                self.err(sp, format!(
                                    "`{}` takes {} generic argument(s), {} given",
                                    self.name(name),
                                    d.generics.len(),
                                    seg.generics.len()
                                ));
                                return TY_I32;
                            }
                            let args: Vec<TypeId> = seg
                                .generics
                                .iter()
                                .map(|g| self.resolve_type(*g, env))
                                .collect();
                            return self.mk_data_inst(name, args, sp);
                        }
                // the removed engine-contract names (v20): a spelled
                // `Iterable`/`Disposal` in type position names its
                // marker. (`Future`/`RunContext` resolve through the
                // native-type match above when bound; unbound they fall
                // to the use-gate's scope fix like any gated name.)
                if name == sym::ITERABLE || name == sym::DISPOSAL {
                    if let Some(msg) = self.engine_contract_head_error(name) {
                        self.err(sp, msg);
                        return TY_I32;
                    }
                }
                        if let Some(t) = self.find_iface(name).cloned() {
                            // an interface name in type position IS the
                            // object type — the bare name spells it
                            if seg.generics.is_empty() {
                                if t.id == u32::MAX {
                                    self.err(sp, format!(
                                        "generic interface `{}` needs type arguments (e.g. `{}<i32>`)",
                                        self.name(name),
                                        self.name(name)
                                    ));
                                    return TY_I32;
                                }
                                return self.mk_iface_obj(t.id);
                            }
                            if seg.generics.len() != t.generics.len() {
                                self.err(sp, format!(
                                    "`{}` takes {} type argument(s), {} given", self.name(name),
                                    t.generics.len(), seg.generics.len()
                                ));
                                return TY_I32;
                            }
                            let args: Vec<TypeId> = seg
                                .generics
                                .iter()
                                .map(|g| self.resolve_type(*g, env))
                                .collect();
                            let id = self.mk_iface_inst(name, args);
                            return self.mk_iface_obj(id);
                        }
                        // a used module's exported interface:
                        // the object type here, exactly like a declared
                        // one. A GENERIC head instantiates the carried
                        // placeholder descriptor — the interface's shape
                        // crosses the surface, so the spelling dispatches
                        // by the existing law (single concrete origin ⇒
                        // static, merged origins ⇒ vtable).
                        if let Some(ext) = self.extern_trait(name).cloned() {
                            if seg.generics.is_empty() && ext.generics == 0 {
                                return self.mk_iface_obj(ext.id);
                            }
                            if seg.generics.is_empty() {
                                self.err(sp, format!(
                                    "generic interface `{}` needs type arguments (e.g. `{}<i32>`)",
                                    self.name(name),
                                    self.name(name)
                                ));
                                return TY_I32;
                            }
                            if seg.generics.len() != ext.generics {
                                self.err(sp, format!(
                                    "`{}` takes {} type argument(s), {} given", self.name(name),
                                    ext.generics, seg.generics.len()
                                ));
                                return TY_I32;
                            }
                            let args: Vec<TypeId> = seg
                                .generics
                                .iter()
                                .map(|g| self.resolve_type(*g, env))
                                .collect();
                            let id = self.mint_extern_iface_inst(name, args);
                            return self.mk_iface_obj(id);
                        }
                        // used type: the exporter's
                        // scope-qualified id; link rebases it
                        if let Some(g) = self.extern_generics.get(&name).cloned() {
                            // a linked generic: the consumer spells the
                            // instantiation, the declaring package owns
                            // it — lay the mirror row out of the
                            // template and request the bodies
                            if seg.generics.len() != g.params.len() {
                                self.err(sp, format!(
                                    "`{}` takes {} generic argument(s), {} given",
                                    self.name(name),
                                    g.params.len(),
                                    seg.generics.len()
                                ));
                                return TY_I32;
                            }
                            let args: Vec<TypeId> = seg
                                .generics
                                .iter()
                                .map(|g| self.resolve_type(*g, env))
                                .collect();
                            return self.mk_data_inst(name, args, sp);
                        }
                        if let Some(&t) = self.extern_types.get(&name) {
                            if !seg.generics.is_empty() {
                                self.err(sp, format!("used type `{}` takes no generic arguments", self.name(name)));
                            }
                            return t;
                        }
                        // a local type alias: transparent — the
                        // name binds the target's id. A union alias errors
                        // here: unions are bound-only.
                        if self.find_alias(name).is_some() {
                            self.validate_alias(name);
                            let idx = self.aliases.iter().position(|a| a.name == name).unwrap();
                            if !seg.generics.is_empty() {
                                self.err(sp, format!("alias `{}` takes no generic arguments — aliases are non-generic", self.name(name)));
                            }
                            return match self.aliases[idx].resolved {
                                Some(AliasTarget::Ty(t)) => t,
                                Some(AliasTarget::Union) => {
                                    self.err(sp, format!(
                                        "`{}` is a union alias — unions are bound-only, legal only in `requires` bounds",
                                        self.name(name)
                                    ));
                                    TY_I32
                                }
                                _ => TY_I32,
                            };
                        }
                        if name == sym::SELF_TY {
                            self.err(sp, "`Self` is only valid inside a type body");
                            return TY_I32;
                        }
                        let msg = self
                            .not_in_core_scope(name)
                            .unwrap_or_else(|| format!("unknown type `{}`", self.name(name)));
                        self.err(sp, msg);
                        TY_I32
                    }
            }
        }
    }

}
