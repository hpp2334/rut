//! Struct-literal field-init shorthand — `Task { id, .. }` reads as
//! `Task { id: id, .. }`: the parser desugars the bare field name into a
//! single-segment path value spanned over exactly the name token (no
//! shorthand flag in the AST — the value is an ordinary expression
//! downstream, so lowering/checking see a plain variable read).

mod common;

use common::{last_fn_body, parse};
use rut_ast::parser::Mode;
use rut_ast::{Ast, ExprRef, FieldInit, ItemRef, Lit, MethodRef, StmtRef, TyRef};

/// the one let's initializer in the last fn's body (clean parse required)
fn let_init(src: &str) -> (Ast, ExprRef) {
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    match last_fn_body(&ast).get(&ast).stmts[0] {
        StmtRef::Let(h) => {
            let init = h.get(&ast).init;
            (ast, init)
        }
        ref s => panic!("expected a let, got {s:?}"),
    }
}

/// the field inits of a struct literal
fn fields(ast: &Ast, e: ExprRef) -> &[FieldInit] {
    let ExprRef::Struct(sh) = e else {
        panic!("expected a struct literal, found {e:?}");
    };
    &sh.get(ast).fields
}

#[test]
fn shorthand_desugars_to_a_single_seg_path() {
    let src = "struct Task { id: i32; title: str; completed: bool }\n\
               fn mk(id: i32) -> Task { let t = Task { id, title: \"t\", completed: false }; t }";
    let (ast, init) = let_init(src);
    let fs = fields(&ast, init);
    assert_eq!(fs.len(), 3);
    // the shorthand: `id` carries a single-segment path value whose
    // span is exactly the field-name token
    assert_eq!(ast.name(fs[0].name), "id");
    let ExprRef::Path(ph) = fs[0].value else {
        panic!(
            "the shorthand must desugar to a path, got {:?}",
            fs[0].value
        );
    };
    let segs = &ph.get(&ast).segs;
    assert_eq!(segs.len(), 1);
    assert_eq!(ast.name(segs[0].name), "id");
    assert!(segs[0].generics.is_empty());
    assert_eq!(
        ph.span(&ast).text(src),
        "id",
        "the value's span is the name token"
    );
    assert_eq!(
        segs[0].name_span.text(src),
        "id",
        "the recorded seg span is the name token"
    );
    // the spelled fields are untouched
    assert_eq!(ast.name(fs[1].name), "title");
    let ExprRef::Lit(lh) = fs[1].value else {
        panic!("a str lit, got {:?}", fs[1].value);
    };
    assert!(matches!(lh.get(&ast).lit, Lit::Str(_)));
    assert_eq!(ast.name(fs[2].name), "completed");
    let ExprRef::Lit(bh) = fs[2].value else {
        panic!("a bool lit, got {:?}", fs[2].value);
    };
    assert!(matches!(bh.get(&ast).lit, Lit::Bool(false)));
}

#[test]
fn self_literal_takes_shorthand() {
    let src = "struct P { x: i32; y: i32 }\n\
               impl P { fn with(x: i32) -> P { let p = Self { x, y: 0 }; p } }";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.root().items(&ast);
    let ItemRef::Impl(ih) = items[1] else {
        panic!("expected an impl block, got {:?}", items[1]);
    };
    let MethodRef::Method(mh) = ih.get(&ast).methods[0] else {
        panic!("expected a method");
    };
    let body = mh.get(&ast).body.expect("a body");
    let StmtRef::Let(lh) = body.get(&ast).stmts[0] else {
        panic!("expected a let, got {:?}", body.get(&ast).stmts[0]);
    };
    let fs = fields(&ast, lh.get(&ast).init);
    assert_eq!(ast.name(fs[0].name), "x");
    let ExprRef::Path(ph) = fs[0].value else {
        panic!(
            "the shorthand must desugar to a path, got {:?}",
            fs[0].value
        );
    };
    let segs = &ph.get(&ast).segs;
    assert_eq!(segs.len(), 1);
    assert_eq!(ast.name(segs[0].name), "x");
}

#[test]
fn generic_literal_takes_shorthand() {
    let src = "struct P<T> { v: T; w: i32 }\n\
               fn mk<T>(v: T) -> P<T> { let p = P<T> { v, w: 0 }; p }";
    let (ast, init) = let_init(src);
    let fs = fields(&ast, init);
    assert_eq!(ast.name(fs[0].name), "v");
    let ExprRef::Path(ph) = fs[0].value else {
        panic!(
            "the shorthand must desugar to a path, got {:?}",
            fs[0].value
        );
    };
    assert_eq!(ph.get(&ast).segs.len(), 1);
    assert_eq!(ast.name(ph.get(&ast).segs[0].name), "v");
    // the literal head keeps its generic arguments
    let ExprRef::Struct(sh) = init else {
        unreachable!()
    };
    let TyRef::Path(t) = sh.get(&ast).ty else {
        panic!("expected a path head, got {:?}", sh.get(&ast).ty);
    };
    let segs = &t.get(&ast).segs;
    assert_eq!(ast.name(segs[0].name), "P");
    assert_eq!(segs[0].generics.len(), 1);
}

#[test]
fn trailing_comma_after_shorthand() {
    let src = "struct Task { id: i32; done: bool }\n\
               fn mk(id: i32) -> Task { let t = Task { id, done: false, }; t }";
    let (ast, init) = let_init(src);
    let fs = fields(&ast, init);
    assert_eq!(fs.len(), 2, "the trailing comma adds no field");
    assert!(matches!(fs[0].value, ExprRef::Path(_)));
}

#[test]
fn dotted_field_name_stays_on_the_colon_error() {
    // `Task { id.x }` is not a shorthand — the `field: expr` grammar
    // keeps diagnosing with the clear "expected :"
    let src = "struct Task { id: i32 }\n\
               fn f(id: i32) -> Task { let t = Task { id.x }; t }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("expected :")),
        "`Task {{ id.x }}` must stay on the expected-`:` error: {diags:?}"
    );
}

#[test]
fn bare_name_at_eof_stays_on_the_colon_error() {
    let src = "struct Task { id: i32 }\nfn f() -> Task { let t = Task { id";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("expected :")),
        "`Task {{ id` at EOF must stay on the expected-`:` error: {diags:?}"
    );
}
