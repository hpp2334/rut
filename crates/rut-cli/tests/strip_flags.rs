//! The strip flags through the binary: `pack --strip` emits the bundle
//! plus the sibling `.rutsym` sidecar (and prints both); `run --symbols`
//! demands a compiled `.rutbundle` (usage error otherwise); and the
//! sidecar restores a trace's real names through the binary end to end.
//! The heavy lanes (determinism, refusal matrix, exact restoration)
//! are pinned driver-side; this file is the CLI face.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-cli-strip-{tag}-{}", std::process::id()));
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

/// `rut` as a pre-configured Command: $RUT_HOME pinned to a scratch
/// dir created once per test process (nextest runs one test per
/// process), so every spawn — the run-key cache and the dep store
/// included — stays off the real `~/.rut` — and removed again at
/// process exit (an atexit hook, below).
fn rut_cmd() -> Command {
    static HOME: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    let home = HOME.get_or_init(|| {
        let h = scratch("rut-home");
        remove_at_exit(&h);
        h
    });
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rut"));
    cmd.env("RUT_HOME", home);
    cmd
}

/// Remove `dir` when the process exits — the scratch rut home is
/// litter, never a keeper.
fn remove_at_exit(dir: &Path) {
    extern "C" {
        fn atexit(cb: extern "C" fn()) -> i32;
    }
    static PENDING: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);
    extern "C" fn clean() {
        let dir = PENDING.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(dir) = dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
    *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir.to_path_buf());
    // registered once per process (the OnceLock above); a refused
    // registration only leaves the litter
    let _ = unsafe { atexit(clean) };
}

/// A single-group world whose `main` panics with a stack-trace render —
/// the trap message carries the render to stderr, so name restoration
/// is observable without any package mounts.
fn panic_world(tag: &str) -> PathBuf {
    let root = scratch(tag);
    let app = root.join("app");
    write(&app, "rut.jsonc", r#"{"name": "app"}"#);
    let mut src = String::from("fn boom() -> str {\n");
    for i in 0..25 {
        src.push_str(&format!("    let p{i} = {i};\n"));
    }
    src.push_str("    let t = capture_stacktrace();\n");
    src.push_str("    return t.render();\n");
    src.push_str("}\n");
    src.push_str("entry fn main() -> nil {\n");
    src.push_str("    panic(boom());\n");
    src.push_str("}\n");
    write(&app, "src/mod.rut", &src);
    root
}

#[test]
fn pack_strip_writes_and_prints_the_sidecar() {
    let root = panic_world("packflag");
    let out = rut_cmd()
        .args([
            "pack",
            root.join("app").to_str().unwrap(),
            "--mode",
            "bin",
            "--strip",
        ])
        .output()
        .expect("run rut");
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let bundle = root.join("app.rutbundle");
    let sidecar = root.join("app.rutsym");
    assert!(bundle.is_file(), "the bundle rides as today");
    assert!(sidecar.is_file(), "the sidecar lands at the sibling path");
    assert!(stdout.contains("app.rutbundle"), "{stdout}");
    assert!(stdout.contains("app.rutsym"), "both paths print: {stdout}");
    assert!(
        stdout.contains("PRIVATE"),
        "the sidecar is labeled private: {stdout}"
    );
    // the sidecar is the private half — never an entry inside the bundle
    let bytes = std::fs::read(&bundle).unwrap();
    assert!(
        !bytes.windows(5).any(|w| w == b"RUTS\0"),
        "no RUTS magic inside the archive"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn run_symbols_on_a_non_bundle_input_is_a_usage_error() {
    let root = scratch("usage");
    let src = root.join("main.rut");
    write(&root, "main.rut", "entry fn main() -> i32 { return 4; }\n");
    // a directory input reaches the pairing check: `--symbols` is a
    // compiled-bundle-only restore
    let out = rut_cmd()
        .args(["run", root.to_str().unwrap(), "--symbols", "x.rutsym"])
        .output()
        .expect("run rut");
    assert_eq!(out.status.code(), Some(2), "usage error, exit 2");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--symbols"), "{stderr}");
    assert!(stderr.contains(".rutbundle"), "{stderr}");
    // a loose file never reaches the pairing check — the run lane's
    // allowlist door refuses it first (`run_lane.rs` pins that message)
    let out = rut_cmd()
        .args(["run", src.to_str().unwrap(), "--symbols", "x.rutsym"])
        .output()
        .expect("run rut");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("not a runnable unit"), "{stderr}");
    // a dangling flag value is the same loud lane
    let out = rut_cmd()
        .args(["run", src.to_str().unwrap(), "--symbols"])
        .output()
        .expect("run rut");
    assert_eq!(out.status.code(), Some(2));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn run_on_a_stripped_bundle_restores_with_symbols_and_degrades_without() {
    let root = panic_world("e2e");
    let out = rut_cmd()
        .args([
            "pack",
            root.join("app").to_str().unwrap(),
            "--mode",
            "bin",
            "--strip",
        ])
        .output()
        .expect("run rut");
    assert!(out.status.success());

    // without the sidecar: mangled names, pc-only degradation
    let out = rut_cmd()
        .args(["run", root.join("app.rutbundle").to_str().unwrap()])
        .output()
        .expect("run rut");
    assert_eq!(out.status.code(), Some(1), "the panic carries the render");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("trap:"), "{stderr}");
    assert!(
        stderr.contains("at %"),
        "mangled name in the render: {stderr}"
    );
    assert!(stderr.contains("@ pc"), "pc-only degradation: {stderr}");

    // with the sidecar: the real fn name and a line/col position
    let out = rut_cmd()
        .args([
            "run",
            root.join("app.rutbundle").to_str().unwrap(),
            "--symbols",
            root.join("app.rutsym").to_str().unwrap(),
        ])
        .output()
        .expect("run rut");
    assert_eq!(
        out.status.code(),
        Some(1),
        "the panic still traps: {stderr}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("at boom ("), "restored name: {stderr}");
    assert!(!stderr.contains("@ pc"), "positions restored: {stderr}");
    let _ = std::fs::remove_dir_all(&root);
}
