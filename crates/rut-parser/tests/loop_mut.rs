//! `mut` in loop headers — the syntax surface: `for (let mut x of xs)`
//! and `for (let mut i = 0; ..)` parse, the flag rides the AST
//! (`ForOf`/`ForC` `is_mut`), the dump shows it, the span stays the
//! whole statement, and malformed headers still diagnose. (The
//! mut-binding LAW itself is a later phase — here the flag only flows.)

use rut_ast::ast::*;
use rut_ast::dump::{render_json, to_dump_tree};
use rut_parser::{parse, Mode};

/// the first `for` statement of the last fn's body
fn first_for(src: &str) -> (Ast, NodeHandle<AnyStmt>) {
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
    let s = ast.block(body)[0];
    assert!(
        matches!(ast.kind(s.id()), Kind::Stmt(StmtKind::ForOf { .. } | StmtKind::ForC { .. })),
        "expected a for statement"
    );
    (ast, s)
}

#[test]
fn for_of_takes_mut() {
    let src = "fn f(xs: Vec<i32>) -> nil { for (let mut t of xs) { } }";
    let (ast, s) = first_for(src);
    let Kind::Stmt(StmtKind::ForOf { var, is_mut, iter, body }) = ast.kind(s.id()) else {
        panic!("expected a for-of");
    };
    assert!(is_mut, "`mut` must set the flag");
    assert_eq!(ast.name(*var), "t");
    // the iterable and the body ride untouched
    assert!(matches!(ast.expr(*iter), ExprKind::Path { .. }));
    assert!(ast.block(*body).is_empty());
    // the span covers the whole statement (statement spans end at the
    // next token, so the fn's own `}` rides along)
    let sp = ast.span(s.id());
    assert_eq!(&src[sp.lo as usize..sp.hi as usize], "for (let mut t of xs) { } }");
}

#[test]
fn for_of_without_mut_is_unchanged() {
    let src = "fn f(xs: Vec<i32>) -> nil { for (let t of xs) { } }";
    let (ast, s) = first_for(src);
    let Kind::Stmt(StmtKind::ForOf { var, is_mut, .. }) = ast.kind(s.id()) else {
        panic!("expected a for-of");
    };
    assert!(!is_mut, "no `mut`, no flag");
    assert_eq!(ast.name(*var), "t");
}

#[test]
fn for_c_takes_mut() {
    let src = "fn f(n: i32) -> nil { for (let mut i = 0; i < n; i += 1) { } }";
    let (ast, s) = first_for(src);
    let Kind::Stmt(StmtKind::ForC { var, is_mut, init, cond, update, body }) = ast.kind(s.id()) else {
        panic!("expected a for-c");
    };
    assert!(is_mut, "`mut` must set the flag");
    assert_eq!(ast.name(*var), "i");
    assert!(matches!(ast.expr(*init), ExprKind::Lit(Lit::Int(..))));
    assert!(matches!(ast.expr(*cond), ExprKind::Binary { .. }));
    assert!(matches!(ast.expr(*update), ExprKind::Assign { .. }));
    assert!(ast.block(*body).is_empty());
    let sp = ast.span(s.id());
    assert_eq!(
        &src[sp.lo as usize..sp.hi as usize],
        "for (let mut i = 0; i < n; i += 1) { } }"
    );
}

#[test]
fn for_c_without_mut_is_unchanged() {
    let src = "fn f(n: i32) -> nil { for (let i = 0; i < n; i += 1) { } }";
    let (ast, s) = first_for(src);
    let Kind::Stmt(StmtKind::ForC { var, is_mut, .. }) = ast.kind(s.id()) else {
        panic!("expected a for-c");
    };
    assert!(!is_mut, "no `mut`, no flag");
    assert_eq!(ast.name(*var), "i");
}

#[test]
fn dump_shows_the_mut_flag_only_when_spelled() {
    let with = "fn f(xs: Vec<i32>) -> nil { for (let mut t of xs) { } }";
    let (ast, _) = first_for(with);
    let json = render_json(&to_dump_tree(&ast));
    assert!(json.contains("\"ForOf\""), "{json}");
    assert!(json.contains("\"mut\""), "the dump must carry the flag: {json}");

    let without = "fn f(xs: Vec<i32>) -> nil { for (let t of xs) { } }";
    let (ast, _) = first_for(without);
    let json = render_json(&to_dump_tree(&ast));
    assert!(json.contains("\"ForOf\""), "{json}");
    assert!(!json.contains("\"mut\""), "no flag without `mut`: {json}");
}

#[test]
fn malformed_headers_still_diagnose() {
    // `of` / `=` where the loop variable belongs
    for src in [
        "fn f(xs: Vec<i32>) -> nil { for (let mut of xs) { } }",
        "fn f(n: i32) -> nil { for (let mut = 0; n < 3; n += 1) { } }",
        "fn f(xs: Vec<i32>) -> nil { for (let mut) { } }",
        // pre-existing shapes stay diagnosed too
        "fn f(xs: Vec<i32>) -> nil { for (let of xs) { } }",
    ] {
        let (_, diags) = parse(src, Mode::Impl);
        assert!(!diags.is_empty(), "`{src}` must diagnose");
    }
}
