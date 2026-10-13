//! Struct-literal field-init shorthand — `Task { id, .. }` reads as
//! `Task { id: id, .. }`: the parser desugars the bare field name into a
//! single-segment path value spanned over exactly the name token (no
//! shorthand flag in the AST — the value is an ordinary expression
//! downstream, so lowering/checking see a plain variable read).

use rut_ast::ast::*;
use rut_parser::{parse, Mode};

/// the one let's initializer in the last fn's body (clean parse required)
fn let_init(src: &str) -> (Ast, NodeHandle<AnyExpr>) {
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let f = items
        .iter()
        .filter(|i| matches!(ast.item(**i), ItemKind::Fn(_)))
        .next_back()
        .expect("fn");
    let body = match ast.item(*f) {
        ItemKind::Fn(d) => d.body,
        _ => unreachable!(),
    };
    let init: NodeHandle<AnyExpr> = match ast.kind(ast.block(body)[0].id()) {
        Kind::Stmt(StmtKind::LetStmt { init, .. }) => *init,
        _ => panic!("expected a let"),
    };
    (ast, init)
}

/// the (name, value) pairs of a struct literal
fn fields<'a>(ast: &'a Ast, e: NodeHandle<AnyExpr>) -> &'a [(IdentId, NodeHandle<AnyExpr>)] {
    match ast.expr(e) {
        ExprKind::Struct { fields, .. } => fields,
        other => panic!("expected a struct literal, found {other:?}"),
    }
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
    assert_eq!(ast.name(fs[0].0), "id");
    let ExprKind::Path { segs } = ast.expr(fs[0].1) else {
        panic!("the shorthand must desugar to a path, got {:?}", ast.expr(fs[0].1));
    };
    assert_eq!(segs.len(), 1);
    assert_eq!(ast.name(segs[0].name), "id");
    assert!(segs[0].generics.is_empty());
    let sp = ast.span(fs[0].1.id());
    assert_eq!(&src[sp.lo as usize..sp.hi as usize], "id", "the value's span is the name token");
    // the spelled fields are untouched
    assert_eq!(ast.name(fs[1].0), "title");
    assert!(matches!(ast.expr(fs[1].1), ExprKind::Lit(Lit::Str(_))));
    assert_eq!(ast.name(fs[2].0), "completed");
    assert!(matches!(ast.expr(fs[2].1), ExprKind::Lit(Lit::Bool(false))));
}

#[test]
fn self_literal_takes_shorthand() {
    let src = "struct P { x: i32; y: i32 }\n\
               impl P { fn with(x: i32) -> P { let p = Self { x, y: 0 }; p } }";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let ItemKind::Impl { methods, .. } = ast.item(items[1]) else {
        panic!("expected an impl block");
    };
    let md = ast.method_decl(methods[0]);
    let body = md.body.expect("a body");
    let Kind::Stmt(StmtKind::LetStmt { init, .. }) = ast.kind(ast.block(body)[0].id()) else {
        panic!("expected a let");
    };
    let fs = fields(&ast, *init);
    assert_eq!(ast.name(fs[0].0), "x");
    let ExprKind::Path { segs } = ast.expr(fs[0].1) else {
        panic!("the shorthand must desugar to a path");
    };
    assert_eq!(segs.len(), 1);
    assert_eq!(ast.name(segs[0].name), "x");
}

#[test]
fn generic_literal_takes_shorthand() {
    let src = "struct P<T> { v: T; w: i32 }\n\
               fn mk<T>(v: T) -> P<T> { let p = P<T> { v, w: 0 }; p }";
    let (ast, init) = let_init(src);
    let fs = fields(&ast, init);
    assert_eq!(ast.name(fs[0].0), "v");
    let ExprKind::Path { segs } = ast.expr(fs[0].1) else {
        panic!("the shorthand must desugar to a path");
    };
    assert_eq!(segs.len(), 1);
    assert_eq!(ast.name(segs[0].name), "v");
    // the literal head keeps its generic arguments
    let ExprKind::Struct { ty, .. } = ast.expr(init) else {
        unreachable!()
    };
    let TypeKind::TyPath { segs } = ast.ty(*ty) else {
        panic!("expected a path head");
    };
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
    assert!(matches!(ast.expr(fs[0].1), ExprKind::Path { .. }));
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
