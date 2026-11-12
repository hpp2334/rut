//! The per-project install lock's binary laws — real `rut install`
//! processes on ONE scratch project against a GATED localhost server
//! (offline: no external network), scratch worlds, every run
//! `RUT_HOME`-env'd at a scratch root (never the real `~/.rut`):
//!
//! 1. the HOLDER WINDOW: while an install is mid-flight (its GET held
//!    at the server's gate), the project's lockfile under
//!    `$RUT_HOME/locks/` names it — pid + start time + command; that
//!    content is what a waiter announces;
//! 2. SERIALIZATION: a second install on the same project WAITS (the
//!    `rut: waiting` line naming the first's pid) and only proceeds
//!    after the first's rut.lock landed — proved by the second's own
//!    summary reading `rut.lock unchanged (… no drift)`: it saw the
//!    first's write, so the two rut.lock writes never interleaved.
//!    Both exit 0 and the lock is whole.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// One localhost HTTP server serving fixed payloads by path — the tail
/// after `host:port/`; 404 for anything the table does not name. The
/// GATE holds every connection at the accept (50 ms steps) until
/// [`TestServer::release`] — that is the mid-install window: the
/// first install's GET sits in the handler while the test observes
/// the lockfile. A 15 s hard cap keeps a bug from hanging the suite.
struct TestServer {
    port: u16,
    open: Arc<AtomicBool>,
}

impl TestServer {
    fn serve(payloads: BTreeMap<String, Vec<u8>>) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind localhost");
        let port = listener.local_addr().unwrap().port();
        let open = Arc::new(AtomicBool::new(false));
        let gate = open.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                // the gate: hold the request until released (15 s cap)
                let mut held = 0;
                while !gate.load(Ordering::SeqCst) && held < 15_000 {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    held += 50;
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
        TestServer { port, open }
    }

    fn url(&self, name: &str) -> String {
        format!("http://127.0.0.1:{}/{}", self.port, name)
    }

    /// Let the held requests through.
    fn release(&self) {
        self.open.store(true, Ordering::SeqCst);
    }
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-cli-locklane-{tag}-{}", std::process::id()));
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

/// The same lane, SPAWNED (non-blocking) with piped stdio — the
/// mid-flight holder (wait_with_output reads only piped children).
fn rut_spawn(home: &Path, args: &[&str]) -> Child {
    use std::process::Stdio;
    Command::new(env!("CARGO_BIN_EXE_rut"))
        .env("RUT_HOME", home)
        .env_remove("XDG_CACHE_HOME")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args(args)
        .spawn()
        .expect("spawn the rut binary")
}

fn output(out: Output) -> (i32, String, String) {
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
    write(&dir, "src/mod.rut", body);
    rut_native::pack_dir(&dir).expect("pack the leaf")
}

/// The project's lockfile under `$RUT_HOME/locks/` — the derivation
/// law (`sha256(canonical project path)[:16]`), spelled against this
/// lane's scratch home (the children carry it as `RUT_HOME`; this
/// side computes rather than env-reads).
fn lockfile_for(home: &Path, project: &Path) -> PathBuf {
    let canonical = project.canonicalize().unwrap();
    let hash = rut_driver::sha256_hex(canonical.to_string_lossy().as_bytes());
    home.join(rut_driver::LOCKS_DIR)
        .join(format!("{}.lock", &hash[..16]))
}

#[test]
fn a_second_install_waits_for_the_holder_then_proceeds() {
    let bytes = leaf_bundle(
        "holder leaf",
        "util",
        "pub fn twice(v: i64) -> i64 {\n    return v * 2;\n}\n",
    );
    let mut payloads = BTreeMap::new();
    payloads.insert("util.rutbundle".to_string(), bytes.clone());
    let server = TestServer::serve(payloads);
    let url = server.url("util.rutbundle");

    let root = scratch("holder world");
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest("app", &format!(r#""util": {{"url": "{url}"}}"#)),
    );
    write(
        &app,
        "src/mod.rut",
        "use util::{twice};\n\nentry fn main() {\n    let x = twice(21);\n}\n",
    );
    let home = root.join("home");

    // the FIRST install: acquires the lock, then its GET parks at the
    // server's gate — mid-install for as long as the test holds it
    let first = rut_spawn(&home, &["install", app.to_str().unwrap()]);

    // THE HOLDER WINDOW: while the first is mid-install, the lockfile
    // under $RUT_HOME/locks/ names it (pid + started + command) — the
    // advisory content waiters read
    let lockfile = lockfile_for(&home, &app);
    let mut holder = String::new();
    for _ in 0..200 {
        // up to ~20 s for the holder info to land (CI slowness slack)
        if let Ok(text) = std::fs::read_to_string(&lockfile) {
            if text.contains("pid=") {
                holder = text;
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(
        holder.contains(&format!("pid={}", first.id())),
        "the lockfile names the holding install ({first:?}): {holder:?} @ {}",
        lockfile.display()
    );
    assert!(holder.contains("started="), "{holder}");
    assert!(holder.contains("command="), "{holder}");
    // and the first has NOT written its rut.lock yet — it is genuinely
    // mid-install (its GET still parks at the gate)
    assert!(
        !app.join("rut.lock").exists(),
        "the holder is mid-install — rut.lock has not landed"
    );

    // let the second contend for a moment WHILE the first still holds
    // (the gate stays closed), then release: the second's first try
    // finds the lock held, so it MUST have announced the wait
    let second = rut_spawn(&home, &["install", app.to_str().unwrap()]);
    std::thread::sleep(std::time::Duration::from_millis(600));
    server.release();

    let (code, out, err) = output(second.wait_with_output().unwrap());
    assert_eq!(code, 0, "the waiter proceeds after the holder: {err}");
    assert!(
        err.contains("rut: waiting — another install holds this project"),
        "the second install announces the wait: {err}"
    );
    assert!(
        err.contains(&format!("pid {}", first.id())),
        "the wait names the holding pid: {err}"
    );

    // SERIALIZED WRITES: the second ran only after the first's
    // rut.lock landed — its own summary reads the no-drift marker
    // (a racer would print `rut.lock written`), and the file on disk
    // is whole and pins the served bytes
    assert!(
        out.contains("rut.lock unchanged (1 rows, no drift)"),
        "the second install saw the first's lock — the writes serialized: {out}"
    );
    let (code, _out, err) = output(first.wait_with_output().unwrap());
    assert_eq!(code, 0, "the first install lands: {err}");
    let pin = rut_driver::sha256_hex(&bytes);
    let lock = rut_driver::parse_lock(&std::fs::read_to_string(app.join("rut.lock")).unwrap())
        .expect("rut.lock is whole after both installs");
    assert_eq!(lock.deps()[0].sha256, pin);
}
