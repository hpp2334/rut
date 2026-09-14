//! `pub` visibility (RFC 0003 §2) on members: the scoped forms parse on
//! class fields and methods (RFC 0010 §2), dataclasses reject the dial
//! (RFC 0009), and the dropped `private` keyword is a lexer hard error
//! naming its replacement (RFC 0002 §4).

use rut_ast::ast::*;
use rut_parser::{parse, Mode};

#[test]
fn member_pub_scopes_parse() {
    let src = "\
class Widget {
    pub w: f32;
    pub(mod) h: f32;
    pub(self) inner: i32;
    pub(super) spare: i32;
    plain: i32;                       // unannotated = the module-private default

    pub fn new() -> Self { return Self { w: 0, h: 0, inner: 0, spare: 0, plain: 0 }; }
    pub fn size(self) -> f32 { return self.w * self.h; }
    pub(mod) fn reset(mut self) -> unit { self.inner = 0; }
    pub(self) fn helper(self) -> i32 { return self.inner; }
    fn secret(self) -> i32 { return self.plain; }
}
";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Class { fields, methods, .. } = ast.item(items[0]) else {
        panic!("expected a class");
    };
    let expect = |h: NodeHandle<FieldDeclNode>, want: Option<Vis>| {
        assert_eq!(ast.field_decl(h).vis, want);
    };
    expect(fields[0], Some(Vis::Pub));
    expect(fields[1], Some(Vis::Mod));
    expect(fields[2], Some(Vis::Self_));
    expect(fields[3], Some(Vis::Super));
    expect(fields[4], None);
    let m = |i: usize| ast.method_decl(methods[i]).vis;
    assert_eq!(m(0), Some(Vis::Pub));
    assert_eq!(m(1), Some(Vis::Pub));
    assert_eq!(m(2), Some(Vis::Mod));
    assert_eq!(m(3), Some(Vis::Self_));
    assert_eq!(m(4), None);
}

#[test]
fn member_pub_combines_with_modifiers() {
    let src = "\
class Sink {
    pub static CAP: i32 = 4;
    pub suspend fn later(self) -> unit { }
}
";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Class { fields, methods, .. } = ast.item(items[0]) else {
        panic!("expected a class");
    };
    let fd = ast.field_decl(fields[0]);
    assert_eq!(fd.vis, Some(Vis::Pub));
    assert!(fd.is_static);
    let md = ast.method_decl(methods[0]);
    assert_eq!(md.vis, Some(Vis::Pub));
    assert!(md.is_suspend);
}

#[test]
fn dataclass_rejects_member_pub() {
    // RFC 0009: struct members are always public — no visibility dial
    let (_, diags) = parse("struct P { pub x: i32; }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("dataclasses have no member visibility")),
        "member pub must be diagnosed in a struct: {diags:?}"
    );
}

#[test]
fn private_is_a_dropped_word() {
    // RFC 0002 §4: dropped keywords hard-error naming the replacement
    let (_, diags) = parse("class C { private n: i32; }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("members are private by default")),
        "`private` must name its replacement: {diags:?}"
    );
}

#[test]
fn pub_scope_rejects_unknown_scope() {
    let (_, diags) = parse("class C { pub(crate) fn m(self) -> unit { } }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("expected `mod`, `super`, or `self` in pub")),
        "unknown scope must be diagnosed: {diags:?}"
    );
}
