//! Type aliases, union bounds, and inline `requires`: parse
//! shapes, the stray-`where` ordinary ladder, misplaced bounds
//! rejected, dumper output, and `pub(..)` on `type`.

mod common;

use common::parse;
use rut_ast::parser::Mode;
use rut_ast::{AstPrinter, ItemRef, MethodRef, TyRef, Vis};

#[test]
fn alias_parses_transparent_shape() {
    let src = "type Meters = i64;\n";
    let (doc, diags) = parse(src, Mode::Impl);
    let ast = doc.ast();
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.root().items(ast);
    let ItemRef::Alias(h) = items[0] else {
        panic!("expected an alias");
    };
    let d = h.get(ast);
    assert_eq!(d.vis(), Vis::Self_);
    assert_eq!(ast.name(d.name.ident), "Meters");
    let TyRef::Path(t) = d.target else {
        panic!("expected a path target");
    };
    assert_eq!(ast.name(t.get(ast).segs[0].name.ident), "i64");
}

#[test]
fn union_alias_parses_ty_union() {
    let src = "type Num = i32 | str;\n";
    let (doc, diags) = parse(src, Mode::Impl);
    let ast = doc.ast();
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.root().items(ast);
    let ItemRef::Alias(h) = items[0] else {
        panic!("expected an alias");
    };
    let TyRef::Union(u) = h.get(ast).target else {
        panic!("expected a union target");
    };
    let elems = &u.get(ast).elems;
    assert_eq!(elems.len(), 2);
    let names: Vec<&str> = elems
        .iter()
        .map(|&e| match e {
            TyRef::Path(ph) => ast.name(ph.get(ast).segs[0].name.ident),
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
    let (doc, diags) = parse(src, Mode::Impl);
    let ast = doc.ast();
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.root().items(ast);
    assert_eq!(items.len(), 2);
    for it in items {
        assert!(matches!(it, ItemRef::Alias(_)));
    }
}

#[test]
fn inline_requires_on_fn_parses() {
    // one bound, single member
    let src = "interface Show { fn show(self) -> str; }\nfn tagged<T requires Show>(x: T) -> str { return \"\"; }\n";
    let (doc, diags) = parse(src, Mode::Impl);
    let ast = doc.ast();
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.root().items(ast);
    let ItemRef::Fn(fh) = items[1] else {
        panic!("expected a fn");
    };
    let f = fh.get(ast);
    assert_eq!(f.bounds.len(), 1);
    assert_eq!(ast.name(f.bounds[0].0), "T");
    let TyRef::Path(t) = f.bounds[0].1 else {
        panic!("expected a path bound");
    };
    assert_eq!(ast.name(t.get(ast).segs[0].name.ident), "Show");
}

#[test]
fn inline_union_bound_and_multiple_params_parse() {
    // a union bound spanning two members, plus an unbounded generic
    let src = "fn f<A, T requires i32 | str, B>(x: T) -> i32 { return 0; }\n";
    let (doc, diags) = parse(src, Mode::Impl);
    let ast = doc.ast();
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.root().items(ast);
    let ItemRef::Fn(fh) = items[0] else {
        panic!("expected a fn");
    };
    let f = fh.get(ast);
    assert_eq!(
        f.generics.len(),
        3,
        "A, T, B all collected: {:?}",
        f.generics
    );
    assert_eq!(f.bounds.len(), 1);
    let TyRef::Union(u) = f.bounds[0].1 else {
        panic!("expected a union bound");
    };
    assert_eq!(u.get(ast).elems.len(), 2);
}

#[test]
fn inline_requires_on_method_parses() {
    let src = "\
interface Enc { fn enc(self) -> bytes; }
impl Widget {
    fn wrap<T requires Enc>(self, x: T) -> bytes { return x.enc(); }
}
";
    let (doc, diags) = parse(src, Mode::Impl);
    let ast = doc.ast();
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.root().items(ast);
    let ItemRef::Impl(ih) = items[1] else {
        panic!("expected an impl block");
    };
    let MethodRef::Method(mh) = ih.get(ast).methods[0] else {
        panic!("expected a method");
    };
    let md = mh.get(ast);
    assert_eq!(md.bounds.len(), 1);
    assert_eq!(ast.name(md.bounds[0].0), "T");
}

#[test]
fn stray_where_is_the_ordinary_error_ladder() {
    // `where` is an ordinary identifier — no tailored skip arm; a
    // stray clause in signature position hits the ordinary expected-X
    // errors and the parse resyncs (termination is the contract)
    let src = "interface D { fn d(self) -> u64; }\nfn load<T>(v: str) -> T where T requires D { panic(\"x\"); }\n";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(
        !diags.is_empty() && !diags.iter().any(|d| d.msg.contains("where clause")),
        "stray where fails through the ordinary ladder: {diags:?}"
    );
    // bodiless position too
    let src2 = "interface D { fn d(self) -> u64; }\ninterface Tr { fn m<T>(self) -> i32 where T requires D; }\n";
    let (_, diags2) = parse(src2, Mode::Impl);
    assert!(
        !diags2.is_empty(),
        "stray where on a method fails: {diags2:?}"
    );
}

#[test]
fn misplaced_bounds_are_rejected() {
    // struct / interface / builtin generics reject `requires` — class
    // generics TAKE them (the admission bounds)
    let cases = [
        (
            "struct S<T requires D> { v: T }",
            "`struct` generic parameters take no `requires` bounds",
            Mode::Impl,
        ),
        (
            "interface Tr<T requires D> { }",
            "`interface` generic parameters take no `requires` bounds",
            Mode::Impl,
        ),
        // the engine surface's own gate (v20: the `builtin trait` row
        // kind is gone — the builtin CLASS carries the check)
        (
            "interface D { fn d(self) -> u64; }\nprelude builtin class Bt<T requires D> { }",
            "`builtin class` generic parameters take no `requires` bounds",
            Mode::Decl,
        ),
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
    let (doc, diags) = parse(src, Mode::Impl);
    let ast = doc.ast();
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.root().items(ast);
    let ItemRef::Class(ch) = items[1] else {
        panic!("expected a class");
    };
    let c = ch.get(ast);
    assert_eq!(c.generics.len(), 2, "K, V both collected: {:?}", c.generics);
    assert_eq!(c.requires.len(), 1);
    assert_eq!(ast.name(c.requires[0].0), "K");
    let TyRef::Path(t) = c.requires[0].1 else {
        panic!("expected a path bound");
    };
    assert_eq!(ast.name(t.get(ast).segs[0].name.ident), "D");
}

#[test]
fn class_union_bound_and_unbounded_params_parse() {
    // a union bound on one param, an unbounded param after it — the
    // list continues through the absorbed bounds like the fn grammar
    let src = "class C<A, T requires i32 | str, B> { v: T; }\n";
    let (doc, diags) = parse(src, Mode::Impl);
    let ast = doc.ast();
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.root().items(ast);
    let ItemRef::Class(ch) = items[0] else {
        panic!("expected a class");
    };
    let c = ch.get(ast);
    assert_eq!(
        c.generics.len(),
        3,
        "A, T, B all collected: {:?}",
        c.generics
    );
    assert_eq!(c.requires.len(), 1);
    let TyRef::Union(u) = c.requires[0].1 else {
        panic!("expected a union bound");
    };
    assert_eq!(u.get(ast).elems.len(), 2);
}

#[test]
fn dumper_renders_class_requires() {
    let src = "interface D { fn d(self) -> u64; }\nclass Box<K requires D, V> { k: K; v: V; }\n";
    let (doc, diags) = parse(src, Mode::Impl);
    let ast = doc.ast();
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let text = AstPrinter::text_of(ast, src);
    assert!(text.contains("Class"), "class node in the dump: {text}");
    assert!(
        text.contains("bounds:"),
        "requires field in the dump: {text}"
    );
}

#[test]
fn dumper_renders_alias_bounds_and_union() {
    let src = "pub type Meters = i64;\ntype Num = i32 | str;\nfn f<T requires i32 | str>(x: T) -> i32 { return 0; }\n";
    let (doc, diags) = parse(src, Mode::Impl);
    let ast = doc.ast();
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let text = AstPrinter::text_of(ast, src);
    assert!(
        text.contains("Alias Meters"),
        "alias node in the dump: {text}"
    );
    assert!(text.contains("TyUnion"), "union node in the dump: {text}");
    assert!(text.contains("bounds:"), "bounds field in the dump: {text}");
    let json = AstPrinter::json_of(ast);
    assert!(json.contains("\"Alias\""), "alias kind on the wire: {json}");
}

#[test]
fn pub_scopes_parse_on_type_alias() {
    let src = "\
pub type A = i64;
pub(super) type C = i64;
pub(pkg) type P = i64;
";
    let (doc, diags) = parse(src, Mode::Impl);
    let ast = doc.ast();
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.root().items(ast);
    let expect = |i: usize| match items[i] {
        ItemRef::Alias(h) => h.get(ast).vis(),
        k => panic!("expected an alias, got {k:?}"),
    };
    assert_eq!(expect(0), Vis::Pub);
    assert_eq!(expect(1), Vis::Super);
    assert_eq!(expect(2), Vis::Pkg);
}

#[test]
fn pub_mod_self_scopes_are_the_ordinary_scope_error() {
    // the repealed paren spellings draw the ordinary pub(..) scope
    // error (one per spelling) and the aliases still parse at the
    // default
    let (doc, diags) = parse(
        "pub(mod) type B = i64;\npub(self) type D = i64;\n",
        Mode::Impl,
    );
    assert_eq!(diags.len(), 2, "one diag per repealed spelling: {diags:?}");
    assert!(
        diags
            .iter()
            .all(|d| d.msg.contains("expected `super` or `pkg` in pub(..)")),
        "each diag is the ordinary scope error: {diags:?}"
    );
    let ast = doc.ast();
    let items = ast.root().items(ast);
    for item in items.iter().take(2) {
        match item {
            ItemRef::Alias(h) => {
                assert_eq!(
                    h.get(ast).vis(),
                    Vis::Self_,
                    "the default survives recovery"
                )
            }
            k => panic!("expected an alias, got {k:?}"),
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
    assert!(!rut_ast::parser::is_reserved_kw("where"));
}
