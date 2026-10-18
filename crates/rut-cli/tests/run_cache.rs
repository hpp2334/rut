//! The cache-first `rut run <dir>`: the whole-program compile is the
//! deterministic reference, so a run may emit its archive into
//! `.rut/cache/` keyed by the closure's content — and every later run
//! of the SAME closure mounts the cached bundle. The debug door
//! (`--no-cache`, alias `--repack`) forces the miss path; the entry
//! still refreshes. A corrupt entry self-heals: the hit falls through
//! to the miss and rewrites it.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-cli-runcache-{tag}-{}", std::process::id()));
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

fn rut_run(args: &[&str]) -> (i32, String) {
    let out = Command::new(rut())
        .args(args)
        .output()
        .expect("run the rut binary");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The smallest runnable world: one manifest, one source, no deps.
fn hello_world(tag: &str) -> PathBuf {
    let root = scratch(tag);
    let dir = root.join("hello");
    write(
        &dir,
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 12, "name": "hello"}"#,
    );
    write(&dir, "mod.rut", "entry fn main() -> nil { return; }\n");
    root
}

fn cache_entries(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir.join(".rut").join("cache"))
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default();
    out.sort();
    out
}

#[test]
fn run_dir_writes_the_cache_and_the_second_run_hits_it() {
    let root = hello_world("hit");
    let dir = root.join("hello");

    // miss: the reference compile runs and emits the archive
    let (code, err) = rut_run(&["run", dir.to_str().unwrap()]);
    assert_eq!(code, 0, "first run: {err}");
    let entries = cache_entries(&dir);
    assert_eq!(entries.len(), 1, "one cache entry after the miss");
    assert!(entries[0].extension().map_or(false, |e| e == "rutbundle"));

    // hit: the same closure mounts the cached archive — the entry is
    // untouched (a miss would rewrite it through the pack lane)
    let before = std::fs::read(&entries[0]).unwrap();
    let stamp = std::fs::metadata(&entries[0]).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    let (code, err) = rut_run(&["run", dir.to_str().unwrap()]);
    assert_eq!(code, 0, "second run: {err}");
    assert_eq!(
        std::fs::metadata(&entries[0]).unwrap().modified().unwrap(),
        stamp,
        "a hit never rewrites the entry"
    );
    assert_eq!(std::fs::read(&entries[0]).unwrap(), before);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_edited_closure_keys_a_fresh_entry() {
    let root = hello_world("keyed");
    let dir = root.join("hello");

    let (code, err) = rut_run(&["run", dir.to_str().unwrap()]);
    assert_eq!(code, 0, "first run: {err}");
    assert_eq!(cache_entries(&dir).len(), 1);

    // the source is part of the key: the edit lands a SECOND entry and
    // the run still succeeds (the new closure compiles fresh)
    write(&dir, "mod.rut", "entry fn main() -> nil {\n    let n = 6 * 7;\n    return;\n}\n");
    let (code, err) = rut_run(&["run", dir.to_str().unwrap()]);
    assert_eq!(code, 0, "edited run: {err}");
    assert_eq!(cache_entries(&dir).len(), 2, "one entry per closure");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn no_cache_forces_the_miss_but_refreshes_the_entry() {
    let root = hello_world("nocache");
    let dir = root.join("hello");

    let (code, err) = rut_run(&["run", dir.to_str().unwrap()]);
    assert_eq!(code, 0, "first run: {err}");
    let entry = cache_entries(&dir).remove(0);

    // corrupt the entry: a plain run self-heals — the hit falls
    // through to the miss and the fresh archive rewrites the entry
    std::fs::write(&entry, b"garbage").unwrap();
    let (code, err) = rut_run(&["run", dir.to_str().unwrap()]);
    assert_eq!(code, 0, "the corrupt entry never fails a run: {err}");
    assert_ne!(std::fs::read(&entry).unwrap(), b"garbage", "the entry rewrote");

    // --no-cache: the miss path runs even on a valid entry, and the
    // entry STILL refreshes (one rule: the entry mirrors what the
    // reference engine just produced)
    let stamp = std::fs::metadata(&entry).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    let (code, err) = rut_run(&["run", dir.to_str().unwrap(), "--no-cache"]);
    assert_eq!(code, 0, "--no-cache run: {err}");
    assert!(
        std::fs::metadata(&entry).unwrap().modified().unwrap() != stamp,
        "--no-cache rewrites the entry"
    );
    // the visible alias spells the same door
    let (code, err) = rut_run(&["run", dir.to_str().unwrap(), "--repack"]);
    assert_eq!(code, 0, "--repack run: {err}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_dir_with_a_path_dep_caches_one_closed_entry() {    let root = scratch("dep");
    let hello = root.join("hello");
    write(
        &hello,
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 12, "name": "hello", "deps": {"calc": {"path": "../calc"}}}"#,
    );
    write(&hello, "mod.rut", "use calc::{ seven };\nentry fn main() -> nil {\n    let n = seven();\n    return;\n}\n");
    let calc = root.join("calc");
    write(&calc, "rut.jsonc", r#"{"name": "calc"}"#);
    write(&calc, "mod.rut", "pub fn seven() -> i64 { return 7; }\n");

    let (code, err) = rut_run(&["run", hello.to_str().unwrap()]);
    assert_eq!(code, 0, "first run: {err}");
    // the whole closure packs into ONE closed entry, and the second
    // run mounts it (untouched — a miss would rewrite it)
    let entries = cache_entries(&hello);
    assert_eq!(entries.len(), 1, "the whole closure packs into ONE entry");
    let stamp = std::fs::metadata(&entries[0]).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    let (code, err) = rut_run(&["run", hello.to_str().unwrap()]);
    assert_eq!(code, 0, "second run: {err}");
    assert_eq!(
        std::fs::metadata(&entries[0]).unwrap().modified().unwrap(),
        stamp,
        "a hit never rewrites the entry"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_run_help_names_the_cache_law_and_the_debug_door() {
    let out = Command::new(rut()).args(["run", "--help"]).output().expect("rut run --help");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("--no-cache"), "{text}");
    assert!(text.contains("--repack"), "{text}");
    assert!(text.contains("cache"), "{text}");
    // glossary vocabulary: a packed archive, not house words
    assert!(text.contains("archive") || text.contains(".rutbundle"), "{text}");
}
