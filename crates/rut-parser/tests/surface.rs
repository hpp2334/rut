//! `host`/`builtin` surface decls: `.d.rut` declares the
//! host functions/structs the runtime binds and the engine builtin
//! contracts — `host fn string_len(s: str) -> i32;`, `prelude builtin
//! primitive opaque { .. }`. Parses in declaration mode only; the
//! removed forms (`host primitive`, `host class`, `extern`) are
//! rejected, and `builtin` has TWO strict spellings — `prelude builtin`
//! (ambient) and `pub builtin` (import-gated); the bare keyword
//! diagnoses.

use rut_ast::ast::*;
use rut_parser::{parse, Mode};

const FN_SURFACE: &str = "\
// the string natives, declared where the host binds them
pub host fn string_len(s: str) -> i32;
prelude builtin fn own<T>(x: T) -> T;
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
    assert!(
        matches!(*linkage, Linkage::Builtin { ambient: true }),
        "`prelude builtin fn` is the ambient builtin linkage: {linkage:?}"
    );
    assert_eq!(generics.len(), 1, "builtin fn keeps its generics");
}

#[test]
fn mod_decls_parse_in_decl_files() {
    // the host surface tree's edges: `pub? mod NAME;` parses in a
    // `.d.rut` (Mode::Decl) exactly as in a body file — the vis rides
    // the declaring edge (`pub mod net;` is the cross-package shape
    // the tree law reads), and the rows around it keep parsing
    let src = "\
pub mod net;
mod helper;
pub host fn legacy_ping() -> bool;
";
    let (ast, diags) = parse(src, Mode::Decl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    assert_eq!(items.len(), 3, "one item per line: {items:?}");
    let edge = |i: usize| match ast.item(items[i]) {
        ItemKind::ModDecl { vis, name } => (*vis, ast.name(*name).to_string()),
        k => panic!("expected a mod decl at {i}, got {k:?}"),
    };
    assert_eq!(edge(0), (Vis::Pub, "net".to_string()));
    assert_eq!(edge(1), (Vis::Self_, "helper".to_string()));
    let ItemKind::SurfaceFn { vis, linkage, name, .. } = ast.item(items[2]) else {
        panic!("expected a SurfaceFn after the edges");
    };
    assert_eq!(*vis, Vis::Pub, "`pub host fn` keeps its pub beside mod edges");
    assert_eq!(*linkage, Linkage::Host);
    assert_eq!(ast.name(*name), "legacy_ping");
}


#[test]
fn builtin_trait_spelling_is_removed() {
    let src = "prelude builtin trait Index<T> {\n    fn len(self) -> i32;\n}\n";
    let (ast, diags) = parse(src, Mode::Decl);
    assert!(
        diags.iter().any(|d| d.msg.contains("`builtin trait` is removed")),
        "the removed spelling must be diagnosed: {diags:?}"
    );
    // recovery: the members still parse (as a class)
    let items = ast.module_items(ast.root);
    assert_eq!(items.len(), 1);
    let ItemKind::BuiltinTy { name, generics, members, .. } = ast.item(items[0]) else {
        panic!("expected the recovered BuiltinTy item, got {:?}", ast.item(items[0]));
    };
    assert_eq!(ast.name(*name), "Index");
    assert_eq!(generics.len(), 1);
    assert_eq!(members.len(), 1);
}

const BUILTIN_SURFACE: &str = "\
prelude builtin class Option<T> {
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
    // builtin decls spell their kind — `prelude builtin primitive` /
    // `prelude builtin class` / `prelude builtin trait`; a
    // bare `builtin Name { .. }` is diagnosed
    let (_, diags) = parse("prelude builtin Option<T> { fn some(v: T) -> Self; }", Mode::Decl);
    assert!(
        diags.iter().any(|d| d.msg.contains("spells its kind")),
        "a kind-less `builtin` decl must be diagnosed: {diags:?}"
    );
}

#[test]
fn zero_member_builtin_bodies_parse() {
    // an empty member contract is legal — e.g. a marker trait or a type
    // whose members are entirely compiler-lowered and invisible
    let (ast, diags) = parse("prelude builtin class Mark<T> { }", Mode::Decl);
    assert!(diags.is_empty(), "zero-member builtin class must parse clean: {diags:?}");
    let items = ast.module_items(ast.root);
    assert_eq!(items.len(), 1);
    let ItemKind::BuiltinTy { members, .. } = ast.item(items[0]) else {
        panic!("expected a BuiltinTy item, got {:?}", ast.item(items[0]));
    };
    assert!(members.is_empty());

    let (ast, diags) = parse("prelude builtin trait Mark { }", Mode::Decl);
    assert!(
        diags.iter().any(|d| d.msg.contains("`builtin trait` is removed")),
        "the removed spelling must be diagnosed: {diags:?}"
    );
    let items = ast.module_items(ast.root);
    assert_eq!(items.len(), 1);
    let ItemKind::BuiltinTy { members, .. } = ast.item(items[0]) else {
        panic!("expected the recovered BuiltinTy item, got {:?}", ast.item(items[0]));
    };
    assert!(members.is_empty());
}

