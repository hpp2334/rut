//! Depth budgets and the C2/C3 contracts (RFC 0030 §4/§7): deep input is
//! one clean `Diag`, never a host crash; the iterative parser runs on a
//! tiny thread stack that recursive descent would overflow.

use rut_parser::{parse, Mode};

fn in_fn(body: &str) -> String {
    format!("fn f() -> nil {{ {body} }}")
}

/// N nested `if (true)` blocks — rut has no bare block statements, so
/// nesting goes through loop bodies (v1's enter() counted these)
fn nested_ifs(n: usize, inner: &str) -> String {
    format!("{}{inner}{}", "if (true) { ".repeat(n), "} ".repeat(n))
}

#[test]
fn depth_budget_is_a_diag_not_a_crash() {
    // 100k-deep parens: one clean "nesting too deep", never a host crash (C3)
    let src = format!("fn f() -> nil {{ let x = {}1{}; }}", "(".repeat(100_000), ")".repeat(100_000));
    let (_, diags) = parse(&src, Mode::Impl);
    assert!(diags.iter().any(|d| d.msg.contains("nesting too deep")), "want a nesting diag, got: {diags:?}");
}

#[test]
fn unary_chain_is_one_diag() {
    // 100k prefix `!` — v1 recursed parse_unary per op; the frame machine
    // sweeps iteratively and bounds the chain by the expression budget
    let src = in_fn(&format!("let x = {}1;", "!".repeat(100_000)));
    let (_, diags) = parse(&src, Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("nesting too deep")),
        "want one nesting diag, got: {diags:?}"
    );
    assert!(diags.iter().filter(|d| d.msg.contains("nesting too deep")).count() <= 1);
}

#[test]
fn block_budget_still_1024() {
    // 1023 nested if-bodies + the fn body = 1024 block frames: in budget
    let ok = in_fn(&nested_ifs(1023, "let x = 1;"));
    let (_, diags) = parse(&ok, Mode::Impl);
    assert!(diags.is_empty(), "1024 block frames must parse clean: {diags:?}");

    // far beyond: one "nesting too deep"
    let deep = in_fn(&nested_ifs(4_000, "let x = 1;"));
    let (_, diags) = parse(&deep, Mode::Impl);
    assert!(diags.iter().any(|d| d.msg.contains("nesting too deep")), "want a nesting diag, got: {diags:?}");
}

#[test]
fn expression_budget_is_64() {
    let ok = in_fn(&format!("let x = {}1{};", "(".repeat(63), ")".repeat(63)));
    let (_, diags) = parse(&ok, Mode::Impl);
    assert!(diags.is_empty(), "63 nested parens must parse clean: {diags:?}");

    let deep = in_fn(&format!("let x = {}1{};", "(".repeat(100), ")".repeat(100)));
    let (_, diags) = parse(&deep, Mode::Impl);
    assert!(diags.iter().any(|d| d.msg.contains("nesting too deep")), "want a nesting diag, got: {diags:?}");
}

/// C2 in the flesh: the frame machine parses nesting that is deep for a
/// host stack but well inside the budgets, on a thread whose stack a
/// recursive-descent parser (13 frames/level) would overflow. Parse only
/// — the downstream walks (dump/typecheck) are recursive by design under
/// the 64/1024 split.
#[test]
fn parses_on_a_tiny_stack() {
    let src = in_fn(&format!(
        "{}let x = {}1{} + f({}2{});{}",
        "if (true) { ".repeat(500),
        "(".repeat(30),
        ")".repeat(30),
        "(".repeat(20),
        ")".repeat(20),
        "} ".repeat(500)
    ));
    let child = std::thread::Builder::new()
        .stack_size(96 * 1024) // 96 KiB — a fraction of the 1 MiB default
        .spawn(move || {
            let (_, diags) = parse(&src, Mode::Impl);
            assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
        })
        .expect("spawn");
    child.join().expect("tiny-stack parse must not crash");
}

/// malformed input must terminate (no hang, no panic); the v1-hang sites
/// (unterminated blocks/bodies) terminate cleanly via the frame machine
#[test]
fn malformed_input_terminates() {
    let cases = [
        "fn f() -> nil {",
        "fn f() -> nil ",
        "class C {",
        "class C { fn m(",
        "interface T { fn m",
        "when (x) {",
        "fn f() { when (x) { 1 -> } }",
        "impl T for C {",
        "import {",
        "struct D { x",
        "enum E {",
        "fn f() -> nil { let ",
        "fn f() -> nil { let x = (1, }",
        "fn f() -> nil { for (let x }",
        "host class K {",
        "fn f() -> nil { let x = f\"{;",
        "$",
        "fn f() -> nil { ] }",
        "pub(super) fn",
        "fn f<K where K",
        "fn f() -> nil { let x = await select { a -> 1, ; }",
    ];
    for src in cases {
        let (ast, diags) = parse(src, Mode::Impl);
        let _ = std::format!("{:?}", ast.root.id());
        // termination itself is the contract; most cases also diagnose
        let _ = diags.len();
    }
    // a few known shapes must produce diags
    for src in ["enum E {", "import {", "when (x) {", "fn f() -> nil { let "] {
        let (_, diags) = parse(src, Mode::Impl);
        assert!(!diags.is_empty(), "`{src}` should produce diags");
    }
}
