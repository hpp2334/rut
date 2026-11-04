//! The v6 decl root's CLI face: `rut pack <host-dir>` works (the bundle
//! lands), `rut run <host.rutbundle>` refuses with intent (a host
//! bundle carries a declaration surface — nothing to run), and
//! `pack --strip` refuses on a host root (no programs, no sidecar).
//! The mechanism tests are pinned driver-side (`decl_bundles.rs`).

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-cli-decl-{tag}-{}", std::process::id()));
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
/// included — stays off the real `~/.rut`.
fn rut_cmd() -> Command {
    static HOME: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    let home = HOME.get_or_init(|| scratch("rut-home"));
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rut"));
    cmd.env("RUT_HOME", home);
    cmd
}

fn host_world(tag: &str) -> PathBuf {
    let root = scratch(tag);
    let dir = root.join("logger_host");
    write(
        &dir,
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 12, "name": "logger_host", "type": "host", "entry": {"type": "./logger_host.d.rut"}}"#,
    );
    write(
        &dir,
        "src/logger_host.d.rut",
        "pub host fn log(line: str);\n",
    );
    root
}

#[test]
fn pack_a_host_dir_and_run_refuses_it() {
    let root = host_world("cliface");
    let dir = root.join("logger_host");

    // pack works, prints the output path
    let out = rut_cmd()
        .args(["pack", dir.to_str().unwrap(), "--mode", "bin"])
        .output()
        .expect("run rut pack");
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let bundle = root.join("logger_host.rutbundle");
    assert!(bundle.exists(), "the packed bundle lands");
    assert!(String::from_utf8_lossy(&out.stdout).contains("logger_host.rutbundle"));

    // `rut run` refuses with intent — nothing to run, bind the rows
    let out = rut_cmd()
        .args(["run", bundle.to_str().unwrap()])
        .output()
        .expect("run rut run");
    assert!(!out.status.success(), "a host bundle has nothing to run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("declaration surface"), "{stderr}");
    assert!(stderr.contains("nothing to run"), "{stderr}");
    assert!(stderr.contains("logger_host"), "{stderr}");
}

#[test]
fn pack_strip_refuses_on_a_host_root() {
    let root = host_world("clistrip");
    let out = rut_cmd()
        .args([
            "pack",
            root.join("logger_host").to_str().unwrap(),
            "--mode",
            "bin",
            "--strip",
        ])
        .output()
        .expect("run rut pack --strip");
    assert!(!out.status.success(), "--strip has nothing to strip");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no symbols to strip"), "{stderr}");
    assert!(
        !root.join("logger_host.rutsym").exists(),
        "no sidecar is written"
    );
}
