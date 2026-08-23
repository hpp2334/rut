//! Collection (RFC 0031 SS1): pass 1 declares types (enums, dataclasses,
//! classes, traits), pass 2 impls/fns/lets. Field layout is repr-C
//! (RFC 0015 SS2); impl methods enter the instantiation queue eagerly.

use rut_core::binary::TraitDesc;
use rut_core::types::*;
use super::*;

impl<'a> Ctx<'a> {

    // ---- collection ----

    pub fn collect(&mut self) {
        let NodeKind::Module { items } = &self.ast.node(self.ast.root).kind else {
            return;
        };
        let items = items.clone();
        // pass 1: type declarations (enums, dataclasses, classes, traits)
        for it in &items {
            match &self.ast.node(*it).kind {
                NodeKind::Enum { vis, name, members } => self.collect_enum(*it, *vis, *name, members),
                NodeKind::Dataclass { vis, name, generics, fields, methods }
                | NodeKind::Class { vis, name, generics, fields, methods } => {
                    let kind = match self.ast.node(*it).kind {
                        NodeKind::Dataclass { .. } => DataKind::Dataclass,
                        _ => DataKind::Class,
                    };
                    self.collect_data(*it, kind, *vis, *name, generics, fields, methods);
                }
                NodeKind::Trait { vis, name, generics, methods, .. } => {
                    self.collect_trait(*it, *vis, *name, generics, methods)
                }
                _ => {}
            }
        }
        // data field types may reference enums/other datas — re-resolve now
        // (fields were resolved with a second pass inside collect_data)
        // pass 2: impls, fns, lets
        for it in &items {
            match &self.ast.node(*it).kind {
                NodeKind::Impl { trait_ref, target, methods } => {
                    self.collect_impl(*it, *trait_ref, *target, methods)
                }
                NodeKind::Fn { vis, name, .. } => {
                    let is_pub = *vis == Vis::Pub;
                    let n = self.name(*name).to_string();
                    if self.fn_index.contains(name) {
                        self.err(self.ast.node(*it).span, format!("duplicate fn `{n}`"));
                    }
                    self.fn_index.push(*name);
                    self.fn_nodes.push((*name, *it));
                    if is_pub {
                        // name recorded; the func id binds at finalize
                        self.exports.push((n, u32::MAX));
                    }
                }
                NodeKind::ModuleLet { name, ty, init, .. } => {
                    self.lets.push((*name, *ty, *init));
                }
                NodeKind::Import { .. } => {
                    // module loading is M2+ (RFC 0035): the corpus parses,
                    // and unresolvable imports are compile errors only when
                    // their names are used
                    for n in match &self.ast.node(*it).kind {
                        NodeKind::Import { names, .. } => names.clone(),
                        _ => vec![],
                    } {
                        let sp = self.ast.node(*it).span;
                        self.err(
                            sp,
                            format!(
                                "module loading is not available in this build (RFC 0035, M2) — cannot import `{}`",
                                self.name(n)
                            ),
                        );
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
            size: 8,
            align: 8,
        });
        let member_ids: Vec<IdentId> = members.iter().map(|(m, _)| *m).collect();
        self.enums.push((name, EnumDecl { ty, members: member_ids }));
    }

    pub(crate) fn collect_data(
        &mut self,
        node: NodeId,
        kind: DataKind,
        _vis: Vis,
        name: IdentId,
        generics: &[IdentId],
        fields: &[NodeId],
        methods: &[NodeId],
    ) {
        let sp = self.ast.node(node).span;
        if self.find_data(name).is_some() || self.find_enum(name).is_some() || self.find_trait(name).is_some() {
            self.err(sp, format!("duplicate type name `{}`", self.name(name)));
            return;
        }
        if !generics.is_empty() {
            self.err(
                sp,
                format!(
                    "generic user types are not supported in this build (`{}<...>`) — RFC 0013 monomorphization lands in M2",
                    self.name(name)
                ),
            );
        }
        // two passes over fields: first intern the record type with field
        // count, then resolve field types (self-reference is legal —
        // RFC 0009 recursive shapes)
        let placeholder = self.types.intern(RutType {
            name: self.name(name).to_string(),
            kind: TyKind::Data { fields: vec![] },
            size: 0,
            align: 8,
        });
        let mut resolved: Vec<FieldInfo> = Vec::new();
        for f in fields {
            if let NodeKind::FieldDecl { name: fname, ty, is_private, is_static, .. } = &self.ast.node(*f).kind {
                if *is_static {
                    self.err(
                        self.ast.node(*f).span,
                        "class `static` fields are not supported in this build (module-static slot table, RFC 0010 §2)",
                    );
                }
                let fty = self.resolve_type(*ty, &[]);
                resolved.push(FieldInfo {
                    name: self.name(*fname).to_string(),
                    ty: fty,
                    offset: 0,
                });
                // remember privacy for the body compiler
                self.field_privacy(*f, *is_private);
            }
        }
        let (size, align) = {
            // compute layout with resolved fields
            let saved = self.types.types[placeholder as usize].kind.clone();
            self.types.types[placeholder as usize].kind = TyKind::Data { fields: resolved.clone() };
            let l = self.layout_of(placeholder);
            self.types.types[placeholder as usize].kind = saved;
            l
        };
        self.types.types[placeholder as usize].kind = TyKind::Data { fields: resolved };
        self.types.types[placeholder as usize].size = size;
        self.types.types[placeholder as usize].align = align;

        // collect fields with initializers + methods for the compiler
        let mut flds: Vec<(IdentId, TypeId, Option<NodeId>, bool)> = Vec::new();
        for f in fields {
            if let NodeKind::FieldDecl { name: fname, ty: _, init, is_private, .. } = &self.ast.node(*f).kind {
                let fty = match self.types.kind(placeholder) {
                    TyKind::Data { fields } => fields
                        .iter()
                        .find(|x| x.name == self.name(*fname))
                        .map(|x| x.ty)
                        .unwrap_or(TY_I32),
                    _ => TY_I32,
                };
                flds.push((*fname, fty, *init, *is_private));
            }
        }
        let mut mths: Vec<(IdentId, NodeId)> = Vec::new();
        for m in methods {
            if let NodeKind::MethodDecl { name: mname, .. } = &self.ast.node(*m).kind {
                mths.push((*mname, *m));
            }
        }
        self.datas.push((
            name,
            DataDecl { kind, ty: placeholder, fields: flds, methods: mths, generics: generics.to_vec() },
        ));
    }

    pub(crate) fn field_privacy(&mut self, _f: NodeId, _is_private: bool) {}

    pub(crate) fn collect_trait(&mut self, node: NodeId, vis: Vis, name: IdentId, generics: &[NodeId2], methods: &[NodeId]) {
        let sp = self.ast.node(node).span;
        if self.find_trait(name).is_some() || self.find_data(name).is_some() || self.find_enum(name).is_some() {
            self.err(sp, format!("duplicate type name `{}`", self.name(name)));
            return;
        }
        if !generics.is_empty() {
            self.err(
                sp,
                "generic traits are not supported in this build (RFC 0005 Slice<T> lands with M2)",
            );
        }
        let mut tms = Vec::new();
        for m in methods {
            if let NodeKind::MethodDecl { name: mname, params, ret, .. } = &self.ast.node(*m).kind {
                let mut ptys = Vec::new();
                for p in params {
                    match &self.ast.node(*p).kind {
                        NodeKind::SelfParam { .. } => ptys.push(TY_VOID), // placeholder: Self resolved at impl
                        NodeKind::Param { ty: Some(t), .. } => {
                            ptys.push(self.resolve_type(*t, &[]));
                        }
                        NodeKind::Param { ty: None, .. } => {
                            self.err(self.ast.node(*p).span, "trait method parameters need types");
                            ptys.push(TY_I32);
                        }
                        _ => ptys.push(TY_I32),
                    }
                }
                let rty = ret.map(|r| self.resolve_type(r, &[]));
                tms.push((self.name(*mname).to_string(), ptys, rty));
            }
        }
        // first param must be self (RFC 0012 §2: trait methods are instance
        // methods)
        let id = self.traits.len() as u32;
        let mut desc = TraitDesc { name: self.name(name).to_string(), methods: vec![] };
        for (mname, ptys, rty) in tms {
            if ptys.first() == Some(&TY_VOID) {
                // replace the self placeholder: params exclude self in the
                // binary desc; the compiler passes self as arg0
                desc.methods.push(rut_core::binary::TraitMethod {
                    name: mname,
                    params: ptys[1..].to_vec(),
                    ret: rty.unwrap_or(TY_VOID),
                });
            } else {
                self.err(sp, format!("trait method `{mname}` must take `self` (RFC 0012 §2)"));
            }
        }
        let _ = vis;
        self.traits.push(desc);
        self.trait_decls.push((name, TraitDeclInfo { id, node }));
    }

    pub(crate) fn collect_impl(&mut self, node: NodeId, trait_ref: NodeId, target: NodeId, methods: &[NodeId]) {
        let sp = self.ast.node(node).span;
        let Some(trait_id) = self.resolve_trait_ref(trait_ref) else {
            return;
        };
        // the target must be a local dataclass/class (RFC 0012 §2 placement)
        let target_ty = self.resolve_naming_type(target);
        let is_local = match &self.ast.node(target).kind {
            NodeKind::TyPath { segs, .. } if segs.len() == 1 => self
                .find_data(segs[0].name)
                .map(|d| d.ty)
                .is_some(),
            _ => false,
        };
        if !is_local {
            self.err(
                sp,
                "impl target must be a dataclass or class of this module — builtin/foreign impls are registered natively (RFC 0012 §2)",
            );
            return;
        }
        if let Some(_prev) = self.find_impl(trait_id, target_ty) {
            self.err(sp, "duplicate impl for the same (trait, type) pair (RFC 0012 §2)");
            return;
        }
        let mut mths = Vec::new();
        for m in methods {
            if let NodeKind::MethodDecl { name: mname, .. } = &self.ast.node(*m).kind {
                mths.push((*mname, *m));
            }
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
                self.err(
                    self.ast.node(*mnode).span,
                    format!("`{}` is not a member of {} — put inherent methods in the type body (RFC 0012 §2)", self.name(*n), tdesc.name),
                );
            }
        }
        self.impls.push(ImplDecl { trait_id, target: target_ty, methods: mths });
        // every impl method enters the monomorphization queue — vtables need
        // their bodies (RFC 0015 §6)
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
}
