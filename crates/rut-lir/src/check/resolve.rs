//! Type resolution: primitives, builtins (Array/Option/Result), user types
//! (including `pouch`'s `Vec` class), trait objects, fn types;
//! naming-position resolution.

use rut_core::types::*;
use super::*;

impl<'a> Ctx<'a> {

    pub fn resolve_trait_ref(&mut self, node: NodeHandle<AnyTy>) -> Option<u32> {
        match self.ast.ty(node).clone() {
            TypeKind::TyPath { segs, .. } if segs.len() == 1 => {
                let tname = segs[0].name;
                // a used core trait (RFC 0028) — the prelude's
                // only builtin trait is the `Iterator<E>` protocol (RFC 0012 §6)
                let core_trait = self.extern_traits.get(&tname).copied();
                if core_trait == Some(rut_core::binary::NativeTrait::Iterator) {
                    let args: Vec<TypeId> = segs[0]
                        .generics
                        .iter()
                        .map(|g| self.resolve_type(*g, &[]))
                        .collect();
                    if args.len() != 1 {
                        self.err(self.ast.span(node.id()), format!(
                            "`Iterator` takes 1 type parameter, {} given — `Iterator<E>` (RFC 0012 §6)",
                            args.len()
                        ));
                        return None;
                    }
                    return Some(self.mk_iterator_inst(tname, args[0]));
                }
                let id = if let Some(t) = self.find_trait(tname).cloned() {
                    if segs[0].generics.is_empty() {
                        if t.id == u32::MAX {
                            self.err(self.ast.span(node.id()), format!(
                                "generic trait `{}` needs type arguments in an impl head (e.g. `impl {}<i32> for ..`)",
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
                    let msg = self
                        .not_in_core_scope(tname)
                        .unwrap_or_else(|| format!("unknown trait `{}`", self.name(tname)));
                    self.err(self.ast.span(node.id()), msg);
                    None
                };
                id
            }
            _ => {
                self.err(self.ast.span(node.id()), "expected a trait name");
                None
            }
        }
    }

    /// The `Iterator<E>` protocol contract (RFC 0012 §6): one trait per
    /// type-argument list, its single method `__iterate(emit: fn(E) -> bool)`.
    /// Duck-typed satisfaction fills its vtable slot from the iterable's own
    /// member — the contract is engine-woven, not user-declarable.
    pub fn mk_iterator_inst(&mut self, name: IdentId, arg: TypeId) -> u32 {
        if let Some(&id) = self.trait_inst.get(&(name, vec![arg])) {
            return id;
        }
        let id = self.traits.len() as u32;
        let tname = self.intern(&format!("Iterator<{}>", self.type_name(arg)));
        let emit = self.mk_fn_ty(vec![arg], TY_BOOL);
        self.traits.push(TraitDesc {
            name: tname,
            methods: vec![rut_core::binary::TraitMethod {
                name: sym::ITERATE,
                params: vec![emit],
                ret: TY_NIL,
            }],
        });
        self.trait_inst.insert((name, vec![arg]), id);
        id
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
            TypeKind::TyPtr { inner } => {
                let elem = self.resolve_type(*inner, env);
                self.mk_ptr(elem)
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
                // v1.1 removals first: a removed type explains itself
                if let Some(msg) = rut_core::binary::removed_core(self.name(name)) {
                    self.err(sp, msg);
                    return TY_I32;
                }
                let prim = sym::primitive_ty(name);
                if let Some(p) = prim {
                    if !seg.generics.is_empty() {
                        self.err(sp, format!("`{}` takes no generic arguments", self.name(name)));
                    }
                    return p;
                }
                // Weak<T> — not in this build
                if self.name(name) == "Weak" {
                    self.err(sp, "Weak references are not supported in this build (RFC 0017, M5)");
                    return TY_I32;
                }
                // a used core builtin container (RFC 0028): the
                // prelude is used, never ambient — `Array`/`Opaque`
                // resolve only when the name was bound from the core
                // surface
                let core_ty = self.extern_native_types.get(&name).copied();
                // a declared or used type shadows a builtin name (RFC
                // 0005: `pouch`'s `Vec` is an ordinary class, so it
                // never reaches the builtin table)
                let shadow = matches!(name, sym::ARRAY | sym::OPAQUE)
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
                            return self.mk_data_inst(name, args);
                        }
                        // the `Iterator<E>` protocol (RFC 0012 §6):
                        // engine-woven — its trait is built directly per
                        // type-argument list, before the AST-decl lookup
                        if self.extern_traits.get(&name).copied()
                            == Some(rut_core::binary::NativeTrait::Iterator)
                        {
                            let args: Vec<TypeId> = seg
                                .generics
                                .iter()
                                .map(|g| self.resolve_type(*g, env))
                                .collect();
                            if args.len() != 1 {
                                self.err(sp, format!(
                                    "`Iterator` takes 1 type parameter, {} given — `Iterator<E>` (RFC 0012 §6)",
                                    args.len()
                                ));
                                return TY_I32;
                            }
                            let id = self.mk_iterator_inst(name, args[0]);
                            return self.mk_trait_obj(id);
                        }
                        if let Some(t) = self.find_trait(name).cloned() {
                            // a trait name in type position IS the
                            // object type (RFC 0012) — the bare name spells it
                            if seg.generics.is_empty() {
                                if t.id == u32::MAX {
                                    self.err(sp, format!(
                                        "generic trait `{}` needs type arguments (e.g. `{}<i32>`)",
                                        self.name(name),
                                        self.name(name)
                                    ));
                                    return TY_I32;
                                }
                                return self.mk_trait_obj(t.id);
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
                            let id = self.mk_trait_inst(name, args);
                            return self.mk_trait_obj(id);
                        }
                        // used type (RFC 0035 §1): the exporter's
                        // scope-qualified id; link rebases it
                        if let Some(&t) = self.extern_types.get(&name) {
                            if !seg.generics.is_empty() {
                                self.err(sp, format!("used type `{}` takes no generic arguments", self.name(name)));
                            }
                            return t;
                        }
                        if name == sym::SELF_TY {
                            self.err(sp, "`Self` is only valid inside a type body (RFC 0010 §1)");
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
