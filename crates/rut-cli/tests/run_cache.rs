//! The cache-first `rut run <dir>`: the whole-program compile is the
//! deterministic reference, so a run may emit its archive into
//! `.rut/cache/` keyed by the closure's content — and every later run
//! of the SAME closure mounts the cached bundle. The debug door
//! (`--no-cache`, alias `--repack`) forces the miss path; the entry
//! still refreshes. A corrupt entry self-heals: the hit falls through
//! to the miss and rewrites it.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;

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
fn a_dir_with_a_path_dep_caches_one_closed_entry() {
    let root = scratch("dep");
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

// ── the url-dep closure: the key rides the RESOLVED bytes ─────────────
//
// A url dep's cache entry is part of the run key, so the entry written
// after a run must key the POST-fetch state — a key hashed while a url
// row is still `<uncached>` is dead by construction (the fetch lands
// during the run and moves the key), and none is ever written beside
// the resolved one.

/// One-shot HTTP server on 127.0.0.1:0 serving ONE fixed payload at
/// any path (the url_fetch harness, trimmed to the serving half). The
/// listener rides the detached accept loop — it lives as long as the
/// test process.
fn serve(payload: Vec<u8>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind localhost");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&payload);
            let _ = stream.flush();
        }
    });
    port
}

#[test]
fn a_url_dep_closure_writes_only_the_resolved_key_and_the_second_run_hits_it() {
    let util_root = scratch("url-leaf");
    let util = util_root.join("util");
    write(
        &util,
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 12, "name": "util"}"#,
    );
    write(&util, "mod.rut", "pub fn twice(v: i64) -> i64 {\n    return v * 2;\n}\n");
    let bytes = rut_native::pack_dir(&util).expect("pack util");
    let port = serve(bytes.clone());
    let url = format!("http://127.0.0.1:{port}/util.rutbundle");
    let pin = rut_driver::sha256_hex(&bytes);

    let root = scratch("url-dep");
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &format!(
            r#"{{"format": "rutbundle", "format_version": 12, "name": "app", "deps": {{"util": {{"url": "{url}"}}}}}}"#
        ),
    );
    // the pin ledger: the manifest row is unpinned — the lock pins it
    write(
        &app,
        "rut.lock",
        &rut_driver::render_lock(&rut_driver::LockFile::from_rows(vec![rut_driver::LockRow {
            pkg: "util".into(),
            url: url.clone(),
            sha256: pin.into(),
        }])),
    );
    write(
        &app,
        "mod.rut",
        "use util::{twice};\nentry fn main() {\n    let x = twice(21);\n    return;\n}\n",
    );

    // install first: run never downloads — the install warms the url
    // entry (and leaves the identical lock as-is)
    let (code, err) = rut_run(&["install", app.to_str().unwrap()]);
    assert_eq!(code, 0, "install: {err}");

    // run 1 (warm): the cache holds exactly the url entry + ONE
    // run-cache entry keyed on the resolved bytes. No dead
    // `<uncached>` entry beside them.
    let (code, err) = rut_run(&["run", app.to_str().unwrap()]);
    assert_eq!(code, 0, "first run: {err}");
    let url_entry = app
        .join(".rut")
        .join("cache")
        .join(format!("{}.rutbundle", rut_driver::sha256_hex(url.as_bytes())));
    assert!(url_entry.is_file(), "the url dep's cache entry lands");
    let run_entries: Vec<PathBuf> = cache_entries(&app)
        .into_iter()
        .filter(|e| *e != url_entry)
        .collect();
    assert_eq!(
        run_entries.len(),
        1,
        "one run-cache entry, keyed on the resolved bytes — no dead pre-fetch entry"
    );

    // run 2: the same closure keys the same resolved bytes — the hit
    // mounts the cached archive and neither entry is rewritten (a miss
    // would rewrite through the pack lane)
    let stamp = std::fs::metadata(&run_entries[0]).unwrap().modified().unwrap();
    let url_stamp = std::fs::metadata(&url_entry).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    let (code, err) = rut_run(&["run", app.to_str().unwrap()]);
    assert_eq!(code, 0, "second run: {err}");
    assert_eq!(
        std::fs::metadata(&run_entries[0]).unwrap().modified().unwrap(),
        stamp,
        "a hit never rewrites the run-cache entry"
    );
    assert_eq!(
        std::fs::metadata(&url_entry).unwrap().modified().unwrap(),
        url_stamp,
        "a hit never rewrites the url entry"
    );
    let _ = std::fs::remove_dir_all(&util_root);
    let _ = std::fs::remove_dir_all(&root);
}
