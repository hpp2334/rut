//! The seed types — the owner side of consumer instantiation
//! requests, handed to [`super::program::compile_program_resolved`].

/// The owner side of consumer requests, seeded into a recompiled unit.
/// One group per requester: the requester's reachable type descriptors
/// (registered as a sparse block under the requester's scope — the
/// locals match the requester's own layout, which is what makes the
/// instantiation keys agree), and the instantiations to materialize.
#[derive(Clone, Default)]
pub struct Seeds<'a> {
    pub groups: &'a [SeedGroup<'a>],
}

/// One requester's seed block: the descriptor closure as
/// `(scope, local) → row` pairs + the requester program's interner
/// (the descriptors' name ids resolve against it, then re-intern into
/// the owner's). The attach side merges EVERY group's rows into one
/// run per scope — two requesters' rows share a scope (the classes of
/// a common dependency), and a second base registration would strand
/// the first group's ids.
#[derive(Clone)]
pub struct SeedGroup<'a> {
    pub rows: Vec<((rut_core::ScopeId, u32), rut_core::types::RutType)>,
    pub names: &'a rut_core::Interner,
    /// `(decl name text, args in the registered blocks' id space, the
    /// method names whose bodies this request needs — empty for a
    /// plain type instantiation)`
    pub insts: Vec<(String, Vec<rut_core::types::TypeId>, Vec<String>)>,
    /// `(fn name text, type arguments)` — the generic fn bodies whose
    /// mirrors this requester calls
    pub fns: Vec<(String, Vec<rut_core::types::TypeId>)>,
    /// `(trait text, target row in the requester's space, method text)`
    /// — the generic-target impl methods whose mirrors this requester
    /// calls; the owner mints the template impl at the target and
    /// compiles the body
    pub impl_methods: Vec<(String, rut_core::types::TypeId, String)>,
    /// the requester's own impl registrations, scope-qualified into
    /// the requester's scope: a generic body the owner compiles for
    /// this requester dispatches through THESE impls (a consumer's
    /// `impl JsonSerialize for Json` lives in the consumer — the
    /// owner's monomorphized `encodeJson<Json>` binds it as a foreign
    /// registration; link rebases the fn ids onto the requester's
    /// block)
    pub impls: Vec<SeedImpl>,
}

/// One seed impl row (see [`SeedGroup::impls`]).
#[derive(Clone)]
pub struct SeedImpl {
    pub trait_name: String,
    /// the target type id, packed with the requester's scope
    pub target: rut_core::types::TypeId,
    /// (method name text, requester-scope fn id) — slot ABI
    pub methods: Vec<(String, u32)>,
    /// (method name text, requester-scope fn id) — concrete ABI
    pub methods_concrete: Vec<(String, u32)>,
}

impl<'a> Seeds<'a> {
    pub fn none() -> Seeds<'a> {
        Seeds { groups: &[] }
    }
}

