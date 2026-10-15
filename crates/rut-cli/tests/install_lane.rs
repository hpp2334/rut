//! The `rut install` lane end to end, through the `rut` binary — ONE
//! localhost server (offline: no external network), scratch worlds,
//! `RUT_CACHE_DIR`-env'd runs. The seven laws:
//!
//! 1. install writes a DETERMINISTIC lock (byte-identical on re-run);
//! 2. `--locked` passes when the lock is fresh;
//! 3. `--locked` refuses (exit 2) with no lock, naming the fix;
//! 4. `--locked` refuses manifest SET drift (an added and a removed
//!    dep — the `+`/`-` diff rows);
//! 5. `--locked` refuses CONTENT drift (the server re-serves different
//!    bytes — the `~` row names the url and BOTH hashes; the lock file
//!    stays untouched on disk);
//! 6. install refuses a `.rutbundle` input (bundles are closed);
//! 7. the progress chatter rides stderr (`downloading` / `downloaded` /
//!    `cache hit`), the summaries ride stdout — asserted strictly,
//!    stderr loosely (wording-tweak-proof).
//!
//! The drift/diff LAW itself is install.rs's (phase 3); what this file
//! proves is the wire-side behavior an editor or CI would see.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// One localhost HTTP server serving fixed payloads by path — the tail
/// after `host:port/` (a query string is tolerated and ignored); 404
/// for anything the table does not name. `re_serve` swaps a payload —
/// the content-drift simulation (the "CDN re-served the file").
struct TestServer {
    port: u16,
    payloads: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
}

impl TestServer {
    fn serve(payloads: BTreeMap<String, Vec<u8>>) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind localhost");
        let port = listener.local_addr().unwrap().port();
        let payloads = Arc::new(Mutex::new(payloads));
        let served = payloads.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                // read the request head (a GET has no body worth reading)
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]);
                let path = head
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("")
                    .trim_start_matches('/')
                    .split(['?', '#'])
                    .next()
                    .unwrap_or("")
                    .to_string();
                let payload = served.lock().unwrap().get(&path).cloned();
                let Some(payload) = payload else {
                    let not_found = "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nnot found";
                    let _ = stream.write_all(not_found.as_bytes());
                    continue;
                };
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&payload);
                let _ = stream.flush();
            }
        });
        TestServer { port, payloads }
    }

    fn url(&self, name: &str) -> String {
        format!("http://127.0.0.1:{}/{}", self.port, name)
    }

    /// the CDN moved: `name` now serves different bytes
    fn re_serve(&self, name: &str, bytes: Vec<u8>) {
        self.payloads.lock().unwrap().insert(name.to_string(), bytes);
    }
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-cli-install-{tag}-{}", std::process::id()));
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

