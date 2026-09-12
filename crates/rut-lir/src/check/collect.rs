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
        // pass 1a: declare types (enums, dataclasses, classes, traits) so
        // every name is in scope before any field/signature is resolved
        for it in &items {
            match self.ast.item(*it) {
            ItemKind::Enum { vis, name, members } => self.collect_enum(it.id(), *vis, *name, members),
            ItemKind::Dataclass { vis, name, generics, methods, .. } => {
                self.declare_data(it.id(), DataKind::Dataclass, *vis, *name, generics, methods);
            }
            ItemKind::Class { vis, name, generics, methods, .. } => {
                self.declare_data(it.id(), DataKind::Class, *vis, *name, generics, methods);
            }
            ItemKind::Trait { vis, name, generics, assoc, methods, .. } => {
                self.declare_trait(it.id(), *vis, *name, generics, assoc, methods)
            }
                _ => {}
            }
        }
        // pass 1b: resolve field types & trait signatures — with the names
        // declared, self/forward/mutual references are legal (RFC 0009
        // recursive shapes)
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
                ItemKind::Impl { trait_ref, target, assoc, methods } => {
                    self.collect_impl(it.id(), *trait_ref, *target, assoc, methods)
                }
                ItemKind::Fn(f) => {
                    let is_pub = f.vis == Vis::Pub;
                    let n = self.name(f.name).to_string();
                    if self.fn_index.contains(&f.name) {
                        self.err(self.ast.span(it.id()), format!("duplicate fn `{n}`"));
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
                        self.exports.push((n, u32::MAX));
                    }
                }
                ItemKind::ModuleLet { name, ty, init, .. } => {
                    self.lets.push((*name, *ty, *init));
                }
                ItemKind::Import { names, .. } => {
                    // module loading is resolved by the driver before body
                    // compilation (RFC 0035 §1); without it, imports are a
                    // compile error only when the names are used
                    if !self.allow_imports {
                        let sp = self.ast.span(it.id());
                        for n in names {
                            self.err(
                                sp,
                                format!(
                                    "module loading is not available in this build (RFC 0035, M2) — cannot import `{}`",
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
        if self.find_enum(name).is_some() || self.find_data(name).is_some() || self.find_trait(name).is_some() {
            self.err(sp, format!("duplicate type name `{}`", self.name(name)));
            return;
        }
        // member values: sequential from 0 or explicit (RFC 0006)
        let mut vals: Vec<(String, i64)> = Vec::new();
        let mut next = 0i64;
        for (m, v) in members {
            let val = v.unwrap_or(next);
            next = val + 1;
            vals.push((self.name(*m).to_string(), val));
        }
        let ty = self.types.intern(RutType {
            name: self.name(name).to_string(),
            kind: TyKind::Enum { members: vals },
        });
        let member_ids: Vec<IdentId> = members.iter().map(|(m, _)| *m).collect();
        self.enums.push((name, EnumDecl { ty, members: member_ids }));
    }

    /// Pass 1a — intern a dataclass/class placeholder and register its name.
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
        if self.find_data(name).is_some() || self.find_enum(name).is_some() || self.find_trait(name).is_some() {
            self.err(sp, format!("duplicate type name `{}`", self.name(name)));
            return;
        }
        // generic records stay a template (empty fields) until instantiated;
        // `Vec<T>` and RFC 0013 monomorphization enter at `mk_data_inst`
        let placeholder = self.types.intern(RutType {
            name: self.name(name).to_string(),
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
                name: self.name(fd.name).to_string(),
                ty: fty,
            });
        }
        // publish the resolved field table on the descriptor (construction
        // rules for dataclass vs class differ; the field table does not)
        let pi = self.types.dense(placeholder) as usize;
        self.types.types[pi].kind = TyKind::Data { fields: resolved.clone() };

        // collect fields with initializers + methods for the compiler
        let mut flds: Vec<(IdentId, TypeId, Option<NodeHandle<AnyExpr>>, Option<Vis>)> = Vec::new();
        for f in fields {
            let fd = self.ast.field_decl(*f);
            let fty = resolved
                .iter()
                .find(|x| x.name == self.name(fd.name))
                .map(|x| x.ty)
                .unwrap_or(TY_I32);
            flds.push((fd.name, fty, fd.init, fd.vis));
        }
        self.datas[idx].1.fields = flds;
    }

    /// Pass 1a — reserve the trait's id and register its name; signatures are
    /// resolved in pass 1b, once every type name is in scope.
    pub(crate) fn declare_trait(
        &mut self,
        node: NodeId,
        vis: Vis,
        name: IdentId,
        generics: &[NodeId2],
        assoc: &[AssocType],
        _methods: &[NodeHandle<MethodDeclNode>],
    ) {
        let sp = self.ast.span(node);
        if self.find_trait(name).is_some() || self.find_data(name).is_some() || self.find_enum(name).is_some() {
            self.err(sp, format!("duplicate type name `{}`", self.name(name)));
            return;
        }
        let id = if generics.is_empty() {
            let id = self.traits.len() as u32;
            self.traits.push(TraitDesc { name: self.name(name).to_string(), methods: vec![] });
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
            assoc: assoc.iter().map(|a| self.name(a.name).to_string()).collect(),
        }));
        let _ = vis;
    }

    /// Pass 1b — resolve a declared trait's method signatures.
    pub(crate) fn resolve_trait_sigs(
        &mut self,
        node: NodeId,
        name: IdentId,
        methods: &[NodeHandle<MethodDeclNode>],
    ) {
        let sp = self.ast.span(node);
        let Some(id) = self.trait_id_of(name) else {
            return;
        };
        let assoc = self.find_trait(name).map(|t| t.assoc.clone()).unwrap_or_default();
        let mut tms = Vec::new();
        for m in methods {
            let md = self.ast.method_decl(*m);
            let mut ptys = Vec::new();
            for p in &md.params {
                match self.ast.param(*p) {
                    MemberKind::SelfParam(_) => ptys.push(TY_UNIT), // placeholder: Self resolved at impl
                    MemberKind::Param(ParamData { ty: Some(t), .. }) => {
                        ptys.push(self.resolve_trait_sig_ty(*t, id, &[], &assoc));
                    }
                    MemberKind::Param(ParamData { ty: None, .. }) => {
                        self.err(self.ast.span(p.id()), "trait method parameters need types");
                        ptys.push(TY_I32);
                    }
                    _ => ptys.push(TY_I32),
                }
            }
            let rty = md.ret.map(|r| self.resolve_trait_sig_ty(r, id, &[], &assoc));
            tms.push((self.name(md.name).to_string(), ptys, rty));
        }
        // first param must be self (RFC 0012 §2: trait methods are instance
        // methods)
        let mut desc = TraitDesc { name: self.name(name).to_string(), methods: vec![] };
        for (mname, ptys, rty) in tms {
            if ptys.first() == Some(&TY_UNIT) {
                // replace the self placeholder: params exclude self in the
                // binary desc; the compiler passes self as arg0
                desc.methods.push(rut_core::binary::TraitMethod {
                    name: mname,
                    params: ptys[1..].to_vec(),
                    ret: rty.unwrap_or(TY_UNIT),
                });
            } else {
                self.err(sp, format!("trait method `{mname}` must take `self` (RFC 0012 §2)"));
            }
        }
        self.traits[id as usize] = desc;
    }

    /// Resolve a trait-method signature type under `env`: a bare `Self` is
    /// the trait's `dyn` object; a bare associated type name is an opaque
    /// placeholder (the concrete type comes from the impl binding).
    fn resolve_trait_sig_ty(
        &mut self,
        node: NodeHandle<AnyTy>,
        trait_id: u32,
        env: &[(IdentId, TypeId)],
        assoc: &[String],
    ) -> TypeId {
        if let TypeKind::TyPath { segs, .. } = self.ast.ty(node) {
            if segs.len() == 1 && segs[0].generics.is_empty() {
                let name = segs[0].name;
                if self.name(name) == "Self" {
                    return self.mk_dyn(trait_id);
                }
                if let Some(idx) = assoc.iter().position(|a| a == self.name(name)) {
                    return self.mk_assoc(trait_id, idx as u32, &assoc[idx]);
                }
            }
            // projection `Self.Target` — the trait's associated type under
            // `Self`; resolved per impl at the use site
            if segs.len() == 2
                && segs[0].generics.is_empty()
                && segs[1].generics.is_empty()
                && self.name(segs[0].name) == "Self"
            {
                if let Some(idx) = assoc.iter().position(|a| a == self.name(segs[1].name)) {
                    return self.mk_assoc(trait_id, idx as u32, &assoc[idx]);
                }
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
            self.name(name).to_string()
        } else {
            format!(
                "{}<{}>",
                self.name(name),
                args.iter().map(|a| self.types.name(*a).to_string()).collect::<Vec<_>>().join(", ")
            )
        };
        self.traits.push(TraitDesc { name: tname, methods: vec![] });
        self.trait_inst.insert((name, args.clone()), id);
        let Some(info) = self.find_trait(name).cloned() else { return id };
        let subst: Vec<(IdentId, TypeId)> =
            info.generics.iter().cloned().zip(args.iter().cloned()).collect();
        let methods = match self.ast.item(rut_ast::ast::NodeHandle::new(info.node)) {
            ItemKind::Trait { methods, .. } => methods.clone(),
            _ => Vec::new(),
        };
        let mut desc = TraitDesc { name: self.traits[id as usize].name.clone(), methods: vec![] };
        for m in &methods {
            let md = self.ast.method_decl(*m);
            let mut ptys = Vec::new();
            for p in &md.params {
                match self.ast.param(*p) {
                    MemberKind::SelfParam(_) => ptys.push(TY_UNIT),
                    MemberKind::Param(ParamData { ty: Some(t), .. }) => {
                        ptys.push(self.resolve_trait_sig_ty(*t, id, &subst, &info.assoc));
                    }
                    _ => ptys.push(TY_I32),
                }
            }
            let rty = md.ret.map(|r| self.resolve_trait_sig_ty(r, id, &subst, &info.assoc));
            if ptys.first() == Some(&TY_UNIT) {
                desc.methods.push(rut_core::binary::TraitMethod {
                    name: self.name(md.name).to_string(),
                    params: ptys[1..].to_vec(),
                    ret: rty.unwrap_or(TY_UNIT),
                });
            }
        }
        self.traits[id as usize] = desc;
        id
    }

    pub(crate) fn collect_impl(&mut self, node: NodeId, trait_ref: NodeHandle<AnyTy>, target: NodeHandle<AnyTy>, assoc: &[AssocType], methods: &[NodeHandle<MethodDeclNode>]) {
        let sp = self.ast.span(node);
        let Some(trait_id) = self.resolve_trait_ref(trait_ref) else {
            return;
        };
        // the target must be a local dataclass/class (RFC 0012 §2 placement).
        // A generic target (`impl Slice<T> for Vec<T>`, RFC 0005) is kept as a
        // template: its method bodies are inlined at the use site, never
        // monomorphized as standalone fns.
        let (target_ty, target_data) = match self.ast.ty(target) {
            TypeKind::TyPath { segs, .. } if segs.len() == 1 => {
                let name = segs[0].name;
                let generics = segs[0].generics.clone();
                let Some(d) = self.find_data(name).cloned() else {
                    self.err(
                        sp,
                        "impl target must be a dataclass or class of this module — builtin/foreign impls are registered natively (RFC 0012 §2)",
                    );
                    return;
                };
                if d.generics.is_empty() {
                    (d.ty, None)
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
                    (d.ty, Some((name, params)))
                }
            }
            _ => {
                self.err(
                    sp,
                    "impl target must be a dataclass or class of this module — builtin/foreign impls are registered natively (RFC 0012 §2)",
                );
                return;
            }
        };
        if let Some(_prev) = self.find_impl(trait_id, target_ty) {
            self.err(sp, "duplicate impl for the same (trait, type) pair (RFC 0012 §2)");
            return;
        }
        let mut mths = Vec::new();
        for m in methods {
            mths.push((self.ast.method_decl(*m).name, *m));
        }
        // coverage: every trait methsig covered exactly once, no extras
        let tdesc = self.traits[trait_id as usize].clone();
        for tm in &tdesc.methods {
            if !mths.iter().any(|(n, _)| self.name(*n) == tm.name) {
                self.err(sp, format!("impl is missing `{}` from {}", tm.name, tdesc.name));
            }
        }
        for (n, mnode) in &mths {
            if !tdesc.methods.iter().any(|tm| tm.name == self.name(*n)) {
                // mutable indexing is an optional hook on the read-only
                // `Iter` contract (RFC 0012): `Array`/`Vec` provide `set`,
                // `string`/`bytes` do not
                if self.name(*n) == "set" {
                    continue;
                }
                self.err(
                    self.ast.span(mnode.id()),
                    format!("`{}` is not a member of {} — put inherent methods in the type body (RFC 0012 §2)", self.name(*n), tdesc.name),
                );
            }
        }
        let trait_args = match self.ast.ty(trait_ref) {
            TypeKind::TyPath { segs, .. } if segs.len() == 1 => {
                self.ty_generic_idents(&segs[0].generics).unwrap_or_default()
            }
            _ => Vec::new(),
        };
        // associated-type bindings (RFC 0012): every trait `type` member
        // must be bound exactly once; no extras
        let trait_assoc: Vec<String> = self
            .trait_decls
            .iter()
            .find(|(_, ti)| ti.id == trait_id)
            .map(|(_, ti)| ti.assoc.clone())
            .unwrap_or_default();
        let mut assoc_bindings: Vec<(IdentId, NodeHandle<AnyTy>)> = Vec::new();
        for a in assoc {
            if !trait_assoc.iter().any(|n| n == self.name(a.name)) {
                self.err(sp, format!(
                    "`type {}` is not an associated type of {}",
                    self.name(a.name), tdesc.name
                ));
                continue;
            }
            match a.ty {
                Some(t) => assoc_bindings.push((a.name, t)),
                None => self.err(sp, format!(
                    "impl associated type `{}` needs a binding (`type {} = ..;`)",
                    self.name(a.name), self.name(a.name)
                )),
            }
        }
        for name in &trait_assoc {
            if !assoc_bindings.iter().any(|(n, _)| self.name(*n) == *name) {
                self.err(sp, format!(
                    "impl is missing `type {name}` from {}", tdesc.name
                ));
            }
        }
        let is_generic = target_data.is_some();
        self.impls.push(ImplDecl {
            trait_id,
            target: target_ty,
            target_data,
            trait_args,
            assoc: assoc_bindings,
            methods: mths,
        });
        // every impl method enters the monomorphization queue — vtables need
        // their bodies (RFC 0015 §6). Generic-target impls are inlined at the
        // use site instead (RFC 0005 `Slice<T>`).
        if is_generic {
            return;
        }
        let idx = self.impls.len() - 1;
        let method_names: Vec<IdentId> =
            self.impls[idx].methods.iter().map(|(n, _)| *n).collect();
        for mname in method_names {
            let inst = Inst {
                key: FnKey::ImplMethod { idx, name: mname },
                subst: vec![],
            };
            self.ensure_inst(inst);
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
        let name = format!(
            "{}<{}>",
            self.name(data),
            args.iter()
                .map(|a| self.types.name(*a).to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
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
                name: self.name(fd.name).to_string(),
                ty: fty,
            });
        }
        let pi = self.types.dense(ty) as usize;
        self.types.types[pi].kind = TyKind::Data { fields: resolved };
        ty
    }
}
