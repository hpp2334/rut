//! `pub` visibility on members: the scoped forms parse on
//! class fields and on inherent impl methods (`pub(super)`,
//! `pub(pkg)`), the repealed `pub(mod)`/`pub(self)` spellings
//! diagnose naming the fix, structs reject the dial, and the dropped
//! `private` keyword is a lexer hard error naming its replacement.
//! Type bodies are FIELDS ONLY — methods live in `impl` blocks.

mod common;

use common::parse;
use rut_ast::{FieldRef, ItemRef, MethodRef, TyRef, Vis};
use rut_parser::Mode;

#[test]
fn member_pub_scopes_parse() {
    let src = "\
class Widget {
    pub w: f32;
    pub(pkg) g: f32;
    pub(super) spare: i32;
    plain: i32;                       // unannotated = the module-private default
}

impl Widget {
    pub fn new() -> Self { return Self { w: 0, g: 0, spare: 0, plain: 0 }; }
    pub fn size(self) -> f32 { return self.w * self.g; }
    pub(super) fn reset(mut self) -> nil { self.plain = 0; }
    pub(pkg) fn tint(self) -> i32 { return self.spare; }
    fn secret(self) -> i32 { return self.plain; }
}
";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.root().items(&ast);
    let ItemRef::Class(ch) = items[0] else {
        panic!("expected a class, got {:?}", items[0]);
    };
    let fields = ch.get(&ast).fields.clone();
    let ItemRef::Impl(ih) = items[1] else {
        panic!("expected an impl block, got {:?}", items[1]);
    };
    let methods = ih.get(&ast).methods.clone();
    let field_vis = |r: FieldRef, want: Option<Vis>| {
        let FieldRef::Field(fh) = r else {
            panic!("a field, got {r:?}");
        };
        assert_eq!(fh.get(&ast).vis, want);
    };
    field_vis(fields[0], Some(Vis::Pub));
    field_vis(fields[1], Some(Vis::Pkg));
    field_vis(fields[2], Some(Vis::Super));
    field_vis(fields[3], None);
    let method_vis = |i: usize| {
        let MethodRef::Method(mh) = methods[i] else {
            panic!("a method, got {:?}", methods[i]);
        };
        mh.get(&ast).vis
    };
    assert_eq!(method_vis(0), Some(Vis::Pub));
    assert_eq!(method_vis(1), Some(Vis::Pub));
    assert_eq!(method_vis(2), Some(Vis::Super));
    assert_eq!(method_vis(3), Some(Vis::Pkg));
    assert_eq!(method_vis(4), None);
}

#[test]
fn member_pub_mod_self_are_gone() {
    // the paren spellings are repealed — one diag naming the fix, and
    // the member still parses (the default) so the tail stays sane
    let (_, diags) = parse("class C { pub(mod) n: i32; pub(self) m: i32; }", Mode::Impl);
    assert!(
        diags
            .iter()
            .any(|d| d.msg.contains("`pub(mod)`/`pub(self)` are gone")
                && d.msg.contains("bare private is the default")),
        "the repealed spellings must name the fix: {diags:?}"
    );
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
    let items = ast.root().items(&ast);
    let ItemRef::Class(ch) = items[0] else {
        panic!("expected a class, got {:?}", items[0]);
    };
    let FieldRef::Field(fh) = ch.get(&ast).fields[0] else {
        panic!("a field");
    };
    let fd = fh.get(&ast);
    assert_eq!(fd.vis, Some(Vis::Pub));
    assert!(fd.is_static);
    let ItemRef::Impl(ih) = items[1] else {
        panic!("expected an impl block, got {:?}", items[1]);
    };
    let MethodRef::Method(mh) = ih.get(&ast).methods[0] else {
        panic!("a method");
    };
    let md = mh.get(&ast);
    assert_eq!(md.vis, Some(Vis::Pub));
    assert!(md.is_async);
}

#[test]
fn struct_rejects_member_pub() {
    // struct members are always public — no visibility dial
    let (_, diags) = parse("struct P { pub x: i32; }", Mode::Impl);
    assert!(
        diags
            .iter()
            .any(|d| d.msg.contains("structs have no field visibility")),
        "member pub must be diagnosed in a struct: {diags:?}"
    );
}

