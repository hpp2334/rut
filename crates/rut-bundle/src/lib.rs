//! The `.rutbundle` container: a deterministic zip archive carrying a
//! module's `rut.toml` and its rut sources. One crate owns the whole
//! format — the container codec ([`container`]), the manifest grammar
//! ([`manifest`]), the deterministic packer ([`pack`](pack::pack)), and
//! the reader ([`read`]).
//!
//! **The crate never touches the filesystem.** Everything that reads a
//! file goes through the one-method [`Source`] trait: [`FsSource`] is
//! the real-filesystem implementation (the CLI, native hosts), and an
//! in-memory map serves tests and wasm hosts. [`pack`](pack::pack)
//! returns the bundle bytes; writing the output file stays with the
//! caller. The packer only ever reads manifest-named paths, never lists
//! directories.
//!
//! Format, layout ledger, and load-time checks:
//! `docs/src/reference/bundles.md` (the rut book).

pub mod container;
pub mod manifest;
pub mod pack;
pub mod read;

pub use container::{crc32, parse_bundle, write_bundle, BundleError};
pub use manifest::{parse_manifest, valid_spec, Entry, Manifest, ManifestError};
pub use pack::{bundle_key, default_out_path, entry_rel, pack, read_entry, read_manifest};
pub use read::Bundle;

/// The one filesystem door: read a file's bytes. One method is the
/// whole trait — the packer only ever reads manifest-named paths, so a
/// host (or a test, or a wasm module) answers with a lookup, not a
/// filesystem.
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
