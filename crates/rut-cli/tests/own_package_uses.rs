//! The todolist shape through the binary: `use imp::{ .. };` beside
//! `mod imp;` in the root — the own-package import lane — runs end to
//! end and packs (the build lane). Both child spellings ride: the
//! directory form (`imp/mod.rut`) and the single-file form (`imp.rut`);
//! `pub` members only (import is no visibility bypass).

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-cli-own-uses-{tag}-{}", std::process::id()));
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

/// The todolist shape: a root module importing its own `imp` child's
/// pub class + struct through `use imp::{ .. };`.
fn todolist(tag: &str) -> PathBuf {
    let root = scratch(tag);
    let dir = root.join("todolist");
    write(
        &dir,
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 12, "name": "todolist"}"#,
    );
    write(
        &dir,
        "mod.rut",
        r#"use imp::{ TodoList, Task };

mod imp;

fn boot() -> TodoList {
    let mut list = TodoList { count: 0, last: Task { id: 0, title: "none", done: false } };
    list.add("first");
    list.add("second");
    return list;
}

entry fn main() {
    let list = boot();
    let t: Task = list.last;
    let _ = t.id;
}
"#,
    );
    write(
        &dir,
        "imp/mod.rut",
        r#"pub struct Task {
    id: i64;
    title: str;
    done: bool;
}

pub class TodoList {
    pub count: i64;
    pub last: Task;
}

impl TodoList {
    pub fn add(mut self, title: str) {
        self.count += 1;
        self.last = Task { id: self.count, title: title, done: false };
    }
}
"#,
    );
    root
}

#[test]
fn the_todolist_shape_runs_and_packs() {
    let root = todolist("positive");
    let dir = root.join("todolist");

    let out = Command::new(rut())
        .args(["run", dir.to_str().unwrap()])
        .output()
        .expect("run rut run <dir>");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out = Command::new(rut())
        .args(["pack", dir.to_str().unwrap()])
        .output()
        .expect("run rut pack");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bundle = root.join("todolist.rutbundle");
    assert!(bundle.is_file(), "the bundle lands beside the dir");

    // and the packed bundle runs too
    let out = Command::new(rut())
        .args(["run", bundle.to_str().unwrap()])
        .output()
        .expect("run rut run <bundle>");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_private_own_member_refuses_at_the_use_line() {
    // import is no visibility bypass: a bare member of imp is refused
    // where the import is written
    let root = todolist("private");
    let dir = root.join("todolist");
    write(
        &dir,
        "mod.rut",
        "use imp::{ Task };\n\
         mod imp;\n\
         entry fn main() {}\n",
    );
    write(&dir, "imp/mod.rut", "struct Task { id: i64; }\n");
    let out = Command::new(rut())
        .args(["run", dir.to_str().unwrap()])
        .output()
        .expect("run rut run <dir>");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(
            "`Task` is not visible from `mod.rut` — mark it `pub` (or `pub(pkg)`) in `imp/mod.rut`"
        ),
        "{stderr}"
    );
    assert!(stderr.contains("--> todolist:1:1"), "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

/// The same refusal, single-file flavor: the declaring file is spelled
/// `imp.rut` — the physical name the loader mounted, never the
/// directory-form guess.
#[test]
fn the_single_file_private_refusal_spells_the_file() {
    let root = scratch("file-form-private");
    let dir = root.join("todolist");
    write(
        &dir,
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 12, "name": "todolist"}"#,
    );
    write(
        &dir,
        "mod.rut",
        "use imp::{ Task };\n\
         mod imp;\n\
         entry fn main() {}\n",
    );
    write(&dir, "imp.rut", "struct Task { id: i64; }\n");
    let out = Command::new(rut())
        .args(["run", dir.to_str().unwrap()])
        .output()
        .expect("run rut run <dir>");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(
            "`Task` is not visible from `mod.rut` — mark it `pub` (or `pub(pkg)`) in `imp.rut`"
        ),
        "{stderr}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// The same shape, single-file flavor: `imp.rut` beside the declaring
/// file mounts at the logical mod path `imp` — the loader's mounting
/// law's second spelling, end to end through the binary.
#[test]
fn the_single_file_module_shape_runs() {
    let root = scratch("file-form");
    let dir = root.join("todolist");
    write(
        &dir,
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 12, "name": "todolist"}"#,
    );
    write(
        &dir,
        "mod.rut",
        r#"use imp::{ Task };

mod imp;

entry fn main() {
    let t = Task { id: 1, title: "first", done: false };
    let _ = t.id;
}
"#,
    );
    write(
        &dir,
        "imp.rut",
        r#"pub struct Task {
    id: i64;
    title: str;
    done: bool;
}
"#,
    );
    let out = Command::new(rut())
        .args(["run", dir.to_str().unwrap()])
        .output()
        .expect("run rut run <dir>");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn both_child_spellings_are_the_loud_ambiguity() {
    // `imp/mod.rut` AND `imp.rut` — the loader refuses, naming both
    let root = scratch("ambiguous");
    let dir = root.join("todolist");
    write(&dir, "rut.jsonc", r#"{"name": "todolist"}"#);
    write(&dir, "mod.rut", "mod imp;\nentry fn main() {}\n");
    write(&dir, "imp/mod.rut", "pub fn one() -> i64 { return 1; }\n");
    write(&dir, "imp.rut", "pub fn two() -> i64 { return 2; }\n");
    let out = Command::new(rut())
        .args(["run", dir.to_str().unwrap()])
        .output()
        .expect("run rut run <dir>");
    assert!(!out.status.success(), "the ambiguous mount refuses");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("both `imp/mod.rut` and `imp.rut` exist"),
        "{stderr}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
