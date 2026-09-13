//! Type resolution: primitives, builtins (Array/Option/Result), user types
//! (including `std:collection`'s `Vec` class), dyn I, fn types;
//! naming-position resolution.

use rut_core::types::*;
use super::*;

impl<'a> Ctx<'a> {

    pub fn resolve_trait_ref(&mut self, node: NodeHandle<AnyTy>) -> Option<u32> {
        match self.ast.ty(node).clone() {
            TypeKind::TyPath { segs, .. } if segs.len() == 1 => {
                let tname = segs[0].name;
                // an imported std:core interface (RFC 0028): `Index`/
                // `Iterator`/`Disposal` are ordinary names gated on the
                // import — never ambient
                let core_iface = self.extern_ifaces.get(&tname).copied();
                let id = if let Some(t) = self.find_trait(tname).cloned() {
                    if segs[0].generics.is_empty() {
                        if t.id == u32::MAX {
                            self.err(self.ast.span(node.id()), format!(
                                "generic interface `{}` needs type arguments in an impl head (e.g. `impl {}<i32> for ..`)",
                                self.name(tname), self.name(tname)
                            ));
                            None
                        } else {
                            Some(t.id)
                        }
                    } else {
                        let args: Vec<TypeId> = segs[0]
                            .generics
                            .iter()
                            .map(|g| self.resolve_type(*g, &[]))
                            .collect();
                        if args.len() != t.generics.len() {
                            self.err(self.ast.span(node.id()), format!(
                                "`{}` takes {} type parameter(s), {} given",
                                self.name(tname), t.generics.len(), args.len()
                            ));
                            None
                        } else {
                            Some(self.mk_trait_inst(tname, args))
                        }
                    }
                } else {
                    match core_iface {
                        Some(rut_core::binary::NativeIface::Index) => {
                            Some(self.builtin_contract(tname, true))
                        }
                        Some(rut_core::binary::NativeIface::Iterator) => {
                            Some(self.builtin_contract(tname, false))
                        }
                        Some(rut_core::binary::NativeIface::Disposal) => {
                            Some(self.builtin_disposal(tname))
                        }
                        None => {
                            let n = self.name(tname).to_string();
                            let msg = self
                                .not_in_core_scope(&n)
                                .unwrap_or_else(|| format!("unknown interface `{n}`"));
                            self.err(self.ast.span(node.id()), msg);
                            None
                        }
                    }
                };
                if core_iface == Some(rut_core::binary::NativeIface::Index) {
                    self.seq_trait = id;
                }
                if core_iface == Some(rut_core::binary::NativeIface::Iterator) {
                    self.iter_trait = id;
                }
                id
            }
            _ => {
                self.err(self.ast.span(node.id()), "expected an interface name");
                None
            }
        }
    }

    /// The builtin sequence (`seq = true` → `Index`: `len`/`get`/`set`) or
    /// iterator (`seq = false` → `Iterator`: `next`) contract. Undeclarable
    /// in source, registered on first reference. The element type is carried
    /// as the impl/interface ref's type argument and resolved at the use
    /// site.
    pub fn builtin_contract(&mut self, name: IdentId, seq: bool) -> u32 {
        if let Some(id) = self.trait_id_of(name) {
            return id;
        }
        let id = self.traits.len() as u32;
        use rut_core::binary::TraitMethod;
        let (desc_name, methods) = if seq {
            (
                "Index",
                vec![
                    TraitMethod { name: "len".to_string(), params: vec![], ret: TY_I32 },
                    TraitMethod { name: "get".to_string(), params: vec![TY_I32], ret: TY_I32 },
                    TraitMethod { name: "set".to_string(), params: vec![TY_I32, TY_I32], ret: TY_UNIT },
                ],
            )
        } else {
            ("Iterator", vec![TraitMethod { name: "next".to_string(), params: vec![], ret: TY_I32 }])
        };
        self.traits.push(TraitDesc { name: desc_name.to_string(), methods });
        self.trait_decls.push((name, TraitDeclInfo {
            id,
            node: rut_ast::ast::NodeId(0),
            generics: vec![],
        }));
        id
    }

    /// The builtin `Disposal` contract (RFC 0011/0016, declared in
    /// std:core): `fn dispose(mut self) -> unit`. Registered on first
    /// reference like the sequence contracts — an ordinary trait with
    /// ordinary nominal impls; the rc-0 destructor wiring is the host/VM's,
    /// not a compiler name match.
    pub fn builtin_disposal(&mut self, name: IdentId) -> u32 {
        if let Some(id) = self.trait_id_of(name) {
            return id;
        }
        let id = self.traits.len() as u32;
        self.traits.push(TraitDesc {
            name: "Disposal".to_string(),
            methods: vec![rut_core::binary::TraitMethod {
                name: "dispose".to_string(),
                params: vec![],
                ret: TY_UNIT,
            }],
        });
        self.trait_decls.push((name, TraitDeclInfo {
            id,
            node: rut_ast::ast::NodeId(0),
            generics: vec![],
        }));
        id
    }

