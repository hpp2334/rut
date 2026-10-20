//! Parser surface pins for block tail expressions: the `}`-closed
//! last statement becomes the block's tail (no `;`), `let` still
//! always demands its `;`, and the tail law never reaches across a
//! statement boundary.

use rut_ast::ast::*;
use rut_parser::{parse, Mode};

fn fn_body(src: &str) -> (Ast, NodeHandle<BlockNode>) {
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let f = items
        .iter()
        .find(|i| matches!(ast.item(**i), ItemKind::Fn(_)))
        .expect("fn");
    let body = match ast.item(*f) {
        ItemKind::Fn(d) => d.body,
        _ => unreachable!(),
    };
    (ast, body)
}

#[test]
fn bare_last_statement_is_the_tail() {
    // `x` closed by the block's `}` — the tail, not an ExprStmt
    let (ast, body) = fn_body("fn f() -> i32 { x }");
    assert_eq!(
        ast.block(body).len(),
        0,
        "no statements: the expr became the tail"
    );
    let tail = ast.block_tail(body).expect("tail");
    assert!(matches!(ast.expr(tail), ExprKind::Path { segs } if segs.len() == 1));
}

#[test]
fn semicolon_keeps_it_a_statement() {
    // `x;` is an ExprStmt — the `;` decides
    let (ast, body) = fn_body("fn f() -> i32 { x; }");
    assert_eq!(ast.block(body).len(), 1);
    assert!(ast.block_tail(body).is_none());
    let Kind::Stmt(StmtKind::ExprStmt(_)) = ast.kind(ast.block(body)[0].id()) else {
        panic!("expected an ExprStmt");
    };
}

#[test]
fn tail_after_statements() {
    let (ast, body) = fn_body("fn f() -> i32 { let a = 1; a + a }");
    assert_eq!(
        ast.block(body).len(),
        1,
        "the let; the tail rides beside it"
    );
    let tail = ast.block_tail(body).expect("tail");
    assert!(matches!(ast.expr(tail), ExprKind::Binary { .. }));
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
    assert_eq!(ast.block(body).len(), 1);
    assert!(ast.block_tail(body).is_some());
}

#[test]
fn if_branches_carry_tails_as_blocks() {
    // statement-if with tail arms: the arms are BLOCKS with tails —
    // the statement-leading if is the statement grammar's (the if ATOM
    // claims only expression positions)
    let (ast, body) = fn_body("fn f(c: bool) -> i32 { if (c) { 1 } else { 2 } }");
    assert_eq!(
        ast.block(body).len(),
        1,
        "the if is the body's one statement"
    );
    let Kind::Stmt(StmtKind::If { then, els, .. }) = ast.kind(ast.block(body)[0].id()) else {
        panic!("expected an if statement");
    };
    assert!(matches!(
        ast.expr(ast.block_tail(*then).expect("then tail")),
        ExprKind::Lit(Lit::Int(1, _))
    ));
    match els {
        Some(ElseBranch::Block(b)) => {
            assert!(matches!(
                ast.expr(ast.block_tail(*b).expect("else tail")),
                ExprKind::Lit(Lit::Int(2, _))
            ));
        }
        _ => panic!("expected a block else"),
    }
}

#[test]
fn when_block_arms_carry_tails() {
    let (ast, body) =
        fn_body("fn f(x: i32) -> i32 { let w = when (x) { 0 -> { 1 }, else -> { 2 } }; w }");
    let Kind::Stmt(StmtKind::LetStmt { init, .. }) = ast.kind(ast.block(body)[0].id()) else {
        panic!("expected the let");
    };
    let ExprKind::WhenExpr { arms, .. } = ast.expr(*init) else {
        panic!("expected a when expression");
    };
    let bodies: Vec<Option<NodeHandle<AnyExpr>>> = arms
        .iter()
        .map(|&a| match ast.arm(a) {
            ArmKind::WhenArm { body, .. } => {
                ast.narrow_block(*body).and_then(|b| ast.block_tail(b))
            }
        })
        .collect();
    assert!(matches!(
        ast.expr(bodies[0].expect("arm 0 tail")),
        ExprKind::Lit(Lit::Int(1, _))
    ));
    assert!(matches!(
        ast.expr(bodies[1].expect("arm 1 tail")),
        ExprKind::Lit(Lit::Int(2, _))
    ));
}

#[test]
fn lambda_block_body_takes_a_tail() {
    let (ast, body) = fn_body("fn f() -> i32 { let g: fn() -> i32 = fn () -> i32 { 4 }; g }");
    let Kind::Stmt(StmtKind::LetStmt { init, .. }) = ast.kind(ast.block(body)[0].id()) else {
        panic!("expected the let");
    };
    let ExprKind::Lambda { body: lb, .. } = ast.expr(*init) else {
        panic!("expected a lambda");
    };
    let b = ast.narrow_block(*lb).expect("block body");
    assert_eq!(ast.block(b).len(), 0);
    assert!(matches!(
        ast.expr(ast.block_tail(b).expect("lambda tail")),
        ExprKind::Lit(Lit::Int(4, _))
    ));
}

#[test]
fn expression_statement_then_closing_brace_of_outer_block_is_still_a_tail() {
    // nested: the inner block's tail is the inner `x`; the outer's is
    // the if statement — each `}` closes its own block
    let (ast, body) = fn_body("fn f() -> i32 { if (true) { x } y }");
    let Kind::Stmt(StmtKind::If { then, .. }) = ast.kind(ast.block(body)[0].id()) else {
        panic!("expected the if statement");
    };
    assert!(ast.block_tail(*then).is_some(), "the inner block's tail");
    assert!(ast.block_tail(body).is_some(), "the outer block's tail");
}

#[test]
fn recovery_without_semicolon_still_resyncs() {
    // a broken expr mid-block keeps the recovery law: the incomplete
    // expression diagnoses, the parse ends (no hang, no panic)
    let (_, diags) = parse("fn f() -> i32 { let a = 1 + }", Mode::Impl);
    assert!(!diags.is_empty(), "the broken expr must diagnose");
}
