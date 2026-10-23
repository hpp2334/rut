//! The `rut new` lane through the binary: the scaffold's three files
//! (manifest + lock + root module), the refusals (clobber, illegal
//! name), the born-formatted template, the pins against the committed
//! `dist/std-v14` snapshot, and the offline end-to-end — the
//! full-deps manifest mounts clean and the ink-only hello runs.
//!
//! The templates are real files (`../template/`), so every byte law
//! here reads the SAME files the binary embeds — no restatement, the
//! template dir is the single carrier.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The scaffold's three templates — the exact files `src/new.rs`
/// embeds; the scaffolded bytes must equal these (the manifest with
/// `__NAME__` substituted), so a template edit and its law move
/// together.
const MOD_RUT: &str = include_str!("../template/src/mod.rut");
const MANIFEST_TEMPLATE: &str = include_str!("../template/rut.jsonc");
const LOCK_TEMPLATE: &str = include_str!("../template/rut.lock");

/// The nine dep keys the template declares — the swappable std set
/// (no `core`, no `*_host`, no `bench_cross`).
const STD_PKGS: [&str; 9] = [
    "ink", "pouch", "flow", "nmapset", "json", "strbuild", "calc", "futures", "http",
];

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-cli-newlane-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn rut() -> &'static str {
    env!("CARGO_BIN_EXE_rut")
}

/// `rut <args>` with this lane's cache dir, env-cleaned (no XDG/HOME
/// surprises).
fn rut_cmd(cache: &Path, args: &[&str]) -> Output {
    Command::new(rut())
        .env("RUT_CACHE_DIR", cache)
        .env_remove("XDG_CACHE_HOME")
        .args(args)
        .output()
        .expect("run the rut binary")
}

/// Scaffold `<root>/<name>` and assert the creation succeeded; the
/// cache rides under the scratch root.
fn scaffold(tag: &str, name: &str) -> (PathBuf, PathBuf) {
    let root = scratch(tag);
    let dir = root.join(name);
    let out = rut_cmd(&root.join("cache"), &["new", dir.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "scaffold must succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    (root, dir)
}

#[test]
fn scaffold_creates_the_manifest_and_root_module() {
    let (_root, dir) = scaffold("scaffold", "todolist");

    let manifest = std::fs::read_to_string(dir.join("rut.jsonc")).unwrap();
    let m = rut_driver::parse_manifest(&manifest).expect("the scaffolded manifest parses");
    assert_eq!(
        m.name.as_deref(),
        Some("todolist"),
        "the name is derived from the dir's basename"
    );
    assert_eq!(
        m.deps.len(),
        9,
        "exactly the nine — no core, no *_host, no bench_cross"
    );
    for pkg in STD_PKGS {
        let dep = m
            .deps
            .get(pkg)
            .unwrap_or_else(|| panic!("the manifest declares `{pkg}`"));
        let url = &dep["url"];
        assert!(
            url.ends_with(&format!("/dist/std/{pkg}.zip")),
            "`{pkg}` rides the std tag: {url}"
        );
        assert!(
            dep.get("sha256").is_none(),
            "manifest rows are UNPINNED — the pin lives in rut.lock: {url}"
        );
    }

    assert_eq!(
        std::fs::read_to_string(dir.join("src/mod.rut")).unwrap(),
        MOD_RUT,
        "the root module is the book's hello, byte-for-byte"
    );
}

#[test]
fn the_template_files_are_the_carrier() {
    let (_root, dir) = scaffold("carrier", "hello");
    // the scaffold writes the template bytes verbatim — the manifest
    // with the name substituted, the lock and the root module as-is
    let manifest = std::fs::read_to_string(dir.join("rut.jsonc")).unwrap();
    assert_eq!(
        manifest,
        MANIFEST_TEMPLATE.replace("__NAME__", "hello"),
        "the scaffolded manifest is the template, name substituted"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("rut.lock")).unwrap(),
        LOCK_TEMPLATE,
        "the scaffolded lock is the template, byte-for-byte"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("src/mod.rut")).unwrap(),
        MOD_RUT,
        "the scaffolded root module is the template, byte-for-byte"
    );
    // the lock template is CANONICAL: parse ∘ render is the identity —
    // a hand edit that breaks the sorted, aligned render is caught here
    let lock = rut_driver::parse_lock(LOCK_TEMPLATE).expect("the template lock parses");
    assert_eq!(
        rut_driver::render_lock(&lock),
        LOCK_TEMPLATE,
        "the template lock is exactly what `rut install` would write"
    );
    // the two templates tell the same story: every manifest url has
    // its lock row (the pins law's shape, held over the files themselves)
    let m = rut_driver::parse_manifest(&manifest).unwrap();
    assert_eq!(
        m.deps.len(),
        lock.deps().len(),
        "one lock row per manifest url row"
    );
    for (pkg, dep) in &m.deps {
        assert!(
            lock.deps()
                .iter()
                .any(|r| &r.pkg == pkg && r.url == dep["url"]),
            "the template lock pins `{pkg}`'s url"
        );
    }
}

