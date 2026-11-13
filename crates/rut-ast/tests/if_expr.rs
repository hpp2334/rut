//! Parser surface pins for the `if` ATOM — the expression form
//! (`let a = if (cond) { x } else { y };`). The guard mirrors `when`:
//! the parens are required, the statement-leading `if` is the statement
//! grammar's (claimed before the atoms), and the popped statement node's
//! field set is re-minted as the if EXPRESSION node.

mod common;

use common::{last_fn_body, parse};
use rut_ast::parser::Mode;
use rut_ast::{
    render_text, to_dump_tree, Ast, BlockNode, ElseBranch, ExprRef, Handle, Lit, StmtRef,
};

fn fn_body(src: &str) -> (Ast, Handle<BlockNode>) {
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let body = last_fn_body(&ast);
    (ast, body)
}

/// the one let's initializer
fn let_init(ast: &Ast, body: Handle<BlockNode>) -> ExprRef {
    let StmtRef::Let(h) = body.get(ast).stmts[0] else {
        panic!("expected a let, got {:?}", body.get(ast).stmts[0]);
    };
    h.get(ast).init
}

#[test]
fn let_init_if_is_an_if_expression() {
    let (ast, body) = fn_body("fn f(c: bool) -> i32 { let x = if (c) { 1 } else { 2 }; x }");
    let ExprRef::If(ih) = let_init(&ast, body) else {
        panic!("expected an if expression");
    };
    let i = ih.get(&ast);
    assert!(matches!(i.cond, ExprRef::Path(_)));
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
fn statement_leading_if_stays_a_statement() {
    // StmtFrame dispatches `if` before the atoms — the atom guard never
    // sees it (the `;`-less tail-if shape is phase 1's law; the `0`
    // after the if is the block's TAIL, not a second statement)
    let (ast, body) = fn_body("fn f(c: bool) -> i32 { if (c) { 1 } else { 2 } 0 }");
    let b = body.get(&ast);
    assert_eq!(b.stmts.len(), 1, "the if statement; the 0 is the tail");
    assert!(b.tail.is_some());
    assert!(matches!(b.stmts[0], StmtRef::If(_)));
}

#[test]
fn statement_if_trailing_semicolon_is_eaten() {
    // the `;` decides: after the statement form it only ever discards
    // (the statement form keeps no value) — it must not strand
    let (ast, body) = fn_body("fn f(c: bool) -> i32 { if (c) { 1 } else { 2 }; 0 }");
    let b = body.get(&ast);
    assert_eq!(b.stmts.len(), 1, "the if statement; no stray `;` statement");
    assert!(b.tail.is_some(), "the 0 is the tail");
    assert!(matches!(b.stmts[0], StmtRef::If(_)));
}

#[test]
fn else_if_chain_in_expr_position() {
    let (ast, body) = fn_body(
        "fn f(a: bool, b: bool) -> i32 { let x = if (a) { 1 } else if (b) { 2 } else { 3 }; x }",
    );
    let ExprRef::If(ih) = let_init(&ast, body) else {
        panic!("expected an if expression");
    };
    // the link is the statement grammar's if node — the shared enum
    let Some(ElseBranch::If(link)) = ih.get(&ast).els else {
        panic!("expected an else-if link");
    };
    let l = link.get(&ast);
    let ExprRef::Lit(lh) = l.then.get(&ast).tail.expect("link tail") else {
        panic!("a lit tail");
    };
    assert!(matches!(lh.get(&ast).lit, Lit::Int(2, _)));
    assert!(l.els.is_some(), "the chain's last else");
}

#[test]
fn no_else_if_expression() {
    let (ast, body) = fn_body("fn f(c: bool) -> i32 { let x = if (c) { 1 }; x }");
    let ExprRef::If(ih) = let_init(&ast, body) else {
        panic!("expected an if expression");
    };
    assert!(ih.get(&ast).els.is_none());
}

#[test]
fn return_if_expression() {
    let (ast, body) = fn_body("fn f(c: bool) -> i32 { return if (c) { 1 } else { 2 }; }");
    let StmtRef::Return(rh) = body.get(&ast).stmts[0] else {
        panic!("expected a return, got {:?}", body.get(&ast).stmts[0]);
    };
    let Some(v) = rh.get(&ast).value else {
        panic!("a valued return");
    };
    assert!(matches!(v, ExprRef::If(_)));
}

#[test]
fn if_expression_in_call_args() {
    // the call is the body's tail; its argument is the atom
    let (ast, body) =
        fn_body("fn g(v: i32) -> i32 { v } fn f(c: bool) -> i32 { g(if (c) { 1 } else { 2 }) }");
    let tail = body.get(&ast).tail.expect("the call is the tail");
    let ExprRef::Call(ch) = tail else {
        panic!("expected a call, got {tail:?}");
    };
    let args = &ch.get(&ast).args;
    assert_eq!(args.len(), 1);
    assert!(matches!(args[0], ExprRef::If(_)));
}

#[test]
fn if_expression_as_binary_operand() {
    let (ast, body) = fn_body("fn f(c: bool) -> i32 { let x = 1 + if (c) { 2 } else { 3 }; x }");
    let ExprRef::Binary(bh) = let_init(&ast, body) else {
        panic!("expected a binary expression");
    };
    assert!(matches!(bh.get(&ast).rhs, ExprRef::If(_)));
}

#[test]
fn nested_if_expressions() {
    // the outer if is an ATOM (beside `=`); the inner one is
    // statement-leading INSIDE the then block — the phase-1 statement
    // shape (its value is the block's tail at compile time)
    let (ast, body) = fn_body(
        "fn f(a: bool, b: bool) -> i32 { let x = if (a) { if (b) { 1 } else { 2 } } else { 3 }; x }",
    );
    let ExprRef::If(ih) = let_init(&ast, body) else {
        panic!("expected an if expression");
    };
    let StmtRef::If(inner) = ih.get(&ast).then.get(&ast).stmts[0] else {
        panic!("the then arm holds the statement-if");
    };
    assert!(inner.get(&ast).els.is_some(), "the inner if's else");
}

#[test]
fn nested_if_atoms_in_expr_positions() {
    // both levels are atoms: the inner if rides the then block's TAIL
    let (ast, body) = fn_body(
        "fn f(a: bool, b: bool) -> i32 { let x = if (a) { let y = if (b) { 1 } else { 2 }; y } else { 3 }; x }",
    );
    let ExprRef::If(ih) = let_init(&ast, body) else {
        panic!("expected an if expression");
    };
    let then = ih.get(&ast).then.get(&ast);
    assert_eq!(then.stmts.len(), 1, "the inner let");
    let StmtRef::Let(lh) = then.stmts[0] else {
        panic!("expected the inner let");
    };
    assert!(matches!(lh.get(&ast).init, ExprRef::If(_)));
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
    assert!(matches!(let_init(&ast, body), ExprRef::WhenExpr(_)));
}

#[test]
fn if_expression_dump_arm() {
    // the dump mapping covers the new kind end to end (no panic)
    let (ast, _) = fn_body("fn f(c: bool) -> i32 { let x = if (c) { 1 } else { 2 }; x }");
    let tree = to_dump_tree(&ast);
    let text = render_text(&tree, "");
    assert!(text.contains("If"), "the if node is in the dump: {text}");
}
