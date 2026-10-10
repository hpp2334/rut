//! The url-dep lane end to end, through the `rut` binary — ONE
//! localhost TcpListener (offline: no external network), the real
//! cache policy, the real eviction law:
//!
//! fetch → run (cache miss ⇒ GET) → run offline (server silent ⇒
//! cache hit) → poisoned cache entry ⇒ pin refusal, eviction, exit 2 →
//! re-serve ⇒ healed. Plus `rut fetch` as the CI warm-up.
//!
//! The pin law itself is the loader's (mount door, every load — see
//! rut-driver's url_deps tests); what this file proves is the CLI's
//! HOW: the cache-first fetcher and the healing loop around it.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// One-shot HTTP server on 127.0.0.1:0 serving ONE fixed payload at
/// any path. `up = false` closes connections immediately — the
/// offline simulation — without unbinding the port, so the url (and
/// the manifest naming it) stays stable for the whole test.
struct TestServer {
    port: u16,
    up: Arc<AtomicBool>,
}

impl TestServer {
    fn serve(payload: Vec<u8>) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind localhost");
        let port = listener.local_addr().unwrap().port();
        let up = Arc::new(AtomicBool::new(true));
        let up_thread = up.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                if !up_thread.load(Ordering::SeqCst) {
                    continue; // "offline": accept nothing, close at once
                }
                // read the request head (a GET has no body worth reading)
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
        TestServer { port, up }
    }

    fn url(&self, name: &str) -> String {
        format!("http://127.0.0.1:{}/{}", self.port, name)
    }

    fn set_up(&self, up: bool) {
        self.up.store(up, Ordering::SeqCst);
    }
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-cli-fetch-{tag}-{}", std::process::id()));
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

fn manifest(name: &str, _entry: &str, extra: &str) -> String {
    format!(r#"{{"format": "rutbundle", "format_version": 10, "name": "{name}"{extra}}}"#)
}

/// `rut <args>` with this test's cache dir, env-cleaned (no XDG/HOME
/// surprises): returns (exit code, stderr).
fn rut(cache: &Path, args: &[&str]) -> (i32, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_rut"))
        .env("RUT_CACHE_DIR", cache)
        .env_remove("XDG_CACHE_HOME")
        .args(args)
        .output()
        .expect("run the rut binary");
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stderr).into_owned())
}

/// The consumer world: `util` served by url, pinned; app calls it.
fn consumer_world(tag: &str, url: &str, pin: &str) -> PathBuf {
    let root = scratch(tag);
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "mod.rut",
            &format!(r#", "deps": {{"util": {{"url": "{url}", "sha256": "{pin}"}}}}"#),
        ),
    );
    write(
        &app,
        "mod.rut",
        "use util::{twice};\n\nentry fn main() {\n    let x = twice(21);\n}\n",
    );
    root
}

fn leaf_bundle(tag: &str) -> Vec<u8> {
    let root = scratch(tag);
    let util = root.join("util");
    write(&util, "rut.jsonc", &manifest("util", "mod.rut", ""));
    write(&util, "mod.rut", "pub fn twice(v: i64) -> i64 {\n    return v * 2;\n}\n");
    rut_native::pack_dir(&util).expect("pack util")
}

#[test]
fn fetch_cache_offline_pin_refusal_and_eviction() {
    let bytes = leaf_bundle("leaf");
    let server = TestServer::serve(bytes.clone());
    let url = server.url("util.rutbundle");
    let pin = rut_driver::sha256_hex(&bytes);
    let root = consumer_world("consumer", &url, &pin);
    let app = root.join("app");
    let cache = root.join("cache");

    // 1. fetch → run: cache miss ⇒ GET ⇒ cached ⇒ the program runs
    let (code, err) = rut(&cache, &["run", app.to_str().unwrap()]);
    assert_eq!(code, 0, "first run must succeed: {err}");
    assert!(err.is_empty(), "{err}");

    // 2. offline reload: the server goes silent; the cache answers —
    // a hit NEVER touches the network
    server.set_up(false);
    let (code, err) = rut(&cache, &["run", app.to_str().unwrap()]);
    assert_eq!(code, 0, "offline run must hit the cache: {err}");

    // 3. poisoned cache entry: the bytes no longer match the pin — the
    // mount door refuses naming dep + url + both hashes, and the CLI
    // evicts, exiting 2
    let entry = cache.join(format!(
        "{}.rutbundle",
        rut_driver::sha256_hex(url.as_bytes())
    ));
    assert!(entry.is_file(), "the cache entry must exist");
    std::fs::write(&entry, b"garbage").unwrap();
    let (code, err) = rut(&cache, &["run", app.to_str().unwrap()]);
    assert_eq!(code, 2, "a pin refusal exits 2: {err}");
    assert!(err.contains("sha256 pin mismatch"), "{err}");
    assert!(err.contains(url.as_str()), "{err}");
    assert!(!entry.exists(), "the poisoned entry must be evicted");

    // 4. healed: serve again — the entry re-fetches and the run passes
    server.set_up(true);
    let (code, err) = rut(&cache, &["run", app.to_str().unwrap()]);
    assert_eq!(code, 0, "the cache heals on the next run: {err}");
    assert!(entry.is_file(), "re-fetched");
}

#[test]
fn rut_fetch_warms_the_cache_for_offline_run() {
    let bytes = leaf_bundle("warml leaf");
    let server = TestServer::serve(bytes.clone());
    let url = server.url("util.rutbundle");
    let pin = rut_driver::sha256_hex(&bytes);
    let root = consumer_world("warm consumer", &url, &pin);
    let app = root.join("app");
    let cache = root.join("cache");

    // fetch: load with the cache-first fetcher, discard the session
    let (code, err) = rut(&cache, &["fetch", app.to_str().unwrap()]);
    assert_eq!(code, 0, "fetch must succeed: {err}");
    let entry = cache.join(format!(
        "{}.rutbundle",
        rut_driver::sha256_hex(url.as_bytes())
    ));
    assert!(entry.is_file(), "fetch must populate the cache");

    // the offline run — the server never serves a byte
    server.set_up(false);
    let (code, err) = rut(&cache, &["run", app.to_str().unwrap()]);
    assert_eq!(code, 0, "the warmed cache answers offline: {err}");
}

#[test]
fn fetch_refuses_non_directories() {
    // a `.rutbundle` is CLOSED — there is nothing to fetch; anything
    // that is not a module directory is a usage error (exit 2)
    let root = scratch("refuse");
    let bundle = root.join("x.rutbundle");
    std::fs::write(&bundle, b"not really a bundle").unwrap();
    let (code, err) = rut(&root.join("cache"), &["fetch", bundle.to_str().unwrap()]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("not a module directory"), "{err}");
}
