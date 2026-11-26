//! Dev-only test support: a scratch directory that dies with the test.

use std::path::{Path, PathBuf};

/// A test's scratch directory. It dies with the guard — on pass, on
/// panic, on early return.
pub struct Scratch {
    path: PathBuf,
}

impl Scratch {
    /// A new empty directory under the temp root:
    /// `rut-{tag}-{pid}-{n}` — `n` from a process-wide counter, so two
    /// guards in one test never share a directory.
    pub fn new(tag: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = temp_root().join(format!("rut-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path)
            .unwrap_or_else(|e| panic!("rut-testkit: create {}: {e}", path.display()));
        Scratch { path }
    }
}

impl std::ops::Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The temp root — the one door a test reads it through. A test that
/// asserts nothing leaked lists it.
pub fn temp_root() -> PathBuf {
    std::env::temp_dir()
}
