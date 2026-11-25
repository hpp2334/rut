//! The compiled-dependency lane: a dep unit's binary rides the closure
//! archive as a `deps/<identity>/mod.rutc` group, and the drive decodes
//! it back — the toolchain reads what it writes. The pin is the book's
//! async shape (`launch_future(async { .. })`, `docs/src/reference/
//! async.md`) against the `futures` + `ink` packages as path
//! dependencies on this repo's sources.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-cli-compileddep-{tag}-{}", std::process::id()));
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

/// `rut <args>` with a fresh scratch rut home — the run cache and the
/// dep store both live under it; nothing ever touches `~/.rut`.
fn rut_run(home: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(rut())
        .env("RUT_HOME", home)
        .args(args)
        .output()
        .expect("run the rut binary");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The repo's `rut/<pkg>` sources, as seen from this test's crate.
fn repo_pkg(pkg: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../rut")
        .join(pkg)
        .to_string_lossy()
        .into_owned()
}

const LAUNCHED_ASYNC_BLOCK: &str = r#"use ink::{ Logger };
use futures::{ launch_future };

struct P { a: i32 }

async fn fut() -> P {
    return P { a: 7 };
}

entry fn main() {
    let log = Logger("t");
    launch_future(async {
        let p = await fut();
        let x = p.a;
        log.info(f"{x}");
    });
}
"#;

#[test]
fn a_launched_async_block_through_a_compiled_dependency_runs() {
    let root = scratch("launched");
    let dir = root.join("app");
    write(
        &dir,
        "rut.jsonc",
        &format!(
            r#"{{"name": "app", "deps": {{"futures": {{"path": "{}"}}, "ink": {{"path": "{}"}}}}}}"#,
            repo_pkg("futures"),
            repo_pkg("ink"),
        ),
    );
    write(&dir, "src/mod.rut", LAUNCHED_ASYNC_BLOCK);
    let home = root.join("home");

    // install warms the lane the book teaches; run mounts the deps'
    // compiled binaries inside the closure archive and drives them
    let (code, _out, err) = rut_run(&home, &["install", dir.to_str().unwrap()]);
    assert_eq!(code, 0, "install: {err}");
    let (code, out, err) = rut_run(&home, &["run", "--no-cache", dir.to_str().unwrap()]);
    assert_eq!(code, 0, "run: {err}");
    assert!(
        out.contains('7'),
        "the launched block's answer rides stdout: {out}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
