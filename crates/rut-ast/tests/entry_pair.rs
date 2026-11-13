//! The entry pair's own gate — the pinned law fixtures from the plan
//! (ast-v2-typed-arenas.md, P1): the recovery ladder's three homes,
//! the flat/positional error views, fail-fast, docs-at-allocation,
//! and the span×src join.

use rut_ast::parser::{parse, parse_with_recover, Mode, ParseOpts, Parsed};
use rut_ast::{ExprRef, FieldRef, ItemRef, MemberRef, ParamRef, StmtRef};

const IMPL: Mode = Mode::Impl;

#[test]
fn fail_fast_is_err_first_diag_no_tree() {
    // clean source: Ok, and Ok implies clean
    let p = parse(
        "fn main() -> nil {\n}\n".to_string(),
        IMPL,
        ParseOpts::default(),
    )
    .unwrap();
    assert!(p.ast().is_clean());

    // broken source: Err IS the first diag
    let err = match parse(
        "fn main( -> nil {\n    let = ;\n}\n".to_string(),
        IMPL,
        ParseOpts::default(),
    ) {
        Err(d) => d,
        Ok(_) => panic!("fail-fast must stop at the first diag"),
    };
    assert!(!err.msg.is_empty());
}

// ---- the FENCED fixture: identity damage + the lex tier ----

#[test]
fn fenced_fixture_three_homes() {
    let src = concat!(
        "use calc::{ sqrt };\n",
        "\n",
        "entry fn main() -> nil {\n",
        "    let y = sqrt(2.0);\n",
        "    let = ;\n", // E1: broken STMT inside a good fn
        "    let z = 1;\n",
        "}\n",
        "\n",
        "fn ( -> i64 {\n", // E2: nameless fn — identity damage
        "    return 0;\n",
        "}\n",
        "\n",
        "/* never closed\n", // E3: LEX — unterminated comment
    );
    let p = parse_with_recover(src.to_string(), IMPL, ParseOpts::default());
    let ast = p.ast();

    assert!(!ast.is_clean());
    // the flat view carries every row, lex first
    let msgs: Vec<&str> = ast.errors().iter().map(|d| d.msg.as_str()).collect();
    assert!(
        msgs.iter()
            .any(|m| m.contains("unterminated block comment")),
        "lex row present: {msgs:?}"
    );
    assert!(
        msgs.iter().any(|m| m.contains("expected a function name")),
        "the nameless fn's row: {msgs:?}"
    );

    // positional view: the three homes
    let items = ast.root().items(ast);
    let kinds: Vec<&str> = items
        .iter()
        .map(|i| match i {
            ItemRef::Use(_) => "use",
            ItemRef::Fn(_) => "fn",
            ItemRef::Error(_) => "error",
            _ => "other",
        })
        .collect();
    // [0] the pre-seeded lex row, then the salvage, then the fences:
    // the nameless fn AND the stray `}` it stranded are two fences
    assert_eq!(
        kinds,
        vec!["error", "use", "fn", "error", "error"],
        "{kinds:?}"
    );

    // the good fn was salvaged WHOLE — its broken stmt is an in-block hole
    let ItemRef::Fn(fh) = items[2] else { panic!() };
    let f = fh.get(ast);
    let stmt_kinds: Vec<&str> = f
        .body
        .get(ast)
        .stmts
        .iter()
        .map(|s| match s {
            StmtRef::Let(_) => "let",
            StmtRef::Error(_) => "error",
            _ => "other",
        })
        .collect();
    assert_eq!(stmt_kinds, vec!["let", "error", "let"], "{stmt_kinds:?}");

    // the nameless fn's ErrorNode: span covers the region, diags ride
    let ItemRef::Error(eh) = items[3] else {
        panic!()
    };
    let e = eh.get(ast);
    assert!(!e.diags.is_empty());
    assert!(
        e.span.text(p.src()).starts_with("fn ("),
        "the fence covers the whole construct: {:?}",
        e.span.text(p.src())
    );

    // the lex row's root-level home: pre-seeded, span at EOF
    let ItemRef::Error(lx) = items[0] else {
        panic!()
    };
    assert!(lx.get(ast).span.text(p.src()).contains("never closed"));
}

