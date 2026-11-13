//! `mut` in loop headers — the syntax surface: `for (let mut x of xs)`
//! and `for (let mut i = 0; ..)` parse, the flag rides the AST
//! (`ForOf`/`ForC` `is_mut`), the dump shows it, the span stays the
//! whole statement, and malformed headers still diagnose. (The
//! mut-binding LAW itself is a later phase — here the flag only flows.)

mod common;

use common::{last_fn_body, parse};
use rut_ast::parser::Mode;
use rut_ast::{render_json, to_dump_tree, Ast, ExprRef, Lit, StmtRef};

/// the first `for` statement of the last fn's body
fn first_for(src: &str) -> (Ast, StmtRef) {
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let s = last_fn_body(&ast).get(&ast).stmts[0];
    assert!(
        matches!(s, StmtRef::ForOf(_) | StmtRef::ForC(_)),
        "expected a for statement, got {s:?}"
    );
    (ast, s)
}

#[test]
fn for_of_takes_mut() {
    let src = "fn f(xs: Vec<i32>) -> nil { for (let mut t of xs) { } }";
    let (ast, s) = first_for(src);
    let StmtRef::ForOf(fh) = s else {
        panic!("expected a for-of, got {s:?}");
    };
    let d = fh.get(&ast);
    assert!(d.is_mut, "`mut` must set the flag");
    assert_eq!(ast.name(d.var), "t");
    // the iterable and the body ride untouched
    assert!(matches!(d.iter, ExprRef::Path(_)));
    assert!(d.body.get(&ast).stmts.is_empty());
    // the span covers the whole statement — EXACT now: it ends at the
    // loop's own `}`, not the next token's tail (the v1 span carried
    // the fn's closing brace along)
    assert_eq!(
        d.span.text(src),
        "for (let mut t of xs) { }",
        "the exact span law"
    );
}

#[test]
fn for_of_without_mut_is_unchanged() {
    let src = "fn f(xs: Vec<i32>) -> nil { for (let t of xs) { } }";
    let (ast, s) = first_for(src);
    let StmtRef::ForOf(fh) = s else {
        panic!("expected a for-of, got {s:?}");
    };
    let d = fh.get(&ast);
    assert!(!d.is_mut, "no `mut`, no flag");
    assert_eq!(ast.name(d.var), "t");
}

#[test]
fn for_c_takes_mut() {
    let src = "fn f(n: i32) -> nil { for (let mut i = 0; i < n; i += 1) { } }";
    let (ast, s) = first_for(src);
    let StmtRef::ForC(fh) = s else {
        panic!("expected a for-c, got {s:?}");
    };
    let d = fh.get(&ast);
    assert!(d.is_mut, "`mut` must set the flag");
    assert_eq!(ast.name(d.var), "i");
    let ExprRef::Lit(lh) = d.init else {
        panic!("a lit init, got {:?}", d.init);
    };
    assert!(matches!(lh.get(&ast).lit, Lit::Int(..)));
    assert!(matches!(d.cond, ExprRef::Binary(_)));
    assert!(matches!(d.update, ExprRef::Assign(_)));
    assert!(d.body.get(&ast).stmts.is_empty());
    // exact spans: the statement ends at its own `}`
    assert_eq!(
        d.span.text(src),
        "for (let mut i = 0; i < n; i += 1) { }",
        "the exact span law"
    );
}

#[test]
fn for_c_without_mut_is_unchanged() {
    let src = "fn f(n: i32) -> nil { for (let i = 0; i < n; i += 1) { } }";
    let (ast, s) = first_for(src);
    let StmtRef::ForC(fh) = s else {
        panic!("expected a for-c, got {s:?}");
    };
    let d = fh.get(&ast);
    assert!(!d.is_mut, "no `mut`, no flag");
    assert_eq!(ast.name(d.var), "i");
}

#[test]
fn dump_shows_the_mut_flag_only_when_spelled() {
    let with = "fn f(xs: Vec<i32>) -> nil { for (let mut t of xs) { } }";
    let (ast, _) = first_for(with);
    let json = render_json(&to_dump_tree(&ast));
    assert!(json.contains("\"ForOf\""), "{json}");
    assert!(
        json.contains("\"mut\""),
        "the dump must carry the flag: {json}"
    );

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