    /// A type in a NAMING position (impl heads, requires lists, is RHS) —
    /// bare trait name → the trait's dyn-obj type here means the TYPE;
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
            TypeKind::TyConst(_) => {
                self.err(sp, "a const expression is not a type here");
                TY_I32
            }
            TypeKind::TyPath { segs, .. } => {
                if segs.len() > 1 {
                    self.err(sp, format!("unknown type `{}`", seg_str(self, segs)));
                    return TY_I32;
                }
                let seg = &segs[0];
                let name = seg.name;
                let n = self.name(name).to_string();
                // generic param?
                if seg.generics.is_empty() {
                    if let Some((_, t)) = env.iter().find(|(p, _)| *p == name) {
                        return *t;
                    }
                }
                // primitives & builtins
                let prim = match n.as_str() {
                    "unit" => Some(TY_UNIT),
                    "u8" => Some(TY_U8), "u16" => Some(TY_U16), "u32" => Some(TY_U32), "u64" => Some(TY_U64),
                    "i8" => Some(TY_I8), "i16" => Some(TY_I16), "i32" => Some(TY_I32), "i64" => Some(TY_I64),
                    "f32" => Some(TY_F32), "f64" => Some(TY_F64),
                    "bool" => Some(TY_BOOL), "char" => Some(TY_CHAR),
                    "str" => Some(TY_STR),
                    "bytes" => Some(TY_BYTES),
                    _ => None,
                };
                if let Some(p) = prim {
                    if !seg.generics.is_empty() {
                        self.err(sp, format!("`{n}` takes no generic arguments"));
                    }
                    return p;
                }
                // Weak<T> — not in this build
                if n == "Weak" {
                    self.err(sp, "Weak references are not supported in this build (RFC 0017, M5)");
                    return TY_I32;
                }
                if n == "dyn" {
                    self.err(sp, "`dyn` was removed — an interface name in type position is the object type (RFC 0012)");
                    return TY_I32;
                }
                // an imported std:core builtin container (RFC 0028): the
                // prelude is imported, never ambient — `Array`/`Option`/
                // `Result`/`Opaque` resolve only when the name was bound
                // from the std:core surface
                let core_ty = self.extern_native_types.get(&name).copied();
                // a declared or imported type shadows a builtin name (RFC
                // 0005: `std:collection`'s `Vec` is an ordinary class, so it
                // never reaches the builtin table)
                let shadow = matches!(n.as_str(), "Array" | "Option" | "Result" | "Opaque")
                    && (self.find_data(name).is_some() || self.extern_types.contains_key(&name));
                if let Some(kind) = core_ty.filter(|_| !shadow) {
                    let generics = seg.generics.clone();
                    return match (kind, generics.as_slice()) {
                        (rut_core::binary::NativeTy::Array, [e]) => {
                            let t = self.resolve_type(*e, env);
                            self.mk_array(t)
                        }
                        (rut_core::binary::NativeTy::Array, _) => {
                            self.err(sp, "Array takes one generic argument: Array<T>");
                            TY_I32
                        }
                        (rut_core::binary::NativeTy::Option, [e]) => {
                            let t = self.resolve_type(*e, env);
                            self.mk_option(t)
                        }
                        (rut_core::binary::NativeTy::Option, _) => {
                            self.err(sp, "Option takes one generic argument");
                            TY_I32
                        }
                        (rut_core::binary::NativeTy::Result, [o, e]) => {
                            let ok = self.resolve_type(*o, env);
                            let err = self.resolve_type(*e, env);
                            self.mk_result(ok, err)
                        }
                        (rut_core::binary::NativeTy::Result, _) => {
                            self.err(sp, "Result takes two generic arguments");
                            TY_I32
                        }
                        (rut_core::binary::NativeTy::Opaque, []) => TY_OPAQUE,
                        (rut_core::binary::NativeTy::Opaque, _) => {
                            self.err(sp, "`Opaque` takes no generic arguments");
                            TY_I32
                        }
                    };
                }
                {
                    // user types
                        if let Some(e) = self.find_enum(name).cloned() {
                            if !seg.generics.is_empty() {
                                self.err(sp, format!("enum `{n}` takes no generic arguments"));
                            }
                            return e.ty;
                        }
                        if let Some(d) = self.find_data(name).cloned() {
                            if d.generics.is_empty() {
                                if !seg.generics.is_empty() {
                                    self.err(sp, format!("`{n}` takes no generic arguments"));
                                }
                                return d.ty;
                            }
                            if seg.generics.len() != d.generics.len() {
                                self.err(sp, format!(
                                    "`{n}` takes {} generic argument(s), {} given",
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
                            return self.mk_data_inst(name, args);
                        }
                        if let Some(t) = self.find_trait(name).cloned() {
                            // an interface name in type position IS the
                            // object type (RFC 0012) — no `dyn` prefix
                            if seg.generics.is_empty() {
                                if t.id == u32::MAX {
                                    self.err(sp, format!(
                                        "generic interface `{n}` needs type arguments (e.g. `{n}<i32>`)"
                                    ));
                                    return TY_I32;
                                }
                                return self.mk_dyn(t.id);
                            }
                            if seg.generics.len() != t.generics.len() {
                                self.err(sp, format!(
                                    "`{n}` takes {} type argument(s), {} given",
                                    t.generics.len(), seg.generics.len()
                                ));
                                return TY_I32;
                            }
                            let args: Vec<TypeId> = seg
                                .generics
                                .iter()
                                .map(|g| self.resolve_type(*g, env))
                                .collect();
                            let id = self.mk_trait_inst(name, args);
                            return self.mk_dyn(id);
                        }
                        // imported type (RFC 0035 §1): the exporter's
                        // scope-qualified id; link rebases it
                        if let Some(&t) = self.extern_types.get(&name) {
                            if !seg.generics.is_empty() {
                                self.err(sp, format!("imported type `{n}` takes no generic arguments"));
                            }
                            return t;
                        }
                        if n == "Self" {
                            self.err(sp, "`Self` is only valid inside a type body (RFC 0010 §1)");
                            return TY_I32;
                        }
                        let msg = self
                            .not_in_core_scope(&n)
                            .unwrap_or_else(|| format!("unknown type `{n}`"));
                        self.err(sp, msg);
                        TY_I32
                    }
            }
        }
    }

}