// ---- the RECOVERED fixture: part damage → member slots ----

#[test]
fn recovered_fixture_member_slots() {
    let src = concat!(
        "fn f(x: , y: i64) -> i64 {\n", // broken PARAM beside a good one
        "    return y;\n",
        "}\n",
        "\n",
        "class Pt {\n",
        "    x: i64;\n",
        "    :: f64;\n", // broken FIELD beside good ones
        "    y: i64;\n",
        "}\n",
    );
    let p = parse_with_recover(src.to_string(), IMPL, ParseOpts::default());
    let ast = p.ast();
    assert!(!ast.is_clean());

    let items = ast.root().items(ast);
    // the fn ALLOCATED despite its broken param
    let ItemRef::Fn(fh) = items[0] else {
        panic!("the fn must survive part damage: {:?}", items.len())
    };
    let f = fh.get(ast);
    let param_kinds: Vec<&str> = f
        .params
        .iter()
        .map(|p| match p {
            ParamRef::Param(_) => "param",
            ParamRef::SelfParam(_) => "self",
            ParamRef::Error(_) => "error",
        })
        .collect();
    assert_eq!(param_kinds, vec!["error", "param"], "{param_kinds:?}");

    // the class ALLOCATED with its field hole beside the survivors
    let ItemRef::Class(ch) = items[1] else {
        panic!()
    };
    let c = ch.get(ast);
    let field_kinds: Vec<&str> = c
        .fields
        .iter()
        .map(|f| match f {
            FieldRef::Field(_) => "field",
            FieldRef::Error(_) => "error",
        })
        .collect();
    assert_eq!(
        field_kinds,
        vec!["field", "error", "field"],
        "{field_kinds:?}"
    );
}

#[test]
fn enum_members_fence_in_the_list() {
    let src = "enum Color {\n    Red,\n    3,\n    Green,\n}\n";
    let p = parse_with_recover(src.to_string(), IMPL, ParseOpts::default());
    let ast = p.ast();
    let items = ast.root().items(ast);
    let ItemRef::Enum(eh) = items[0] else {
        panic!()
    };
    let kinds: Vec<&str> = eh
        .get(ast)
        .members
        .iter()
        .map(|m| match m {
            MemberRef::Member(_) => "member",
            MemberRef::Error(_) => "error",
        })
        .collect();
    assert_eq!(kinds, vec!["member", "error", "member"], "{kinds:?}");
    assert!(!ast.is_clean());
}

// ---- docs at allocation + the span×src join ----

#[test]
fn docs_ride_the_tree_and_views_borrow() {
    let src = "// adds one\nfn inc(x: i64) -> i64 { x + 1 }\n";
    let p = parse_with_recover(
        src.to_string(),
        IMPL,
        ParseOpts {
            include_comments: true,
        },
    );
    let ast = p.ast();
    let items = ast.root().items(ast);
    let ItemRef::Fn(h) = items[0] else { panic!() };
    let f = h.get(ast);
    assert_eq!(
        f.doc.lines(p.src()).collect::<Vec<_>>(),
        vec!["adds one"],
        "the Go-convention chain, borrowed views"
    );
    // the name verdict — recorded, not recovered
    assert_eq!(f.name_span.text(p.src()), "inc");
    // spans are exact: the fn ends at its own `}`
    assert!(
        f.span.text(p.src()).ends_with("}"),
        "{:?}",
        f.span.text(p.src())
    );

    // and WITHOUT the opt the tree carries no docs
    let p2 = parse_with_recover(src.to_string(), IMPL, ParseOpts::default());
    let ItemRef::Fn(h2) = p2.ast().root().items(p2.ast())[0] else {
        panic!()
    };
    assert!(h2.get(p2.ast()).doc.is_empty());
    // default opts also collect no comments ⇒ no behavior change
    assert!(p2.ast().is_clean());
}

#[test]
fn the_pair_artifact_is_the_normalized_text() {
    // CRLF input: spans index the NORMALIZED text the pair carries
    let p = parse_with_recover(
        "fn main() -> nil {\r}\r\n".to_string(),
        IMPL,
        ParseOpts::default(),
    );
    assert_eq!(p.src(), "fn main() -> nil {\n}\n");
    let items = p.ast().root().items(p.ast());
    let ItemRef::Fn(h) = items[0] else { panic!() };
    assert_eq!(h.get(p.ast()).span.text(p.src()), "fn main() -> nil {\n}");
}

