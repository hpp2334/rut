//! Associativity and precedence parity with v1's precedence climb
//! (RFC 0030 §4 binding-power table): relational/`is` are
//! non-associative (RFC 0012 §3), assignment is right-associative, and
//! unary/postfix chains compose.

use rut_ast::ast::*;
use rut_parser::{parse, Mode};

fn expr_of(src: &str) -> Ast {
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    ast
}

/// the statement's initializer expression
fn init_of(src: &str) -> NodeHandle<AnyExpr> {
    let ast = expr_of(src);
    let items = ast.module_items(ast.root);
    let f = items.iter().find(|i| matches!(ast.item(**i), ItemKind::Fn(_))).expect("fn");
    let ItemKind::Fn(d) = ast.item(*f) else { unreachable!() };
    let stmts = ast.block(d.body);
    let Kind::Stmt(StmtKind::LetStmt { init, .. }) = ast.kind(stmts[0].id()) else {
        panic!("let stmt");
    };
    NodeHandle::new(init.id())
}

#[test]
fn relational_is_non_associative() {
    // `a < b < c`: v1's parse_rel takes ONE relational and leaves the
    // rest for the caller's expect — the same shape must hold (a diag at
    // the second `<`), never a silently chained tree. v1 then resyncs and
    // reports the stray `<` again as a failed expression statement.
    let (_, diags) = parse("fn f(): unit { let x = 1 < 2 < 3; }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("expected") && d.msg.contains('<')),
        "the second `<` must be left unconsumed: {diags:?}"
    );
}

#[test]
fn is_is_non_associative() {
    let (_, diags) = parse("fn f(): unit { let x = y is A is B; }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("expected") && (d.msg.contains("`is`") || d.msg.contains("is"))),
        "the second `is` must be left unconsumed: {diags:?}"
    );
}

#[test]
fn relational_allowed_across_looser_ops() {
    // `a < b && c < d` — one relational per context, fresh on each side
    let ast = expr_of("fn f(): unit { let x = 1 < 2 && 3 < 4; }");
    let e = init_of("fn f(): unit { let x = 1 < 2 && 3 < 4; }");
    let Kind::Expr(ExprKind::Binary { op: BinOp::And, .. }) = ast.kind(e.id()) else {
        panic!("expected And(..) at the top, got {:?}", ast.kind(e.id()));
    };
}

#[test]
fn relational_blocked_after_tighter_op() {
    // v1: `a < b + c < d` parses (a < (b+c)) and leaves `< d` — a second
    // relational in the same context is refused even across a level-10 op
    let (_, diags) = parse("fn f(): unit { let x = 1 < 2 + 3 < 4; }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains('<')),
        "the second `<` must be left unconsumed: {diags:?}"
    );
}

#[test]
fn assignment_is_right_associative() {
    let ast = expr_of("fn f(): unit { let mut x = 0; let mut y = 0; let mut z = 0; x = y = z; }");
    let items = ast.module_items(ast.root);
    let f = items.iter().find(|i| matches!(ast.item(**i), ItemKind::Fn(_))).expect("fn");
    let ItemKind::Fn(d) = ast.item(*f) else { unreachable!() };
    let stmts = ast.block(d.body);
    let Kind::Stmt(StmtKind::ExprStmt(e)) = ast.kind(stmts[3].id()) else {
        panic!("expr stmt");
    };
    // outer: x = (y = z); inner must also be an Assign
    let Kind::Expr(ExprKind::Assign { target: outer_t, value: outer_v, .. }) = ast.kind(e.id()) else {
        panic!("outer Assign, got {:?}", ast.kind(e.id()));
    };
    let Kind::Expr(ExprKind::Assign { target: inner_t, .. }) = ast.kind(outer_v.id()) else {
        panic!("inner Assign, got {:?}", ast.kind(outer_v.id()));
    };
    let Kind::Expr(ExprKind::Path { segs }) = ast.kind(outer_t.id()) else { panic!("x") };
    assert_eq!(ast.name(segs[0].name), "x");
    let Kind::Expr(ExprKind::Path { segs }) = ast.kind(inner_t.id()) else { panic!("y") };
    assert_eq!(ast.name(segs[0].name), "y");
}

