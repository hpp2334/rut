//! Associated `type` members in trait/impl bodies (RFC 0012): a trait
//! declares `type Name;` (optionally with a default), an impl binds
//! `type Name = Ty;`. `type` is a reserved word, matched by text.

use rut_ast::ast::*;
use rut_parser::{parse, Mode};

#[test]
fn trait_declares_associated_type() {
    let src = "trait Iter {\n    type Target;\n    fn get(self, i: i32) -> Target;\n}\n";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Trait { name, assoc, methods, .. } = ast.item(items[0]) else {
        panic!("expected a Trait, got {:?}", ast.item(items[0]));
    };
    assert_eq!(ast.name(*name), "Iter");
    assert_eq!(assoc.len(), 1);
    assert_eq!(ast.name(assoc[0].name), "Target");
    assert!(assoc[0].ty.is_none(), "no default");
    assert_eq!(methods.len(), 1);
}

#[test]
fn trait_associated_type_default() {
    let src = "trait Iter<T> {\n    type Target = T;\n}\n";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Trait { generics, assoc, .. } = ast.item(items[0]) else {
        panic!("expected a Trait");
    };
    assert_eq!(generics.len(), 1);
    assert!(assoc[0].ty.is_some(), "default present");
}

#[test]
fn impl_binds_associated_type() {
    let src = "impl Iter for Vec<i32> {\n    type Target = i32;\n    fn len(self) -> i32 { return 0; }\n    fn get(self, i: i32) -> i32 { return 0; }\n}\n";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Impl { assoc, methods, .. } = ast.item(items[0]) else {
        panic!("expected an Impl");
    };
    assert_eq!(assoc.len(), 1);
    assert_eq!(ast.name(assoc[0].name), "Target");
    assert!(assoc[0].ty.is_some());
    assert_eq!(methods.len(), 2);
}
