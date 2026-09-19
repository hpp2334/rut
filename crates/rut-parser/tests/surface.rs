//! `host`/`builtin` surface decls (RFC 0029 §2): `.d.rut` declares the
//! host functions/dataclasses the runtime binds and the engine builtin
//! contracts — `host fn string_len(s: str) -> i32;`, `builtin primitive
//! opaque { .. }`. Parses in declaration mode only; the removed forms
//! (`host primitive`, `host class`, `extern`, `pub builtin`) are
//! rejected.

use rut_ast::ast::*;
use rut_parser::{parse, Mode};

const FN_SURFACE: &str = "\
// the string natives, declared where the host binds them
pub host fn string_len(s: str) -> i32;
builtin fn own<T>(x: T) -> T;
";

#[test]
fn host_fn_parses() {
    let (ast, diags) = parse(FN_SURFACE, Mode::Decl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    assert_eq!(items.len(), 2);
    let ItemKind::SurfaceFn { name, params, linkage, .. } = ast.item(items[0]) else {
        panic!("expected a SurfaceFn item, got {:?}", ast.item(items[0]));
    };
    assert_eq!(*linkage, Linkage::Host);
    assert_eq!(ast.name(*name), "string_len");
    assert_eq!(params.len(), 1);
    let ItemKind::SurfaceFn { linkage, generics, .. } = ast.item(items[1]) else {
        panic!("expected a SurfaceFn item, got {:?}", ast.item(items[1]));
    };
    assert_eq!(*linkage, Linkage::Builtin);
    assert_eq!(generics.len(), 1, "builtin fn keeps its generics");
}

const BUILTIN_TRAIT: &str = "\
builtin trait Index<T> {
    fn len(self) -> i32;
    fn get(self, i: i32) -> T;
}
";

#[test]
fn builtin_trait_parses() {
    let (ast, diags) = parse(BUILTIN_TRAIT, Mode::Decl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    assert_eq!(items.len(), 1);
    let ItemKind::BuiltinTrait { name, generics, methods, .. } = ast.item(items[0]) else {
        panic!("expected a BuiltinTrait item, got {:?}", ast.item(items[0]));
    };
    assert_eq!(ast.name(*name), "Index");
    assert_eq!(generics.len(), 1);
    assert_eq!(methods.len(), 2);
}

const BUILTIN_SURFACE: &str = "\
builtin class Option<T> {
    fn some(v: T) -> Self;
    fn is_some(self) -> bool;
}
";

#[test]
fn builtin_ty_parses() {
    let (ast, diags) = parse(BUILTIN_SURFACE, Mode::Decl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    assert_eq!(items.len(), 1);
    let ItemKind::BuiltinTy { name, generics, members, .. } = ast.item(items[0]) else {
        panic!("expected a BuiltinTy item, got {:?}", ast.item(items[0]));
    };
    assert_eq!(ast.name(*name), "Option");
    assert_eq!(generics.len(), 1);
    assert_eq!(members.len(), 2);
}

#[test]
fn builtin_requires_a_kind_word() {
    // builtin decls spell their kind — `builtin primitive` / `builtin
    // class` / `builtin trait` (RFC 0025); a bare `builtin Name { .. }`
    // is diagnosed
    let (_, diags) = parse("builtin Option<T> { fn some(v: T) -> Self; }", Mode::Decl);
    assert!(
        diags.iter().any(|d| d.msg.contains("spells its kind")),
        "a bare `builtin` decl must be diagnosed: {diags:?}"
    );
}

#[test]
fn zero_member_builtin_bodies_parse() {
    // an empty member contract is legal — e.g. a marker trait or a type
    // whose members are entirely compiler-lowered and invisible
    let (ast, diags) = parse("builtin class Mark<T> { }", Mode::Decl);
    assert!(diags.is_empty(), "zero-member builtin class must parse clean: {diags:?}");
    let items = ast.module_items(ast.root);
    assert_eq!(items.len(), 1);
    let ItemKind::BuiltinTy { members, .. } = ast.item(items[0]) else {
        panic!("expected a BuiltinTy item, got {:?}", ast.item(items[0]));
    };
    assert!(members.is_empty());

    let (ast, diags) = parse("builtin trait Mark { }", Mode::Decl);
    assert!(diags.is_empty(), "zero-member builtin trait must parse clean: {diags:?}");
    let items = ast.module_items(ast.root);
    assert_eq!(items.len(), 1);
    let ItemKind::BuiltinTrait { methods, .. } = ast.item(items[0]) else {
        panic!("expected a BuiltinTrait item, got {:?}", ast.item(items[0]));
    };
    assert!(methods.is_empty());
}

const HOST_DATACLASS: &str = "\
pub host struct Location {
    file: str,
    line: i32,
    col: i32,
}
";

