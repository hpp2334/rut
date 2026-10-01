//! The run lane's door through the binary: `rut run` accepts a module
//! directory (`rut.jsonc`) or a packed `.rutbundle` — an explicit
//! allowlist, checked up front. A loose `.rut` file is not a runnable
//! unit (exit 2, the allowlist message), a `.d.rut` is refused as the
//! surface it is (its own pointed message), and a typo/non-rut path
//! never gets past the door. The positive side: the same dir and its
//! packed bundle both run end to end.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-cli-runlane-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, text).unwrap();
}

fn rut() -> &'static str {
    env!("CARGO_BIN_EXE_rut")
}

/// The smallest runnable world: one manifest, one source, no deps.
fn hello_world(tag: &str) -> PathBuf {
    let root = scratch(tag);
    let dir = root.join("hello");
    write(
        &dir,
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 9, "name": "hello", "entry": {"lib": "./main.rut"}}"#,
    );
    write(&dir, "main.rut", "pub fn main() -> nil { return; }\n");
    root
}

#[test]
fn run_accepts_a_module_dir_and_its_packed_bundle() {
    let root = hello_world("positive");
    let dir = root.join("hello");

    let out = Command::new(rut())
        .args(["run", dir.to_str().unwrap()])
        .output()
        .expect("run rut run <dir>");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out = Command::new(rut())
        .args(["pack", dir.to_str().unwrap()])
        .output()
        .expect("run rut pack");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bundle = root.join("hello.rutbundle");
    assert!(bundle.is_file(), "the bundle lands beside the dir");

    let out = Command::new(rut())
        .args(["run", bundle.to_str().unwrap()])
        .output()
        .expect("run rut run <bundle>");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_loose_file_is_not_a_runnable_unit() {
    let root = scratch("loose");
    write(&root, "main.rut", "pub fn main() -> nil { return; }\n");
    let out = Command::new(rut())
        .args(["run", root.join("main.rut").to_str().unwrap()])
        .output()
        .expect("run rut run <loose file>");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("not a runnable unit"), "{stderr}");
    assert!(stderr.contains("rut.jsonc"), "{stderr}");
    assert!(stderr.contains(".rutbundle"), "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_declaration_surface_is_refused_with_its_own_message() {
    let root = scratch("decl");
    write(&root, "surf.d.rut", "pub host fn log(line: str);\n");
    let out = Command::new(rut())
        .args(["run", root.join("surf.d.rut").to_str().unwrap()])
        .output()
        .expect("run rut run <file.d.rut>");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("declaration file"), "{stderr}");
    assert!(stderr.contains("nothing to run"), "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_typo_path_never_gets_past_the_door() {
    let root = scratch("typo");
    for probe in ["nope", "missing.rut", "notes.txt"] {
        let out = Command::new(rut())
            .args(["run", root.join(probe).to_str().unwrap()])
            .output()
            .expect("run rut run <typo path>");
        assert_eq!(out.status.code(), Some(2), "probe {probe}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("not a runnable unit"), "probe {probe}: {stderr}");
    }
    let _ = std::fs::remove_dir_all(&root);
}
