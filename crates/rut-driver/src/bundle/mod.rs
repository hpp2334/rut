//! The `.rutbundle` container — one module owns the whole format:
//! the container codec ([`container`]), the manifest grammar
//! ([`manifest`]), the source file-set helpers
//! ([`files::collect_source_group`]), and the reader ([`read`], whose
//! [`read::Layout`] is the v5 compiled view).
//!
//! **The module never touches the filesystem directly.** Everything
//! that reads a file goes through the one-method [`Source`] trait:
//! [`FsSource`] is the real-filesystem implementation (the CLI, native
//! hosts), and an in-memory map serves tests and wasm hosts. The
//! file-set collector only ever reads manifest-named paths, never
//! lists directories. Packing itself (compiling the closure, emitting
//! the `.rutc` groups) needs the compiler and lives in the pack
//! module ([`crate::pack`]); this module stays compiler-free.
//!
//! Format, layout ledger, and load-time checks:
//! `docs/src/reference/bundles.md` (the rut book).

pub mod container;
pub mod files;
pub mod manifest;
pub mod read;

/// The archive mechanics layer — pure zip plumbing over the `zip`
/// crate, no rut knowledge. Private: the public surface of this
/// module stays the format's policy (`write_bundle`/`parse_bundle`).
mod zip;

pub use container::{crc32, parse_bundle, write_bundle, BundleError};
pub use files::{
    bundle_key, collect_source_group, default_out_path, entry_rel, read_entry, read_manifest,
};
pub use manifest::{parse_manifest, valid_spec, Entry, Manifest, ManifestError, PkgType};
pub use read::{Bundle, GroupKind, Layout};

/// The one filesystem door: read a file's bytes. One method is the
/// whole trait — a host (or a test, or a wasm module) answers with a
/// lookup, not a filesystem.
pub trait Source {
    fn read(&self, path: &std::path::Path) -> Result<Vec<u8>, String>;
}

/// The real filesystem — the CLI's and native hosts' [`Source`].
pub struct FsSource;

impl Source for FsSource {
    fn read(&self, p: &std::path::Path) -> Result<Vec<u8>, String> {
        std::fs::read(p).map_err(|e| e.to_string())
    }
}
