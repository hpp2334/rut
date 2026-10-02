//! The positional one-field class decl — `class Name(Wrapped);`: the
//! newtype spelling. The decl DESUGARS at the parse to the ordinary
//! one-field class (`inner` field, `newtype: true`), so every
//! downstream machine sees a class; the tests pin the desugared shape,
//! the generic and bounded heads, the printed-back spelling, and the
//! malformed forms.

use rut_ast::ast::*;
use rut_parser::{parse, Mode};

#[test]
fn positional_decl_desugars_to_one_inner_field() {
    let src = "class JsonI64(i64);\n";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Class { vis: _, name, generics, requires, newtype, fields, methods } = ast.item(items[0])
    else {
        panic!("expected a class");
    };
    assert_eq!(ast.name(*name), "JsonI64");
    assert!(*newtype, "the positional spelling arms the flag");
    assert!(generics.is_empty() && requires.is_empty() && methods.is_empty());
    assert_eq!(fields.len(), 1, "exactly the wrapped field");
    let fd = ast.field_decl(fields[0]);
    assert_eq!(ast.name(fd.name), "inner");
    assert_eq!(fd.vis, None, "the field rides the module-private default");
    assert!(!fd.is_static && fd.init.is_none());
    // the wrapped type is the field's type node — the same span as the
    // spelled `(i64)` parens' interior
    match ast.ty(fd.ty) {
        TypeKind::TyPath { segs } => {
            assert_eq!(segs.len(), 1);
            assert_eq!(ast.name(segs[0].name), "i64");
        }
        t => panic!("expected a path type, got {t:?}"),
    }
}

#[test]
fn generic_newtype_head_parses() {
    // the binder IN the wrapped arg (`Tail<T>([T])`) and the blanket
    // wrapper (`DebugWrap<T>(T)`) — both ordinary generic heads
    for (src, wrapped) in [
        ("class Tail<T>([T]);\n", "[T]"),
        ("class DebugWrap<T>(T);\n", "T"),
        ("class M<T>(Vec<Future<T>>);\n", "composite"),
    ] {
        let (ast, diags) = parse(src, Mode::Impl);
        assert!(diags.is_empty(), "{src}: expected a clean parse: {diags:?}");
        let items = ast.module_items(ast.root);
        let ItemKind::Class { newtype, generics, fields, .. } = ast.item(items[0]) else {
            panic!("{src}: expected a class");
        };
        assert!(*newtype, "{src}");
        assert_eq!(generics.len(), 1, "{src}");
        assert_eq!(fields.len(), 1, "{src}");
        let _ = wrapped;
    }
}

#[test]
fn bounded_generic_newtype_head_parses() {
    // the class `requires` grammar composes with the positional form
    let src = "class HashWrap<K requires Hashable>(K);\n";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Class { generics, requires, newtype, .. } = ast.item(items[0]) else {
        panic!("expected a class");
    };
    assert!(*newtype);
    assert_eq!(generics.len(), 1);
    assert_eq!(requires.len(), 1, "the bound rides the head");
}

#[test]
fn pub_newtype_decl_parses() {
    let src = "pub class TheirJson(TheirType);\n";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Class { vis, newtype, .. } = ast.item(items[0]) else {
        panic!("expected a class");
    };
    assert_eq!(*vis, Vis::Pub);
    assert!(*newtype);
}

#[test]
fn wrapped_type_may_be_any_type_term() {
    // composites, tuples, fn types, nullables — the wrapped position is
    // a FIELD type, not an impl target
    for src in [
        "class Pair((i32, str));\n",
        "class Cb(fn(i32) -> str);\n",
        "class Opt(?i64);\n",
        "class Arr([[i32]]);\n",
    ] {
        let (ast, diags) = parse(src, Mode::Impl);
        assert!(diags.is_empty(), "{src}: {diags:?}");
        let items = ast.module_items(ast.root);
        let ItemKind::Class { newtype, fields, .. } = ast.item(items[0]) else {
            panic!("{src}: expected a class");
        };
        assert!(*newtype && fields.len() == 1, "{src}");
    }
}

#[test]
fn empty_parens_diagnose() {
    let (ast, diags) = parse("class Foo();\n", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("wraps exactly one type")),
        "the empty positional form names the fix: {diags:?}"
    );
    let _ = ast;
}

#[test]
fn body_after_positional_form_diagnoses() {
    // the positional decl has no body — methods live in impl blocks
    let (ast, diags) = parse("class Foo(i32) { }\n", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("no body")),
        "a body after the wrapped type names the fix: {diags:?}"
    );
    let _ = ast;
}

#[test]
fn structs_have_no_positional_form() {
    // the decl shape is a CLASS surface; a struct head with parens is
    // the ordinary missing-body diagnostic
    let (_, diags) = parse("struct Foo(i32);\n", Mode::Impl);
    assert!(!diags.is_empty(), "a tuple struct is not grammar: {diags:?}");
}
