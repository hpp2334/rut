//! Pipeline driver: Ast ─► resolve ─► typecheck (fused with
//! body compilation, M1) ─► LIR ─► binary emit. The embedder doors:
//! the run chain (`RutRun::new()..compile()`) and
//! `compile_module` (the single-source lane).

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
    calc_pkg, compile_module, compile_program, compile_program_resolved, core_pkg, ir_dump_of,
    std_async_pkgs, CompileOutput, ProgramOutput, SeedGroup, Seeds,
};
pub use decl::lower_decl_module;
pub use graph::GraphOutput;
pub use loader::{
    dir_pkgs, dir_pkgs_with, load_bundle_session, load_dir, load_dir_fetched, load_dir_with,
    load_module_source, load_path_session, load_path_session_with, sha256_hex, Archive, DepRemote,
    HttpRemote, RemoteError,
};
pub use pack::{pack_dir, pack_dir_fetched, pack_dir_opts, pack_dir_opts_fetched, pack_dir_opts_with, pack_dir_with, PackError, PackOpts};
pub use run::{Compiled, Loaded, Pkg, PkgBody, RunError, RutRun, declared_host_fns, host_pkg_ctx};
pub use session::{GenSource, HostRow, PeerDecl};
