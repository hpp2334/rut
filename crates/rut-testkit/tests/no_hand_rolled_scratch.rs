//! THE LAW AS A TEST: no test hand-rolls a scratch directory.
//!
//! Every scratch dir a test creates is owned by one
//! `rut_testkit::Scratch` — it dies when the guard drops, and the guard
//! has one home. No test builds a temp path itself.
//!
//! The only files allowed to spell the temp root are the four PRODUCT
//! files that own a runtime temp dir: the guard's own `temp_root`, the
//! native walk's `RunTempDir`, and the 03-plugin example's fixed cache
//! paths. A fifth name on that list is a finding, not an edit.

use std::path::{Path, PathBuf};

/// The four product files that own a runtime temp dir; every other
/// `.rs` file under `crates/` and `examples/` must ride
/// `rut_testkit::Scratch` / `rut_testkit::temp_root()`.
const PRODUCT: &[&str] = &[
    "crates/rut-testkit/src/lib.rs",
    "crates/rut-native/src/walk.rs",
    "examples/03-plugin/src/lib.rs",
    "examples/03-plugin/src/main.rs",
];

/// Collect every `.rs` file under `dir` (recursive, order-agnostic).
fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_test_hand_rolls_a_scratch_dir() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/rut-testkit sits two levels under the workspace root");
    // spelled apart so this file never matches its own needle
    let needle = concat!("temp_dir", "()");
    let mut files = Vec::new();
    for top in ["crates", "examples"] {
        rs_files(&root.join(top), &mut files);
    }
    let mut findings = Vec::new();
    for path in files {
        let rel = path.strip_prefix(root).unwrap_or(&path);
        let rel = rel.to_string_lossy().replace('\\', "/");
        if PRODUCT.contains(&rel.as_str()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            if line.contains(needle) {
                findings.push(format!("{rel}:{}", i + 1));
            }
        }
    }
    assert!(
        findings.is_empty(),
        "a scratch dir a test creates must ride rut_testkit::Scratch — \
         hand-rolled temp paths found:\n  {}",
        findings.join("\n  ")
    );
}
