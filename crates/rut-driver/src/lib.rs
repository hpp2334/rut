//! Pipeline driver: Ast ─► resolve ─► typecheck (fused with
//! body compilation, M1) ─► LIR ─► binary emit. The embedder doors:
//! the run chain (`RutRun::new()..compile()`) and
//! `compile_module` (the single-source lane). PROVABLY PURE: no fs,
//! no net, no walk, no `Path` anywhere in this crate (the grep gate)
//! — the directory walk, the `Source`/`DepRemote` traits, the
//! remotes, and the pack lanes live in `rut-native`.

pub(crate) mod session;

pub mod bundle;
pub mod compile;
pub mod decl;
pub(crate) mod graph;
pub mod loader;
pub mod pack;
pub mod run;

pub use bundle::{parse_manifest, Manifest};
pub use compile::{
    compile_module, compile_program, compile_program_resolved, ir_dump_of, CompileOutput,
    ProgramOutput, SeedGroup, Seeds,
};
pub use decl::lower_decl_module;
pub use graph::GraphOutput;
pub use loader::{
    bundle_entry_pkg, bundle_walk_bytes, riding_gen_source, sha256_hex,
};
pub use pack::{pack, Archive, PackError, PackOpts, PackWorld, PkgSource, FORMAT_VERSION, FORMAT_VERSION_DECL};
pub use run::{Compiled, Loaded, Pkg, PkgBody, RunError, RutRun, declared_host_fns, host_pkg_ctx};
pub use session::{GenSource, HostRow, PeerDecl};
