//! Parser surface pins for NODE SPANS: every node's span ends at the
//! byte after its OWN last token (the item-span law — `prev_end`), a
//! block's at its `}`, a when-arm's at its terminator (the comma when
//! one is written, else the body's closer) — and a block that never
//! received its `{` owns no token: it spans nothing (the zero-width
//! convention the parser already keeps for an absent thing).

mod common;

use common::{first_fn_body, parse};
use rut_ast::parser::Mode;
use rut_ast::{BlockNode, FnNode, ItemRef, Span, StmtRef};

#[test]
fn block_spans_end_at_their_own_brace() {
    //        0         1         2
    //        01234567890123 4567890123456789 0123456789
    let src = "fn f() {\n    let x = 1;\n}\nfn g() {}\n";
    let (doc, diags) = parse(src, Mode::Impl);
    let ast = doc.ast();
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    // `f`'s body `}` sits at 24..25 — the block's hi is the byte after
    // its own brace, never the next token's hi (`fn` of `g` at 26..28)
    let body = first_fn_body(ast);
    assert_eq!(body.get(ast).span.hi, 25, "the block ends at its own brace");
    assert_eq!(doc.text(body.get(ast).span), "{\n    let x = 1;\n}");
}

#[test]
fn a_when_arm_ends_at_its_own_last_token() {
    // `when` is a statement — module scope fences it — so the pin's
    // when rides a fn body. Layout:
    //
    //    38..39   `1`     — arm 0's own first token
    //    67..68   `}`     — arm 0's body closer (no comma written)
    //    77..81   `else`  — arm 1's own first token
    //   109..110  `}`     — arm 1's body closer
    let src = "fn f(n: i32) {\n    when (n) {\n        1 -> {\n            a\n        }\n        else -> {\n            b\n        }\n    }\n}\n";
    let (doc, diags) = parse(src, Mode::Impl);
    let ast = doc.ast();
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let body = first_fn_body(ast);
    let StmtRef::When(wh) = body.get(ast).stmts[0] else {
        panic!(
            "expected a when statement, got {:?}",
            body.get(ast).stmts[0]
        );
    };
    let arms = &wh.get(ast).arms;
    assert_eq!(arms.len(), 2);
    // each arm runs from its own first token to the byte after its
    // terminator — never from the NEXT arm's opener
    assert_eq!(
        doc.text(arms[0].get(ast).span),
        "1 -> {\n            a\n        }"
    );
    assert_eq!(
        doc.text(arms[1].get(ast).span),
        "else -> {\n            b\n        }"
    );
    assert_eq!(
        arms[0].get(ast).span,
        Span::new(38, 68),
        "arm 0 ends at its own closer"
    );
    assert_eq!(
        arms[1].get(ast).span,
        Span::new(77, 110),
        "arm 1 ends at its own closer"
    );
}

#[test]
fn a_block_that_never_saw_its_brace_spans_nothing() {
    // the parse is NOT clean — the fn never receives its body brace;
    // the pin reads the tree anyway. Layout:
    //
    //   25..27  `fn`    — the fn item's first token
    //   32..34  `->`    — the fn's last consumed token (hi = 34)
    let src = "struct Thing {\nx: i32;\n}\nfn f() -> \n";
    let (doc, _) = parse(src, Mode::Impl);
    let ast = doc.ast();
    assert!(!ast.is_clean(), "the unfinished fn must diagnose");
    let mut f = None;
    for i in ast.root().items(ast) {
        if let ItemRef::Fn(h) = *i {
            f = Some(h);
        }
    }
    let fh = f.expect("the fn survives its unfinished body");
    assert_eq!(
        fh.get(ast).span,
        Span::new(25, 34),
        "the fn ends at its `->`"
    );
    let n = fh.get(ast).span.hi;
    // the token-less body block is zero-width at the fn's own end — it
    // owns no token and claims none of the fn's
    let body = fh.get(ast).body;
    assert_eq!(
        body.get(ast).span,
        Span::new(n, n),
        "the block spans nothing"
    );
    // the fn's last token — the `>` at 33..34 — answers the Fn, never
    // a block
    let at = ast
        .node_at(n - 1)
        .expect("a real node at the fn's last token");
    assert!(
        at.downcast::<FnNode>().is_some(),
        "the fn answers at its own head"
    );
    assert!(
        at.downcast::<BlockNode>().is_none(),
        "no block claims the fn's tokens"
    );
}
