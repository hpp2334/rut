//! The global dep store's binary laws — the wire-side behavior an
//! editor or CI sees after `rut install`, ONE localhost
//! server (offline: no external network), scratch worlds, every run
//! `RUT_HOME`-env'd at a scratch root (never the real `~/.rut`):
//!
//! 1. the store LAYOUT law: a pinned dep lands at
//!    `<RUT_HOME>/deps/<spec>@<sha[:16]>/` — the pin IS the address —
//!    carrying the artifact bytes as downloaded, a `meta.json` that
//!    vouches them (full sha256, url, format), and the extracted
//!    `unpack/` tree;
//! 2. the store-FIRST law: a warm store answers a re-install and an
//!    offline run with the server silent — the wire is never touched
//!    on a hit;
//! 3. the zip transport's layout: the unpacked tree rides `unpack/`
//!    verbatim (manifest, `src/`, embedded dep groups), and the
//!    offline run mounts it;
//! 4. the cold-store refusal: a missing dep set (pinned or not) is
//!    ONE aggregated refusal at the run gate naming every dep and
//!    the `rut install` fix — the walk's own `is not pinned`
//!    mount-door law is rut-driver's (its url_deps tests).

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// One localhost HTTP server serving fixed payloads by path — the tail
/// after `host:port/`; 404 for anything the table does not name.
/// `set_up(false)` closes connections at once — the offline
/// simulation — without unbinding the port.
struct TestServer {
    port: u16,
    up: Arc<AtomicBool>,
}