#[test]
fn host_dataclass_parses() {
    let (ast, diags) = parse(HOST_DATACLASS, Mode::Decl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    assert_eq!(items.len(), 1);
    let ItemKind::SurfaceDataclass { name, fields, .. } = ast.item(items[0]) else {
        panic!("expected a SurfaceDataclass item, got {:?}", ast.item(items[0]));
    };
    assert_eq!(ast.name(*name), "Location");
    assert_eq!(fields.len(), 3);
}

#[test]
fn host_dataclass_members_are_fields_only() {
    // methods and initializers are rejected: the shape is the whole
    // surface, the host constructs the record (RFC 0025)
    let (_, diags) = parse("host struct L { fn f(self) -> i32; }", Mode::Decl);
    assert!(
        diags.iter().any(|d| d.msg.contains("fields only")),
        "a method must be diagnosed: {diags:?}"
    );
    let (_, diags) = parse("host struct L { x: i32 = 0, }", Mode::Decl);
    assert!(
        diags.iter().any(|d| d.msg.contains("no initializers")),
        "an initializer must be diagnosed: {diags:?}"
    );
}

#[test]
fn host_fn_generics_are_rejected() {
    // RFC 0023 §1: host fn signatures are concrete — a generic parameter
    // has no shape the boundary could check
    let (_, diags) = parse("pub host fn downcast<T>(o: opaque) -> Option<T>;", Mode::Decl);
    assert!(
        diags.iter().any(|d| d.msg.contains("concrete")),
        "generic host fn must be diagnosed: {diags:?}"
    );
}

#[test]
fn builtin_impl_decl() {
    // RFC 0032 §1.1 R2: `builtin impl <prim> { .. }` — the integer
    // primitives' numeric methods, bodiless `self` receivers, tuple
    // returns allowed (`checked_*`)
    let src = "builtin impl i32 {\n\
               \x20   fn wrapping_add(self, y: i32) -> i32;\n\
               \x20   fn wrapping_shl(self, n: i32) -> i32;\n\
               \x20   fn checked_add(self, y: i32) -> (i32, bool);\n\
               }";
    let (ast, diags) = parse(src, Mode::Decl);
    assert!(diags.is_empty(), "builtin impl must parse cleanly: {diags:?}");
    let items = ast.module_items(ast.root).to_vec();
    let ItemKind::BuiltinImpl { prim, methods, .. } = ast.item(items[0]) else {
        panic!("expected a BuiltinImpl item, got {:?}", ast.item(items[0]));
    };
    assert_eq!(ast.name(*prim), "i32");
    assert_eq!(methods.len(), 3);
    // it is a declaration form: an implementation file rejects it
    let (_, diags) = parse("builtin impl i32 { fn abs(self) -> i32; }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("belong in a `.d.rut`")),
        "builtin impl in a .rut must be diagnosed: {diags:?}"
    );
}

const BUILTIN_PRIMITIVE: &str = "\
builtin primitive opaque {
    fn new<T>(v: T) -> Self;
    fn downcast<T>(o: Self) -> (T, bool);
}
";