fn manifest(name: &str, deps: &str) -> String {
    format!(r#"{{"format": "rutbundle", "format_version": 11, "name": "{name}", "deps": {{{deps}}}}}"#)
}

/// `rut <args>` with this lane's cache dir, env-cleaned (no XDG/HOME
/// surprises): returns (exit code, stdout, stderr) — the summaries are
/// stdout's, the progress chatter stderr's, so the lane reads both.
fn rut(cache: &Path, args: &[&str]) -> (i32, String, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_rut"))
        .env("RUT_CACHE_DIR", cache)
        .env_remove("XDG_CACHE_HOME")
        .args(args)
        .output()
        .expect("run the rut binary");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The same binary lane, spawned IN a working directory — `rut
/// install` with no `<dir>` acts on the cwd.
fn rut_in(cache: &Path, cwd: &Path, args: &[&str]) -> (i32, String, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_rut"))
        .env("RUT_CACHE_DIR", cache)
        .env_remove("XDG_CACHE_HOME")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("run the rut binary");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Pack one leaf pkg (`name`, `body`) into bundle bytes — the served
/// payload.
fn leaf_bundle(tag: &str, name: &str, body: &str) -> Vec<u8> {
    let root = scratch(tag);
    let dir = root.join(name);
    write(&dir, "rut.jsonc", &manifest(name, ""));
    write(&dir, "mod.rut", body);
    rut_native::pack_dir(&dir).expect("pack the leaf")
}

/// The consumer world: `app` with UNPINNED url rows (the manifest
/// grammar carries no pins — the lock is install's output) and a root
/// module using `util`. `deps` is the manifest's raw deps fragment.
fn consumer_world(tag: &str, deps: &str) -> PathBuf {
    let root = scratch(tag);
    let app = root.join("app");
    write(&app, "rut.jsonc", &manifest("app", deps));
    write(
        &app,
        "mod.rut",
        "use util::{twice};\n\nentry fn main() {\n    let x = twice(21);\n}\n",
    );
    root
}

fn one_dep(url: &str) -> String {
    format!(r#""util": {{"url": "{url}"}}"#)
}

/// The cache entry a url lands in (`.rut/cache/<sha256(url)>.rutbundle`)
/// — the content-drift test must clear it, the fetcher is cache-first.
fn cache_entry(cache: &Path, url: &str) -> PathBuf {
    cache.join(format!(
        "{}.rutbundle",
        rut_driver::sha256_hex(url.as_bytes())
    ))
}

// -- 1 ----------------------------------------------------------------

#[test]
fn install_writes_a_deterministic_lock() {
    let bytes = leaf_bundle("det leaf", "util", "pub fn twice(v: i64) -> i64 {\n    return v * 2;\n}\n");
    let mut payloads = BTreeMap::new();
    payloads.insert("util.rutbundle".to_string(), bytes.clone());
    let server = TestServer::serve(payloads);
    let url = server.url("util.rutbundle");
    let root = consumer_world("det consumer", &one_dep(&url));
    let app = root.join("app");
    let lock = app.join("rut.lock");
    let cache = root.join("cache");

    let (code, out, err) = rut(&cache, &["install", app.to_str().unwrap()]);
    assert_eq!(code, 0, "first install must succeed: {err}");
    assert!(lock.is_file(), "install writes rut.lock");
    assert!(
        out.contains("rut.lock written: 1 rows (1 added, 0 changed, 0 dropped)"),
        "{out}"
    );
    let first = std::fs::read_to_string(&lock).unwrap();

    // re-run: the lock is BYTE-IDENTICAL (render_lock's determinism
    // law, through the wire) and the summary says so
    let (code, out, err) = rut(&cache, &["install", app.to_str().unwrap()]);
    assert_eq!(code, 0, "re-install must succeed: {err}");
    assert!(out.contains("rut.lock unchanged (1 rows, no drift)"), "{out}");
    let second = std::fs::read_to_string(&lock).unwrap();
    assert_eq!(first, second, "the lock is byte-deterministic on re-run");

    // and the lock pins the SERVED bytes under the driver's lock law
    let parsed = rut_driver::parse_lock(&first).expect("install's lock parses");
    assert_eq!(parsed.deps().len(), 1, "one row: {:?}", parsed.deps());
    assert_eq!(parsed.deps()[0].pkg, "util");
    assert_eq!(parsed.deps()[0].url, url);
    assert_eq!(parsed.deps()[0].sha256, rut_driver::sha256_hex(&bytes));
}

// -- 2 ----------------------------------------------------------------

/// `rut install` with no `<dir>`: the working directory is the walk
/// root — same lane, same lock, `.` is the only spelling difference.
#[test]
fn install_defaults_to_the_working_directory() {
    let bytes = leaf_bundle("cwd leaf", "util", "pub fn twice(v: i64) -> i64 {\n    return v * 2;\n}\n");
    let mut payloads = BTreeMap::new();
    payloads.insert("util.rutbundle".to_string(), bytes);
    let server = TestServer::serve(payloads);
    let url = server.url("util.rutbundle");
    let root = consumer_world("cwd consumer", &one_dep(&url));
    let app = root.join("app");
    let cache = root.join("cache");

    let (code, out, err) = rut_in(&cache, &app, &["install"]);
    assert_eq!(code, 0, "dirless install must succeed: {err}");
    assert!(app.join("rut.lock").is_file(), "the lock lands in the cwd");
    assert!(
        out.contains("rut.lock written: 1 rows (1 added, 0 changed, 0 dropped)"),
        "{out}"
    );

    // --locked rides the same default
    let (code, out, err) = rut_in(&cache, &app, &["install", "--locked"]);
    assert_eq!(code, 0, "dirless --locked must verify: {err}");
    assert!(out.contains("verified against rut.lock"), "{out}");
}

#[test]
fn locked_passes_when_fresh() {
    let bytes = leaf_bundle("fresh leaf", "util", "pub fn twice(v: i64) -> i64 {\n    return v * 2;\n}\n");
    let mut payloads = BTreeMap::new();
    payloads.insert("util.rutbundle".to_string(), bytes);
    let server = TestServer::serve(payloads);
    let url = server.url("util.rutbundle");
    let root = consumer_world("fresh consumer", &one_dep(&url));
    let app = root.join("app");
    let cache = root.join("cache");

    let (code, _, err) = rut(&cache, &["install", app.to_str().unwrap()]);
    assert_eq!(code, 0, "install first: {err}");
    let (code, out, err) = rut(&cache, &["install", app.to_str().unwrap(), "--locked"]);
    assert_eq!(code, 0, "--locked must pass on a fresh lock: {err}");
    assert!(
        out.contains("install: 1 url deps verified against rut.lock — 0 downloaded, 1 cache hits, 0 drift"),
        "the verified summary, strictly: {out}"
    );
}

// -- 3 ----------------------------------------------------------------

#[test]
fn locked_without_a_lock_names_the_fix() {
    let bytes = leaf_bundle("nolock leaf", "util", "pub fn twice(v: i64) -> i64 {\n    return v * 2;\n}\n");
    let mut payloads = BTreeMap::new();
    payloads.insert("util.rutbundle".to_string(), bytes);
    let server = TestServer::serve(payloads);
    let url = server.url("util.rutbundle");
    let root = consumer_world("nolock consumer", &one_dep(&url));
    let app = root.join("app");
    let cache = root.join("cache");

    // no install ran — no rut.lock exists, and --locked never writes one
    let (code, _, err) = rut(&cache, &["install", app.to_str().unwrap(), "--locked"]);
    assert_eq!(code, 2, "--locked with no lock refuses: {err}");
    assert!(
        err.contains("no rut.lock"),
        "the refusal names the missing file: {err}"
    );
    assert!(
        err.contains("run `rut install` first"),
        "the refusal names the fix: {err}"
    );
    assert!(!app.join("rut.lock").exists(), "--locked never writes");
}

// -- 4 ----------------------------------------------------------------

#[test]
fn locked_refuses_manifest_set_drift() {
    let util_bytes = leaf_bundle("set leaf util", "util", "pub fn twice(v: i64) -> i64 {\n    return v * 2;\n}\n");
    let extra_bytes = leaf_bundle("set leaf extra", "extra", "pub fn thrice(v: i64) -> i64 {\n    return v * 3;\n}\n");
    let more_bytes = leaf_bundle("set leaf more", "more", "pub fn quad(v: i64) -> i64 {\n    return v * 4;\n}\n");
    let mut payloads = BTreeMap::new();
    payloads.insert("util.rutbundle".to_string(), util_bytes);
    payloads.insert("extra.rutbundle".to_string(), extra_bytes);
    payloads.insert("more.rutbundle".to_string(), more_bytes);
    let server = TestServer::serve(payloads);
    let util_url = server.url("util.rutbundle");
    let extra_url = server.url("extra.rutbundle");
    let more_url = server.url("more.rutbundle");

    let root = consumer_world(
        "set consumer",
        &format!("{}, {}", one_dep(&util_url), format!(r#""extra": {{"url": "{extra_url}"}}"#)),
    );
    let app = root.join("app");
    let cache = root.join("cache");

    let (code, _, err) = rut(&cache, &["install", app.to_str().unwrap()]);
    assert_eq!(code, 0, "install the two-dep world: {err}");
    let lock = std::fs::read_to_string(app.join("rut.lock")).unwrap();

    // ADDED dep: the manifest names a url the lock has no row for —
    // the `+` diff row, exit 2, lock untouched
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            &format!(
                "{}, {}, {}",
                one_dep(&util_url),
                format!(r#""extra": {{"url": "{extra_url}"}}"#),
                format!(r#""more": {{"url": "{more_url}"}}"#)
            ),
        ),
    );
    let (code, _, err) = rut(&cache, &["install", app.to_str().unwrap(), "--locked"]);
    assert_eq!(code, 2, "an added dep is set drift: {err}");
    assert!(err.contains(&format!("+ {more_url}")), "{err}");
    assert!(err.contains("has no rut.lock row"), "{err}");
    assert!(err.contains("rut.lock untouched"), "{err}");
    assert_eq!(
        std::fs::read_to_string(app.join("rut.lock")).unwrap(),
        lock,
        "--locked never writes"
    );

    // REMOVED dep: the manifest drops `util` — the `-` diff row names
    // the url no dep spells anymore
    write(
        &app,
        "rut.jsonc",
        &manifest("app", &format!(r#""extra": {{"url": "{extra_url}"}}"#)),
    );
    let (code, _, err) = rut(&cache, &["install", app.to_str().unwrap(), "--locked"]);
    assert_eq!(code, 2, "a removed dep is set drift: {err}");
    assert!(err.contains(&format!("- {util_url}")), "{err}");
    assert!(err.contains("no dep names this url anymore"), "{err}");
    assert_eq!(
        std::fs::read_to_string(app.join("rut.lock")).unwrap(),
        lock,
        "--locked never writes"
    );
}

// -- 5 ----------------------------------------------------------------

#[test]
fn locked_refuses_content_drift_and_never_touches_the_lock() {
    let bytes = leaf_bundle("drift leaf a", "util", "pub fn twice(v: i64) -> i64 {\n    return v * 2;\n}\n");
    let mut payloads = BTreeMap::new();
    payloads.insert("util.rutbundle".to_string(), bytes.clone());
    let server = TestServer::serve(payloads);
    let url = server.url("util.rutbundle");
    let root = consumer_world("drift consumer", &one_dep(&url));
    let app = root.join("app");
    let cache = root.join("cache");

    let (code, _, err) = rut(&cache, &["install", app.to_str().unwrap()]);
    assert_eq!(code, 0, "install first: {err}");
    let lock = std::fs::read_to_string(app.join("rut.lock")).unwrap();
    // the lock holds the pin install computed from the FIRST bytes
    let stale_pin = rut_driver::parse_lock(&lock).unwrap().deps()[0].sha256.clone();
    assert_eq!(stale_pin, rut_driver::sha256_hex(&bytes));

    // the CDN re-serves different bytes: swap the payload AND clear the
    // cache entry (the fetcher is cache-first — a warm entry would
    // never see the new bytes)
    let fresh = leaf_bundle(
        "drift leaf b",
        "util",
        "pub fn twice(v: i64) -> i64 {\n    return v * 3;\n}\n",
    );
    let fresh_pin = rut_driver::sha256_hex(&fresh);
    assert_ne!(stale_pin, fresh_pin);
    server.re_serve("util.rutbundle", fresh);
    let entry = cache_entry(&cache, &url);
    assert!(entry.is_file(), "the install warmed the cache");
    std::fs::remove_file(&entry).unwrap();

    let (code, _, err) = rut(&cache, &["install", app.to_str().unwrap(), "--locked"]);
    assert_eq!(code, 2, "content drift refuses: {err}");
    // the `~` row names the url and BOTH hashes: the lock's pin, then
    // what the re-served bytes hash to
    assert!(err.contains(&format!("~ {url}")), "{err}");
    assert!(err.contains(&format!("pins {stale_pin}")), "the lock's pin: {err}");
    assert!(
        err.contains(&format!("the fetched bytes hash to {fresh_pin}")),
        "the served bytes' hash: {err}"
    );
    assert!(err.contains("rut.lock untouched"), "{err}");
    // and the lock file on disk is byte-for-byte what install wrote
    assert_eq!(
        std::fs::read_to_string(app.join("rut.lock")).unwrap(),
        lock,
        "--locked never rewrites, never evicts"
    );
}

// -- 6 ----------------------------------------------------------------

#[test]
fn install_refuses_a_bundle() {
    // a `.rutbundle` is CLOSED — there is nothing to install
    let root = scratch("refuse");
    let bundle = root.join("x.rutbundle");
    std::fs::write(&bundle, b"not really a bundle").unwrap();
    let (code, _, err) = rut(&root.join("cache"), &["install", bundle.to_str().unwrap()]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("not a module directory"), "{err}");
}

// -- 7 ----------------------------------------------------------------

#[test]
fn progress_on_stderr_summaries_on_stdout() {
    let bytes = leaf_bundle(
        "chatter leaf",
        "util",
        "pub fn twice(v: i64) -> i64 {\n    return v * 2;\n}\n",
    );
    let total = bytes.len();
    let mut payloads = BTreeMap::new();
    payloads.insert("util.rutbundle".to_string(), bytes);
    let server = TestServer::serve(payloads);
    let url = server.url("util.rutbundle");
    let root = consumer_world("chatter consumer", &one_dep(&url));
    let app = root.join("app");
    let cache = root.join("cache");

    // cold cache: the downloading chatter rides STDERR; the summaries
    // ride STDOUT (asserted strictly — the stdout lane is the stable one)
    let (code, out, err) = rut(&cache, &["install", app.to_str().unwrap()]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains(&format!("install: util — downloading {url}")), "{err}");
    assert!(err.contains("install: util — downloaded ("), "{err}");
    assert!(
        out.contains(&format!(
            "install: 1 url deps resolved — 1 downloaded, 0 cache hits ({total} bytes total)"
        )),
        "the resolved summary, strictly: {out}"
    );
    assert!(
        out.contains("rut.lock written: 1 rows (1 added, 0 changed, 0 dropped)"),
        "the written summary, strictly: {out}"
    );

    // warm cache: one cache-hit line, no fake re-download flash, and
    // the unchanged lock is never rewritten
    let (code, out, err) = rut(&cache, &["install", app.to_str().unwrap()]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("install: util — cache hit ("), "{err}");
    assert!(!err.contains("downloading"), "a hit never touches the wire: {err}");
    assert!(
        out.contains(&format!(
            "install: 1 url deps resolved — 0 downloaded, 1 cache hits ({total} bytes total)"
        )),
        "{out}"
    );
    assert!(
        out.contains("rut.lock unchanged (1 rows, no drift)"),
        "{out}"
    );
}
