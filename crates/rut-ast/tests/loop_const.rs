//! `const` in loop headers — the syntax surface: `for (const x of xs)`
//! parses (a fresh per-iteration binding), a for-c header refuses it
//! (the induction variable is genuinely reassigned — the one dedicated
//! diag), the flag rides the AST (`ForOf` `is_const`), the dump shows
//! it, the span stays the whole statement. (The reassign LAW itself is
//! rut-lir's — here the flag only flows.)

mod common;

use common::{last_fn_body, parse};
use rut_ast::parser::Mode;
use rut_ast::{Ast, AstPrinter, StmtRef};

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
fn for_of_takes_const() {
    let src = "fn f(xs: Vec<i32>) -> nil { for (const t of xs) { } }";
    let (ast, s) = first_for(src);
    let StmtRef::ForOf(fh) = s else {
        panic!("expected a for-of, got {s:?}");
    };
    let d = fh.get(&ast);
    assert!(d.is_const, "`const` must set the flag");
    assert_eq!(ast.name(d.var), "t");
    // the span covers the whole statement
    assert_eq!(
        d.span.text(src),
        "for (const t of xs) { }",
        "the exact span law"
    );
}

#[test]
fn for_of_without_const_is_the_default() {
    // mutability is the DEFAULT — a plain `let` header binds mutable
    let src = "fn f(xs: Vec<i32>) -> nil { for (let t of xs) { } }";
    let (ast, s) = first_for(src);
    let StmtRef::ForOf(fh) = s else {
        panic!("expected a for-of, got {s:?}");
    };
    let d = fh.get(&ast);
    assert!(!d.is_const, "no `const`, no flag");
    assert_eq!(ast.name(d.var), "t");
}

#[test]
fn for_c_refuses_const() {
    // the induction variable is reassigned by the loop — the one
    // dedicated parse diag
    let src = "fn f(n: i32) -> nil { for (const i = 0; i < n; i += 1) { } }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(
        diags
            .iter()
            .any(|d| d.msg.contains("reassigned by the loop")),
        "the dedicated diag must fire: {diags:?}"
    );
}

#[test]
fn for_c_plain_header_is_unchanged() {
    let src = "fn f(n: i32) -> nil { for (let i = 0; i < n; i += 1) { } }";
    let (ast, s) = first_for(src);
    let StmtRef::ForC(fh) = s else {
        panic!("expected a for-c, got {s:?}");
    };
    let d = fh.get(&ast);
    assert_eq!(ast.name(d.var), "i");
    // exact spans: the statement ends at its own `}`
    assert_eq!(
        d.span.text(src),
        "for (let i = 0; i < n; i += 1) { }",
        "the exact span law"
    );
}

#[test]
fn dump_shows_the_const_flag_only_when_spelled() {
    let with = "fn f(xs: Vec<i32>) -> nil { for (const t of xs) { } }";
    let (ast, _) = first_for(with);
    let json = AstPrinter::json_of(&ast);
    assert!(json.contains("\"ForOf\""), "{json}");
    assert!(
        json.contains("\"const\""),
        "the dump must carry the flag: {json}"
    );

    let without = "fn f(xs: Vec<i32>) -> nil { for (let t of xs) { } }";
    let (ast, _) = first_for(without);
    let json = AstPrinter::json_of(&ast);
    assert!(json.contains("\"ForOf\""), "{json}");
    assert!(
        !json.contains("\"const\""),
        "no flag without `const`: {json}"
    );
}

#[test]
fn malformed_headers_still_diagnose() {
    // `of` / `=` where the loop variable belongs
    for src in [
        "fn f(xs: Vec<i32>) -> nil { for (let of xs) { } }",
        "fn f(xs: Vec<i32>) -> nil { for (let) { } }",
        "fn f(n: i32) -> nil { for (const = 0; n < 3; n += 1) { } }",
    ] {
        let (_, diags) = parse(src, Mode::Impl);
        assert!(!diags.is_empty(), "`{src}` must diagnose");
    }
}