const HOST_STRUCT: &str = "\
pub host struct Location {
    file: str,
    line: i32,
    col: i32,
}
";

#[test]
fn host_struct_parses() {
    let (ast, diags) = parse(HOST_STRUCT, Mode::Decl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    assert_eq!(items.len(), 1);
    let ItemKind::SurfaceStruct { name, fields, .. } = ast.item(items[0]) else {
        panic!("expected a SurfaceStruct item, got {:?}", ast.item(items[0]));
    };
    assert_eq!(ast.name(*name), "Location");
    assert_eq!(fields.len(), 3);
}

#[test]
fn host_struct_members_are_fields_only() {
    // methods and initializers are rejected: the shape is the whole
    // surface, the host constructs the record
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
    // host fn signatures are concrete — a generic parameter
    // has no shape the boundary could check
    let (_, diags) = parse("pub host fn downcast<T>(o: opaque) -> Option<T>;", Mode::Decl);
    assert!(
        diags.iter().any(|d| d.msg.contains("concrete")),
        "generic host fn must be diagnosed: {diags:?}"
    );
}

// ---- the host future lane (phase 4): `pub host async fn` ----

const ASYNC_HOST_FN: &str = "\
pub host async fn fetch(u: str) -> bytes;
pub host fn tick() -> i32;
";

#[test]
fn host_async_fn_parses() {
    let (ast, diags) = parse(ASYNC_HOST_FN, Mode::Decl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    assert_eq!(items.len(), 2);
    let ItemKind::SurfaceFn { linkage, is_async, name, params, ret, .. } = ast.item(items[0])
    else {
        panic!("expected a SurfaceFn item, got {:?}", ast.item(items[0]));
    };
    assert_eq!(*linkage, Linkage::Host);
    assert!(*is_async, "the async marker must land on the decl");
    assert_eq!(ast.name(*name), "fetch");
    assert_eq!(params.len(), 1);
    assert!(ret.is_some());
    // the sync lane keeps its flag down
    let ItemKind::SurfaceFn { is_async, .. } = ast.item(items[1]) else {
        panic!("expected a SurfaceFn item, got {:?}", ast.item(items[1]));
    };
    assert!(!*is_async);
}

#[test]
fn host_async_fn_generics_are_rejected_with_the_same_words() {
    // the async lane crosses concretely like its sync twin — the
    // generic refusal is the same diagnostic
    let (_, diags) = parse("pub host async fn fetch<T>(u: T) -> T;", Mode::Decl);
    assert!(
        diags.iter().any(|d| d.msg.contains("concrete") && d.msg.contains("generic parameters")),
        "generic async host fn must be diagnosed: {diags:?}"
    );
}

#[test]
fn host_async_fn_is_decl_only() {
    // Mode-Impl misuse lands in the same diagnostic family as every
    // other decl keyword
    let (_, diags) = parse("pub host async fn fetch(u: str) -> bytes;", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("belong in a `.d.rut`")),
        "impl mode must reject `host async fn`: {diags:?}"
    );
}

#[test]
fn host_async_fn_terminates_on_malformed() {
    for src in [
        "pub host async",
        "pub host async fetch(u: str) -> bytes;",
        "pub host async fn",
        "pub host async fn f(",
    ] {
        let (_, diags) = parse(src, Mode::Decl);
        let _ = diags.len(); // termination is the contract
    }
}


#[test]
fn builtin_impl_decl() {
    // `builtin impl <prim> { .. }` — the integer
    // primitives' numeric methods, bodiless `self` receivers, tuple
    // returns allowed (`checked_*`)
    let src = "prelude builtin impl i32 {\n\
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
    let (_, diags) = parse("prelude builtin impl i32 { fn abs(self) -> i32; }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("belong in a `.d.rut`")),
        "builtin impl in a .rut must be diagnosed: {diags:?}"
    );
}

const BUILTIN_PRIMITIVE: &str = "\
prelude builtin primitive opaque {
    fn new<T>(v: T) -> Self;
    fn downcast<T>(o: Self) -> ?T;
}
";