#[test]
fn broken_init_is_a_slot_not_a_lost_let() {
    // `let a = 2 + ;` — the LET survives; only the missing RHS is the hole
    let p = parse_with_recover(
        "fn f() -> nil {\n    let a = 2 + ;\n    let b = 1;\n}\n".to_string(),
        IMPL,
        ParseOpts::default(),
    );
    let ast = p.ast();
    let items = ast.root().items(ast);
    let ItemRef::Fn(h) = items[0] else { panic!() };
    let stmts = &h.get(ast).body.get(ast).stmts;
    let StmtRef::Let(lh) = stmts[0] else {
        panic!("the let survived")
    };
    let l = lh.get(ast);
    // the plan's recovered shape: the Binary completed — only its
    // missing RHS is the hole
    match l.init {
        ExprRef::Binary(bh) => {
            assert!(
                matches!(bh.get(ast).rhs, ExprRef::Error(_)),
                "the rhs is the hole: {:?}",
                bh.get(ast).rhs
            );
        }
        ref e => panic!("the let survived with its binary: {e:?}"),
    }
    assert!(matches!(stmts[1], StmtRef::Let(_)));
}

// ---- the canonical-text law: the CRLF twin is the SAME artifact ----

/// A runnable fixture — a comment, a module let, a string literal, a
/// call: the equivalence must cover docs, token text, and spans.
const EQUIV_SRC: &str = concat!(
    "// the answer\n",
    "let ANSWER = 42;\n",
    "\n",
    "entry fn main() -> nil {\n",
    "    let s = \"hi\";\n",
    "    print(s);\n",
    "}\n",
);

/// the tree AND every node span, as data — `render_json` carries the
/// span pair per node, so equal renders mean equal trees AND spans
fn tree_spans(p: &Parsed) -> String {
    rut_ast::render_json(&rut_ast::to_dump_tree(p.ast()))
}

#[test]
fn the_crlf_twin_parses_to_the_identical_tree_and_spans() {
    let crlf = EQUIV_SRC.replace('\n', "\r\n");
    assert!(crlf.contains("\r\n"), "the twin is really CRLF");
    let lf = parse_with_recover(EQUIV_SRC.to_string(), IMPL, ParseOpts::default());
    let twin = parse_with_recover(crlf, IMPL, ParseOpts::default());
    // the pair artifact is the NORMALIZED text: the twin's src IS the
    // LF original, and every span indexes that (never the raw CRLF)
    assert_eq!(twin.src(), EQUIV_SRC);
    assert_eq!(twin.src(), lf.src());
    // the tree and its spans identical
    assert_eq!(tree_spans(&twin), tree_spans(&lf));
    // and the span×src join over the twin's own src reads the fixture
    let items = twin.ast().root().items(twin.ast());
    let ItemRef::ModuleLet(h) = items[0] else {
        panic!()
    };
    // exact spans: the let ends at its own `;` (prev_end law)
    assert_eq!(h.get(twin.ast()).span.text(twin.src()), "let ANSWER = 42;");
}

#[test]
fn a_lone_cr_normalizes_like_crlf() {
    // the mapping pinned: a lone `\r` (no following `\n`) becomes `\n`
    // exactly like the paired form — no `\r` survives anywhere, so no
    // span can ever index one
    let lone = "fn f() -> nil {\r    return 1;\r}\r";
    let p = parse_with_recover(lone.to_string(), IMPL, ParseOpts::default());
    assert_eq!(p.src(), "fn f() -> nil {\n    return 1;\n}\n");
    assert!(!p.src().contains('\r'), "no \\r survives: {:?}", p.src());
    // the lone-\r twin spans exactly like its LF spelling
    let lf = parse_with_recover(
        "fn f() -> nil {\n    return 1;\n}\n".to_string(),
        IMPL,
        ParseOpts::default(),
    );
    assert_eq!(tree_spans(&p), tree_spans(&lf));
}
