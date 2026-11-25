//! The matrix gate — for every child kind in every role of the paren
//! law, the formatted output parses clean, its span-free tree equals
//! the input's, and `fmt(out) == out`. This is the lane's referee: a
//! failing cell names an arm the paren lane owes, and the law decides
//! it (a paren is kept iff the bare spelling would not re-parse to
//! the same node in that role).

mod common;

use common::span_free;
use rut_ast::parser::{parse_with_recover, Mode};
use rut_ast::{AstPrinter, Doc, ExprRef, ItemRef};
use rut_fmt::{format, Style};

/// every `ExprRef` variant, named — NO wildcard arm, so a new variant
/// fails to compile here and has to join the matrix
fn kind_of(h: ExprRef) -> &'static str {
    match h {
        ExprRef::Block(_) => "block",
        ExprRef::Lit(_) => "lit",
        ExprRef::Path(_) => "path",
        ExprRef::Self_(_) => "self",
        ExprRef::Call(_) => "call",
        ExprRef::Method(_) => "method",
        ExprRef::Field(_) => "field",
        ExprRef::Index(_) => "index",
        ExprRef::Unary(_) => "unary",
        ExprRef::Binary(_) => "binary",
        ExprRef::Assign(_) => "assign",
        ExprRef::Lambda(_) => "lambda",
        ExprRef::FStr(_) => "fstr",
        ExprRef::Struct(_) => "struct",
        ExprRef::Tuple(_) => "tuple",
        ExprRef::ArrayLit(_) => "array",
        ExprRef::ArrayRepeat(_) => "repeat",
        ExprRef::WhenExpr(_) => "when",
        ExprRef::If(_) => "if",
        ExprRef::Await(_) => "await",
        ExprRef::AsyncBlock(_) => "async",
        ExprRef::Is(_) => "is",
        ExprRef::Cast(_) => "cast",
        ExprRef::Error(_) => "error",
    }
}

/// the number of `ExprRef` variants `kind_of` spells — the table below
/// plus the unspellable kinds must add up to it
const KIND_COUNT: usize = 24;

/// one representative child source per spellable variant; the honesty
/// pass proves each one parses to its claimed kind
const KINDS: &[(&str, &str)] = &[
    ("lit", "1"),
    ("path", "t"),
    ("self", "self"),
    ("call", "f(1)"),
    ("method", "a.m()"),
    ("field", "t.x"),
    ("index", "a[i]"),
    ("unary", "-a"),
    ("binary", "a < b"),
    ("assign", "a = 1"),
    ("lambda", "x => x"),
    ("fstr", "f\"{a}\""),
    ("struct", "P { a: 1 }"),
    ("tuple", "(a, 1)"),
    ("array", "[a, 1]"),
    ("repeat", "[a; 2]"),
    ("when", "when (c) { else -> 1 }"),
    ("if", "if (c) { 1 } else { 2 }"),
    ("await", "await f()"),
    ("async", "async { 1 }"),
    ("is", "a is i64"),
    ("cast", "a as i64"),
];

/// kinds that cannot stand as a `let`-value child at all, so no cell
/// can exist: a BLOCK is only ever a lambda/anon body (the grammar has
/// no `{` atom — `let x = { 1 };` does not parse), and an ERROR node
/// is recovery-only (a clean parse never contains one). Both stay
/// named so the table visibly covers every variant.
const UNSPELLABLE: &[&str] = &["block", "error"];

/// the assign-target grammar accepts only a path, a field or an index
/// (the parser plants the target diag on anything else), so the role's
/// parenthesized input is unclean BY CONSTRUCTION for every other
/// kind — exempt, and the exemption is asserted exact below
const NON_TARGETS: &[&str] = &[
    "lit", "self", "call", "method", "unary", "binary", "assign", "lambda", "fstr", "struct",
    "tuple", "array", "repeat", "when", "if", "await", "async", "is", "cast",
];

/// the side of a binary operator a child sits on
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Lhs,
    Rhs,
}