#[test]
fn builtin_primitive_parses() {
    // builtin-surface phase 1: the boot primitives' surface statement —
    // `builtin primitive <name> { .. }`, never a class. BOTH statics on
    // the erasure primitive live on it.
    let (ast, diags) = parse(BUILTIN_PRIMITIVE, Mode::Decl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    assert_eq!(items.len(), 1);
    let ItemKind::BuiltinPrimitive { name, members, .. } = ast.item(items[0]) else {
        panic!("expected a BuiltinPrimitive item, got {:?}", ast.item(items[0]));
    };
    assert_eq!(ast.name(*name), "opaque");
    assert_eq!(members.len(), 2, "`new` and `downcast` are both on the primitive");
}

#[test]
fn builtin_primitive_takes_no_generics() {
    let (_, diags) = parse("prelude builtin primitive str<T> { fn len(self) -> i32; }", Mode::Decl);
    assert!(
        diags.iter().any(|d| d.msg.contains("no generic arguments")),
        "a primitive takes no generic arguments: {diags:?}"
    );
}

#[test]
fn the_two_builtin_spellings_parse_clean() {
    // TWO strict spellings: `prelude builtin` binds ambient, `pub
    // builtin` is the import-gated form. Both parse cleanly and both
    // keep the full builtin grammar (fn / class / trait / impl /
    // primitive).
    let (ast, diags) = parse("prelude builtin fn own<T>(x: T) -> T;", Mode::Decl);
    assert!(diags.is_empty(), "`prelude builtin fn` must parse clean: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::SurfaceFn { linkage, .. } = ast.item(items[0]) else {
        panic!("expected a SurfaceFn item, got {:?}", ast.item(items[0]));
    };
    assert!(
        matches!(*linkage, Linkage::Builtin { ambient: true }),
        "`prelude builtin` is the ambient spelling: {linkage:?}"
    );

    let (ast, diags) = parse("pub builtin fn own<T>(x: T) -> T;", Mode::Decl);
    assert!(diags.is_empty(), "`pub builtin fn` must parse clean: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::SurfaceFn { linkage, .. } = ast.item(items[0]) else {
        panic!("expected a SurfaceFn item, got {:?}", ast.item(items[0]));
    };
    assert!(
        matches!(*linkage, Linkage::Builtin { ambient: false }),
        "`pub builtin` is the import-gated spelling: {linkage:?}"
    );

    // the kind words compose with both spellings (the `builtin trait`
    // row kind is GONE — v20)
    for src in [
        "pub builtin class opaque { fn new<T>(v: T) -> Self; }",
        "pub builtin impl i32 { fn abs(self) -> i32; }",
        "prelude builtin primitive opaque { fn downcast<T>(o: Self) -> ?T; }",
    ] {
        let (_, diags) = parse(src, Mode::Decl);
        assert!(diags.is_empty(), "`{src}` must parse clean: {diags:?}");
    }
    let (_, diags) = parse("pub builtin trait Mark { }", Mode::Decl);
    assert!(
        diags.iter().any(|d| d.msg.contains("`builtin trait` is removed")),
        "the removed spelling must diagnose: {diags:?}"
    );
}

#[test]
fn bare_builtin_diagnoses_the_two_spellings() {
    // the bare keyword is gone: it diagnoses with the two-spelling
    // message and recovers as ambient so parsing continues
    let (_, diags) = parse("builtin fn assert(cond: bool, msg: str) -> nil;", Mode::Decl);
    assert!(
        diags.iter().any(|d| d.msg.contains(
            "`builtin` must be spelled `prelude builtin` (ambient) or `pub builtin` (import-gated)"
        )),
        "bare `builtin` must diagnose the two spellings: {diags:?}"
    );
}

#[test]
fn prelude_is_contextual() {
    // `prelude` is a legal identifier everywhere except directly
    // before `builtin`
    let (_, diags) = parse("fn f() -> i32 { let prelude = 3; return prelude; }", Mode::Impl);
    assert!(diags.is_empty(), "`prelude` must stay a legal identifier: {diags:?}");
    // ...and `prelude fn` is NOT a declaration form
    let (_, diags) = parse("prelude fn f() -> i32 { return 0; }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("expected a declaration, found `prelude`")),
        "`prelude` must diagnose when not before `builtin`: {diags:?}"
    );
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
    // `host`/`builtin` belong to `.d.rut`
    let (_, diags) = parse("host fn string_len(s: str) -> i32;", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("belong in a `.d.rut`")),
        "impl mode must reject the surface keyword: {diags:?}"
    );
    let (_, diags) = parse("prelude builtin fn own<T>(x: T) -> T;", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("belong in a `.d.rut`")),
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
        "prelude builtin Option<T> {",
        "prelude builtin fn f(",
        "prelude builtin trait I {",
        "builtin fn f(",
        "builtin class C {",
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

#[test]
fn inline_mod_blocks_parse_in_decl_files() {
    // wire v12 — the declaration entry lane: `pub? mod NAME { .. }`
    // nests declarations in the grammar (structure in the grammar,
    // never in paths), recursively, and the block's items parse with
    // their own spans so the deps lane can stamp mod paths from the
    // nesting. Implementation files keep the refusal.
    let src = "\
pub mod net {
    pub mod tls {
        pub host fn handshake(c: opaque) -> opaque;
    }
    pub host fn connect(url: str) -> opaque;
}
mod solo {
    pub host fn ping() -> bool;
}
pub host fn legacy() -> bool;
";
    let (ast, diags) = parse(src, Mode::Decl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    assert_eq!(items.len(), 3);
    let ItemKind::ModInline { vis, name, items: net } = ast.item(items[0]) else {
        panic!("expected a ModInline, got {:?}", ast.item(items[0]));
    };
    assert_eq!(*vis, Vis::Pub);
    assert_eq!(ast.name(*name), "net");
    assert_eq!(net.len(), 2, "the nested edge + the row: {net:?}");
    let ItemKind::ModInline { name, items: tls, .. } = ast.item(net[0]) else {
        panic!("expected the nested ModInline, got {:?}", ast.item(net[0]));
    };
    assert_eq!(ast.name(*name), "tls");
    assert_eq!(tls.len(), 1);
    let ItemKind::SurfaceFn { name, .. } = ast.item(tls[0]) else {
        panic!("expected the nested row");
    };
    assert_eq!(ast.name(*name), "handshake");
    let ItemKind::SurfaceFn { name, .. } = ast.item(net[1]) else {
        panic!("expected the level-one row");
    };
    assert_eq!(ast.name(*name), "connect");

    // the file-form edge still parses beside inline blocks, and the
    // inline form is DECL-ONLY
    let (_, diags) = parse("mod gone { }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("inline `mod { }` blocks are gone")),
        "implementation files keep the refusal: {diags:?}"
    );
}

#[test]
fn bodiless_signatures_parse_clean_in_decl_files() {
    // the declaration entry's fn spellings: a free fn signature and a
    // class's bodiless method signatures are DECLARATIONS (no bodies) —
    // they parse clean in Decl mode; the same shapes in an
    // implementation file keep today's diagnostics.
    let src = "\
pub class Flow<E> {
    drive: fn(fn(E) -> bool) -> nil;

    pub fn new(f: fn(fn(E) -> bool) -> nil) -> Self;
    pub fn map<R>(f: fn(E) -> R) -> Flow<R>;
}

pub fn freeze(v: Flow<u8>) -> bytes;

pub async fn tick() -> bool;

pub struct Pair {
    fst: i32;
    snd: i32;
}
";
    let (ast, diags) = parse(src, Mode::Decl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    assert_eq!(items.len(), 4, "{items:?}");
    // a struct stays fields-only (methods are classes' impl surface)
    let ItemKind::Struct { methods, .. } = ast.item(items[3]) else {
        panic!("expected the struct");
    };
    assert!(methods.is_empty());
    let ItemKind::Class { methods, .. } = ast.item(items[0]) else {
        panic!("expected the class");
    };
    assert_eq!(methods.len(), 2, "the bodiless members ride the class");
    let ItemKind::Fn(d) = ast.item(items[1]) else {
        panic!("expected the free fn");
    };
    assert_eq!(ast.name(d.name), "freeze");
    assert_eq!(d.generics.len(), 0);
    // the bodiless spelling carries an empty block (the arena's fn
    // shape) — no machine lowers it
    assert_eq!(
        ast.span(d.body.id()).lo, ast.span(d.body.id()).hi,
        "the empty body: zero-width at the semi"
    );

    // implementation files keep the laws
    let (_, diags) = parse("pub fn f(x: i32) -> i32;", Mode::Impl);
    assert!(!diags.is_empty(), "a bodiless fn refuses in .rut");
    let (_, diags) = parse("class C {\n    pub fn m(self) -> C;\n}", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("methods live in `impl` blocks")),
        "class members keep the impl-blocks law in .rut: {diags:?}"
    );
}
