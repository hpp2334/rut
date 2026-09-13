//! `host`/`extern` surface decls (RFC 0029 §2): `.d.rut` declares the host
//! functions and classes the runtime binds — `pub host fn string_len(s:
//! string) -> i32;`. Parses in declaration mode only; `host primitive` (the
//! removed member-surface form) is rejected.

use rut_ast::ast::*;
use rut_parser::{parse, Mode};

const FN_SURFACE: &str = "\
// the string natives, declared where the host binds them
pub host fn string_len(s: string) -> i32;
pub extern fn string_encode(s: string) -> bytes;
";

#[test]
fn host_fn_parses() {
    let (ast, diags) = parse(FN_SURFACE, Mode::Decl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    assert_eq!(items.len(), 2);
    let ItemKind::SurfaceFn { name, params, .. } = ast.item(items[0]) else {
        panic!("expected a SurfaceFn item, got {:?}", ast.item(items[0]));
    };
    assert_eq!(ast.name(*name), "string_len");
    assert_eq!(params.len(), 1);
}

#[test]
fn host_primitive_is_rejected() {
    // the `host primitive <ty> { .. }` member-surface grammar was removed:
    // primitive operations are now free `host fn`s
    let (_, diags) = parse("pub host primitive string { fn len(self) -> i32; }", Mode::Decl);
    assert!(
        diags.iter().any(|d| d.msg.contains("expected `fn` or `interface`")),
        "`host primitive` must be diagnosed: {diags:?}"
    );
}

#[test]
fn host_fn_is_decl_only() {
    // RFC 0029 §2: `host`/`extern` belong to `.d.rut`
    let (_, diags) = parse("host fn string_len(s: string) -> i32;", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("RFC 0029")),
        "impl mode must reject the surface keyword: {diags:?}"
    );
}

#[test]
fn host_fn_terminates_on_malformed() {
    for src in ["host fn x(", "host fn", "host class {"] {
        let (_, diags) = parse(src, Mode::Decl);
        let _ = diags.len(); // termination is the contract
    }
}

/// `std/**/*.d.rut` is surface-conformance: the toolchain's declaration
/// files parse clean in Decl mode, like the corpus does for examples.
#[test]
fn std_surface_parses_clean() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rut");
    let mut files = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().map(|x| x == "rut").unwrap_or(false) {
                files.push(p);
            }
        }
    }
    files.sort();
    assert!(files.len() >= 3, "expected the std surface, found {}", files.len());
    let mut failures = Vec::new();
    for f in &files {
        let src = std::fs::read_to_string(f).unwrap();
        let mode = if f.to_string_lossy().ends_with(".d.rut") { Mode::Decl } else { Mode::Impl };
        let (_, diags) = parse(&src, mode);
        if !diags.is_empty() {
            failures.push(format!("{}: first = {}", f.display(), diags[0].msg));
        }
    }
    assert!(failures.is_empty(), "std surface regressions:\n{}", failures.join("\n"));
}
