//! The postfix non-null assert `e!` — the explicit-deref law's
//! grammar. The matrix rides the postfix loop (beside `.`, `(`, `[`),
//! tightest binding, chaining freely: `f!.x()`, `f!!` (one layer each),
//! `f(x!)`, `!x!` = `Not(assert(x))`, `(a + b)!`, `t![0]`,
//! `t![0] = v`. `!=` maximal-munches separately — no lexer change.

mod common;

use common::{first_fn_body, parse};
use rut_ast::parser::Mode;
use rut_ast::{BinOp, ExprRef, StmtRef, UnOp};

/// the fn's first statement's initializer expression
fn init_of(src: &str) -> (rut_ast::Doc, ExprRef) {
    let (doc, diags) = parse(src, Mode::Impl);
    let ast = doc.ast();
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    match first_fn_body(ast).get(ast).stmts[0] {
        StmtRef::Let(h) => {
            let init = h.get(ast).init;
            (doc, init)
        }
        ref s => panic!("let stmt, got {s:?}"),
    }
}

/// the assert's own span spells the whole form (operand first token
/// through the `!`)
#[test]
fn assert_form_parses_and_spans() {
    let src = "fn f() -> nil { let v = t!; }";
    let (doc, e) = init_of(src);
    let ast = doc.ast();
    let ExprRef::Assert(h) = e else {
        panic!("Assert at top, got {e:?}");
    };
    let a = h.get(ast);
    assert!(matches!(a.expr, ExprRef::Path(_)));
    let lo = src.find("t!").expect("the form") as u32;
    assert_eq!(a.span.lo, lo, "the form starts at the operand's token");
    assert_eq!(a.span.hi, lo + 2, "the form ends at the `!`'s own end");
}

#[test]
fn assert_chains_one_layer_per_bang() {
    // `f!!` — assert(assert(f)): ONE `?` layer per `!`
    let (doc, e) = init_of("fn f() -> nil { let v = t!!; }");
    let ast = doc.ast();
    let ExprRef::Assert(outer) = e else {
        panic!("outer Assert, got {e:?}");
    };
    let ExprRef::Assert(inner) = outer.get(ast).expr else {
        panic!("inner Assert, got {:?}", outer.get(ast).expr);
    };
    assert!(matches!(inner.get(ast).expr, ExprRef::Path(_)));
}

#[test]
fn assert_chains_into_the_postfix_loop() {
    // `f!.x()` — the assert rides BESIDE `.`: the method's receiver is
    // the assert, the loop continues
    let (doc, e) = init_of("fn f() -> nil { let v = t!.m(); }");
    let ast = doc.ast();
    let ExprRef::Method(mh) = e else {
        panic!("Method at top, got {e:?}");
    };
    let m = mh.get(ast);
    assert!(
        matches!(m.recv, ExprRef::Assert(_)),
        "the receiver is the assert, got {:?}",
        m.recv
    );
    let name = m.member.expect("a closed method");
    assert_eq!(ast.name(name.ident), "m");

    // `t![0]` — assert, then index
    let (doc, e) = init_of("fn f() -> nil { let v = t![0]; }");
    let ast = doc.ast();
    let ExprRef::Index(ih) = e else {
        panic!("Index at top, got {e:?}");
    };
    assert!(matches!(ih.get(ast).recv, ExprRef::Assert(_)));

    // `t![0] = v;` — the assert rides the assign CHAIN HEAD
    let (doc, diags) = parse("fn f() -> nil { let v = 0; t![0] = v; }", Mode::Impl);
    let ast = doc.ast();
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let s = first_fn_body(ast).get(ast).stmts[1];
    let StmtRef::Expr(e) = s else {
        panic!("expr stmt, got {s:?}");
    };
    let ExprRef::Assign(ah) = e else {
        panic!("Assign at top, got {e:?}");
    };
    let a = ah.get(ast);
    let ExprRef::Index(ih) = a.target else {
        panic!("Index target, got {:?}", a.target);
    };
    assert!(matches!(ih.get(ast).recv, ExprRef::Assert(_)));
}

