//! The native host half — everything with a filesystem or a wire.
//!
//! The driver (`rut-driver`) is provably pure: no fs, no net, no walk,
//! no `std::path`. This crate is its counterpart — the walk and the
//! world:
//!
//! - [`Source`] — the one filesystem door, STRING keys: the key math
//!   (joins, roots, canonicalization) is the impl's, never the walk's.
//!   [`FsSource`] is the real filesystem with the root baked in
//!   ([`FsSource::at`]); a `Path` crosses nothing.
//! - [`DepRemote`] / [`HttpRemote`] — the url-dep policy: how bytes
//!   arrive and how they cache. Cache-first, wire-on-miss, offline
//!   flavor included.
//! - [`load_dir`] / [`load_dir_with`] — the directory walk with all
//!   its laws: `[deps]` recursion, the sha256 pin verified at the
//!   mount door, the root-only `[dev-deps]` pass, the peer gate, url
//!   prefetch. The yield is [`rut_driver::Loaded`] — offer its pkgs to
//!   a run with [`rut_driver::RutRun::pkg`].
//! - [`dir_pkgs`] / [`dir_pkgs_with`] — the OFFER lane: a package
//!   directory walked for someone else's program (no dev pass, no
//!   gate — presence is the run's own `.compile()` law).
//! - [`tree_pkg`] — one toolchain-tree package (`rut/<name>`) as
//!   walked pkgs; the hosts' offer for the engine-adjacent surfaces
//!   (`calc`, `futures`, …).
//! - [`pack_dir`] and family — the pack lanes: walk + dev tables +
//!   the one peer gate, then the driver's pure emit.
//! - [`default_out_path`] — the conventional pack output path.
//!
//! The wasm hosts (`rut-wasm`, the demo page) mount in memory and
//! offer pkgs by hand — they depend on the driver alone, never here.

mod error;
mod pack;
mod remote;
mod source;
mod walk;

pub use error::{LoadError, RemoteError};
pub use pack::{
    default_out_path, pack_dir, pack_dir_fetched, pack_dir_opts, pack_dir_opts_fetched,
    pack_dir_opts_with, pack_dir_with,
};
pub use remote::{DepRemote, HttpRemote};
pub use source::{FsSource, Source};
pub use walk::{
    dir_pkgs, dir_pkgs_with, load_bundle_session, load_dir, load_dir_fetched, load_dir_with,
    load_module_source, load_path_session, load_path_session_with, prefetch_urls, tree_pkg,
    Archive,
};
