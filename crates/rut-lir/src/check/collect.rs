//! Collection (RFC 0031 SS1): pass 1 declares types (enums, dataclasses,
//! classes, traits), pass 2 impls/fns/lets. Record payloads are slot arrays
//! (RFC 0015 SS2); impl methods enter the instantiation queue eagerly.

use rut_core::binary::TraitDesc;
use rut_core::types::*;
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
                self.declare_data(it.id(), DataKind::Dataclass, *vis, *name, generics, methods);
            }
            ItemKind::Class { vis, name, generics, methods, .. } => {
                self.declare_data(it.id(), DataKind::Class, *vis, *name, generics, methods);
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
                    // their field types resolve under a substitution, not here
                    let is_generic = self
                        .find_data(*name)
                        .map(|d| !d.generics.is_empty())
                        .unwrap_or(false);
                    if !is_generic {
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
        methods: &[NodeHandle<MethodDeclNode>],
    ) {
        let sp = self.ast.span(node);
        if self.find_data(name).is_some() || self.find_enum(name).is_some() || self.find_trait(name).is_some() || self.find_alias(name).is_some() {
            self.err(sp, format!("duplicate type name `{}`", self.name(name)));
            return;
        }
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
            },
        ));
    }

    /// Pass 1b — resolve a declared record's field types, stamp the payload
    /// layout, and fill the `DataDecl`'s field list.
    pub(crate) fn resolve_data_fields(
        &mut self,
        name: IdentId,
        fields: &[NodeHandle<FieldDeclNode>],
    ) {
        let Some(idx) = self.datas.iter().position(|(n, _)| *n == name) else {
            return;
        };
        let placeholder = self.datas[idx].1.ty;
        let mut resolved: Vec<FieldInfo> = Vec::new();
        for f in fields {
            let fd = self.ast.field_decl(*f);
            if fd.is_static {
                self.err(
                    self.ast.span(f.id()),
                    "`static` fields do not exist — there is no mutable module state (RFC 0003 §1); thread state explicitly or hold it in an `Opaque` container the host passes back (RFC 0014)",
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
    /// (RFC 0043). The target validates in pass 1b (`validate_alias`), so
    /// forward references are legal; the name shares the duplicate-type-
    /// name check with enums/records/traits.
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
        if let TypeKind::TyPath { segs, .. } = self.ast.ty(node) {
            if segs.len() == 1 && segs[0].generics.is_empty() && segs[0].name == sym::SELF_TY {
                return self.mk_trait_obj(trait_id);
            }
        }
        self.resolve_type(node, env)
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
        let (target_ty, target_data, is_local, is_used) = match self.ast.ty(target) {
            TypeKind::TyPath { segs, .. } if segs.len() == 1 => {
                let name = segs[0].name;
                let generics = segs[0].generics.clone();
                if let Some(d) = self.find_data(name).cloned() {
                    if d.generics.is_empty() {
                        (d.ty, None, true, false)
                    } else {
                        let Some(params) = self.ty_generic_idents(&generics) else {
                            self.err(sp, "a generic impl target must name its type parameters (e.g. `Vec<T>`)");
                            return;
                        };
                        if params.len() != d.generics.len() {
                            self.err(sp, format!(
                                "`{}<..>` takes {} type parameter(s), {} given",
                                self.name(name), d.generics.len(), params.len()
                            ));
                            return;
                        }
                        (d.ty, Some((name, params)), true, false)
                    }
                } else if let Some(kind) = self.extern_native_types.get(&name).copied() {
                    match (kind, generics.as_slice()) {
                        (rut_core::binary::NativeTy::Opaque, []) => (TY_OPAQUE, None, false, false),
                        (rut_core::binary::NativeTy::Array, [g]) => {
                            let Some(params) = self.ty_generic_idents(std::slice::from_ref(g)) else {
                                self.err(sp, "a generic impl target must name its type parameters (e.g. `Array<T>`)");
                                return;
                            };
                            // a template id standing for the generic array —
                            // duplicate detection and inst keys only
                            let ph = self.types.intern(RutType {
                                name,
                                kind: TyKind::Data { fields: vec![] },
                            });
                            (ph, Some((name, params)), false, false)
                        }
                        (rut_core::binary::NativeTy::Array, _) => {
                            self.err(sp, "`Array<T>` takes one type parameter");
                            return;
                        }
                        (rut_core::binary::NativeTy::Opaque, _) => {
                            self.err(sp, "`Opaque` takes no type parameters");
                            return;
                        }
                    }
                } else if segs[0].generics.is_empty() && self.extern_types.contains_key(&name) {
                    // a USED type (RFC 0035 §1): legal as a TRAIT-impl
                    // target only — inherent impls stay in the type's
                    // module (RFC 0012 §2). The id is the exporter's
                    // scope-qualified one; link rebases it.
                    (self.extern_types[&name], None, false, true)
                } else {
                    self.err(
                        sp,
                        "impl target must be a struct or class of this module — a `builtin class` takes impls only in its own module (RFC 0012 §2)",
                    );
                    return;
                }
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
        match trait_ref {
            None => {
                if is_used {
                    self.err(
                        sp,
                        "inherent impls live in the type's module — only `impl Trait for UsedType` may name a used type (RFC 0012 §2)",
                    );
                    return;
                }
                self.collect_impl_inherent(target, target_ty, target_data, is_local, mths)
            }
            Some(tr) => self.collect_impl_trait(sp, tr, target_ty, target_data, mths),
        }
    }

    /// `impl T { .. }` — inherent methods. Local targets attach into the
    /// `DataDecl`; native builtin-class targets register an inherent
    /// `ImplDecl` (dispatch resolves through the native shape). Both
    /// dispatch statically — the receiver's concrete type names the impl.
    fn collect_impl_inherent(
        &mut self,
        target: NodeHandle<AnyTy>,
        target_ty: TypeId,
        target_data: Option<(IdentId, Vec<IdentId>)>,
        is_local: bool,
        mths: Vec<(IdentId, NodeHandle<MethodDeclNode>)>,
    ) {
        let tname = match self.ast.ty(target) {
            TypeKind::TyPath { segs, .. } if !segs.is_empty() => segs[0].name,
            _ => return,
        };
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
                inherent: true,
                methods: mths.clone(),
            });
            // concrete targets' methods are eagerly queued (no call site
            // may exist); generic targets monomorphize at their call sites
            if !generic {
                let idx = self.impls.len() - 1;
                for (n, _) in &mths {
                    self.ensure_inst(Inst {
                        key: FnKey::ImplMethod { idx, name: *n },
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

    /// `impl I for T { .. }` — a trait impl (any module): duplicate
    /// (trait, type) pair, coverage (every trait method implemented;
    /// signature match incl. `is_async` and receiver form), no extras,
    /// then registration and eager monomorphization.
    fn collect_impl_trait(
        &mut self,
        sp: rut_lexer::span::Span,
        trait_ref: NodeHandle<AnyTy>,
        target_ty: TypeId,
        target_data: Option<(IdentId, Vec<IdentId>)>,
        mths: Vec<(IdentId, NodeHandle<MethodDeclNode>)>,
    ) {
        let Some(trait_id) = self.resolve_trait_ref(trait_ref) else {
            return;
        };
        let (trait_name, trait_arg_nodes) = match self.ast.ty(trait_ref) {
            TypeKind::TyPath { segs, .. } if segs.len() == 1 => {
                (segs[0].name, segs[0].generics.clone())
            }
            _ => return,
        };
        if let Some(_prev) = self.find_impl(trait_id, target_ty) {
            self.err(sp, "duplicate impl for the same (trait, type) pair (RFC 0012 §2)");
            return;
        }
        // the trait's required signatures: from the trait's own AST when
        // it is declared here (async + receiver form live only there),
        // resolved under the impl's trait-argument substitution; from the
        // instantiated descriptor otherwise (engine-named contracts)
        let trait_env: Vec<(IdentId, TypeId)> = match self.find_trait(trait_name).cloned() {
            Some(info) if !info.generics.is_empty() => {
                let args: Vec<TypeId> = trait_arg_nodes
                    .iter()
                    .map(|g| self.resolve_type(*g, &[]))
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
        for req in &reqs {
            let Some((_, mnode)) = mths.iter().find(|(n, _)| *n == req.name) else {
                let tname = self.name(self.trait_by_id(trait_id).name);
                self.err(sp, format!("impl is missing `{}` from {}", self.name(req.name), tname));
                continue;
            };
            let md = self.ast.method_decl(*mnode);
            let (self_form, ptys) = self.impl_sig_params(
                &md.params,
                vec![],
                Some(target_ty),
            );
            let ret = md
                .ret
                .map(|r| self.resolve_sig_ty(r, &[], Some(target_ty)))
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
            inherent: false,
            methods: mths.clone(),
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
                key: FnKey::ImplMethod { idx, name: *mname },
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

    /// Instantiate a generic record for concrete type arguments (RFC 0013
    /// monomorphization). The id is interned and cached before fields resolve
    /// so recursive shapes (`Node<T> { next: Option<Node<T>> }`) terminate.
    pub fn mk_data_inst(&mut self, data: IdentId, args: Vec<TypeId>) -> TypeId {
        if let Some(&t) = self.type_inst.get(&(data, args.clone())) {
            return t;
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
                    "`static` fields do not exist — there is no mutable module state (RFC 0003 §1); thread state explicitly or hold it in an `Opaque` container the host passes back (RFC 0014)",
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
        ty
    }
}