impl TestServer {
    fn serve(payloads: BTreeMap<String, Vec<u8>>) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind localhost");
        let port = listener.local_addr().unwrap().port();
        let up = Arc::new(AtomicBool::new(true));
        let up_thread = up.clone();
        let served = payloads;
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                if !up_thread.load(Ordering::SeqCst) {
                    continue; // "offline": accept nothing, close at once
                }
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
                let Some(payload) = served.get(&path) else {
                    let not_found = "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nnot found";
                    let _ = stream.write_all(not_found.as_bytes());
                    continue;
                };
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(payload);
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
    let d = std::env::temp_dir().join(format!("rut-cli-store-{tag}-{}", std::process::id()));
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
    format!(r#"{{"name": "{name}", "deps": {{{deps}}}}}"#)
}

/// `rut <args>` with this lane's rut home, env-cleaned: (exit, stdout,
/// stderr) — the summaries are stdout's, the chatter stderr's.
fn rut(home: &Path, args: &[&str]) -> (i32, String, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_rut"))
        .env("RUT_HOME", home)
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

/// The store dir a pinned dep lands in — the pin IS the address.
fn store_dir_for(home: &Path, spec: &str, pin: &str) -> PathBuf {
    home.join("deps").join(format!("{spec}@{}", &pin[..16]))
}

/// Pack one leaf pkg (`name`, `body`) into bundle bytes — the served
/// payload.
fn leaf_bundle(tag: &str, name: &str, body: &str) -> Vec<u8> {
    let root = scratch(tag);
    let dir = root.join(name);
    write(&dir, "rut.jsonc", &manifest(name, ""));
    write(&dir, "src/mod.rut", body);
    rut_native::pack_dir(&dir).expect("pack the leaf")
}

fn consumer_world(tag: &str, deps: &str, body: &str) -> (PathBuf, PathBuf) {
    let root = scratch(tag);
    let app = root.join("app");
    write(&app, "rut.jsonc", &manifest("app", deps));
    write(&app, "src/mod.rut", body);
    (root, app)
}

// -- 1 ----------------------------------------------------------------

#[test]
fn install_publishes_the_store_layout_and_reruns_are_store_first() {
    let bytes = leaf_bundle(
        "layout leaf",
        "util",
        "pub fn twice(v: i64) -> i64 {\n    return v * 2;\n}\n",
    );
    let mut payloads = BTreeMap::new();
    payloads.insert("util.rutbundle".to_string(), bytes.clone());
    let server = TestServer::serve(payloads);
    let url = server.url("util.rutbundle");
    let pin = rut_driver::sha256_hex(&bytes);
    let (root, app) = consumer_world(
        "layout consumer",
        &format!(r#""util": {{"url": "{url}"}}"#),
        "use util::{twice};\n\nentry fn main() {\n    let x = twice(21);\n}\n",
    );
    let home = root.join("home");

    let (code, _out, err) = rut(&home, &["install", app.to_str().unwrap()]);
    assert_eq!(code, 0, "first install must succeed: {err}");

    // THE LAYOUT: `<deps>/util@<pin16>/` — the pin IS the address —
    // artifact + meta + the extracted tree
    let dir = store_dir_for(&home, "util", &pin);
    let artifact = dir.join("artifact.rutbundle");
    assert!(artifact.is_file(), "the artifact lands: {:?}", dir);
    assert_eq!(
        std::fs::read(&artifact).unwrap(),
        bytes,
        "the artifact is the pinned bytes as downloaded"
    );
    let meta_text = std::fs::read_to_string(dir.join("meta.json")).unwrap();
    let meta = rut_driver::parse_store_meta(&meta_text).expect("the meta parses");
    assert_eq!(meta.name, "util");
    assert_eq!(meta.url, url);
    assert_eq!(meta.sha256, pin, "the FULL sha256, not the 16-hex prefix");
    assert_eq!(meta.format, "rutbundle");
    assert_eq!(meta.format_version, rut_driver::STORE_META_FORMAT_VERSION);
    let unpack = dir.join("unpack");
    // the extracted tree rides the bundle's entries verbatim — the
    // compiled wire: the manifest, the scopes, the compiled root (the
    // manifest carries the pair the pack door stamps — the compat lane
    // reads it back)
    let unpacked = rut_driver::bundle::parse_manifest_compat(
        &std::fs::read_to_string(unpack.join("rut.jsonc")).unwrap(),
    )
    .expect("the extracted manifest parses");
    assert_eq!(unpacked.name.as_deref(), Some("util"));
    assert!(unpack.join("mod.rutc").is_file(), "the compiled root rode");
    assert!(unpack.join("rut.scopes").is_file(), "the scopes rode");
    let deps = home.join("deps");
    let tmp_leftovers: Vec<String> = std::fs::read_dir(&deps)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with('.'))
        .collect();
    assert!(
        tmp_leftovers.is_empty(),
        "no tmp siblings survive: {tmp_leftovers:?}"
    );

    // THE STORE-FIRST LAW: the server goes SILENT — the re-install is
    // all hits (no wire, no re-hash), the store untouched
    let artifact_before = std::fs::read(&artifact).unwrap();
    let meta_before = std::fs::read(dir.join("meta.json")).unwrap();
    server.set_up(false);
    let (code, out, err) = rut(&home, &["install", app.to_str().unwrap()]);
    assert_eq!(code, 0, "a warm store never needs the wire: {err}");
    assert!(err.contains("install: util — cache hit ("), "{err}");
    assert!(
        !err.contains("downloading"),
        "a hit never touches the wire: {err}"
    );
    assert!(
        out.contains("install: 1 url deps resolved — 0 downloaded, 1 cache hits"),
        "{out}"
    );
    assert!(
        out.contains("rut.lock unchanged (1 rows, no drift)"),
        "{out}"
    );
    assert_eq!(std::fs::read(&artifact).unwrap(), artifact_before);
    assert_eq!(std::fs::read(dir.join("meta.json")).unwrap(), meta_before);

    // and the offline run answers from the same store
    let (code, _out, err) = rut(&home, &["run", app.to_str().unwrap()]);
    assert_eq!(code, 0, "the offline run rides the store: {err}");
}

// -- 2 ----------------------------------------------------------------

#[test]
fn a_zip_dep_publishes_the_unpacked_tree_and_runs_offline() {
    // the std-set shape: the dep rides the SOURCE-ZIP transport (the
    // manifest rides multi-line — the zip rewrite drops wire rows
    // line-wise)
    let root = scratch("zip world");
    let gen = root.join("gen");
    write(
        &gen,
        "rut.jsonc",
        r#"{
  "name": "gen"
}"#,
    );
    write(
        &gen,
        "src/mod.rut",
        "pub fn twice(v: i64) -> i64 {\n    return v * 2;\n}\n",
    );
    let zip = rut_native::pack_dir_zip(&gen).expect("pack the source zip");
    let mut payloads = BTreeMap::new();
    payloads.insert("gen.zip".to_string(), zip.clone());
    let server = TestServer::serve(payloads);
    let url = server.url("gen.zip");
    let pin = rut_driver::sha256_hex(&zip);

    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest("app", &format!(r#""gen": {{"url": "{url}"}}"#)),
    );
    let lock = rut_driver::LockFile::from_rows(vec![rut_driver::LockRow {
        pkg: "gen".into(),
        url: url.clone(),
        sha256: pin.clone(),
    }]);
    write(&app, "rut.lock", &rut_driver::render_lock(&lock));
    write(
        &app,
        "src/mod.rut",
        "use gen::{twice};\n\nentry fn main() {\n    let x = twice(21);\n}\n",
    );
    let home = root.join("home");

    let (code, _out, err) = rut(&home, &["install", app.to_str().unwrap()]);
    assert_eq!(code, 0, "install must publish the zip dep: {err}");

    // the layout: the artifact named by the url's kind, the meta
    // naming the SOURCE transport, the tree unpacked verbatim
    let dir = store_dir_for(&home, "gen", &pin);
    assert_eq!(
        std::fs::read(dir.join("artifact.zip")).unwrap(),
        zip,
        "the pinned bytes as downloaded"
    );
    let meta =
        rut_driver::parse_store_meta(&std::fs::read_to_string(dir.join("meta.json")).unwrap())
            .expect("the meta parses");
    assert_eq!(meta.format, "zip");
    assert_eq!(meta.sha256, pin);
    let unpack = dir.join("unpack");
    assert!(unpack.join("rut.jsonc").is_file(), "the manifest rides");
    assert!(
        unpack.join("src/mod.rut").is_file(),
        "the sources ride under the src/ shell"
    );

    // and the offline run answers — the store's zip bytes mount
    server.set_up(false);
    let (code, _out, err) = rut(&home, &["run", app.to_str().unwrap()]);
    assert_eq!(code, 0, "the offline run rides the store: {err}");
}

// -- 3 ----------------------------------------------------------------

#[test]
fn unpinned_rows_refuse_loudly_at_the_gate() {
    let bytes = leaf_bundle(
        "unpinned leaf",
        "util",
        "pub fn twice(v: i64) -> i64 {\n    return v * 2;\n}\n",
    );
    let mut payloads = BTreeMap::new();
    payloads.insert("util.rutbundle".to_string(), bytes);
    let server = TestServer::serve(payloads);
    let url = server.url("util.rutbundle");
    // NO rut.lock — the manifest row is unpinned and nothing vouches it
    let (root, app) = consumer_world(
        "unpinned consumer",
        &format!(r#""util": {{"url": "{url}"}}"#),
        "use util::{twice};\n\nentry fn main() {\n    let x = twice(21);\n}\n",
    );
    let home = root.join("home");

    // the run gate: one aggregated refusal, the fix named, no GET
    server.set_up(false);
    let (code, _out, err) = rut(&home, &["run", app.to_str().unwrap()]);
    assert_eq!(code, 2, "an unpinned dep refuses the run: {err}");
    assert!(err.contains("not in the global dep store"), "{err}");
    assert!(err.contains("run `rut install` first"), "{err}");
    assert!(err.contains("util — "), "{err}");
}

// -- 4 ----------------------------------------------------------------

#[test]
fn cold_run_names_every_missing_dep_at_once() {
    // two url deps pinned by the lock, neither cached — ONE aggregated
    // refusal lists both (not a first-miss mid-walk), and the server's
    // silence proves run never even tried to GET
    let bytes = leaf_bundle(
        "agg leaf",
        "util",
        "pub fn twice(v: i64) -> i64 {\n    return v * 2;\n}\n",
    );
    let mut payloads = BTreeMap::new();
    payloads.insert("a.rutbundle".to_string(), bytes.clone());
    payloads.insert("b.rutbundle".to_string(), bytes);
    let server = TestServer::serve(payloads);
    let url_a = server.url("a.rutbundle");
    let url_b = server.url("b.rutbundle");
    // the pins are never consulted — the store is cold, the gate
    // refuses before any mount door opens
    let pin = rut_driver::sha256_hex(b"same bytes, two names");
    let (root, app) = consumer_world(
        "agg consumer",
        &format!(r#""a": {{"url": "{url_a}"}}, "b": {{"url": "{url_b}"}}"#),
        "entry fn main() -> nil { return; }\n",
    );
    let lock = rut_driver::LockFile::from_rows(vec![
        rut_driver::LockRow {
            pkg: "a".into(),
            url: url_a,
            sha256: pin.clone(),
        },
        rut_driver::LockRow {
            pkg: "b".into(),
            url: url_b,
            sha256: pin,
        },
    ]);
    write(&app, "rut.lock", &rut_driver::render_lock(&lock));
    let home = root.join("home");

    server.set_up(false);
    let (code, _out, err) = rut(&home, &["run", app.to_str().unwrap()]);
    assert_eq!(code, 2, "{err}");
    assert!(
        err.contains("2 url deps not in the global dep store"),
        "{err}"
    );
    assert!(err.contains("run `rut install` first"), "{err}");
    assert!(err.contains("a — "), "{err}");
    assert!(err.contains("b — "), "{err}");
}
