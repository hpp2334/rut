//! The dump lane through the binary: `rut dump` reads one file and
//! prints its AST + IR. A `.d.rut` surface dumps as the declaration
//! unit it is (the graph mounts Decl, so `builtin`/`host` rows are not
//! refused as impl-file keywords), an impl `.rut` still dumps, and a
//! compile diag lands on its line/column — not the bare `--> main`
//! label the renderer emitted when the text was keyed under `""`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-cli-dump-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, text).unwrap();
    p
}

fn dump(path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rut"))
        .arg("dump")
        .arg(path)
        .output()
        .expect("spawn rut")
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn a_decl_surface_dumps_its_ast_and_ir() {
    // the shipped core surface: 19 `builtin`/`host` rows that impl-mode
    // mounting used to refuse outright
    let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut/core/src/core.d.rut");
    let out = dump(&core);
    let rendered = text(&out);
    assert!(
        out.status.success(),
        "decl surface dump must succeed: {rendered}"
    );
    assert!(rendered.contains("== AST =="), "{rendered}");
    assert!(rendered.contains("SurfaceFn panic"), "{rendered}");
    assert!(rendered.contains("== IR =="), "{rendered}");
    assert!(
        !rendered.contains("declaration keyword in an implementation file"),
        "{rendered}"
    );
}

#[test]
fn an_impl_module_still_dumps_ast_and_ir() {
    let dir = scratch("impl");
    let file = write(
        &dir,
        "good.rut",
        "entry fn main() -> i32 {\n    return 0;\n}\n",
    );
    let out = dump(&file);
    let rendered = text(&out);
    assert!(out.status.success(), "impl dump must succeed: {rendered}");
    assert!(rendered.contains("== AST =="), "{rendered}");
    assert!(rendered.contains("Fn main"), "{rendered}");
    assert!(rendered.contains("== IR =="), "{rendered}");
    assert!(rendered.contains("fn #0 main"), "{rendered}");
}

#[test]
fn a_compile_diag_renders_its_line_and_column() {
    let dir = scratch("diag");
    let file = write(
        &dir,
        "bad.rut",
        "entry fn main() {\n    let x = \"s\" + 1;\n}\n",
    );
    let out = dump(&file);
    let rendered = text(&out);
    assert!(
        !out.status.success(),
        "the bad module must refuse: {rendered}"
    );
    // the attributed label carries the position, and the caret line
    // proves the snippet rendered from the source map
    assert!(rendered.contains("--> main:2:"), "{rendered}");
    assert!(
        !rendered.contains("--> main\n"),
        "positionless arrow: {rendered}"
    );
    assert!(rendered.contains("let x = \"s\" + 1;"), "{rendered}");
}