#[test]
fn private_is_a_dropped_word() {
    // dropped keywords hard-error naming the replacement
    let (_, diags) = parse("class C { private n: i32; }", Mode::Impl);
    assert!(
        diags
            .iter()
            .any(|d| d.msg.contains("members are private by default")),
        "`private` must name its replacement: {diags:?}"
    );
}

#[test]
fn pub_scope_rejects_unknown_scope() {
    // the `fn` member itself is now a body error, but the scope
    // diagnostic still fires first (methods live in impls)
    let (_, diags) = parse("class C { pub(crate) fn m(self) -> nil { } }", Mode::Impl);
    assert!(
        diags
            .iter()
            .any(|d| d.msg.contains("expected `super` or `pkg` in pub(..)")),
        "unknown scope must be diagnosed: {diags:?}"
    );
}

#[test]
fn body_methods_are_a_hard_error() {
    // type bodies are fields-only — a `fn` member diagnoses
    let (_, diags) = parse("class C { fn m(self) -> nil { } }", Mode::Impl);
    assert!(
        diags
            .iter()
            .any(|d| d.msg.contains("methods live in `impl` blocks")),
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
    let items = ast.root().items(&ast);
    let ItemRef::Interface(ih) = items[0] else {
        panic!("expected an interface, got {:?}", items[0]);
    };
    let methods = &ih.get(&ast).methods;
    let MethodRef::Method(a) = methods[1] else {
        panic!("a method, got {:?}", methods[1]);
    };
    assert!(a.get(&ast).is_async);
    let MethodRef::Method(b) = methods[2] else {
        panic!("a method, got {:?}", methods[2]);
    };
    assert!(!b.get(&ast).is_async);
}

#[test]
fn inherent_impl_is_the_one_form() {
    let src = "struct P { x: i32 } \
        impl P { fn new(x: i32) -> Self { return Self { x: x }; } } \
        interface Show { fn show(self) -> str; } \
        impl P { fn show(self) -> str { return \"\"; } }";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.root().items(&ast);
    let ItemRef::Impl(ih) = items[1] else {
        panic!("expected an inherent impl, got {:?}", items[1]);
    };
    let TyRef::Path(t) = ih.get(&ast).target else {
        panic!("expected a path target, got {:?}", ih.get(&ast).target);
    };
    assert_eq!(ast.name(t.get(&ast).segs[0].name), "P");
    assert!(matches!(items[3], ItemRef::Impl(_)));
}

#[test]
fn interface_vis_parses_into_the_item() {
    // the interface carries its own visibility — members carry none
    let src = "pub interface Show {\n\
        fn show(self) -> str;\n\
    }";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.root().items(&ast);
    let ItemRef::Interface(ih) = items[0] else {
        panic!("expected an interface, got {:?}", items[0]);
    };
    let d = ih.get(&ast);
    assert_eq!(d.vis, Vis::Pub);
    assert_eq!(ast.name(d.name), "Show");
    assert_eq!(d.methods.len(), 1);
}

#[test]
fn interface_member_pub_is_diagnosed() {
    // the interface's visibility rules govern — members take no `pub`
    let (_, diags) = parse("interface Show { pub fn show(self) -> str; }", Mode::Impl);
    assert!(
        diags
            .iter()
            .any(|d| d.msg.contains("interface members carry no `pub`")),
        "member `pub` must diagnose: {diags:?}"
    );
}

#[test]
fn interface_member_marker_is_diagnosed() {
    // `[marker]` is an inherent-impl spelling — never an interface's
    let (_, diags) = parse("interface I { [iterable] fn it(self) -> i32; }", Mode::Impl);
    assert!(
        diags
            .iter()
            .any(|d| d.msg.contains("marks an inherent impl member")),
        "an interface-body marker must diagnose: {diags:?}"
    );
}

#[test]
fn interface_non_fn_member_is_diagnosed() {
    // interfaces declare method signatures — nothing else
    let (_, diags) = parse("interface I { x: i32; }", Mode::Impl);
    assert!(
        diags
            .iter()
            .any(|d| d.msg.contains("interfaces declare method signatures")
                && d.msg.contains("expected `fn`")),
        "a non-fn member must diagnose: {diags:?}"
    );
}
