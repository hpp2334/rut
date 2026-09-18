//! Type aliases, union bounds, and inline `requires` (RFC 0043): parse
//! shapes, the `where` removal diagnostic, misplaced bounds rejected,
//! dumper output, and `pub(..)` on `type`.

use rut_ast::ast::*;
use rut_ast::dump;
use rut_parser::{parse, Mode};

#[test]
fn alias_parses_transparent_shape() {
    let src = "type Meters = i64;\n";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Alias(d) = ast.item(items[0]) else {
        panic!("expected an alias");
    };
    assert_eq!(d.vis, Vis::Self_);
    assert_eq!(ast.name(d.name), "Meters");
    let TypeKind::TyPath { segs } = ast.ty(d.target) else {
        panic!("expected a path target");
    };
    assert_eq!(ast.name(segs[0].name), "i64");
}

#[test]
fn union_alias_parses_ty_union() {
    let src = "type Num = i32 | str;\n";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Alias(d) = ast.item(items[0]) else {
        panic!("expected an alias");
    };
    let TypeKind::TyUnion { elems } = ast.ty(d.target) else {
        panic!("expected a union target");
    };
    assert_eq!(elems.len(), 2);
    let names: Vec<&str> = elems
        .iter()
        .map(|&e| match ast.ty(e) {
            TypeKind::TyPath { segs } => ast.name(segs[0].name),
            _ => panic!("expected path members"),
        })
        .collect();
    assert_eq!(names, vec!["i32", "str"]);
}

#[test]
fn alias_target_kinds_parse() {
    // a pointer target, a generic target, a chain — all single targets
    let src = "\
type Box = Vec;
type Row = [*i32];
";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    assert_eq!(items.len(), 2);
    for it in items {
        assert!(matches!(ast.item(*it), ItemKind::Alias(_)));
    }
}

#[test]
fn inline_requires_on_fn_parses() {
    // one bound, single member
    let src = "trait Show { fn show(self) -> str; }\nfn tagged<T requires Show>(x: T) -> str { return \"\"; }\n";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Fn(f) = ast.item(items[1]) else {
        panic!("expected a fn");
    };
    assert_eq!(f.bounds.len(), 1);
    assert_eq!(ast.name(f.bounds[0].0), "T");
    let TypeKind::TyPath { segs } = ast.ty(f.bounds[0].1) else {
        panic!("expected a path bound");
    };
    assert_eq!(ast.name(segs[0].name), "Show");
}

#[test]
fn inline_union_bound_and_multiple_params_parse() {
    // a union bound spanning two members, plus an unbounded generic
    let src = "fn f<A, T requires i32 | str, B>(x: T) -> i32 { return 0; }\n";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Fn(f) = ast.item(items[0]) else {
        panic!("expected a fn");
    };
    assert_eq!(f.generics.len(), 3, "A, T, B all collected: {:?}", f.generics);
    assert_eq!(f.bounds.len(), 1);
    let TypeKind::TyUnion { elems } = ast.ty(f.bounds[0].1) else {
        panic!("expected a union bound");
    };
    assert_eq!(elems.len(), 2);
}

#[test]
fn inline_requires_on_method_parses() {
    let src = "\
trait Enc { fn enc(self) -> bytes; }
impl Widget {
    fn wrap<T requires Enc>(self, x: T) -> bytes { return x.enc(); }
}
";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Impl { methods, .. } = ast.item(items[1]) else {
        panic!("expected an impl block");
    };
    let md = ast.method_decl(methods[0]);
    assert_eq!(md.bounds.len(), 1);
    assert_eq!(ast.name(md.bounds[0].0), "T");
}

#[test]
fn stray_where_diagnoses_with_inline_replacement() {
    // after the return type
    let src = "trait D { fn d(self) -> u64; }\nfn load<T>(v: str) -> T where T requires D { panic(\"x\"); }\n";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("`where` clauses are removed") && d.msg.contains("<T requires")),
        "stray where must diagnose with the replacement: {diags:?}"
    );
    // bodiless position too
    let src2 = "trait D { fn d(self) -> u64; }\ntrait Tr { fn m<T>(self) -> i32 where T requires D; }\n";
    let (_, diags2) = parse(src2, Mode::Impl);
    assert!(
        diags2.iter().any(|d| d.msg.contains("`where` clauses are removed")),
        "stray where on a method must diagnose: {diags2:?}"
    );
}

#[test]
fn misplaced_bounds_are_rejected() {
    // struct / class / trait / builtin generics reject `requires`
    let cases = [
        ("struct S<T requires D> { v: T }", "`struct` generic parameters take no `requires` bounds"),
        ("class C<T requires D> { v: T }", "`class` generic parameters take no `requires` bounds"),
        ("trait Tr<T requires D> { }", "`trait` generic parameters take no `requires` bounds"),
        (
            "trait D { fn d(self) -> u64; }\nbuiltin trait Bt<T requires D> { }",
            "`builtin trait` generic parameters take no `requires` bounds",
        ),
    ];
    for (src, want) in cases {
        let full = format!("trait D {{ fn d(self) -> u64; }}\n{src}\n");
        let (_, diags) = parse(&full, Mode::Impl);
        assert!(
            diags.iter().any(|d| d.msg.contains(want)),
            "expected `{want}` for `{src}`: {diags:?}"
        );
    }
}

#[test]
fn dumper_renders_alias_bounds_and_union() {
    let src = "pub type Meters = i64;\ntype Num = i32 | str;\nfn f<T requires i32 | str>(x: T) -> i32 { return 0; }\n";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let text = dump::render_text(&dump::to_dump_tree(&ast), src);
    assert!(text.contains("Alias Meters"), "alias node in the dump: {text}");
    assert!(text.contains("TyUnion"), "union node in the dump: {text}");
    assert!(text.contains("bounds:"), "bounds field in the dump: {text}");
    let json = dump::render_json(&dump::to_dump_tree(&ast));
    assert!(json.contains("\"Alias\""), "alias kind on the wire: {json}");
}

#[test]
fn pub_scopes_parse_on_type_alias() {
    let src = "\
pub type A = i64;
pub(mod) type B = i64;
pub(super) type C = i64;
pub(self) type D = i64;
";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let expect = |i: usize| match ast.item(items[i]) {
        ItemKind::Alias(d) => d.vis,
        _ => panic!("expected an alias"),
    };
    assert_eq!(expect(0), Vis::Pub);
    assert_eq!(expect(1), Vis::Mod);
    assert_eq!(expect(2), Vis::Super);
    assert_eq!(expect(3), Vis::Self_);
}

#[test]
fn alias_in_decl_mode_parses_without_body() {
    // `type` has no body, so it is legal in a `.d.rut`
    let (_, diags) = parse("type Handle = i64;\n", Mode::Decl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
}

#[test]
fn where_is_no_longer_reserved() {
    // `where` left the reserved table (RFC 0043) — it is an ordinary
    // identifier again
    assert!(!rut_parser::is_reserved_kw("where"));
}
