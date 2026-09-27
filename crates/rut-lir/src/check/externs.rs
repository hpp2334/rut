//! The used-module/extern registry: `use`d fns, consts, types, traits, impls and namespaces, plus the `builtin impl` table — the surface bindings the use-both gate (RFC 0012) speaks through.

use super::*;

impl<'a> Ctx<'a> {

    /// Bind a used function before body compilation. `is_async` marks
    /// the host future lane (`host async fn`); `host` carries the
    /// row's registration name (`<scope>::<name>`) when the exporter
    /// is a decl/native module — the minted row family's host ids
    /// derive from it.
    pub fn add_extern_fn(
        &mut self,
        name: IdentId,
        func: u32,
        params: Vec<TypeId>,
        ret: TypeId,
        is_async: bool,
        host: Option<String>,
    ) {
        self.extern_fns
            .insert(name, ExternFn { func, params, ret, is_async, host });
    }

    /// Bind a `builtin impl` numeric method of a primitive (core only,
    /// RFC 0032 §1.1 R2): `(method name → receiver prim, lowering id)`.
    /// AMBIENT — primitive receivers resolve their methods without a
    /// `use`; rut-lir expands the call inline.
    pub fn add_builtin_impl(
        &mut self,
        name: IdentId,
        prim: TypeId,
        intrinsic: rut_core::ops::Intrinsic,
    ) {
        self.builtin_impls.entry(name).or_default().push((prim, intrinsic));
    }

    /// The lowering of `name` on a receiver of primitive type `recv`, if
    /// core's `builtin impl` table declares it.
    pub fn builtin_impl(&self, name: IdentId, recv: TypeId) -> Option<rut_core::ops::Intrinsic> {
        self.builtin_impls
            .get(&name)?
            .iter()
            .find(|(p, _)| *p == recv)
            .map(|(_, i)| *i)
    }

    pub fn extern_fn(&self, name: IdentId) -> Option<&ExternFn> {
        self.extern_fns.get(&name)
    }

    /// Bind a used constant (native modules: `calc::PI`).
    pub fn add_extern_const(&mut self, name: IdentId, ty: TypeId, bits: u64) {
        self.extern_consts.insert(name, (ty, bits));
    }

    pub fn extern_const(&self, name: IdentId) -> Option<(TypeId, u64)> {
        self.extern_consts.get(&name).copied()
    }

    /// Bind a used type name to the exporter's scope-qualified id.
    pub fn add_extern_type(&mut self, name: IdentId, ty: TypeId, is_class: bool) {
        self.extern_types.insert(name, ty);
        if is_class {
            self.extern_classes.insert(ty);
        }
    }

    /// Bind a used core builtin container (`Array`/`Option`/
    /// `Result`/`Opaque` — RFC 0028): the type constructor is the
    /// compiler's own; the binding gates the NAME.
    pub fn add_extern_native_type(&mut self, name: IdentId, kind: rut_core::binary::NativeTy) {
        self.extern_native_types.insert(name, kind);
    }

    /// Bind a used core builtin trait (`Disposal`/`Index`/
    /// `Iterator`): registered as a trait on first reference, like a
    /// declared one — but only for modules that named it.
    pub fn add_extern_trait(&mut self, name: IdentId, native: rut_core::binary::NativeTrait) {
        self.extern_traits.insert(name, native);
    }

    /// Bind a trait from a used module's surface (RFC 0012 §5): the
    /// descriptor joins THIS module's trait table (so slot numbering,
    /// widening and vtables treat it like a declared trait). The name
    /// resolves only when the module used it — the use-both gate's
    /// enforcement point (RFC 0012 §6). Method names re-intern from the
    /// surface's interner; parameter/ret ids pass through verbatim —
    /// they are packed with the scopes the surface's type blocks were
    /// registered under (`use_types`).
    ///
    /// `name` is the consumer-side name (already looked up); `None`
    /// registers the descriptor without a name binding (an impl's trait
    /// the module never named — visible to the use-gate diagnostic,
    /// invisible to resolution).
    pub fn add_extern_trait_decl(
        &mut self,
        name: Option<IdentId>,
        desc: &rut_core::binary::SurfaceTrait,
        surface_names: &Interner,
    ) -> u32 {
        let tname = self.intern(surface_names.name(desc.name));
        let mut methods = Vec::with_capacity(desc.methods.len());
        for m in &desc.methods {
            methods.push(rut_core::binary::TraitMethod {
                name: self.intern(surface_names.name(m.name)),
                params: m.params.clone(),
                ret: m.ret,
            });
        }
        let id = self.traits.len() as u32;
        self.traits.push(TraitDesc { name: tname, methods });
        if let Some(n) = name {
            self.extern_trait_decls.insert(n, ExternTrait { id, generics: desc.generics });
        }
        id
    }

    /// Bind a trait impl registered by a used module's surface
    /// (RFC 0012 §2 — the impl may live in any module). `methods` pair
    /// each trait method with the exporter's scope-qualified fn id
    /// (slot ABI); `methods_concrete` the concrete-ABI twin.
    pub fn add_extern_impl(
        &mut self,
        trait_name: IdentId,
        trait_id: u32,
        target: TypeId,
        methods: Vec<(IdentId, u32)>,
        methods_concrete: Vec<(IdentId, u32)>,
    ) {
        self.extern_impls.push(ExternImpl { trait_id, trait_name, target, methods, methods_concrete });
    }

    /// The id of a used module's exported trait, if the module used the
    /// name (the use-both gate).
    pub fn extern_trait(&self, name: IdentId) -> Option<&ExternTrait> {
        self.extern_trait_decls.get(&name)
    }

    /// Bind a used core compiler-lowered function (`own`,
    /// `downcast`, `assert`/`panic`, the `str`/`bytes` natives).
    pub fn add_extern_native_fn(&mut self, name: IdentId) {
        self.extern_native_fns.insert(name);
    }

    /// Bind a namespace head (RFC 0028): `use calc::{Math}`.
    pub fn add_extern_namespace(&mut self, name: IdentId) {
        self.extern_namespaces.insert(name);
    }

    /// Is `name` a bound namespace head?
    pub fn is_extern_namespace(&self, name: IdentId) -> bool {
        self.extern_namespaces.contains(&name)
    }


    /// Extern impls on `target` whose method set contains `name`
    /// (registered by any module, RFC 0012 §2) — the use-gate diagnostic
    /// reads these even when the trait's name was never used.
    pub fn extern_impl_method(&self, target: TypeId, name: IdentId) -> Option<usize> {
        self.extern_impls.iter().position(|im| {
            im.target == target && im.methods.iter().any(|(n, _)| *n == name)
        })
    }

}
