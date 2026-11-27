//! The display-path law through the CLI's own door: a diagnostic names a
//! module's file by its PATH RELATIVE TO THE PACKAGE'S ROOT-FILE
//! DIRECTORY — `imp.rut`, `m/a/b.rut`, `m/b.rut` — never the
//! directory-form guess (`imp/mod.rut`, `imp/imp.rut`, `m/a/b/b.rut`).
//!
//! One real file must be spelled one way by every message: the
//! own-package visibility refusal, the checker's not-declared hint, the
//! mount refusal, and the dependency lane's walked-module hint all read
//! the same record.
//!
//! The four layouts (ground truth 6 and 10):
//! - a single-file child of the root (`src/imp.rut`);
//! - the nested single-file child `src/m/a/b.rut` (a directory-form
//!   parent);
//! - the child of a single-file parent (`src/m/a.rut` declares `b`,
//!   mounted beside its declaring file as `src/m/b.rut`);
//! - the dependency lane: `rut/calc`'s only child is `src/math.rut`.

use std::path::Path;
use std::process::Command;

fn scratch(tag: &str) -> rut_testkit::Scratch {
    rut_testkit::Scratch::new(&format!("cli-display-paths-{tag}"))
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, text).unwrap();
}

/// `rut` as a pre-configured Command: $RUT_HOME pinned to the test's
/// scratch home, so every spawn — the run-key cache and the dep store
/// included — stays off the real `~/.rut`.
fn rut_cmd(home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rut"));
    cmd.env("RUT_HOME", home);
    cmd
}

