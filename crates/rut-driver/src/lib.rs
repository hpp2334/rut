//! Pipeline driver: Ast ─► resolve ─► typecheck (fused with
//! body compilation, M1) ─► LIR ─► binary emit. One entry point
//! for the CLI and the wasm demo: `compile_module`.

pub mod bundle;
pub mod compile;
pub mod decl;
pub mod graph;
pub mod loader;
pub mod pack;
pub mod session;

pub use compile::{
    compile_module, compile_module_in, compile_program, compile_program_resolved, ir_dump_of,
    mount_calc, mount_std, mount_std_async, mount_std_core, CompileOutput, ProgramOutput,
    SeedGroup, SeedImpl, Seeds,
};
pub use decl::lower_decl_module;
pub use graph::{compile_graph, compile_units, GraphOutput, Units};
pub use loader::{
    apply_symbols_to_session, assemble_peers, compile_dir, load_bundle_bytes,
    load_bundle_session, load_dir_session, load_dir_session_fetched, load_dir_session_with,
    load_module_source, load_path_session, load_path_session_with, mount_bundle_bytes,
    mount_dir, mount_dir_with, sha256_hex, Archive, DepRemote, HttpRemote, LoadError, Loaded,
    LoadedDir, Loader, RemoteError,
};
pub use pack::{
    pack_dir, pack_dir_fetched, pack_dir_opts, pack_dir_opts_fetched, pack_dir_opts_with,
    pack_dir_with, PackError, PackOpts,
};
pub use session::{Module, ModuleBody, PeerDecl, ResolveError, Session};
