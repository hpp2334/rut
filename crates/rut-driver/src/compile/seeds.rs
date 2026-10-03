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
    /// the requesting unit's spec — the carried rows' home (the
    /// carried-member law's mirror owner anchor)
    pub requester: String,
    pub names: &'a rut_core::Interner,
    /// `(decl name text, args in the registered blocks' id space, the
    /// method names whose bodies this request needs — empty for a
    /// plain type instantiation)`
    pub insts: Vec<(String, Vec<rut_core::types::TypeId>, Vec<String>)>,
    /// `(fn name text, type arguments)` — the generic fn bodies whose
    /// mirrors this requester calls
    pub fns: Vec<(String, Vec<rut_core::types::TypeId>)>,
}

impl<'a> Seeds<'a> {
    pub fn none() -> Seeds<'a> {
        Seeds { groups: &[] }
    }
}