/// the roles of the paren law — the position a child sits in
#[derive(Clone, Copy)]
enum Role {
    Callee,
    Receiver,
    AssignTarget,
    Bin(&'static str, Side),
    Unary,
    CastOperand,
    IsOperand,
    AwaitOperand,
    AssignValue,
}

/// every role of the law, named — one representative operator per
/// binary level (`||` `&&` `==` `<` `|` `&` `<<` `+` `*`), both sides
fn roles() -> Vec<(&'static str, Role)> {
    let bin_ops: [&'static str; 9] = ["||", "&&", "==", "<", "|", "&", "<<", "+", "*"];
    let mut out: Vec<(&'static str, Role)> = vec![
        ("callee", Role::Callee),
        ("receiver", Role::Receiver),
        ("assign-target", Role::AssignTarget),
        ("unary", Role::Unary),
        ("cast-operand", Role::CastOperand),
        ("is-operand", Role::IsOperand),
        ("await-operand", Role::AwaitOperand),
        ("assign-value", Role::AssignValue),
    ];
    for op in bin_ops {
        out.push(("bin-lhs", Role::Bin(op, Side::Lhs)));
        out.push(("bin-rhs", Role::Bin(op, Side::Rhs)));
    }
    out
}

/// the cell's source: the child, parenthesized, in the role
fn spell(role: Role, child: &str) -> String {
    match role {
        Role::Callee => format!("let r = ({child})(1);\n"),
        Role::Receiver => format!("let r = ({child}).m();\n"),
        Role::AssignTarget => format!("let r = ({child}) = 1;\n"),
        Role::Bin(op, Side::Lhs) => format!("let r = ({child}) {op} 1;\n"),
        Role::Bin(op, Side::Rhs) => format!("let r = 1 {op} ({child});\n"),
        Role::Unary => format!("let r = -({child});\n"),
        Role::CastOperand => format!("let r = ({child}) as i64;\n"),
        Role::IsOperand => format!("let r = ({child}) is i64;\n"),
        Role::AwaitOperand => format!("let r = await ({child});\n"),
        Role::AssignValue => format!("let r = t = ({child});\n"),
    }
}

/// the init expression of the file's first module `let`
fn let_init(doc: &Doc) -> ExprRef {
    let a = doc.ast();
    for item in a.root().items(a) {
        if let ItemRef::ModuleLet(h) = item {
            return h.get(a).init;
        }
    }
    panic!("the probe carries no module `let`");
}

/// the first differing line of two dumps, for a readable failure
fn first_diff(want: &str, got: &str) -> String {
    let w: Vec<&str> = want.lines().collect();
    let g: Vec<&str> = got.lines().collect();
    for i in 0..w.len().max(g.len()) {
        let a = w.get(i).copied().unwrap_or("<eof>");
        let b = g.get(i).copied().unwrap_or("<eof>");
        if a != b {
            return format!("line {i}: `{a}` != `{b}`");
        }
    }
    String::new()
}

#[test]
fn paren_matrix_every_kind_in_every_role() {
    let mode = Mode::Impl;
    // the table names every variant exactly once
    assert_eq!(
        KINDS.len() + UNSPELLABLE.len(),
        KIND_COUNT,
        "the kind table must name every ExprRef variant — a new variant \
         fails to compile in kind_of first, then joins the matrix"
    );
    let targets = ["path", "field", "index"];
    for (kind, _) in KINDS {
        let is_target = targets.contains(kind);
        let exempt = NON_TARGETS.contains(kind);
        assert!(
            is_target ^ exempt,
            "kind `{kind}` must sit in exactly one of {{assign targets, NON_TARGETS}}"
        );
    }
    assert_eq!(
        NON_TARGETS.len(),
        KINDS.len() - targets.len(),
        "NON_TARGETS must exempt exactly the non-target kinds"
    );

    let mut failures: Vec<String> = Vec::new();
    // the honesty pass: each representative parses clean, to its kind
    for (kind, child) in KINDS {
        let src = format!("let r = ({child});\n");
        let doc = parse_with_recover(src, mode);
        if !doc.ast().is_clean() {
            failures.push(format!(
                "{kind}: the representative does not parse clean: {}",
                doc.ast().errors()[0].msg
            ));
            continue;
        }
        let got = kind_of(let_init(&doc));
        if got != *kind {
            failures.push(format!("{kind}: the representative parses to `{got}`"));
        }
    }

    // the cells
    let mut cells = 0;
    for (kind, child) in KINDS {
        if UNSPELLABLE.contains(kind) {
            continue;
        }
        for (role_name, role) in roles() {
            if role_name == "assign-target" && NON_TARGETS.contains(kind) {
                continue;
            }
            cells += 1;
            let src = spell(role, child);
            let name = format!("{kind} x {}", role_name);
            let in_doc = parse_with_recover(src.clone(), mode);
            if !in_doc.ast().is_clean() {
                failures.push(format!(
                    "{name}: the INPUT does not parse clean: {}",
                    in_doc.ast().errors()[0].msg
                ));
                continue;
            }
            let out = match format(&src, mode, &Style::default()) {
                Ok(o) => o,
                Err(ds) => {
                    failures.push(format!("{name}: fmt refused: {}", ds[0].msg));
                    continue;
                }
            };
            let out_doc = parse_with_recover(out.clone(), mode);
            if !out_doc.ast().is_clean() {
                failures.push(format!(
                    "{name}: the OUTPUT does not parse clean (a load-bearing \
                     paren was dropped?): {}",
                    out_doc.ast().errors()[0].msg
                ));
                continue;
            }
            let want = span_free(&AstPrinter::text_of(
                in_doc.ast(),
                in_doc.text(in_doc.span()),
            ));
            let got = span_free(&AstPrinter::text_of(
                out_doc.ast(),
                out_doc.text(out_doc.span()),
            ));
            if got != want {
                failures.push(format!(
                    "{name}: the tree changed under formatting ({})",
                    first_diff(&want, &got)
                ));
                continue;
            }
            let again = match format(&out, mode, &Style::default()) {
                Ok(a) => a,
                Err(ds) => {
                    failures.push(format!("{name}: fmt(fmt) refused: {}", ds[0].msg));
                    continue;
                }
            };
            if again != out {
                failures.push(format!("{name}: NOT idempotent"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "the paren matrix failed on {} of {cells} cells:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