#[test]
fn invalid_assignment_target_diagnoses() {
    let (_, diags) = parse("fn f(): unit { 1 + 2 = 3; }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("invalid assignment target")),
        "want the target diag: {diags:?}"
    );
}

#[test]
fn precedence_shapes() {
    // `1 + 2 * 3` → Add(1, Mul(2, 3))
    let src = "fn f(): unit { let x = 1 + 2 * 3; }";
    let ast = expr_of(src);
    let e = init_of(src);
    let Kind::Expr(ExprKind::Binary { op: BinOp::Add, rhs, .. }) = ast.kind(e.id()) else {
        panic!("Add at top, got {:?}", ast.kind(e.id()));
    };
    let Kind::Expr(ExprKind::Binary { op: BinOp::Mul, .. }) = ast.kind(rhs.id()) else {
        panic!("Mul on the right, got {:?}", ast.kind(rhs.id()));
    };

    // `1 * 2 + 3` → Add(Mul(1, 2), 3)
    let src = "fn f(): unit { let x = 1 * 2 + 3; }";
    let ast = expr_of(src);
    let e = init_of(src);
    let Kind::Expr(ExprKind::Binary { op: BinOp::Add, lhs, .. }) = ast.kind(e.id()) else {
        panic!("Add at top, got {:?}", ast.kind(e.id()));
    };
    let Kind::Expr(ExprKind::Binary { op: BinOp::Mul, .. }) = ast.kind(lhs.id()) else {
        panic!("Mul on the left, got {:?}", ast.kind(lhs.id()));
    };

    // `a || b && c` → Or(a, And(b, c)); `a && b || c` → Or(And(a, b), c)
    let src = "fn f(): unit { let x = a || b && c; }";
    let ast = expr_of(src);
    let e = init_of(src);
    let Kind::Expr(ExprKind::Binary { op: BinOp::Or, rhs, .. }) = ast.kind(e.id()) else {
        panic!("Or at top, got {:?}", ast.kind(e.id()));
    };
    let Kind::Expr(ExprKind::Binary { op: BinOp::And, .. }) = ast.kind(rhs.id()) else {
        panic!("And on the right, got {:?}", ast.kind(rhs.id()));
    };
}

#[test]
fn prefix_and_postfix_compose() {
    // `-a.b` → Unary(Neg, Path[a.b]) — dotted paths are one Path node;
    // the unary wraps whatever the operand grammar built (v1 parity)
    let src = "fn f(): unit { let x = -a.b; }";
    let ast = expr_of(src);
    let e = init_of(src);
    let Kind::Expr(ExprKind::Unary { op: UnOp::Neg, expr, .. }) = ast.kind(e.id()) else {
        panic!("Unary at top, got {:?}", ast.kind(e.id()));
    };
    let Kind::Expr(ExprKind::Path { segs }) = ast.kind(expr.id()) else {
        panic!("Path under the unary, got {:?}", ast.kind(expr.id()));
    };
    assert_eq!(segs.len(), 2);

    // postfix `.` on a non-path atom IS a Field node: `-(x).b`
    let src = "fn f(): unit { let x = -(x).b; }";
    let ast = expr_of(src);
    let e = init_of(src);
    let Kind::Expr(ExprKind::Unary { op: UnOp::Neg, expr, .. }) = ast.kind(e.id()) else {
        panic!("Unary at top, got {:?}", ast.kind(e.id()));
    };
    let Kind::Expr(ExprKind::Field { .. }) = ast.kind(expr.id()) else {
        panic!("Field under the unary, got {:?}", ast.kind(expr.id()));
    };

    // `- - x` chains prefix ops
    let src = "fn f(): unit { let x = - -1; }";
    expr_of(src);
}
