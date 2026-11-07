//! Parser surface pins for the `if` ATOM — the expression form
//! (`let a = if (cond) { x } else { y };`). The guard mirrors `when`:
//! the parens are required, the statement-leading `if` is the statement
//! grammar's (claimed before the atoms), and the popped statement node's
//! field set is re-minted as `ExprKind::If`.

use rut_ast::ast::*;
use rut_parser::{parse, Mode};

fn fn_body(src: &str) -> (Ast, NodeHandle<BlockNode>) {
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.module_items(ast.root);
    let f = items
        .iter()
        .rfind(|i| matches!(ast.item(**i), ItemKind::Fn(_)))
        .expect("fn");
    let body = match ast.item(*f) {
        ItemKind::Fn(d) => d.body,
        _ => unreachable!(),
    };
    (ast, body)
}

/// the one let's initializer
fn let_init(ast: &Ast, body: NodeHandle<BlockNode>) -> NodeHandle<AnyExpr> {
    let Kind::Stmt(StmtKind::LetStmt { init, .. }) = ast.kind(ast.block(body)[0].id()) else {
        panic!("expected a let");
    };
    *init
}

#[test]
fn let_init_if_is_an_if_expression() {
    let (ast, body) = fn_body("fn f(c: bool) -> i32 { let x = if (c) { 1 } else { 2 }; x }");
    let ExprKind::If { cond, then, els } = ast.expr(let_init(&ast, body)) else {
        panic!("expected an if expression");
    };
    assert!(matches!(ast.expr(*cond), ExprKind::Path { .. }));
    assert!(matches!(
        ast.expr(ast.block_tail(*then).expect("then tail")),
        ExprKind::Lit(Lit::Int(1, _))
    ));
    match els {
        Some(ElseBranch::Block(b)) => assert!(matches!(
            ast.expr(ast.block_tail(*b).expect("else tail")),
            ExprKind::Lit(Lit::Int(2, _))
        )),
        _ => panic!("expected a block else"),
    }
}

#[test]
fn statement_leading_if_stays_a_statement() {
    // StmtFrame dispatches `if` before the atoms — the atom guard never
    // sees it (the `;`-less tail-if shape is phase 1's law; the `0`
    // after the if is the block's TAIL, not a second statement)
    let (ast, body) = fn_body("fn f(c: bool) -> i32 { if (c) { 1 } else { 2 } 0 }");
    assert_eq!(
        ast.block(body).len(),
        1,
        "the if statement; the 0 is the tail"
    );
    assert!(ast.block_tail(body).is_some());
    assert!(matches!(
        ast.kind(ast.block(body)[0].id()),
        Kind::Stmt(StmtKind::If { .. })
    ));
}

#[test]
fn statement_if_trailing_semicolon_is_eaten() {
    // the `;` decides: after the statement form it only ever discards
    // (the statement form keeps no value) — it must not strand
    let (ast, body) = fn_body("fn f(c: bool) -> i32 { if (c) { 1 } else { 2 }; 0 }");
    assert_eq!(
        ast.block(body).len(),
        1,
        "the if statement; no stray `;` statement"
    );
    assert!(ast.block_tail(body).is_some(), "the 0 is the tail");
    assert!(matches!(
        ast.kind(ast.block(body)[0].id()),
        Kind::Stmt(StmtKind::If { .. })
    ));
}

#[test]
fn else_if_chain_in_expr_position() {
    let (ast, body) = fn_body(
        "fn f(a: bool, b: bool) -> i32 { let x = if (a) { 1 } else if (b) { 2 } else { 3 }; x }",
    );
    let ExprKind::If { els: first, .. } = ast.expr(let_init(&ast, body)) else {
        panic!("expected an if expression");
    };
    // the link is the statement grammar's if node — the shared enum
    let Some(ElseBranch::If(h)) = first else {
        panic!("expected an else-if link")
    };
    let Kind::Stmt(StmtKind::If { then, els, .. }) = ast.kind(h.id()) else {
        panic!("the link is an if statement");
    };
    assert!(matches!(
        ast.expr(ast.block_tail(*then).expect("link tail")),
        ExprKind::Lit(Lit::Int(2, _))
    ));
    assert!(els.is_some(), "the chain's last else");
}

#[test]
fn no_else_if_expression() {
    let (ast, body) = fn_body("fn f(c: bool) -> i32 { let x = if (c) { 1 }; x }");
    let ExprKind::If { els, .. } = ast.expr(let_init(&ast, body)) else {
        panic!("expected an if expression");
    };
    assert!(els.is_none());
}

