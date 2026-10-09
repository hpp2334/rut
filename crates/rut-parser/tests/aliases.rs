//! Type aliases, union bounds, and inline `requires`: parse
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
    // a nullable target, a generic target, a chain — all single targets
    let src = "\
type Box = Vec;
type Row = [?i32];
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
    let src = "interface Show { fn show(self) -> str; }\nfn tagged<T requires Show>(x: T) -> str { return \"\"; }\n";
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
interface Enc { fn enc(self) -> bytes; }
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
    let src = "interface D { fn d(self) -> u64; }\nfn load<T>(v: str) -> T where T requires D { panic(\"x\"); }\n";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("`where` clauses are removed") && d.msg.contains("<T requires")),
        "stray where must diagnose with the replacement: {diags:?}"
    );
    // bodiless position too
    let src2 = "interface D { fn d(self) -> u64; }\ninterface Tr { fn m<T>(self) -> i32 where T requires D; }\n";
    let (_, diags2) = parse(src2, Mode::Impl);
    assert!(
        diags2.iter().any(|d| d.msg.contains("`where` clauses are removed")),
        "stray where on a method must diagnose: {diags2:?}"
    );
}

#[test]
fn misplaced_bounds_are_rejected() {
    // struct / interface / builtin generics reject `requires` — class
    // generics TAKE them (the admission bounds)
    let cases = [
        ("struct S<T requires D> { v: T }", "`struct` generic parameters take no `requires` bounds", Mode::Impl),
        ("interface Tr<T requires D> { }", "`interface` generic parameters take no `requires` bounds", Mode::Impl),
        // the engine surface's own gate (v20: the `builtin trait` row
        // kind is gone — the builtin CLASS carries the check)
        ("interface D { fn d(self) -> u64; }\nprelude builtin class Bt<T requires D> { }", "`builtin class` generic parameters take no `requires` bounds", Mode::Decl),
    ];
    for (src, want, mode) in cases {
        let full = format!("interface D {{ fn d(self) -> u64; }}\n{src}\n");
        let (_, diags) = parse(&full, mode);
        assert!(
            diags.iter().any(|d| d.msg.contains(want)),
            "expected `{want}` for `{src}`: {diags:?}"
        );
    }
}

#[test]
fn class_requires_parses_into_the_class_frame() {
    // the bounded-gparam grammar extends to classes —
    // bounds land on the class frame, the body still parses
    let src = "\
interface D { fn d(self) -> u64; }
class Box<K requires D, V> {
    k: K;
    v: V;
}
";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Class { generics, requires, .. } = ast.item(items[1]) else {
        panic!("expected a class");
    };
    assert_eq!(generics.len(), 2, "K, V both collected: {:?}", generics);
    assert_eq!(requires.len(), 1);
    assert_eq!(ast.name(requires[0].0), "K");
    let TypeKind::TyPath { segs } = ast.ty(requires[0].1) else {
        panic!("expected a path bound");
    };
    assert_eq!(ast.name(segs[0].name), "D");
}

#[test]
fn class_union_bound_and_unbounded_params_parse() {
    // a union bound on one param, an unbounded param after it — the
    // list continues through the absorbed bounds like the fn grammar
    let src = "class C<A, T requires i32 | str, B> { v: T; }\n";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Class { generics, requires, .. } = ast.item(items[0]) else {
        panic!("expected a class");
    };
    assert_eq!(generics.len(), 3, "A, T, B all collected: {:?}", generics);
    assert_eq!(requires.len(), 1);
    let TypeKind::TyUnion { elems } = ast.ty(requires[0].1) else {
        panic!("expected a union bound");
    };
    assert_eq!(elems.len(), 2);
}

#[test]
fn dumper_renders_class_requires() {
    let src = "interface D { fn d(self) -> u64; }\nclass Box<K requires D, V> { k: K; v: V; }\n";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let text = dump::render_text(&dump::to_dump_tree(&ast), src);
    assert!(text.contains("Class"), "class node in the dump: {text}");
    assert!(text.contains("bounds:"), "requires field in the dump: {text}");
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
pub(super) type C = i64;
pub(pkg) type P = i64;
";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let expect = |i: usize| match ast.item(items[i]) {
        ItemKind::Alias(d) => d.vis,
        _ => panic!("expected an alias"),
    };
    assert_eq!(expect(0), Vis::Pub);
    assert_eq!(expect(1), Vis::Super);
    assert_eq!(expect(2), Vis::Pkg);
}

#[test]
fn pub_mod_self_scopes_are_gone_on_type_alias() {
    // the repealed paren spellings diagnose (one diag each) but the
    // aliases still parse at the default
    let (ast, diags) = parse("pub(mod) type B = i64;\npub(self) type D = i64;\n", Mode::Impl);
    assert_eq!(diags.len(), 2, "one diag per repealed spelling: {diags:?}");
    assert!(
        diags.iter().all(|d| d.msg.contains("are gone")),
        "each diag names the fix: {diags:?}"
    );
    let items = ast.module_items(ast.root);
    for i in 0..2 {
        match ast.item(items[i]) {
            ItemKind::Alias(d) => assert_eq!(d.vis, Vis::Self_, "the default survives recovery"),
            _ => panic!("expected an alias"),
        }
    }
}

#[test]
fn alias_in_decl_mode_parses_without_body() {
    // `type` has no body, so it is legal in a `.d.rut`
    let (_, diags) = parse("type Handle = i64;\n", Mode::Decl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
}

#[test]
fn where_is_no_longer_reserved() {
    // `where` left the reserved table — it is an ordinary
    // identifier again
    assert!(!rut_parser::is_reserved_kw("where"));
}
