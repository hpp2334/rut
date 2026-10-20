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
pub mod lock;
pub mod mods;
pub mod pack;
pub mod run;

pub use bundle::{parse_manifest, Manifest};
pub use compile::{
    compile_module, compile_program, compile_program_pre, compile_program_resolved, ir_dump_of,
    CompileOutput, ProgramOutput, SeedGroup, Seeds,
};
pub use decl::lower_decl_module;
pub use graph::GraphOutput;
#[allow(deprecated)] // bundle_walk_bytes retires with its family (one release)
pub use loader::bundle_walk_bytes;
pub use loader::{bundle_entry_pkg, sha256_hex};
/// The pin ledger: `rut.lock` parses and renders through here —
/// one lock per walk root, beside the root `rut.jsonc`.
pub use lock::{parse_lock, render_lock, LockError, LockFile, LockRow};
pub use mods::{
    collect_module_set, mount_mod_children, ChildLookup, ModSource, ModuleSet, ModuleUnit,
};
pub use pack::{
    lower_inline_entry, normalize_decl_tree, pack, pack_inline_pkg, rows_agree, Archive, PackError,
    PackOpts, PackWorld, PkgSource, FORMAT_VERSION,
};
#[allow(deprecated)] // the deprecated one-release aliases re-export with the real names
pub use run::{
    declared_host_fns, host_pkg_ctx, Bundle, Compiled, Loaded, Mounted, MountedBody, Offer, Pkg,
    PkgBody, RunError, RutRun, Walk,
};
/// The file-module compile inputs (phase 3) — re-exported for the
/// tests and embedders that spell a unit's module layout directly.
pub use rut_lir::check::{ModInputs, ModScope};
/// The parse mode the mount flavors speak (`Mode::Impl` for a lib
/// body tree, `Mode::Decl` for a host surface tree) — re-exported so
/// the FS/archive lanes can name the mount core's parameter.
pub use rut_parser::Mode;
pub use session::HostRow;