#[test]
fn assert_wraps_arguments_and_prefixes_compose() {
    // `f(x!)` — the argument asserts
    let (doc, e) = init_of("fn f() -> nil { let v = f(x!); }");
    let ast = doc.ast();
    let ExprRef::Call(ch) = e else {
        panic!("Call at top, got {e:?}");
    };
    let c = ch.get(ast);
    assert_eq!(c.args.len(), 1);
    assert!(matches!(c.args[0], ExprRef::Assert(_)));

    // `!x!` — Not(assert(x)): the prefix `!` stays UnOp::Not, the
    // postfix `!` wraps INSIDE it (the postfix loop binds tighter)
    let (doc, e) = init_of("fn f() -> nil { let v = !x!; }");
    let ast = doc.ast();
    let ExprRef::Unary(uh) = e else {
        panic!("Unary at top, got {e:?}");
    };
    let u = uh.get(ast);
    assert_eq!(u.op, UnOp::Not);
    assert!(matches!(u.expr, ExprRef::Assert(_)));
}

#[test]
fn assert_operand_binds_the_whole_binary_only_through_a_group() {
    // `a + b!` — `!` binds tighter: the assert wraps `b`, the Add tops
    let (doc, e) = init_of("fn f() -> nil { let v = a + b!; }");
    let ast = doc.ast();
    let ExprRef::Binary(bh) = e else {
        panic!("Add at top, got {e:?}");
    };
    let b = bh.get(ast);
    assert_eq!(b.op, BinOp::Add);
    assert!(
        matches!(b.rhs, ExprRef::Assert(_)),
        "the rhs is the assert, got {:?}",
        b.rhs
    );

    // `(a + b)!` — the group feeds the postfix loop: the assert wraps
    // the WHOLE binary
    let (doc, e) = init_of("fn f() -> nil { let v = (a + b)!; }");
    let ast = doc.ast();
    let ExprRef::Assert(h) = e else {
        panic!("Assert at top, got {e:?}");
    };
    let ExprRef::Binary(bh) = h.get(ast).expr else {
        panic!("the operand is the Add, got {:?}", h.get(ast).expr);
    };
    assert_eq!(bh.get(ast).op, BinOp::Add);
}

/// the paren class's no-parens side, pinned at the grammar: each
/// postfix-family child absorbs a bare `!` into its own postfix loop —
/// `f!`, `t.x!`, `xs[i]!`, `m()!`, `f!!`, `1!`, `t!`, `self!`,
/// `f"{a}"!` re-parse to Assert(child), never to a re-nested tree
/// (`tests/paren_matrix.rs` in rut-fmt pins the keep side per role)
#[test]
fn assert_absorbs_into_postfix_children_bare() {
    let cells: [(&str, &str); 9] = [
        ("call", "f(1)"),
        ("method", "a.m()"),
        ("field", "t.x"),
        ("index", "a[i]"),
        ("lit", "1"),
        ("path", "t"),
        ("self", "self"),
        ("fstr", "f\"{a}\""),
        ("assert", "f!"),
    ];
    for (kind, child) in cells {
        let src = format!("fn f() -> nil {{ let v = {child}!; }}");
        let (doc, diags) = parse(&src, Mode::Impl);
        let ast = doc.ast();
        assert!(
            diags.is_empty(),
            "{kind}: `{child}!` must parse clean: {diags:?}"
        );
        let e = init_of(&src).1;
        let ExprRef::Assert(h) = e else {
            panic!("{kind}: `{child}!` tops at Assert, got {e:?}");
        };
        let inner = h.get(ast).expr;
        let got = match inner {
            ExprRef::Call(_) => "call",
            ExprRef::Method(_) => "method",
            ExprRef::Field(_) => "field",
            ExprRef::Index(_) => "index",
            ExprRef::Lit(_) => "lit",
            ExprRef::Path(_) => "path",
            ExprRef::Self_(_) => "self",
            ExprRef::FStr(_) => "fstr",
            ExprRef::Assert(_) => "assert",
            other => panic!("{kind}: the operand is {other:?}"),
        };
        assert_eq!(got, kind, "`{child}!` re-parsed its operand");
    }
}

/// `!=` maximal-munches separately: a bare `a != b` never reads as an
/// assert — no lexer change, the comparison stays whole
#[test]
fn not_eq_munches_separately() {
    let (doc, e) = init_of("fn f() -> nil { let v = a != b; }");
    let ast = doc.ast();
    let ExprRef::Binary(bh) = e else {
        panic!("Ne at top, got {e:?}");
    };
    assert_eq!(bh.get(ast).op, BinOp::Ne);
}