/// `rut run <dir>`'s stderr through a scratch home — the CLI's own door.
fn run(tag: &str, dir: &Path) -> String {
    let home = rut_testkit::Scratch::new(&format!("cli-display-paths-home-{tag}"));
    let out = rut_cmd(&home)
        .args(["run", dir.to_str().unwrap()])
        .output()
        .expect("run rut run <dir>");
    assert!(!out.status.success(), "the project must refuse");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// `rut pack <dir>`'s stderr through a scratch home — the pack lane's
/// own door (a pack failure exits 2 through `pack_failed`). Bin mode:
/// the unit-diag poison gate (err_poison.rs's `pack_dir` shape) lives
/// on the compile-before-encode arm — a source zip carries source and
/// packs clean.
fn pack(tag: &str, dir: &Path) -> String {
    let home = rut_testkit::Scratch::new(&format!("cli-display-paths-home-{tag}"));
    let out = rut_cmd(&home)
        .args(["pack", "--mode", "bin", dir.to_str().unwrap()])
        .output()
        .expect("run rut pack <dir>");
    assert!(!out.status.success(), "the pack must refuse");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Assert the refusal names `file` and never the directory-form guess.
fn names(stderr: &str, file: &str, wrong: &str) {
    assert!(
        stderr.contains(file),
        "the diagnostic must name `{file}`; stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains(wrong),
        "the diagnostic must not name the non-existent `{wrong}`; stderr:\n{stderr}"
    );
}

#[test]
fn a_single_file_child_is_named_by_its_file_in_every_message() {
    // `src/imp.rut` (the root's single-file child): the visibility
    // refusal, the checker's hint and the mount refusal all name
    // `imp.rut`.

    // the own-package visibility refusal
    let root = scratch("single-vis");
    let dir = root.join("app");
    write(&dir, "rut.jsonc", r#"{"name": "app"}"#);
    write(
        &dir,
        "src/mod.rut",
        "use imp::{ Task };\nmod imp;\nentry fn main() {}\n",
    );
    write(&dir, "src/imp.rut", "struct Task { id: i64; }\n");
    names(&run("single-vis", &dir), "in `imp.rut`", "in `imp/imp.rut`");

    // the checker's not-declared hint (a qualified value path)
    let root = scratch("single-check");
    let dir = root.join("app");
    write(&dir, "rut.jsonc", r#"{"name": "app"}"#);
    write(
        &dir,
        "src/mod.rut",
        "mod imp;\nentry fn main() -> i64 { return imp::nope(); }\n",
    );
    write(&dir, "src/imp.rut", "pub fn yes() -> i64 { return 1; }\n");
    names(
        &run("single-check", &dir),
        "in module `imp` (imp.rut)",
        "(imp/mod.rut)",
    );

    // the mount refusal inside the single-file child
    let root = scratch("single-mount");
    let dir = root.join("app");
    write(&dir, "rut.jsonc", r#"{"name": "app"}"#);
    write(&dir, "src/mod.rut", "mod imp;\nentry fn main() {}\n");
    write(
        &dir,
        "src/imp.rut",
        "mod gone;\npub fn f() -> i64 { return 1; }\n",
    );
    names(
        &run("single-mount", &dir),
        "`mod gone;` in imp.rut",
        "in imp/imp.rut",
    );
}

#[test]
fn a_nested_single_file_child_is_named_by_its_path() {
    // `src/m/a/b.rut` (a directory-form parent declares the
    // single-file child): all three messages name `m/a/b.rut`.

    let root = scratch("nested-vis");
    let dir = root.join("app");
    write(&dir, "rut.jsonc", r#"{"name": "app"}"#);
    write(
        &dir,
        "src/mod.rut",
        "use m::a::b::{ Task };\nmod m;\nentry fn main() {}\n",
    );
    write(&dir, "src/m/mod.rut", "pub mod a;\n");
    write(&dir, "src/m/a/mod.rut", "pub mod b;\n");
    write(&dir, "src/m/a/b.rut", "struct Task { id: i64; }\n");
    names(&run("nested-vis", &dir), "in `m/a/b.rut`", "in `b.rut`");

    let root = scratch("nested-check");
    let dir = root.join("app");
    write(&dir, "rut.jsonc", r#"{"name": "app"}"#);
    write(
        &dir,
        "src/mod.rut",
        "mod m;\nentry fn main() -> i64 { return m::a::b::nope(); }\n",
    );
    write(&dir, "src/m/mod.rut", "pub mod a;\n");
    write(&dir, "src/m/a/mod.rut", "pub mod b;\n");
    write(&dir, "src/m/a/b.rut", "pub fn yes() -> i64 { return 1; }\n");
    names(
        &run("nested-check", &dir),
        "in module `m/a/b` (m/a/b.rut)",
        "(m/a/b/mod.rut)",
    );

    let root = scratch("nested-mount");
    let dir = root.join("app");
    write(&dir, "rut.jsonc", r#"{"name": "app"}"#);
    write(&dir, "src/mod.rut", "mod m;\nentry fn main() {}\n");
    write(&dir, "src/m/mod.rut", "pub mod a;\n");
    write(&dir, "src/m/a/mod.rut", "pub mod b;\n");
    write(
        &dir,
        "src/m/a/b.rut",
        "mod gone;\npub fn f() -> i64 { return 1; }\n",
    );
    names(
        &run("nested-mount", &dir),
        "`mod gone;` in m/a/b.rut",
        "in m/a/b/b.rut",
    );
}

#[test]
fn a_child_of_a_single_file_parent_is_named_by_its_path() {
    // `src/m/a.rut` (a single-file parent) declares `b`, mounted
    // beside its declaring file as `src/m/b.rut` — all three messages
    // name `m/b.rut`, never the mod-path-derived `m/a/b`.

    let root = scratch("child-vis");
    let dir = root.join("app");
    write(&dir, "rut.jsonc", r#"{"name": "app"}"#);
    write(
        &dir,
        "src/mod.rut",
        "use m::a::b::{ Task };\nmod m;\nentry fn main() {}\n",
    );
    write(&dir, "src/m/mod.rut", "pub mod a;\n");
    write(&dir, "src/m/a.rut", "pub mod b;\n");
    write(&dir, "src/m/b.rut", "struct Task { id: i64; }\n");
    names(&run("child-vis", &dir), "in `m/b.rut`", "in `b.rut`");

    let root = scratch("child-check");
    let dir = root.join("app");
    write(&dir, "rut.jsonc", r#"{"name": "app"}"#);
    write(
        &dir,
        "src/mod.rut",
        "mod m;\nentry fn main() -> i64 { return m::a::b::nope(); }\n",
    );
    write(&dir, "src/m/mod.rut", "pub mod a;\n");
    write(&dir, "src/m/a.rut", "pub mod b;\n");
    write(&dir, "src/m/b.rut", "pub fn yes() -> i64 { return 1; }\n");
    names(
        &run("child-check", &dir),
        "in module `m/a/b` (m/b.rut)",
        "(m/a/b/mod.rut)",
    );

    let root = scratch("child-mount");
    let dir = root.join("app");
    write(&dir, "rut.jsonc", r#"{"name": "app"}"#);
    write(&dir, "src/mod.rut", "mod m;\nentry fn main() {}\n");
    write(&dir, "src/m/mod.rut", "pub mod a;\n");
    write(&dir, "src/m/a.rut", "pub mod b;\n");
    write(
        &dir,
        "src/m/b.rut",
        "mod gone;\npub fn f() -> i64 { return 1; }\n",
    );
    names(
        &run("child-mount", &dir),
        "`mod gone;` in m/b.rut",
        "in m/a/b/b.rut",
    );
}

#[test]
fn the_dep_lane_names_a_single_file_child_by_its_file() {
    // `rut/calc`'s only child is `src/math.rut` (`pub mod math;`) — a
    // path dependency's walked-module hint names `math.rut`.
    let root = scratch("dep-lane");
    let dir = root.join("app");
    let calc = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/calc");
    write(
        &dir,
        "rut.jsonc",
        &format!(r#"{{"name": "app", "deps": {{"calc": {{"path": "{calc}"}}}}}}"#),
    );
    write(
        &dir,
        "src/mod.rut",
        "use calc::math::{ nope };\nentry fn main() {}\n",
    );
    names(
        &run("dep-lane", &dir),
        "`calc`'s module `math` (math.rut)",
        "(math/mod.rut)",
    );
}

#[test]
fn a_pack_failure_message_carries_its_prefix_once() {
    // F74: the PackError family self-identifies — the unit-diag
    // refusal embeds its own `pack: ` (rut-driver's pack lane) — so
    // the CLI's printer states nothing: exactly one prefix, never the
    // `pack: pack: ` double.
    let root = scratch("pack-prefix");
    let dir = root.join("poison");
    write(&dir, "rut.jsonc", r#"{"name": "poison"}"#);
    write(
        &dir,
        "src/mod.rut",
        "entry fn main() {\n    let n: Bogus = 10;\n}\n",
    );
    let stderr = pack("pack-prefix", &dir);
    assert!(
        stderr.contains("pack: "),
        "the refusal carries the lane prefix: {stderr}"
    );
    assert!(
        !stderr.contains("pack: pack: "),
        "one prefix per message — the printer states nothing: {stderr}"
    );
}
