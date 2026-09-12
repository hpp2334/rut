//! `host primitive` surface decls (RFC 0029 §2): the native member
//! surface of a primitive type — `host primitive string { fn len(self) -> i32; }`.
//! Parses in declaration mode only, names a real primitive, terminates
//! on malformed input.

use rut_ast::ast::*;
use rut_parser::{parse, Mode};

const STR_SURFACE: &str = "\
// the string natives, declared where the host binds them
pub host primitive string {
    fn len(self) -> i32;
    fn contains(self, needle: string) -> bool;
}
";

#[test]
fn host_primitive_parses() {
    let (ast, diags) = parse(STR_SURFACE, Mode::Decl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    assert_eq!(items.len(), 1);
    let ItemKind::SurfacePrimitive { name, members, .. } = ast.item(items[0]) else {
        panic!("expected a SurfacePrimitive item, got {:?}", ast.item(items[0]));
    };
    assert_eq!(ast.name(*name), "string");
    assert_eq!(members.len(), 2);
    let m = ast.method_decl(members[0]);
    assert_eq!(ast.name(m.name), "len");
    assert!(m.body.is_none(), "surface members are bodiless");
}

#[test]
fn host_primitive_names_a_primitive() {
    let (_, diags) = parse("pub host primitive Widget { fn m(self) -> unit; }", Mode::Decl);
    assert!(
        diags.iter().any(|d| d.msg.contains("`host primitive` names a primitive")),
        "non-primitive target must be diagnosed: {diags:?}"
    );
}

#[test]
fn host_primitive_is_decl_only() {
    // RFC 0029 §2: `host`/`extern` belong to `.d.rut`
    let (_, diags) = parse("host primitive string { fn len(self) -> i32; }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("RFC 0029")),
        "impl mode must reject the surface keyword: {diags:?}"
    );
}

#[test]
fn host_primitive_terminates_on_malformed() {
    for src in ["host primitive string {", "host primitive {", "host primitive"] {
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
        let (_, diags) = parse(&src, Mode::Decl);
        if !diags.is_empty() {
            failures.push(format!("{}: first = {}", f.display(), diags[0].msg));
        }
    }
    assert!(failures.is_empty(), "std surface regressions:\n{}", failures.join("\n"));
}
