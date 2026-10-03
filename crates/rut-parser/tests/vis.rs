//! `pub` visibility on members: the scoped forms parse on
//! class fields and on inherent impl methods, structs
//! reject the dial, and the dropped `private` keyword is a
//! lexer hard error naming its replacement. Type bodies
//! are FIELDS ONLY — methods live in `impl` blocks.

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
}

impl Widget {
    pub fn new() -> Self { return Self { w: 0, h: 0, inner: 0, spare: 0, plain: 0 }; }
    pub fn size(self) -> f32 { return self.w * self.h; }
    pub(mod) fn reset(mut self) -> nil { self.inner = 0; }
    pub(self) fn helper(self) -> i32 { return self.inner; }
    fn secret(self) -> i32 { return self.plain; }
}
";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Class { fields, .. } = ast.item(items[0]) else {
        panic!("expected a class");
    };
    let ItemKind::Impl { methods, .. } = ast.item(items[1]) else {
        panic!("expected an impl block");
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
    // `pub` + `async` on an inherent impl method; `static` fields parse
    // (the checker owns the "no static fields" diagnostic)
    let src = "\
class Sink {
    pub static CAP: i32 = 4;
}

impl Sink {
    pub async fn later(self) -> nil { }
}
";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Class { fields, .. } = ast.item(items[0]) else {
        panic!("expected a class");
    };
    let ItemKind::Impl { methods, .. } = ast.item(items[1]) else {
        panic!("expected an impl block");
    };
    let fd = ast.field_decl(fields[0]);
    assert_eq!(fd.vis, Some(Vis::Pub));
    assert!(fd.is_static);
    let md = ast.method_decl(methods[0]);
    assert_eq!(md.vis, Some(Vis::Pub));
    assert!(md.is_async);
}

#[test]
fn struct_rejects_member_pub() {
    // struct members are always public — no visibility dial
    let (_, diags) = parse("struct P { pub x: i32; }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("structs have no field visibility")),
        "member pub must be diagnosed in a struct: {diags:?}"
    );
}

#[test]
fn private_is_a_dropped_word() {
    // dropped keywords hard-error naming the replacement
    let (_, diags) = parse("class C { private n: i32; }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("members are private by default")),
        "`private` must name its replacement: {diags:?}"
    );
}

#[test]
fn pub_scope_rejects_unknown_scope() {
    // the `fn` member itself is now a body error, but the scope
    // diagnostic still fires first (methods live in impls)
    let (_, diags) = parse("class C { pub(crate) fn m(self) -> nil { } }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("expected `mod`, `super`, or `self` in pub")),
        "unknown scope must be diagnosed: {diags:?}"
    );
}

#[test]
fn body_methods_are_a_hard_error() {
    // type bodies are fields-only — a `fn` member diagnoses
    let (_, diags) = parse("class C { fn m(self) -> nil { } }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("methods live in `impl` blocks")),
        "body methods must diagnose: {diags:?}"
    );
}

#[test]
fn bodyless_impl_is_rejected() {
    // exactly one braced impl form — the grammar has no `for` branch,
    // so `impl I for T;` is the missing-`{` shape
    let (_, diags) = parse("struct T { x: i32 } impl I for T;", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("expected {")),
        "a bodyless impl must diagnose: {diags:?}"
    );
}

// the `impl I for T { .. }` visibility law (trait impl methods ride the
// trait's visibility) died with trait impls — structural satisfaction
// has no registration form to gate.

#[test]
fn interface_bodies_accept_async_and_no_self_sigs() {
    let src = "interface T {\n\
        fn area(self) -> f64;\n\
        async fn poll(self) -> bool;\n\
        fn yield_now(cx: u32);\n\
    }";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Interface { methods, .. } = ast.item(items[0]) else {
        panic!("expected an interface");
    };
    assert!(ast.method_decl(methods[1]).is_async);
    assert!(!ast.method_decl(methods[2]).is_async);
}

#[test]
fn inherent_impl_is_the_one_form() {
    let src = "struct P { x: i32 } \
        impl P { fn new(x: i32) -> Self { return Self { x: x }; } } \
        interface Show { fn show(self) -> str; } \
        impl P { fn show(self) -> str { return \"\"; } }";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Impl { target, .. } = ast.item(items[1]) else {
        panic!("expected an inherent impl");
    };
    let TypeKind::TyPath { segs } = ast.ty(*target) else {
        panic!("expected a path target");
    };
    assert_eq!(ast.name(segs[0].name), "P");
    let ItemKind::Impl { .. } = ast.item(items[3]) else {
        panic!("expected a second inherent impl");
    };
}

#[test]
fn interface_vis_parses_into_the_item() {
    // the interface carries its own visibility — members carry none
    let src = "pub interface Show {\n\
        fn show(self) -> str;\n\
    }";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Interface { vis, name, methods, .. } = ast.item(items[0]) else {
        panic!("expected an interface");
    };
    assert_eq!(*vis, Vis::Pub);
    assert_eq!(ast.name(*name), "Show");
    assert_eq!(methods.len(), 1);
}

#[test]
fn interface_member_pub_is_diagnosed() {
    // the interface's visibility rules govern — members take no `pub`
    let (_, diags) = parse("interface Show { pub fn show(self) -> str; }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("interface members carry no `pub`")),
        "member `pub` must diagnose: {diags:?}"
    );
}

#[test]
fn interface_member_marker_is_diagnosed() {
    // `[marker]` is an inherent-impl spelling — never an interface's
    let (_, diags) = parse("interface I { [iterable] fn it(self) -> i32; }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("marks an inherent impl member")),
        "an interface-body marker must diagnose: {diags:?}"
    );
}

#[test]
fn interface_non_fn_member_is_diagnosed() {
    // interfaces declare method signatures — nothing else
    let (_, diags) = parse("interface I { x: i32; }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("interfaces declare method signatures") && d.msg.contains("expected `fn`")),
        "a non-fn member must diagnose: {diags:?}"
    );
}
