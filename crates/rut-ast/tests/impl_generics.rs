//! Declared impl generics: `impl<T> ..` — the binder list after
//! `impl` parses on the one impl form (the inherent `impl T { .. }`),
//! carries bounds grammar (accepted for uniformity, collector-ignored),
//! and the AST records it. The undeclared-name diagnostic and
//! collection live in rut-lir; here the surface speaks.

mod common;

use common::parse;
use rut_ast::parser::Mode;
use rut_ast::{ItemRef, TyRef};

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
    let items = ast.root().items(&ast);
    let ItemRef::Impl(ih) = items[1] else {
        panic!("expected an inherent impl");
    };
    let d = ih.get(&ast);
    assert_eq!(d.generics.len(), 1, "the inherent impl declares one binder");
    let TyRef::Path(t) = d.target else {
        panic!("expected a path target, got {:?}", d.target);
    };
    assert_eq!(ast.name(t.get(&ast).segs[0].name), "Vec2");
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
    let items = ast.root().items(&ast);
    let ItemRef::Impl(ih) = items[1] else {
        panic!("expected an impl block");
    };
    assert_eq!(ih.get(&ast).generics.len(), 2);
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
    let items = ast.root().items(&ast);
    let ItemRef::Impl(ih) = items[2] else {
        panic!("expected an impl block");
    };
    let d = ih.get(&ast);
    assert_eq!(d.generics.len(), 1);
    assert_eq!(d.bounds.len(), 1, "the bound node rides the impl item");
    assert_eq!(ast.name(d.bounds[0].0), "T");
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
    let items = ast.root().items(&ast);
    let ItemRef::Impl(ih) = items[1] else {
        panic!("expected an impl block");
    };
    assert!(
        ih.get(&ast).generics.is_empty(),
        "a concrete impl declares no binders"
    );
}
