//! The `.rutbundle` container — one module owns the whole format:
//! the container codec ([`container`]), the manifest grammar
//! ([`manifest`]), the entry-key helpers ([`files`]), and the reader
//! ([`read`], whose [`read::Layout`] is the v5 compiled view).
//!
//! **The module never touches the filesystem.** Everything is pure:
//! bytes in, values out. The file-reading half of the format — the
//! directory walk's `Source`/`FsSource` seam and the manifest reader
//! over it — lives in `rut-native` (the native host's crate); this
//! module stays compiler-free and FS-free. Packing itself (compiling
//! the closure, emitting the `.rutc` groups) lives in the pack module
//! ([`crate::pack`]); this module stays compiler-free.
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
    bundle_key, entry_rel, read_entry, MANIFEST_NAME,
};
pub use manifest::{parse_manifest, valid_spec, Entry, Manifest, ManifestError, PkgType};
pub use read::{Bundle, GroupKind, Layout};