#[test]
fn builtin_primitive_parses() {
    // builtin-surface phase 1: the boot primitives' surface statement —
    // `builtin primitive <name> { .. }`, never a class. BOTH statics on
    // the erasure primitive live on it (RFC 0014).
    let (ast, diags) = parse(BUILTIN_PRIMITIVE, Mode::Decl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    assert_eq!(items.len(), 1);
    let ItemKind::BuiltinPrimitive { name, members } = ast.item(items[0]) else {
        panic!("expected a BuiltinPrimitive item, got {:?}", ast.item(items[0]));
    };
    assert_eq!(ast.name(*name), "opaque");
    assert_eq!(members.len(), 2, "`new` and `downcast` are both on the primitive");
}

#[test]
fn builtin_primitive_takes_no_generics() {
    let (_, diags) = parse("builtin primitive str<T> { fn len(self) -> i32; }", Mode::Decl);
    assert!(
        diags.iter().any(|d| d.msg.contains("no generic arguments")),
        "a primitive takes no generic arguments: {diags:?}"
    );
}

#[test]
fn pub_builtin_is_removed() {
    // builtin-surface phase 1: builtin names are AMBIENT — the old
    // `pub builtin` spelling diagnoses (no deprecation tolerance)
    let (_, diags) = parse("pub builtin fn own<T>(x: T) -> T;", Mode::Decl);
    assert!(
        diags.iter().any(|d| d.msg.contains("`pub builtin` is removed")),
        "`pub builtin` must be diagnosed: {diags:?}"
    );
    let (_, diags) = parse("pub builtin class opaque { fn new<T>(v: T) -> Self; }", Mode::Decl);
    assert!(
        diags.iter().any(|d| d.msg.contains("`pub builtin` is removed")),
        "`pub builtin class` must be diagnosed: {diags:?}"
    );
    // the bare spelling stays clean — every builtin decl drops `pub`
    let (ast, diags) = parse("builtin fn assert(cond: bool, msg: str) -> nil;", Mode::Decl);
    assert!(diags.is_empty(), "no-pub builtin fn must parse clean: {diags:?}");
    let items = ast.module_items(ast.root);
    assert_eq!(items.len(), 1);
}

#[test]
fn removed_forms_are_rejected() {
    // `host primitive` — the removed member-surface grammar
    let (_, diags) = parse("pub host primitive string { fn len(self) -> i32; }", Mode::Decl);
    assert!(
        diags.iter().any(|d| d.msg.contains("expected `fn` or `struct` after `host`")),
        "`host primitive` must be diagnosed: {diags:?}"
    );
    // `host class` — removed: wrap native state in a rut class over opaque
    let (_, diags) = parse("pub host class Canvas { fn flush(self) -> nil; }", Mode::Decl);
    assert!(
        diags.iter().any(|d| d.msg.contains("`host class` is removed")),
        "`host class` must be diagnosed: {diags:?}"
    );
    // `extern` — removed linkage
    let (_, diags) = parse("pub extern fn string_encode(s: str) -> bytes;", Mode::Decl);
    assert!(
        diags.iter().any(|d| d.msg.contains("`extern` linkage is removed")),
        "`extern` must be diagnosed: {diags:?}"
    );
}

#[test]
fn host_fn_is_decl_only() {
    // RFC 0029 §2: `host`/`builtin` belong to `.d.rut`
    let (_, diags) = parse("host fn string_len(s: str) -> i32;", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("RFC 0029")),
        "impl mode must reject the surface keyword: {diags:?}"
    );
    let (_, diags) = parse("builtin fn own<T>(x: T) -> T;", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("RFC 0029")),
        "impl mode must reject the surface keyword: {diags:?}"
    );
}

#[test]
fn host_fn_terminates_on_malformed() {
    for src in [
        "host fn x(",
        "host fn",
        "host class {",
        "host struct L {",
        "host struct L { x",
        "builtin Option<T> {",
        "builtin fn f(",
        "builtin trait I {",
        "extern fn f();",
    ] {
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
    assert!(files.iter().any(|f| f.to_string_lossy().ends_with("core.d.rut")));
    assert!(failures.is_empty(), "std surface regressions:\n{}", failures.join("\n"));
}
