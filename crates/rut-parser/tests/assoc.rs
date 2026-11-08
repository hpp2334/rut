//! Associativity and precedence parity with v1's precedence climb
//! (the binding-power table): relational/`is` are
//! non-associative, assignment is right-associative, and
//! unary/postfix chains compose.

mod common;

use common::{first_fn_body, parse};
use rut_ast::{BinOp, ExprRef, StmtRef, UnOp};
use rut_parser::Mode;

/// the fn's first statement's initializer expression
fn init_of(src: &str) -> (rut_ast::Ast, ExprRef) {
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    match first_fn_body(&ast).get(&ast).stmts[0] {
        StmtRef::Let(h) => {
            let init = h.get(&ast).init;
            (ast, init)
        }
        ref s => panic!("let stmt, got {s:?}"),
    }
}

#[test]
fn relational_is_non_associative() {
    // `a < b < c`: v1's parse_rel takes ONE relational and leaves the
    // rest for the caller's expect — the same shape must hold (a diag at
    // the second `<`), never a silently chained tree. v1 then resyncs and
    // reports the stray `<` again as a failed expression statement.
    let (_, diags) = parse("fn f() -> nil { let x = 1 < 2 < 3; }", Mode::Impl);
    assert!(
        diags
            .iter()
            .any(|d| d.msg.contains("expected") && d.msg.contains('<')),
        "the second `<` must be left unconsumed: {diags:?}"
    );
}

#[test]
fn is_is_non_associative() {
    let (_, diags) = parse("fn f() -> nil { let x = y is A is B; }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("expected") && (d.msg.contains("`is`") || d.msg.contains("is"))),
        "the second `is` must be left unconsumed: {diags:?}"
    );
}

#[test]
fn relational_allowed_across_looser_ops() {
    // `a < b && c < d` — one relational per context, fresh on each side
    let (ast, e) = init_of("fn f() -> nil { let x = 1 < 2 && 3 < 4; }");
    let ExprRef::Binary(b) = e else {
        panic!("expected And(..) at the top, got {e:?}");
    };
    assert_eq!(b.get(&ast).op, BinOp::And);
}

#[test]
fn relational_blocked_after_tighter_op() {
    // v1: `a < b + c < d` parses (a < (b+c)) and leaves `< d` — a second
    // relational in the same context is refused even across a level-10 op
    let (_, diags) = parse("fn f() -> nil { let x = 1 < 2 + 3 < 4; }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains('<')),
        "the second `<` must be left unconsumed: {diags:?}"
    );
}

#[test]
fn assignment_is_right_associative() {
    let src = "fn f() -> nil { let mut x = 0; let mut y = 0; let mut z = 0; x = y = z; }";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let stmts = &first_fn_body(&ast).get(&ast).stmts;
    let StmtRef::Expr(outer) = stmts[3] else {
        panic!("expr stmt, got {:?}", stmts[3]);
    };
    // outer: x = (y = z); inner must also be an Assign
    let ExprRef::Assign(oh) = outer else {
        panic!("outer Assign, got {outer:?}");
    };
    let o = oh.get(&ast);
    let ExprRef::Assign(ih) = o.value else {
        panic!("inner Assign, got {:?}", o.value);
    };
    let ExprRef::Path(pt) = o.target else {
        panic!("x, got {:?}", o.target);
    };
    assert_eq!(ast.name(pt.get(&ast).segs[0].name), "x");
    let ExprRef::Path(it) = ih.get(&ast).target else {
        panic!("y, got {:?}", ih.get(&ast).target);
    };
    assert_eq!(ast.name(it.get(&ast).segs[0].name), "y");
}

#[test]
fn invalid_assignment_target_diagnoses() {
    let (_, diags) = parse("fn f() -> nil { 1 + 2 = 3; }", Mode::Impl);
    assert!(
        diags
            .iter()
            .any(|d| d.msg.contains("invalid assignment target")),
        "want the target diag: {diags:?}"
    );
}

#[test]
fn precedence_shapes() {
    // `1 + 2 * 3` → Add(1, Mul(2, 3))
    let (ast, e) = init_of("fn f() -> nil { let x = 1 + 2 * 3; }");
    let ExprRef::Binary(bh) = e else {
        panic!("Add at top, got {e:?}");
    };
    let b = bh.get(&ast);
    assert_eq!(b.op, BinOp::Add);
    let ExprRef::Binary(rh) = b.rhs else {
        panic!("Mul on the right, got {:?}", b.rhs);
    };
    assert_eq!(rh.get(&ast).op, BinOp::Mul);

    // `1 * 2 + 3` → Add(Mul(1, 2), 3)
    let (ast, e) = init_of("fn f() -> nil { let x = 1 * 2 + 3; }");
    let ExprRef::Binary(bh) = e else {
        panic!("Add at top, got {e:?}");
    };
    let b = bh.get(&ast);
    assert_eq!(b.op, BinOp::Add);
    let ExprRef::Binary(lh) = b.lhs else {
        panic!("Mul on the left, got {:?}", b.lhs);
    };
    assert_eq!(lh.get(&ast).op, BinOp::Mul);

    // `a || b && c` → Or(a, And(b, c)); `a && b || c` → Or(And(a, b), c)
    let (ast, e) = init_of("fn f() -> nil { let x = a || b && c; }");
    let ExprRef::Binary(bh) = e else {
        panic!("Or at top, got {e:?}");
    };
    let b = bh.get(&ast);
    assert_eq!(b.op, BinOp::Or);
    let ExprRef::Binary(rh) = b.rhs else {
        panic!("And on the right, got {:?}", b.rhs);
    };
    assert_eq!(rh.get(&ast).op, BinOp::And);
}

#[test]
fn prefix_and_postfix_compose() {
    // `-a.b` → Unary(Neg, Field{recv: Path[a]}) — the postfix `.` is
    // value access now: the path closes at `a`, the field chains as
    // the postfix loop's Field node; the unary wraps whatever the
    // operand grammar built
    let (ast, e) = init_of("fn f() -> nil { let x = -a.b; }");
    let ExprRef::Unary(uh) = e else {
        panic!("Unary at top, got {e:?}");
    };
    let u = uh.get(&ast);
    assert_eq!(u.op, UnOp::Neg);
    assert!(matches!(u.expr, ExprRef::Field(_)));

    // postfix `.` on a non-path atom IS a Field node: `-(x).b`
    let (ast, e) = init_of("fn f() -> nil { let x = -(x).b; }");
    let ExprRef::Unary(uh) = e else {
        panic!("Unary at top, got {e:?}");
    };
    assert!(matches!(uh.get(&ast).expr, ExprRef::Field(_)));

    // `- - x` chains prefix ops
    let (_, diags) = parse("fn f() -> nil { let x = - -1; }", Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
}
