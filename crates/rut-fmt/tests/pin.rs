//! Unit pins — the exact-byte laws the corpus's conformance suite
//! deliberately does NOT pin: comment reattachment shapes, literal
//! requoting, re-paren decisions, `[style]` parsing.

use rut_fmt::{format, Style};
use rut_parser::Mode;

fn fmt(src: &str) -> String {
    format(src, Mode::Impl, &Style::default()).unwrap_or_else(|ds| panic!("refused: {}", ds[0]))
}

fn fmt_style(src: &str, style: &Style) -> String {
    format(src, Mode::Impl, style).unwrap_or_else(|ds| panic!("refused: {}", ds[0]))
}

// ---- comments ----

#[test]
fn trailing_comment_rides_its_statement() {
    assert_eq!(
        fmt("fn f() {\n    let x = 1;   // the answer\n}"),
        "fn f() {\n    let x = 1; // the answer\n}\n"
    );
}

#[test]
fn own_line_comment_stays_own_line() {
    assert_eq!(
        fmt("fn f() {\n    // preamble\n    let x = 1;\n}"),
        "fn f() {\n    // preamble\n    let x = 1;\n}\n"
    );
}

#[test]
fn file_header_comments_survive() {
    assert_eq!(
        fmt("// header\nuse ink::{ Logger };\n"),
        "// header\nuse ink::{ Logger };\n"
    );
}

#[test]
fn file_tail_comments_survive() {
    assert_eq!(
        fmt("fn f() {}\n// the tail\n"),
        "fn f() {\n}\n// the tail\n"
    );
}

#[test]
fn runs_of_blanks_collapse_to_one() {
    assert_eq!(
        fmt("fn a() {}\n\n\n\nfn b() {}\n"),
        "fn a() {\n}\n\nfn b() {\n}\n"
    );
}

#[test]
fn block_comments_survive_multi_line() {
    let out = fmt("fn f() {\n    /* one\n   two */\n    let x = 1;\n}");
    assert!(out.contains("/* one"), "{out}");
    assert!(out.contains("two */"), "{out}");
}

#[test]
fn comment_inside_empty_block_survives() {
    let out = fmt("fn f() { /* only */ }");
    assert!(out.contains("/* only */"), "{out}");
}

// ---- literals ----

#[test]
fn floats_keep_their_dot_and_suffix() {
    // `9.0f32` must not come back as `9f32` (that re-lexes as a
    // suffixed INT and refuses to compile)
    assert_eq!(fmt("fn f() {\n    let a = 9.0f32;\n}"), "fn f() {\n    let a = 9.0f32;\n}\n");
    assert_eq!(fmt("fn f() {\n    let a = 2.50;\n}"), "fn f() {\n    let a = 2.5;\n}\n");
}

#[test]
fn ints_lose_the_radix_and_keep_the_suffix() {
    assert_eq!(fmt("fn f() {\n    let a = 0xff;\n}"), "fn f() {\n    let a = 255;\n}\n");
    assert_eq!(fmt("fn f() {\n    let a = 7u64;\n}"), "fn f() {\n    let a = 7u64;\n}\n");
}

#[test]
fn strings_requote_with_the_exact_inverse_escapes() {
    assert_eq!(
        fmt("fn f() {\n    let s = \"a\\tb\\n\\\"q\\\"\";\n}"),
        "fn f() {\n    let s = \"a\\tb\\n\\\"q\\\"\";\n}\n"
    );
}

#[test]
fn raw_strings_survive_verbatim() {
    assert_eq!(
        fmt("fn f() {\n    let r = r\"C:\\t\\x\";\n}"),
        "fn f() {\n    let r = r\"C:\\t\\x\";\n}\n"
    );
}

// ---- parens / precedence ----

#[test]
fn grouping_parens_survive_where_flat_would_invert() {
    // `(1 + 2) * 3` — the parens ARE the tree
    assert_eq!(fmt("fn f() {\n    let a = (1 + 2) * 3;\n}"), "fn f() {\n    let a = (1 + 2) * 3;\n}\n");
    // flat shapes stay bare
    assert_eq!(fmt("fn f() {\n    let a = 1 + 2 * 3;\n}"), "fn f() {\n    let a = 1 + 2 * 3;\n}\n");
    // same-level rhs keeps its grouping
    assert_eq!(fmt("fn f() {\n    let a = 10 - (2 + 3);\n}"), "fn f() {\n    let a = 10 - (2 + 3);\n}\n");
    assert_eq!(fmt("fn f() {\n    let a = 10 - 2 - 3;\n}"), "fn f() {\n    let a = 10 - 2 - 3;\n}\n");
}

#[test]
fn unary_needs_parens_over_a_binary_operand() {
    // `-((1 * 2))` in the source is Unary(Mul) — printing `-1 * 2` would
    // parse to a DIFFERENT tree (Mul(Neg, ..)) — the printer re-parens
    let out = fmt("fn f() {\n    let a = -(1 * 2);\n}");
    assert!(out.contains("-(1 * 2)"), "{out}");
}

// ---- [style] ----

#[test]
fn style_defaults_are_the_corpus_conventions() {
    let s = Style::default();
    assert_eq!(s.indent_width, 4);
    assert_eq!(s.max_width, 100);
}

#[test]
fn style_manifest_parses_and_validates() {
    let mut m = std::collections::BTreeMap::new();
    m.insert("indent_width".to_string(), "2".to_string());
    m.insert("max_width".to_string(), "60".to_string());
    m.insert("brand_new_knob".to_string(), "ignored".to_string()); // forward-compat
    let s = rut_fmt::style::from_manifest(&m).unwrap();
    assert_eq!(s.indent_width, 2);
    assert_eq!(s.max_width, 60);

    m.insert("indent_width".to_string(), "9".to_string());
    assert!(rut_fmt::style::from_manifest(&m).is_err());
    m.insert("indent_width".to_string(), "wide".to_string());
    assert!(rut_fmt::style::from_manifest(&m).is_err());
}

#[test]
fn style_knobs_actually_color_the_output() {
    let src = "fn f() {\n    let x = 1;\n}";
    let two = fmt_style(src, &Style { indent_width: 2, max_width: 100 });
    assert!(two.contains("\n  let x = 1;"), "{two}");
}

// ---- the width law ----

#[test]
fn long_call_breaks_one_per_line_with_trailing_comma() {
    let out = fmt("fn f() {\n    let x = a_very_long_function_name_deliberately_so(alpha, beta, gamma, delta, epsilon, zeta, eta, theta, iota, kappa, lambda, mu);\n}");
    assert!(
        out.contains("a_very_long_function_name_deliberately_so(\n"),
        "expected a broken arg list:\n{out}"
    );
    assert!(out.contains("mu,\n"), "expected a trailing comma:\n{out}");
}

#[test]
fn short_stays_flat() {
    assert_eq!(
        fmt("fn f() {\n    let x = add(1, 2);\n}"),
        "fn f() {\n    let x = add(1, 2);\n}\n"
    );
}

// ---- decl mode ----

#[test]
fn decl_mode_formats_bodiless_methods() {
    let src = "prelude builtin class StrBuf {\nfn push(self, b: bytes);\nfn len(self) -> i32;\n}\n";
    assert_eq!(
        format(src, Mode::Decl, &Style::default()).unwrap(),
        "prelude builtin class StrBuf {\n    fn push(self, b: bytes);\n    fn len(self) -> i32;\n}\n"
    );
}

// ---- refusal ----

#[test]
fn a_source_with_diags_is_refused_never_guessed() {
    let ds = format("fn f( {", Mode::Impl, &Style::default());
    assert!(ds.is_err());
}
