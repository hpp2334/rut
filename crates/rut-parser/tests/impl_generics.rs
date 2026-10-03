//! Declared impl generics: `impl<T> ..` — the binder list after
//! `impl` parses on the one impl form (the inherent `impl T { .. }`),
//! carries bounds grammar (accepted for uniformity, collector-ignored),
//! and the AST records it. The undeclared-name diagnostic and
//! collection live in rut-lir; here the surface speaks.

use rut_ast::ast::*;
use rut_parser::{parse, Mode};

#[test]
fn declared_binders_parse_on_the_inherent_impl() {
    let src = "\
class Vec2<T> { items: [T]; }
impl<T> Vec2<T> {
    pub fn new() -> Self { return Self { items: [] }; }
}
";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Impl { generics, target, .. } = ast.item(items[1]) else {
        panic!("expected an inherent impl");
    };
    assert_eq!(generics.len(), 1, "the inherent impl declares one binder");
    let TypeKind::TyPath { segs } = ast.ty(*target) else {
        panic!("expected a path target");
    };
    assert_eq!(ast.name(segs[0].name), "Vec2");
}

#[test]
fn multi_binder_heads_parse() {
    let src = "\
class Map2<K, V> { }
impl<K, V> Map2<K, V> {
    pub fn new() -> Self { return Self { }; }
}
";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Impl { generics, .. } = ast.item(items[1]) else {
        panic!("expected an impl block");
    };
    assert_eq!(generics.len(), 2);
}

#[test]
fn binder_bounds_grammar_is_accepted() {
    // the list grammar matches fn/class generics: `requires` bounds
    // parse (accepted for grammar uniformity — the collector ignores
    // impl head bounds)
    let src = "\
class Box2<T> { v: T; }
interface Hash2 { fn hash(self) -> u64; }
impl<T requires Hash2> Box2<T> {
    pub fn new(v: T) -> Self { return Self { v: v }; }
}
";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Impl { generics, bounds, .. } = ast.item(items[2]) else {
        panic!("expected an impl block");
    };
    assert_eq!(generics.len(), 1);
    assert_eq!(bounds.len(), 1, "the bound node rides the impl item");
    assert_eq!(ast.name(bounds[0].0), "T");
}

#[test]
fn concrete_impls_parse_without_a_binder_list() {
    let src = "\
struct P { x: i32 }
impl P {
    fn zero() -> Self { return P { x: 0 }; }
}
";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Impl { generics, .. } = ast.item(items[1]) else {
        panic!("expected an impl block");
    };
    assert!(generics.is_empty(), "a concrete impl declares no binders");
}
