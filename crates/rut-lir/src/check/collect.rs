//! Collection (RFC 0031 SS1): pass 1 declares types (enums, dataclasses,
//! classes, traits), pass 2 impls/fns/lets. Field layout is repr-C
//! (RFC 0015 SS2); impl methods enter the instantiation queue eagerly.

use rut_core::binary::TraitDesc;
use rut_core::types::*;
use super::*;

impl<'a> Ctx<'a> {

    // ---- collection ----

    pub fn collect(&mut self) {
        let items = self.ast.module_items(self.ast.root).to_vec();
        // pass 1: type declarations (enums, dataclasses, classes, traits)
        for it in &items {
            match self.ast.item(*it) {
            ItemKind::Enum { vis, name, members } => self.collect_enum(it.id(), *vis, *name, members),
            ItemKind::Dataclass { vis, name, generics, fields, methods } => {
                self.collect_data(it.id(), DataKind::Dataclass, *vis, *name, generics, fields, methods);
            }
            ItemKind::Class { vis, name, generics, fields, methods } => {
                self.collect_data(it.id(), DataKind::Class, *vis, *name, generics, fields, methods);
            }
            ItemKind::Trait { vis, name, generics, methods, .. } => {
                self.collect_trait(it.id(), *vis, *name, generics, methods)
            }
                _ => {}
            }
        }
        // data field types may reference enums/other datas — re-resolve now
        // (fields were resolved with a second pass inside collect_data)
        // pass 2: impls, fns, lets
        for it in &items {
            match self.ast.item(*it) {
                ItemKind::Impl { trait_ref, target, methods } => {
                    self.collect_impl(it.id(), *trait_ref, *target, methods)
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
                    // module loading is M2+ (RFC 0035): the corpus parses,
                    // and unresolvable imports are compile errors only when
                    // their names are used
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
        fields: &[NodeHandle<FieldDeclNode>],
        methods: &[NodeHandle<MethodDeclNode>],
    ) {
        let sp = self.ast.span(node);
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
                offset: 0,
            });
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
        let mut flds: Vec<(IdentId, TypeId, Option<NodeHandle<AnyExpr>>, Option<Vis>)> = Vec::new();
        for f in fields {
            let fd = self.ast.field_decl(*f);
            let fty = match self.types.kind(placeholder) {
                TyKind::Data { fields } => fields
                    .iter()
                    .find(|x| x.name == self.name(fd.name))
                    .map(|x| x.ty)
                    .unwrap_or(TY_I32),
                _ => TY_I32,
            };
            flds.push((fd.name, fty, fd.init, fd.vis));
        }
        let mut mths: Vec<(IdentId, NodeHandle<MethodDeclNode>)> = Vec::new();
        for m in methods {
            mths.push((self.ast.method_decl(*m).name, *m));
        }
        self.datas.push((
            name,
            DataDecl { kind, ty: placeholder, fields: flds, methods: mths, generics: generics.to_vec() },
        ));
    }

    pub(crate) fn collect_trait(&mut self, node: NodeId, vis: Vis, name: IdentId, generics: &[NodeId2], methods: &[NodeHandle<MethodDeclNode>]) {
        let sp = self.ast.span(node);
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
            let md = self.ast.method_decl(*m);
            let mut ptys = Vec::new();
            for p in &md.params {
                match self.ast.param(*p) {
                    MemberKind::SelfParam(_) => ptys.push(TY_UNIT), // placeholder: Self resolved at impl
                    MemberKind::Param(ParamData { ty: Some(t), .. }) => {
                        ptys.push(self.resolve_type(*t, &[]));
                    }
                    MemberKind::Param(ParamData { ty: None, .. }) => {
                        self.err(self.ast.span(p.id()), "trait method parameters need types");
                        ptys.push(TY_I32);
                    }
                    _ => ptys.push(TY_I32),
                }
            }
            let rty = md.ret.map(|r| self.resolve_type(r, &[]));
            tms.push((self.name(md.name).to_string(), ptys, rty));
        }
        // first param must be self (RFC 0012 §2: trait methods are instance
        // methods)
        let id = self.traits.len() as u32;
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
        let _ = vis;
        self.traits.push(desc);
        self.trait_decls.push((name, TraitDeclInfo { id, node }));
    }

    pub(crate) fn collect_impl(&mut self, node: NodeId, trait_ref: NodeHandle<AnyTy>, target: NodeHandle<AnyTy>, methods: &[NodeHandle<MethodDeclNode>]) {
        let sp = self.ast.span(node);
        let Some(trait_id) = self.resolve_trait_ref(trait_ref) else {
            return;
        };
        // the target must be a local dataclass/class (RFC 0012 §2 placement)
        let target_ty = self.resolve_naming_type(target);
        let is_local = match self.ast.ty(target) {
            TypeKind::TyPath { segs, .. } if segs.len() == 1 => self
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
                self.err(
                    self.ast.span(mnode.id()),
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
