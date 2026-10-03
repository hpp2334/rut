//! Pkg-graph compilation — the driver half.
//!
//! Walks a root module's `use` statements through a [`Session`]. Every
//! module compiles under its own scope and LINKS: its `Surface` binds
//! into each using module, class methods cross on the surface's
//! inherent rows (the linkable-classes phase), and a generic export's
//! instantiations are requested from the declaring package
//! (owner-anchored generics). No source crosses a boundary — there is
//! no splice, no second compilation model.
//!
//! The dispatch is on the mounted [`PkgBody`]: a source body takes
//! the compile path (its peer-integration groups, presence-gated, ride
//! the same unit), a host body synthesizes its placeholder program,
//! and a compiled body (a v5 bundle's decoded `.rutc`) is pushed after
//! a fresh-scope rebase of its packed ids.
//!
//! Programs are pushed in post-order, so the link order registers every
//! scope before a dependent references it.


mod compiler;
mod seeds;

pub(crate) use compiler::uses_of;

use compiler::{GraphCompiler};
use seeds::build_seed_groups;

use std::collections::{HashMap, HashSet};

use rut_lexer::diag::Diag;
use rut_lexer::span::Span;
use rut_core::binary::Program;

use crate::session::Session;

/// A linked module graph.
pub struct GraphOutput {
    pub diags: Vec<Diag>,
    /// the flattened (dense-id) program — ready for `encode` and the VM
    pub program: Option<Program>,
}

/// Compile `root_spec` and its transitive uses from `session`.
pub(crate) fn compile_graph(session: &Session, root_spec: &str) -> GraphOutput {
    let units = compile_units(session, root_spec);
    if !units.ok {
        return GraphOutput { diags: units.diags, program: None };
    }
    match rut_core::link::link(units.programs) {
        Ok(p) => GraphOutput { diags: units.diags, program: Some(p) },
        Err(e) => {
            let mut diags = units.diags;
            diags.push(Diag::new(Span::new(0, 0), format!("link: {e}")));
            GraphOutput { diags, program: None }
        }
    }
}

/// Per-package compile results — [`compile_graph`]'s walk, exposed for
/// the packer: the pushed programs in post-order, and each linked
/// module's own program and scope.
pub(crate) struct Units {
    pub diags: Vec<Diag>,
    /// every pushed program, post-order (compile_graph links these)
    pub programs: Vec<Program>,
    /// each ensured-and-linked module: spec → (index into `programs`, scope)
    pub linked: HashMap<String, (usize, rut_core::ScopeId)>,
    /// `true` when the root produced a program (the walk succeeded)
    pub ok: bool,
}

/// Compile (or mount) every module in `root_spec`'s use closure — the
/// packer's view of [`compile_graph`].
pub(crate) fn compile_units(session: &Session, root_spec: &str) -> Units {
    let mut c = GraphCompiler {
        session,
        next_scope: 1,
        programs: Vec::new(),
        done: HashMap::new(),
        visiting: HashSet::new(),
        diags: Vec::new(),
        unit_src: HashMap::new(),
        in_flight: HashMap::new(),
        body_kind: HashMap::new(),
        requests: Vec::new(),
        seed_pool: HashMap::new(),
        seeded_keys: HashSet::new(),
        fresh_owners: HashSet::new(),
    };
    let ok = c.ensure(root_spec).is_some();
    c.resolve_requests();
    let (diags, programs, linked) = c.finish();
    Units { diags, programs, linked, ok }
}