#[test]
fn scaffold_refuses_to_clobber() {
    for seeded in ["rut.jsonc", "src/mod.rut", "rut.lock"] {
        let root = scratch(&format!("clobber-{seeded}"));
        let dir = root.join("hello");
        let seeded_path = dir.join(seeded);
        std::fs::create_dir_all(seeded_path.parent().unwrap()).unwrap();
        std::fs::write(&seeded_path, "// precious\n").unwrap();

        let out = rut_cmd(&root.join("cache"), &["new", dir.to_str().unwrap()]);
        assert_eq!(
            out.status.code(),
            Some(2),
            "seeding {seeded} must refuse: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(seeded), "{stderr}");
        assert!(stderr.contains("refusing to overwrite"), "{stderr}");
        assert_eq!(
            std::fs::read_to_string(&seeded_path).unwrap(),
            "// precious\n",
            "the seeded file is untouched"
        );
    }
}

#[test]
fn scaffold_refuses_an_illegal_name() {
    let root = scratch("illegal");
    let dir = root.join("my-app");
    let out = rut_cmd(&root.join("cache"), &["new", dir.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("my-app"), "{stderr}");
    assert!(stderr.contains("not a legal package name"), "{stderr}");
    assert!(stderr.contains("[a-zA-Z0-9_]+"), "{stderr}");
    assert!(!dir.exists(), "nothing is created on the refusal");
}

#[test]
fn template_is_fmt_clean() {
    let (root, dir) = scaffold("fmt-clean", "hello");
    let out = rut_cmd(
        &root.join("cache"),
        &["fmt", dir.to_str().unwrap(), "--check"],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "the template is born formatted\nstderr: {}\nstdout: {}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn pins_match_the_frozen_std_snapshot() {
    let (_root, dir) = scaffold("pins", "hello");
    // the scaffolded lock is the carrier — parse ITS rows and hold
    // each pin against the committed artifact; no table duplication
    let lock_text = std::fs::read_to_string(dir.join("rut.lock")).unwrap();
    let lock = rut_driver::parse_lock(&lock_text).expect("the scaffolded rut.lock parses");
    assert_eq!(lock.deps().len(), 9, "the lock pins exactly the nine");
    let frozen = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/std-v14");
    for row in lock.deps() {
        assert!(
            row.url.ends_with(&format!("/dist/std/{}.zip", row.pkg)),
            "`{}` rides the std tag: {}",
            row.pkg,
            row.url
        );
        let bytes = std::fs::read(frozen.join(format!("{}.zip", row.pkg)))
            .unwrap_or_else(|e| panic!("the frozen dist/std-v14/{}.zip: {e}", row.pkg));
        assert_eq!(
            row.sha256,
            rut_driver::sha256_hex(&bytes),
            "the `{}` pin is the sha256 of the frozen artifact",
            row.pkg
        );
    }
    // the manifest and the lock tell the same story: every manifest
    // url has its pin row, unpinned in the manifest itself
    let manifest = std::fs::read_to_string(dir.join("rut.jsonc")).unwrap();
    let m = rut_driver::parse_manifest(&manifest).unwrap();
    assert_eq!(
        m.deps.len(),
        lock.deps().len(),
        "one lock row per manifest url row"
    );
    for (pkg, dep) in &m.deps {
        assert!(
            lock.deps()
                .iter()
                .any(|r| &r.pkg == pkg && r.url == dep["url"]),
            "the lock pins `{pkg}`'s url"
        );
    }
}

/// A localhost HTTP server serving fixed payloads by path — the tail
/// after `host:port/` (a query string is tolerated and ignored);
/// 404 for anything the table does not name. Shaped after
/// tests/url_fetch.rs's one-payload server, times N.
struct BundleServer {
    port: u16,
}

impl BundleServer {
    fn serve(payloads: BTreeMap<String, Vec<u8>>) -> BundleServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind localhost");
        let port = listener.local_addr().unwrap().port();
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
                let Some(payload) = payloads.get(&path) else {
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
        BundleServer { port }
    }
}

#[test]
fn scaffolded_project_runs_against_a_local_server() {
    // the wire: the whole frozen snapshot — the nine the manifest
    // names, plus the host surfaces they can pull (bundles are closed,
    // but the server answers whatever the graph asks)
    let frozen = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/std-v14");
    let mut payloads = BTreeMap::new();
    for e in std::fs::read_dir(&frozen).unwrap() {
        let e = e.unwrap();
        let n = e.file_name().to_string_lossy().into_owned();
        if n.ends_with(".rutbundle") || n.ends_with(".zip") {
            payloads.insert(
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap(),
            );
        }
    }
    let server = BundleServer::serve(payloads);

    // the scaffold with ONLY the dep urls rewritten to the local
    // server — manifest AND lock (the lock's rows carry the same urls;
    // the pins are the same bytes either way)
    let (root, dir) = scaffold("e2e", "hello");
    for file in ["rut.jsonc", "rut.lock"] {
        let path = dir.join(file);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            &path,
            text.replace(
                "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v14/dist/std/",
                &format!("http://127.0.0.1:{}/", server.port),
            ),
        )
        .unwrap();
    }

    // the install-first law: run never downloads — install warms the
    // cache from the local server (and confirms the lock is fresh)
    let out = rut_cmd(&root.join("cache"), &["install", dir.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "the scaffold install must succeed: {}\nstdout: {}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );

    // the ink-only hello runs clean against the FULL manifest — unused
    // deps are legal (host pkgs merge inert unless the program mounts
    // their pkg), so no
    // unused-dep diagnostics, and the output is the book's
    let out = rut_cmd(&root.join("cache"), &["run", dir.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "the scaffolded hello must run: {}\nstdout: {}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("hello, rut!"),
        "stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}
