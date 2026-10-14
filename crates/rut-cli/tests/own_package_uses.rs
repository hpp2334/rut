//! The todolist shape through the binary: `use imp::{ .. };` beside
//! `mod imp;` in the root — the own-package import lane — runs end to
//! end and packs (the build lane). Directory-form modules only
//! (`imp/mod.rut`); `pub` members (import is no visibility bypass).

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
        r#"{"format": "rutbundle", "format_version": 10, "name": "todolist"}"#,
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
    count: i64;
    last: Task;
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
    write(
        &dir,
        "imp/mod.rut",
        "struct Task { id: i64; }\n",
    );
    let out = Command::new(rut())
        .args(["run", dir.to_str().unwrap()])
        .output()
        .expect("run rut run <dir>");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("private to module `imp`"), "{stderr}");
    assert!(stderr.contains("--> todolist:1:1"), "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}
