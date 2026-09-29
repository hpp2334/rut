//! Collection (RFC 0031 SS1): pass 1 declares types (enums, dataclasses,
//! classes, traits), pass 2 impls/fns/lets. Record payloads are slot arrays
//! (RFC 0015 SS2); impl methods enter the instantiation queue eagerly.

use rut_core::binary::TraitDesc;
use rut_core::types::*;
use super::*;


impl<'a> Ctx<'a> {

    // ---- collection ----

    pub fn collect(&mut self) {
        let items = self.ast.module_items(self.ast.root).to_vec();
        // pass 1a: declare types (enums, dataclasses, classes, traits,
        // aliases) so every name is in scope before any field/signature
        // is resolved
        for it in &items {
            match self.ast.item(*it) {
            ItemKind::Enum { vis, name, members } => self.collect_enum(it.id(), *vis, *name, members),
            ItemKind::Dataclass { vis, name, generics, methods, .. } => {
                self.declare_data(it.id(), DataKind::Dataclass, *vis, *name, generics, &[], methods);
            }
            ItemKind::Class { vis, name, generics, requires, methods, .. } => {
                self.declare_data(it.id(), DataKind::Class, *vis, *name, generics, requires, methods);
            }
            ItemKind::Trait { vis, name, generics, methods, .. } => {
                self.declare_trait(it.id(), *vis, *name, generics, methods)
            }
            ItemKind::Alias(d) => self.declare_alias(it.id(), d),
                _ => {}
            }
        }
        // pass 1b: validate alias targets first (each member resolves as
        // type or trait — forward refs legal, cycles error), then resolve
        // field types & trait signatures — with the names declared,
        // self/forward/mutual references are legal (RFC 0009 recursive
        // shapes)
        for it in &items {
            if let ItemKind::Alias(d) = self.ast.item(*it) {
                self.validate_alias(d.name);
            }
        }
        for it in &items {
            match self.ast.item(*it) {
                ItemKind::Dataclass { name, fields, .. } | ItemKind::Class { name, fields, .. } => {
                    // generic records instantiate on use (mk_data_inst), so
                    // their field types resolve under the PARAMETER
                    // PLACEHOLDERS here — the template row crosses the
                    // surface complete, and a consumer lays a concrete
                    // instantiation out by substituting the placeholders
                    let is_generic = self
                        .find_data(*name)
                        .map(|d| !d.generics.is_empty())
                        .unwrap_or(false);
                    if is_generic {
                        self.resolve_template_fields(*name, fields);
                    } else {
                        self.resolve_data_fields(*name, fields);
                    }
                }
                ItemKind::Trait { name, methods, .. } => {
                    self.resolve_trait_sigs(it.id(), *name, methods)
                }
                _ => {}
            }
        }
        // pass 2: impls, fns, lets
        for it in &items {
            match self.ast.item(*it) {
                // the two impl forms (RFC 0012): inherent + trait impls
                ItemKind::Impl { trait_ref, target, methods } => {
                    let methods = methods.clone();
                    self.collect_impl(it.id(), *trait_ref, *target, &methods)
                }
                ItemKind::Fn(f) => {
                    let is_pub = f.vis == Vis::Pub;
                    if self.fn_index.contains(&f.name) {
                        self.err(self.ast.span(it.id()), format!("duplicate fn `{}`", self.name(f.name)));
                    }
                    self.fn_index.push(f.name);
                    self.fn_nodes.push((f.name, NodeHandle::new(it.id())));
                    // `entry fn` — the host-callable surface (RFC 0035 §3);
                    // signature checked against the crossing rule below
                    if f.entry {
                        self.entries.push(f.name);
                    }
                    if is_pub {
                        // name recorded; the func id binds at finalize
                        self.exports.push((f.name, u32::MAX));
                    }
                }
                ItemKind::ModuleLet { name, ty, init, .. } => {
                    self.lets.push((*name, *ty, *init));
                }
                ItemKind::Use { names, .. } => {
                    // record what the module wrote — the binding gate for
                    // used surfaces (RFC 0028: used, never ambient)
                    for n in names {
                        self.used.insert(*n);
                    }
                    // module loading is resolved by the driver before body
                    // compilation (RFC 0035 §1); without it, use statements
                    // are a compile error only when the names are used
                    if !self.allow_uses {
                        let sp = self.ast.span(it.id());
                        for n in names {
                            self.err(
                                sp,
                                format!(
                                    "module loading is not available in this build (RFC 0035, M2) — cannot use `{}`",
                                    self.name(*n)
                                ),
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }

    pub(crate) fn collect_enum(&mut self, node: NodeId, _vis: Vis, name: IdentId, members: &[(IdentId, Option<i64>)]) {
        let sp = self.ast.node(node).span;
        if self.find_enum(name).is_some() || self.find_data(name).is_some() || self.find_trait(name).is_some() || self.find_alias(name).is_some() {
            self.err(sp, format!("duplicate type name `{}`", self.name(name)));
            return;
        }
        // member values: sequential from 0 or explicit (RFC 0006)
        let mut vals: Vec<(IdentId, i64)> = Vec::new();
        let mut next = 0i64;
        for (m, v) in members {
            let val = v.unwrap_or(next);
            next = val + 1;
            vals.push((*m, val));
        }
        let ty = self.types.intern(RutType {
            name,
            kind: TyKind::Enum { members: vals },
        });
        let member_ids: Vec<IdentId> = members.iter().map(|(m, _)| *m).collect();
        self.enums.push((name, EnumDecl { ty, members: member_ids }));
    }

    /// Pass 1a — intern a struct/class placeholder and register its name.
    /// Fields are resolved later (pass 1b), so a field may name this type or
    /// any type declared later in the module (RFC 0009 recursive shapes).
    pub(crate) fn declare_data(
        &mut self,
        node: NodeId,
        kind: DataKind,
        _vis: Vis,
        name: IdentId,
        generics: &[IdentId],
        requires: &[(IdentId, NodeHandle<AnyTy>)],
        methods: &[NodeHandle<MethodDeclNode>],
    ) {
        let sp = self.ast.span(node);
        if self.find_data(name).is_some() || self.find_enum(name).is_some() || self.find_trait(name).is_some() || self.find_alias(name).is_some() {
            self.err(sp, format!("duplicate type name `{}`", self.name(name)));
            return;
        }
        // the instantiation ledger's owner anchor: this decl's bodies
        // live in the pkg whose source it was parsed from (the splice
        // origin, or this unit)
        self.decl_owner
            .insert(name, self.origin_of(sp.lo).to_string());
        // generic records stay a template (empty fields) until instantiated;
        // `Vec<T>` and RFC 0013 monomorphization enter at `mk_data_inst`
        let placeholder = self.types.intern(RutType {
            name,
            kind: TyKind::Data { fields: vec![] },
        });
        let mut mths: Vec<(IdentId, NodeHandle<MethodDeclNode>)> = Vec::new();
        for m in methods {
            mths.push((self.ast.method_decl(*m).name, *m));
        }
        self.datas.push((
            name,
            DataDecl {
                kind,
                ty: placeholder,
                node: NodeHandle::new(node),
                fields: vec![],
                methods: mths,
                generics: generics.to_vec(),
                requires: requires.to_vec(),
            },
        ));
    }

    /// Pass 1b, generic templates — resolve a generic record's field types
    /// under the PARAMETER PLACEHOLDERS (`param_placeholder`'s `#T` rows)
    /// and stamp them on the template row. The template then crosses the
    /// surface complete: a consumer lays `Vec<i64>` out by substituting
    /// the placeholders, without the declaring body in sight. The
    /// concrete instantiations still enter at `mk_data_inst`, which
    /// re-reads the declaration under the caller's substitution.
    pub(crate) fn resolve_template_fields(
        &mut self,
        name: IdentId,
        fields: &[NodeHandle<FieldDeclNode>],
    ) {
        let Some(idx) = self.datas.iter().position(|(n, _)| *n == name) else {
            return;
        };
        let ty = self.datas[idx].1.ty;
        let generics = self.datas[idx].1.generics.clone();
        let env: Vec<(IdentId, TypeId)> = generics
            .iter()
            .map(|&p| (p, self.param_placeholder(p)))
            .collect();
        let mut resolved: Vec<FieldInfo> = Vec::new();
        for f in fields {
            let fd = self.ast.field_decl(*f);
            let fty = self.resolve_type(fd.ty, &env);
            resolved.push(FieldInfo { name: fd.name, ty: fty });
        }
        let pi = self.types.dense(ty) as usize;
        self.types.types[pi].kind = TyKind::Data { fields: resolved };
    }

    /// Pass 1b — resolve a declared record's field types, stamp the payload
    /// layout, and fill the `DataDecl`'s field list.
    pub(crate) fn resolve_data_fields(
        &mut self,
        name: IdentId,
        fields: &[NodeHandle<FieldDeclNode>],
    ) {        let Some(idx) = self.datas.iter().position(|(n, _)| *n == name) else {
            return;
        };
        let placeholder = self.datas[idx].1.ty;
        let mut resolved: Vec<FieldInfo> = Vec::new();
        for f in fields {
            let fd = self.ast.field_decl(*f);
            if fd.is_static {
                self.err(
                    self.ast.span(f.id()),
                    "`static` fields do not exist — there is no mutable module state (RFC 0003 §1); thread state explicitly or hold it in an `opaque` container the host passes back (RFC 0014)",
                );
            }
            let fty = self.resolve_type(fd.ty, &[]);
            resolved.push(FieldInfo {
                name: fd.name,
                ty: fty,
            });
        }
        // publish the resolved field table on the descriptor (construction
        // rules for struct vs class differ; the field table does not)
        let pi = self.types.dense(placeholder) as usize;
        self.types.types[pi].kind = TyKind::Data { fields: resolved.clone() };

        // collect fields with initializers + methods for the compiler
        let mut flds: Vec<(IdentId, TypeId, Option<NodeHandle<AnyExpr>>, Option<Vis>)> = Vec::new();
        for f in fields {
            let fd = self.ast.field_decl(*f);
            let fty = resolved
                .iter()
                .find(|x| x.name == fd.name)
                .map(|x| x.ty)
                .unwrap_or(TY_I32);
            flds.push((fd.name, fty, fd.init, fd.vis));
        }
        self.datas[idx].1.fields = flds;
    }

    /// Pass 1a — register a `type X = A;` / `type X = A | B;` alias
    /// (RFC 0043), plain and union forms — one name, one decl: the
    /// alias head admits no members (a generic head is a parse error)
    /// and the duplicate-type-name check is the plain symmetric
    /// four-way, exactly like the enum/data/trait declare sites. The
    /// target validates in pass 1b (`validate_alias`), so forward
    /// references are legal.
    pub(crate) fn declare_alias(&mut self, node: NodeId, d: &AliasData) {
        let sp = self.ast.span(node);
        if self.find_alias(d.name).is_some()
            || self.find_data(d.name).is_some()
            || self.find_enum(d.name).is_some()
            || self.find_trait(d.name).is_some()
        {
            self.err(sp, format!("duplicate type name `{}`", self.name(d.name)));
            return;
        }
        self.aliases.push(AliasDecl {
            name: d.name,
            node,
            target: d.target,
            resolved: None,
        });
    }

    /// Pass 1a — reserve the trait's id and register its name; signatures
    /// are resolved in pass 1b, once every type name is in scope.
    pub(crate) fn declare_trait(
        &mut self,
        node: NodeId,
        vis: Vis,
        name: IdentId,
        generics: &[NodeId2],
        _methods: &[NodeHandle<MethodDeclNode>],
    ) {
        let sp = self.ast.span(node);
        if self.find_trait(name).is_some() || self.find_data(name).is_some() || self.find_enum(name).is_some() || self.find_alias(name).is_some() {
            self.err(sp, format!("duplicate type name `{}`", self.name(name)));
            return;
        }
        let id = if generics.is_empty() {
            let id = self.traits.len() as u32;
            self.traits.push(TraitDesc { name, methods: vec![] });
            id
        } else {
            // a generic trait has no single id — `mk_trait_inst` allocates
            // one per type-argument list (RFC 0013 monomorphization)
            u32::MAX
        };
        self.trait_decls.push((name, TraitDeclInfo {
            id,
            node,
            generics: generics.to_vec(),
        }));
        let _ = vis;
    }

    /// Pass 1b — resolve a declared trait's method signatures.
    pub(crate) fn resolve_trait_sigs(
        &mut self,
        _node: NodeId,
        name: IdentId,
        methods: &[NodeHandle<MethodDeclNode>],
    ) {
        let Some(id) = self.trait_id_of(name) else {
            return;
        };
        let mut tms = Vec::new();
        for m in methods {
            let md = self.ast.method_decl(*m);
            let mut ptys = Vec::new();
            for (i, p) in md.params.iter().enumerate() {
                match self.ast.param(*p) {
                    MemberKind::SelfParam(_) if i == 0 => {} // receiver — resolved at the impl
                    MemberKind::SelfParam(_) => {
                        self.err(self.ast.span(p.id()), "`self` must be the first parameter");
                    }
                    MemberKind::Param(ParamData { ty: Some(t), .. }) => {
                        ptys.push(self.resolve_trait_sig_ty(*t, id, &[]));
                    }
                    MemberKind::Param(ParamData { ty: None, .. }) => {
                        self.err(self.ast.span(p.id()), "trait method parameters need types");
                        ptys.push(TY_I32);
                    }
                    _ => ptys.push(TY_I32),
                }
            }
            let rty = md.ret.map(|r| self.resolve_trait_sig_ty(r, id, &[]));
            tms.push((md.name, ptys, rty));
        }
        // RFC 0012 §2: trait methods take `self` — except engine-contract
        // members, which may spell plain parameters only (`fn yield(cx: ..)`).
        // Either way the binary desc's params exclude the receiver: the
        // compiler passes self as arg0 for receiver methods.
        let mut desc = TraitDesc { name, methods: vec![] };
        for (mname, ptys, rty) in tms {
            desc.methods.push(rut_core::binary::TraitMethod {
                name: mname,
                params: ptys,
                ret: rty.unwrap_or(TY_NIL),
            });
        }
        self.traits[id as usize] = desc;
    }

    /// Resolve a trait-method signature type under `env`: a bare
    /// `Self` is the trait's object type.
    fn resolve_trait_sig_ty(
        &mut self,
        node: NodeHandle<AnyTy>,
        trait_id: u32,
        env: &[(IdentId, TypeId)],
    ) -> TypeId {
        // a trait declaration's `Self` is the trait object at any
        // structural depth (`(?Self, ?E)` — the rut-json batch phase 1,
        // gap 2's signature half); the impl side resolves against its
        // concrete target through resolve_sig_ty
        let tobj = self.mk_trait_obj(trait_id);
        self.resolve_sig_ty_deep(node, env, Some(tobj))
    }

    /// Instantiate a generic trait for concrete type arguments (RFC 0013):
    /// one `TraitDesc` (and trait id) per type-argument list, cached.
    pub fn mk_trait_inst(&mut self, name: IdentId, args: Vec<TypeId>) -> u32 {
        if let Some(&id) = self.trait_inst.get(&(name, args.clone())) {
            return id;
        }
        let id = self.traits.len() as u32;
        let tname = if args.is_empty() {
            self.interner.name(name).to_string()
        } else {
            format!(
                "{}<{}>",
                self.interner.name(name),
                args.iter().map(|a| self.type_name(*a).to_string()).collect::<Vec<_>>().join(", ")
            )
        };
        let tname = self.intern(&tname);
        self.traits.push(TraitDesc { name: tname, methods: vec![] });
        self.trait_inst.insert((name, args.clone()), id);
        let Some(info) = self.find_trait(name).cloned() else { return id };
        let subst: Vec<(IdentId, TypeId)> =
            info.generics.iter().cloned().zip(args.iter().cloned()).collect();
        let methods = match self.ast.item(rut_ast::ast::NodeHandle::new(info.node)) {
            ItemKind::Trait { methods, .. } => methods.clone(),
            _ => Vec::new(),
        };
        let mut desc = TraitDesc { name: tname, methods: vec![] };
        for m in &methods {
            let md = self.ast.method_decl(*m);
            let mut ptys = Vec::new();
            for p in md.params.iter() {
                match self.ast.param(*p) {
                    MemberKind::Param(ParamData { ty: Some(t), .. }) => {
                        ptys.push(self.resolve_trait_sig_ty(*t, id, &subst));
                    }
                    _ => {}
                }
            }
            let rty = md.ret.map(|r| self.resolve_trait_sig_ty(r, id, &subst));
            desc.methods.push(rut_core::binary::TraitMethod {
                name: md.name,
                params: ptys,
                ret: rty.unwrap_or(TY_NIL),
            });
        }
        self.traits[id as usize] = desc;
        id
    }

    /// A template-level placeholder type for a generic target's parameter
    /// (the `impl Readable<T> for Source<T>` form): a synthetic interned
    /// type named `#<param>`. `#` is no identifier character, so the name
    /// cannot collide with a user-spelled type, and structural interning
    /// (`TypeTable::intern` dedups on `(name, kind)`) gives one stable id
    /// per parameter name — which is what lands exact template duplicates
    /// on the same (trait, type) pair. The placeholder exists so trait-ref
    /// resolution and the coverage check can proceed; it is a
    /// template-level type, never a runtime one — the phase-2 dispatch
    /// half substitutes the class's concrete argument per instantiation.
    pub(crate) fn param_placeholder(&mut self, p: IdentId) -> TypeId {
        let name = self.intern(&format!("#{}", self.name(p)));
        self.types.intern_own(RutType {
            name,
            kind: TyKind::Data { fields: vec![] },
        })
    }

    /// Does this type node mention any of `params` at any depth? The v1
    /// guard's nested-parameter test: `Readable<Vec<T>>` over a target
    /// parameter `T` is exactly the shape the template registration
    /// cannot carry yet, while a fully concrete nest (`Vec<i32>`) is not.
    pub(crate) fn ty_mentions_any(&self, node: NodeHandle<AnyTy>, params: &[IdentId]) -> bool {
        match self.ast.ty(node) {
            TypeKind::TyPath { segs } => {
                segs.iter().any(|s| params.contains(&s.name))
                    || segs
                        .iter()
                        .any(|s| s.generics.iter().any(|g| self.ty_mentions_any(*g, params)))
            }
            TypeKind::TyFn { params: ps, ret } => {
                ps.iter().any(|p| self.ty_mentions_any(*p, params))
                    || self.ty_mentions_any(*ret, params)
            }
            TypeKind::TyOpt { inner } => self.ty_mentions_any(*inner, params),
            TypeKind::TyArray { elem } => self.ty_mentions_any(*elem, params),
            TypeKind::TyTuple { elems } => elems.iter().any(|e| self.ty_mentions_any(*e, params)),
            _ => false,
        }
    }

    /// Instantiate a generic record for concrete type arguments (RFC 0013
    /// monomorphization). The id is interned and cached before fields resolve
    /// so recursive shapes (`Node<T> { next: Option<Node<T>> }`) terminate.
    /// The class's inline bounds gate the substitution here (RFC 0043 §A5,
    /// admission-only) — once per concrete argument list, at the spelling
    /// that created it.
    pub fn mk_data_inst(&mut self, data: IdentId, args: Vec<TypeId>, sp: Span) -> TypeId {
        if let Some(&t) = self.type_inst.get(&(data, args.clone())) {
            return t;
        }
        // a generic bound from a used module's surface: the DECLARING
        // package owns the instantiation. The consumer lays the concrete
        // row out of the template's placeholder fields and routes the
        // bodies request to the owner — it never compiles them itself.
        if self.find_data(data).is_none() {
            if let Some(g) = self.extern_generics.get(&data).cloned() {
                return self.mk_extern_data_inst(data, &g, args);
            }
            return TY_I32;
        }
        let Some(decl) = self.find_data(data).cloned() else {
            return TY_I32;
        };
        let name = self.intern(&format!(
            "{}<{}>",
            self.name(data),
            args.iter()
                .map(|a| self.type_name(*a).to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        let ty = self.types.intern(RutType {
            name,
            kind: TyKind::Data { fields: vec![] },
        });
        self.type_inst.insert((data, args.clone()), ty);
        self.inst_data.insert(ty, (data, args.clone()));
        let env: Vec<(IdentId, TypeId)> =
            decl.generics.iter().cloned().zip(args.iter().cloned()).collect();
        // inline bounds gate the completed substitution (RFC 0043 §A5) —
        // the cache insert above keeps a bound-triggering instantiation of
        // the same record from recursing
        self.admit_bounds(&decl.requires, &env, sp);
        let field_nodes: Vec<NodeHandle<FieldDeclNode>> = match self.ast.item(decl.node) {
            ItemKind::Dataclass { fields, .. } | ItemKind::Class { fields, .. } => fields.clone(),
            _ => Vec::new(),
        };
        let mut resolved: Vec<FieldInfo> = Vec::new();
        for f in &field_nodes {
            let fd = self.ast.field_decl(*f);
            if fd.is_static {
                self.err(
                    self.ast.span(f.id()),
                    "`static` fields do not exist — there is no mutable module state (RFC 0003 §1); thread state explicitly or hold it in an `opaque` container the host passes back (RFC 0014)",
                );
            }
            let fty = self.resolve_type(fd.ty, &env);
            resolved.push(FieldInfo {
                name: fd.name,
                ty: fty,
            });
        }
        let pi = self.types.dense(ty) as usize;
        self.types.types[pi].kind = TyKind::Data { fields: resolved };
        // the owner-anchored ledger row: wherever this unit's copy of the
        // instantiation travels, the key says whose it is — link unifies
        // rows sharing it into ONE program-wide instantiation
        let owner = self.intern(&self.owner_of_data(data));
        self.ledger_types.push(rut_core::binary::InstTy {
            owner,
            decl: data,
            args: args.clone(),
            ty,
        });
        ty
    }

    /// The foreign-generic instantiation path: substitute the template's
    /// placeholder fields with the concrete arguments, intern the mirror
    /// row, and route the request. `Placeholder` leaves match by their
    /// `#<param>` name text; structural wrappers (`[?T]`, `?T`, `Weak<T>`,
    /// `fn(..)`) rebuild through the structural interning, so equal shapes
    /// land on the rows the unit already uses. A row-kinded field
    /// (`Node<#T>` spelled inside the template) keeps the template's row —
    /// the mirror law covers the direct-argument shapes.
    fn mk_extern_data_inst(&mut self, data: IdentId, g: &ExternGeneric, args: Vec<TypeId>) -> TypeId {
        let name = self.intern(&format!(
            "{}<{}>",
            self.name(data),
            args.iter()
                .map(|a| self.type_name(*a).to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        let mut env: std::collections::HashMap<String, TypeId> = std::collections::HashMap::new();
        for (p, &a) in g.params.iter().zip(args.iter()) {
            // the template's placeholder leaves spell `#<param>` — the
            // same convention `param_placeholder` mints with (the `#`
            // is no identifier character, so no user type collides)
            env.insert(format!("#{}", self.name(*p)), a);
        }
        let TyKind::Data { fields: template } = self.types.kind(g.template) else {
            return TY_I32;
        };
        let template = template.clone();
        let fields: Vec<FieldInfo> = template
            .into_iter()
            .map(|mut f| {
                f.ty = self.subst_template_ty(f.ty, &env);
                f
            })
            .collect();
        let ty = self.types.intern(RutType {
            name,
            kind: TyKind::Data { fields },
        });
        self.type_inst.insert((data, args.clone()), ty);
        self.inst_data.insert(ty, (data, args.clone()));
        // a `class` has no outside literal — the gate names the mirror row
        if g.is_class {
            self.extern_classes.insert(ty);
        }
        let owner = self.intern(&g.owner);
        self.ledger_types.push(rut_core::binary::InstTy {
            owner,
            decl: data,
            args: args.clone(),
            ty,
        });
        self.request_inst(g.owner.clone(), data, args);
        ty
    }

    /// Substitute one template field type: a placeholder leaf becomes its
    /// argument; structural wrappers rebuild per element.
    fn subst_template_ty(&mut self, id: TypeId, env: &std::collections::HashMap<String, TypeId>) -> TypeId {
        let text = self.types.type_at(id).name;
        let text = self.interner.name(text).to_string();
        if let Some(&arg) = env.get(&text) {
            return arg;
        }
        let kind = self.types.kind(id).clone();
        let rebuilt = match kind {
            TyKind::Array { elem } => {
                let e = self.subst_template_ty(elem, env);
                Some(self.mk_array(e))
            }
            TyKind::Opt { elem } => {
                let e = self.subst_template_ty(elem, env);
                Some(self.mk_opt(e))
            }
            TyKind::Weak { elem } => {
                let e = self.subst_template_ty(elem, env);
                Some(self.mk_weak(e))
            }
            TyKind::Fn { params, ret } => {
                let r = self.subst_template_ty(ret, env);
                let ps = params.iter().map(|&p| self.subst_template_ty(p, env)).collect();
                Some(self.mk_fn_ty(ps, r))
            }
            _ => None,
        };
        rebuilt.unwrap_or(id)
    }
}
