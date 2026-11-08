//! Parser surface pins for block tail expressions: the `}`-closed
//! last statement becomes the block's tail (no `;`), `let` still
//! always demands its `;`, and the tail law never reaches across a
//! statement boundary.

mod common;

use common::{first_fn_body, parse};
use rut_ast::{Ast, BlockNode, ElseBranch, ExprRef, Handle, Lit, StmtRef, WhenArmNode};
use rut_parser::Mode;

fn fn_body(src: &str) -> (Ast, Handle<BlockNode>) {
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let body = first_fn_body(&ast);
    (ast, body)
}

#[test]
fn bare_last_statement_is_the_tail() {
    // `x` closed by the block's `}` — the tail, not an ExprStmt
    let (ast, body) = fn_body("fn f() -> i32 { x }");
    assert_eq!(
        body.get(&ast).stmts.len(),
        0,
        "no statements: the expr became the tail"
    );
    let ExprRef::Path(ph) = body.get(&ast).tail.expect("tail") else {
        panic!("a path tail, got {:?}", body.get(&ast).tail);
    };
    assert_eq!(ph.get(&ast).segs.len(), 1);
}

#[test]
fn semicolon_keeps_it_a_statement() {
    // `x;` is an ExprStmt — the `;` decides
    let (ast, body) = fn_body("fn f() -> i32 { x; }");
    let b = body.get(&ast);
    assert_eq!(b.stmts.len(), 1);
    assert!(b.tail.is_none());
    assert!(matches!(b.stmts[0], StmtRef::Expr(_)));
}

#[test]
fn tail_after_statements() {
    let (ast, body) = fn_body("fn f() -> i32 { let a = 1; a + a }");
    let b = body.get(&ast);
    assert_eq!(b.stmts.len(), 1, "the let; the tail rides beside it");
    assert!(matches!(b.tail, Some(ExprRef::Binary(_))));
}

#[test]
fn let_still_demands_its_semicolon_before_another_statement() {
    // the tail law is `}`-only: a `;`-less let followed by another
    // statement still diagnoses
    let (_, diags) = parse("fn f() -> i32 { let x = 1 let y = 2; y }", Mode::Impl);
    assert!(
        diags
            .iter()
            .any(|d| d.msg.contains("expected ;") && d.msg.contains("let")),
        "the missing `;` must be diagnosed: {diags:?}"
    );
}

#[test]
fn let_still_demands_its_semicolon_before_the_brace() {
    let (_, diags) = parse("fn f() -> i32 { let x = 1 }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("expected ;")),
        "`let` never becomes a tail: {diags:?}"
    );
}

#[test]
fn statement_let_with_value_tail_afterwards() {
    // `let x = 1;` then the bare tail — the mixed shape compiles clean
    let (ast, body) = fn_body("fn f() -> i32 { let x = 1; x }");
    let b = body.get(&ast);
    assert_eq!(b.stmts.len(), 1);
    assert!(b.tail.is_some());
}

#[test]
fn if_branches_carry_tails_as_blocks() {
    // statement-if with tail arms: the arms are BLOCKS with tails —
    // the statement-leading if is the statement grammar's (the if ATOM
    // claims only expression positions)
    let (ast, body) = fn_body("fn f(c: bool) -> i32 { if (c) { 1 } else { 2 } }");
    let b = body.get(&ast);
    assert_eq!(b.stmts.len(), 1, "the if is the body's one statement");
    let StmtRef::If(ih) = b.stmts[0] else {
        panic!("expected an if statement, got {:?}", b.stmts[0]);
    };
    let i = ih.get(&ast);
    let ExprRef::Lit(lh) = i.then.get(&ast).tail.expect("then tail") else {
        panic!("a lit tail");
    };
    assert!(matches!(lh.get(&ast).lit, Lit::Int(1, _)));
    match i.els {
        Some(ElseBranch::Block(bh)) => {
            let ExprRef::Lit(lh) = bh.get(&ast).tail.expect("else tail") else {
                panic!("a lit tail");
            };
            assert!(matches!(lh.get(&ast).lit, Lit::Int(2, _)));
        }
        ref e => panic!("expected a block else, got {e:?}"),
    }
}

#[test]
fn when_block_arms_carry_tails() {
    let (ast, body) =
        fn_body("fn f(x: i32) -> i32 { let w = when (x) { 0 -> { 1 }, else -> { 2 } }; w }");
    let StmtRef::Let(lh) = body.get(&ast).stmts[0] else {
        panic!("expected the let");
    };
    let ExprRef::WhenExpr(wh) = lh.get(&ast).init else {
        panic!("expected a when expression");
    };
    let tail_of = |arm: Handle<WhenArmNode>| -> Option<ExprRef> {
        match arm.get(&ast).body {
            ExprRef::Block(bh) => bh.get(&ast).tail,
            ref e => panic!("a block arm, got {e:?}"),
        }
    };
    let arms = &wh.get(&ast).arms;
    let ExprRef::Lit(a0) = tail_of(arms[0]).expect("arm 0 tail") else {
        panic!("a lit tail");
    };
    assert!(matches!(a0.get(&ast).lit, Lit::Int(1, _)));
    let ExprRef::Lit(a1) = tail_of(arms[1]).expect("arm 1 tail") else {
        panic!("a lit tail");
    };
    assert!(matches!(a1.get(&ast).lit, Lit::Int(2, _)));
}

#[test]
fn lambda_block_body_takes_a_tail() {
    let (ast, body) = fn_body("fn f() -> i32 { let g: fn() -> i32 = fn () -> i32 { 4 }; g }");
    let StmtRef::Let(lh) = body.get(&ast).stmts[0] else {
        panic!("expected the let");
    };
    let ExprRef::Lambda(lm) = lh.get(&ast).init else {
        panic!("expected a lambda");
    };
    let ExprRef::Block(bh) = lm.get(&ast).body else {
        panic!("block body, got {:?}", lm.get(&ast).body);
    };
    let b = bh.get(&ast);
    assert_eq!(b.stmts.len(), 0);
    let ExprRef::Lit(lh) = b.tail.expect("lambda tail") else {
        panic!("a lit tail");
    };
    assert!(matches!(lh.get(&ast).lit, Lit::Int(4, _)));
}

#[test]
fn expression_statement_then_closing_brace_of_outer_block_is_still_a_tail() {
    // nested: the inner block's tail is the inner `x`; the outer's is
    // the if statement — each `}` closes its own block
    let (ast, body) = fn_body("fn f() -> i32 { if (true) { x } y }");
    let StmtRef::If(ih) = body.get(&ast).stmts[0] else {
        panic!("expected the if statement");
    };
    assert!(
        ih.get(&ast).then.get(&ast).tail.is_some(),
        "the inner block's tail"
    );
    assert!(body.get(&ast).tail.is_some(), "the outer block's tail");
}

#[test]
fn recovery_without_semicolon_still_resyncs() {
    // a broken expr mid-block keeps the recovery law: the incomplete
    // expression diagnoses, the parse ends (no hang, no panic)
    let (_, diags) = parse("fn f() -> i32 { let a = 1 + }", Mode::Impl);
    assert!(!diags.is_empty(), "the broken expr must diagnose");
}