#[test]
fn return_if_expression() {
    let (ast, body) = fn_body("fn f(c: bool) -> i32 { return if (c) { 1 } else { 2 }; }");
    let Kind::Stmt(StmtKind::Return { value: Some(v) }) = ast.kind(ast.block(body)[0].id()) else {
        panic!("expected a return");
    };
    assert!(matches!(ast.expr(*v), ExprKind::If { .. }));
}

#[test]
fn if_expression_in_call_args() {
    // the call is the body's tail; its argument is the atom
    let (ast, body) =
        fn_body("fn g(v: i32) -> i32 { v } fn f(c: bool) -> i32 { g(if (c) { 1 } else { 2 }) }");
    let tail = ast.block_tail(body).expect("the call is the tail");
    let ExprKind::Call { args, .. } = ast.expr(tail) else {
        panic!("expected a call")
    };
    assert_eq!(args.len(), 1);
    assert!(matches!(ast.expr(args[0]), ExprKind::If { .. }));
}

#[test]
fn if_expression_as_binary_operand() {
    let (ast, body) = fn_body("fn f(c: bool) -> i32 { let x = 1 + if (c) { 2 } else { 3 }; x }");
    let ExprKind::Binary { rhs, .. } = ast.expr(let_init(&ast, body)) else {
        panic!("expected a binary expression");
    };
    assert!(matches!(ast.expr(*rhs), ExprKind::If { .. }));
}

#[test]
fn nested_if_expressions() {
    // the outer if is an ATOM (beside `=`); the inner one is
    // statement-leading INSIDE the then block — the phase-1 statement
    // shape (its value is the block's tail at compile time)
    let (ast, body) = fn_body(
        "fn f(a: bool, b: bool) -> i32 { let x = if (a) { if (b) { 1 } else { 2 } } else { 3 }; x }",
    );
    let ExprKind::If { then, .. } = ast.expr(let_init(&ast, body)) else {
        panic!("expected an if expression");
    };
    let Kind::Stmt(StmtKind::If { els, .. }) = ast.kind(ast.block(*then)[0].id()) else {
        panic!("the then arm holds the statement-if");
    };
    assert!(els.is_some(), "the inner if's else");
}

#[test]
fn nested_if_atoms_in_expr_positions() {
    // both levels are atoms: the inner if rides the then block's TAIL
    let (ast, body) = fn_body(
        "fn f(a: bool, b: bool) -> i32 { let x = if (a) { let y = if (b) { 1 } else { 2 }; y } else { 3 }; x }",
    );
    let ExprKind::If { then, .. } = ast.expr(let_init(&ast, body)) else {
        panic!("expected an if expression");
    };
    assert_eq!(ast.block(*then).len(), 1, "the inner let");
    let Kind::Stmt(StmtKind::LetStmt { init, .. }) = ast.kind(ast.block(*then)[0].id()) else {
        panic!("expected the inner let");
    };
    assert!(matches!(ast.expr(*init), ExprKind::If { .. }));
}

#[test]
fn parens_stay_required() {
    // the guard: `if` without `(` never enters the frame — the spelling
    // diagnoses (the struct-literal path is closed to it too: reserved)
    let (_, diags) = parse(
        "fn f(c: bool) -> i32 { let x = if c { 1 } else { 2 }; x }",
        Mode::Impl,
    );
    assert!(!diags.is_empty(), "paren-less `if` must diagnose");
}

#[test]
fn when_guard_shape_untouched() {
    // the when atom beside the new one still parses to a WhenExpr
    let (ast, body) =
        fn_body("fn f(x: i32) -> i32 { let w = when (x) { 0 -> { 1 }, else -> { 2 } }; w }");
    assert!(matches!(
        ast.expr(let_init(&ast, body)),
        ExprKind::WhenExpr { .. }
    ));
}

#[test]
fn if_expression_dump_arm() {
    // the dump mapping covers the new kind end to end (no panic)
    let (ast, _) = fn_body("fn f(c: bool) -> i32 { let x = if (c) { 1 } else { 2 }; x }");
    let tree = rut_ast::dump::to_dump_tree(&ast);
    let text = rut_ast::dump::render_text(&tree, "");
    assert!(text.contains("If"), "the if node is in the dump: {text}");
}
