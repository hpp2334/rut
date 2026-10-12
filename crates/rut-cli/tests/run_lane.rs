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
        r#"{"format": "rutbundle", "format_version": 10, "name": "hello"}"#,
    );
    write(&dir, "mod.rut", "entry fn main() -> nil { return; }\n");
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
fn run_defaults_to_the_working_directory() {
    let root = hello_world("default-dir");
    let dir = root.join("hello");

    let out = Command::new(rut())
        .current_dir(&dir)
        .args(["run"])
        .output()
        .expect("run rut run (no dir)");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn run_in_a_dir_without_a_manifest_refuses_naming_the_cwd_lock() {
    let root = scratch("default-bare");
    let out = Command::new(rut())
        .current_dir(&root)
        .args(["run"])
        .output()
        .expect("run rut run (bare cwd)");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("./rut.jsonc"), "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_loose_file_is_not_a_runnable_unit() {
    let root = scratch("loose");
    write(&root, "main.rut", "entry fn main() -> nil { return; }\n");
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

/// A world with several `entry fn`s in the root module (the 02-digest
/// shape) — `rut run` refuses to guess; `--entry <name>` designates.
fn two_entries(tag: &str) -> PathBuf {
    let root = scratch(tag);
    let dir = root.join("hello");
    write(
        &dir,
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 10, "name": "hello"}"#,
    );
    write(
        &dir,
        "mod.rut",
        "entry fn main() -> nil { return; }\nentry fn other() -> nil { return; }\n",
    );
    root
}

#[test]
fn several_entry_fns_refuse_to_guess_and_entry_designates() {
    let root = two_entries("designation");
    let dir = root.join("hello");

    // no --entry: ambiguous, loud, nothing runs
    let out = Command::new(rut())
        .args(["run", dir.to_str().unwrap()])
        .output()
        .expect("run rut run <dir>");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("entry fns"), "{stderr}");
    assert!(stderr.contains("main") && stderr.contains("other"), "{stderr}");
    assert!(stderr.contains("--entry"), "{stderr}");

    // --entry <name>: the designated entry runs
    let out = Command::new(rut())
        .args(["run", dir.to_str().unwrap(), "--entry", "other"])
        .output()
        .expect("run rut run --entry other");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn entry_naming_a_non_entry_fn_is_loud() {
    let root = two_entries("misdesignated");
    let dir = root.join("hello");
    write(&dir, "mod.rut", "fn plain() -> nil { return; }\nentry fn main() -> nil { return; }\n");
    let out = Command::new(rut())
        .args(["run", dir.to_str().unwrap(), "--entry", "plain"])
        .output()
        .expect("run rut run --entry plain");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no `entry fn plain`"), "{stderr}");
    assert!(stderr.contains("main"), "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn no_entry_fn_is_nothing_to_run_not_a_source_error() {
    let root = scratch("entryless");
    let dir = root.join("hello");
    write(
        &dir,
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 10, "name": "hello"}"#,
    );
    // a pure library shape compiles clean — the designation is where
    // the run stops, never a source diagnostic
    write(
        &dir,
        "mod.rut",
        "pub fn helper() -> i32 { return 7; }\n",
    );
    let out = Command::new(rut())
        .args(["run", dir.to_str().unwrap()])
        .output()
        .expect("run rut run <dir>");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no `entry fn`"), "{stderr}");
    assert!(stderr.contains("nothing to run"), "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_dangling_entry_value_is_a_usage_error() {
    let root = hello_world("dangling-entry");
    let dir = root.join("hello");
    let out = Command::new(rut())
        .args(["run", dir.to_str().unwrap(), "--entry"])
        .output()
        .expect("run rut run --entry");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--entry"), "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

// ---- diagnostics name file, line, and column ----

/// A parse error in the ROOT file renders rustc-style against the
/// owning pkg's spec: `--> hello:2:13`, the 1-based line and char
/// column, with the caret line under the offending text.
#[test]
fn a_root_parse_error_names_the_file_line_and_column() {
    let root = scratch("diag-root");
    let dir = root.join("hello");
    write(
        &dir,
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 10, "name": "hello"}"#,
    );
    write(
        &dir,
        "mod.rut",
        "entry fn main() -> nil {\n    let x = @@;\n    return;\n}\n",
    );
    let out = Command::new(rut())
        .args(["run", dir.to_str().unwrap()])
        .output()
        .expect("run rut run <broken root>");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--> hello:2:"), "{stderr}");
    // the caret line: the gutter form, carets under the offending text
    assert!(
        stderr.lines().any(|l| l.contains('|') && l.contains('^')),
        "expected a caret line, got: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A parse error inside a mounted `mod` child is labeled with the
/// child's mod path under the spec (`spec/path`), rebased to the
/// CHILD's own line numbering.
#[test]
fn an_error_in_a_mod_child_names_the_spec_path_label() {
    let root = scratch("diag-child");
    let dir = root.join("hello");
    write(
        &dir,
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 10, "name": "hello"}"#,
    );
    write(&dir, "mod.rut", "mod layout;\nentry fn main() -> nil { return; }\n");
    write(
        &dir,
        "layout/mod.rut",
        "pub fn cell() -> i32 {\n    let = 7;\n}\n",
    );
    let out = Command::new(rut())
        .args(["run", dir.to_str().unwrap()])
        .output()
        .expect("run rut run <broken mod child>");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--> hello/layout:2:"), "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

/// A parse error in a DEP is labeled with the dep's own spec — the
/// walk attributes each unit's diags to their owning pkg, never to
/// the consumer that pulled it in.
#[test]
fn an_error_in_a_dep_names_the_dep_spec_label() {
    let root = scratch("diag-dep");
    let hello = root.join("hello");
    write(
        &hello,
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 10, "name": "hello", "deps": {"calc": {"path": "../calc"}}}"#,
    );
    write(&hello, "mod.rut", "use calc::{pi};\nentry fn main() -> nil { return; }\n");
    let calc = root.join("calc");
    write(
        &calc,
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 10, "name": "calc"}"#,
    );
    write(&calc, "mod.rut", "pub fn pi() -> f64 {\n    let = 7;\n}\n");
    let out = Command::new(rut())
        .args(["run", hello.to_str().unwrap()])
        .output()
        .expect("run rut run <broken dep>");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--> calc:2:"), "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

/// A non-nil fn that falls off the end of its body: the compile
/// diagnoses (`missing return`) and `rut run` exits 1 — the diagnostic
/// aborts the run BEFORE the entry executes, so the caller never reads
/// the never-written answer register (the old null-reference crash).
/// The renderer spells the rustc-style `-->` position line.
#[test]
fn a_missing_return_diagnoses_with_a_position_not_a_crash() {
    let root = scratch("missing-return");
    let dir = root.join("hello");
    write(
        &dir,
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 10, "name": "hello"}"#,
    );
    write(
        &dir,
        "mod.rut",
        "fn f() -> str {\n    let x = 1;\n}\nentry fn main() -> str {\n    return f();\n}\n",
    );
    let out = Command::new(rut())
        .args(["run", dir.to_str().unwrap()])
        .output()
        .expect("run rut run <missing return>");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("missing return"), "{stderr}");
    assert!(stderr.contains("--> hello:1:"), "{stderr}");
    // the caret line under the offending signature, the rustc form
    assert!(
        stderr.lines().any(|l| l.contains('|') && l.contains('^')),
        "expected a caret line, got: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
