//! The one filesystem door — a STRING-keyed [`Source`]. The walk never
//! does key math: it asks the source to resolve a relative name under
//! a directory key and to read a key's bytes. [`FsSource`] bakes the
//! root in at construction ([`FsSource::at`]); a `Path` lives ONLY
//! inside this module's impls — the walk sees strings.

use std::path::PathBuf;

/// The one filesystem door: read a file's bytes, and say where a
/// relative name lives. One trait is the whole seam — a host (or a
/// test) answers with lookups, not a filesystem.
///
/// String keys by law; ALL key math is the impl's:
///
/// - [`Source::read`] — the bytes at `key` (a file, an archive entry
///   root, a memory cell — the impl's choice).
/// - [`Source::resolve`] — the key for `spec` (a file or directory
///   name, manifest-relative) inside the directory at `from`. An empty
///   `from` means the source's own root.
/// - [`Source::root`] — the source's root key (the walk's entry
///   directory, for sources that bake one in).
pub trait Source {
    /// The bytes at `key`.
    fn read(&self, key: &str) -> Result<Vec<u8>, String>;

    /// The key for `spec` under the directory key `from` (empty =
    /// the source's root).
    fn resolve(&self, from: &str, spec: &str) -> Result<String, String>;

    /// The root key — empty when the source has none.
    fn root(&self) -> &str {
        ""
    }
}

/// The real filesystem — the CLI's and native hosts' [`Source`]. The
/// root is baked at construction; every key is a path string, and the
/// `Path` math lives here and nowhere else.
#[derive(Clone, Debug)]
pub struct FsSource {
    root: String,
}

impl FsSource {
    /// A source rooted at `dir` — the walk's entry directory. Keys
    /// are the plain path strings under it.
    pub fn at(dir: &std::path::Path) -> FsSource {
        FsSource {
            root: dir.to_string_lossy().into_owned(),
        }
    }

    /// The root as a path — the native lanes' own door (the walk's
    /// cycle guard canonicalizes through it).
    pub(crate) fn root_path(&self) -> std::path::PathBuf {
        std::path::PathBuf::from(&self.root)
    }
}

impl Default for FsSource {
    fn default() -> FsSource {
        FsSource {
            root: String::new(),
        }
    }
}

impl Source for FsSource {
    fn read(&self, key: &str) -> Result<Vec<u8>, String> {
        std::fs::read(key).map_err(|e| e.to_string())
    }

    fn resolve(&self, from: &str, spec: &str) -> Result<String, String> {
        let base = if from.is_empty() {
            self.root.clone()
        } else {
            from.to_string()
        };
        Ok(PathBuf::from(base).join(spec).to_string_lossy().into_owned())
    }

    fn root(&self) -> &str {
        &self.root
    }
}
