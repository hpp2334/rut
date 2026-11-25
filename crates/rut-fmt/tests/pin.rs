//! Unit pins — the exact-byte laws the corpus's conformance suite
//! deliberately does NOT pin: comment reattachment shapes, literal
//! verbatim preservation, re-paren decisions, `[style]` parsing.

use rut_ast::parser::{parse_with_recover, Mode};
use rut_fmt::{format, Style};

fn fmt(src: &str) -> String {
    format(src, Mode::Impl, &Style::default()).unwrap_or_else(|ds| panic!("refused: {}", ds[0].msg))
}

fn fmt_style(src: &str, style: &Style) -> String {
    format(src, Mode::Impl, style).unwrap_or_else(|ds| panic!("refused: {}", ds[0].msg))
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

// ---- file modules: mod decls, use paths ----

#[test]
fn mod_decls_print_with_their_vis() {
    assert_eq!(
        fmt("mod layout;\npub mod widget;\npub(pkg) mod ui;\npub(super) mod w;\n"),
        "mod layout;\npub mod widget;\npub(pkg) mod ui;\npub(super) mod w;\n"
    );
}

#[test]
fn use_paths_print_all_segments_then_the_leaf() {
    // the mod path spells out in full; the leaf names ride the brace
    // form (the single-leaf spelling canonicalizes to it)
    assert_eq!(
        fmt("use pkg::A::B::{C, D};\n"),
        "use pkg::A::B::{ C, D };\n"
    );
    assert_eq!(fmt("use pkg::A::B::C;\n"), "use pkg::A::B::{ C };\n");
    // the flat degenerate shapes are byte-stable
    assert_eq!(fmt("use ink::{ Logger };\n"), "use ink::{ Logger };\n");
    assert_eq!(fmt("use ink::Logger;\n"), "use ink::{ Logger };\n");
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
    // suffixed INT and refuses to compile) — verbatim guarantees it
    assert_eq!(
        fmt("fn f() {\n    let a = 9.0f32;\n}"),
        "fn f() {\n    let a = 9.0f32;\n}\n"
    );
    // non-canonical spellings survive byte-exact
    assert_eq!(
        fmt("fn f() {\n    let a = 2.50;\n}"),
        "fn f() {\n    let a = 2.50;\n}\n"
    );
    assert_eq!(
        fmt("fn f() {\n    let a = 1e10f64;\n}"),
        "fn f() {\n    let a = 1e10f64;\n}\n"
    );
}

#[test]
fn inf_reprints_bare_with_no_suffix() {
    // the ident-fold literal carries the WORD's span — the verbatim
    // slice re-lexes as the same ident, which folds again
    assert_eq!(
        fmt("fn f() {\n    let a = inf;\n    let b = -inf;\n}"),
        "fn f() {\n    let a = inf;\n    let b = -inf;\n}\n"
    );
}

#[test]
fn ints_keep_their_radix_and_underscores() {
    assert_eq!(
        fmt("fn f() {\n    let a = 0xff;\n}"),
        "fn f() {\n    let a = 0xff;\n}\n"
    );
    // the reported case: the color literal's hex + suffix survive
    assert_eq!(
        fmt("pub let PAGE_BG: u64 = 0xF8FAFCFFu64;\n"),
        "pub let PAGE_BG: u64 = 0xF8FAFCFFu64;\n"
    );
    assert_eq!(
        fmt("fn f() {\n    let a = 0b1010;\n    let b = 0o777;\n    let c = 1_000_000;\n    let d = 7u64;\n}"),
        "fn f() {\n    let a = 0b1010;\n    let b = 0o777;\n    let c = 1_000_000;\n    let d = 7u64;\n}\n"
    );
}

#[test]
fn strings_keep_their_escape_spellings() {
    assert_eq!(
        fmt("fn f() {\n    let s = \"a\\tb\\n\\\"q\\\"\";\n}"),
        "fn f() {\n    let s = \"a\\tb\\n\\\"q\\\"\";\n}\n"
    );
}

#[test]
fn non_canonical_escapes_stay_as_written() {
    // `\u{41}` is `A` — the hex spelling survives anyway (the old
    // printer re-spelled the decoded content to a bare `A`)
    assert_eq!(
        fmt("fn f() {\n    let s = \"\\u{41}\";\n}"),
        "fn f() {\n    let s = \"\\u{41}\";\n}\n"
    );
    // the escaped-apostrophe spelling survives (the value needs no
    // escape inside a double-quoted string)
    assert_eq!(
        fmt("fn f() {\n    let s = \"it\\'s\";\n}"),
        "fn f() {\n    let s = \"it\\'s\";\n}\n"
    );
}

#[test]
fn the_reported_repro_formats_to_itself() {
    let src = "pub let PAGE_BG: u64 = 0xF8FAFCFFu64;\n";
    let once = fmt(src);
    assert_eq!(once, src, "byte-identical");
    assert_eq!(fmt(&once), once, "idempotent");
}

#[test]
fn raw_strings_survive_verbatim() {
    assert_eq!(
        fmt("fn f() {\n    let r = r\"C:\\t\\x\";\n}"),
        "fn f() {\n    let r = r\"C:\\t\\x\";\n}\n"
    );
}

// ---- hash-leveled multi-line literals ----

#[test]
fn multiline_raw_prints_verbatim_at_level_one() {
    assert_eq!(
        fmt("fn f() {\n    let s = r#\"a\nb\"#;\n}"),
        "fn f() {\n    let s = r#\"a\nb\"#;\n}\n"
    );
}

#[test]
fn raw_embedded_quotes_print_at_level_one() {
    assert_eq!(
        fmt("fn f() {\n    let q = r#\"she said \"never\"\"#;\n}"),
        "fn f() {\n    let q = r#\"she said \"never\"\"#;\n}\n"
    );
}

#[test]
fn raw_hash_runs_pick_the_minimal_level() {
    // value `a"#b` needs level 2 — verbatim keeps it (and nothing
    // downgrades a fitting level anymore)
    assert_eq!(
        fmt("fn f() {\n    let s = r##\"a\"#b\"##;\n}"),
        "fn f() {\n    let s = r##\"a\"#b\"##;\n}\n"
    );
}

#[test]
fn raw_hash_levels_survive_verbatim() {
    // the author's hash level is spelling, not value — it survives
    // even when a lower level would fit
    assert_eq!(
        fmt("fn f() {\n    let x = r#\"plain\"#;\n}"),
        "fn f() {\n    let x = r#\"plain\"#;\n}\n"
    );
}

#[test]
fn multiline_fstr_prints_hash_form_with_real_newlines() {
    assert_eq!(
        fmt("fn f() {\n    let t = f#\"line {x}\nmore\"#;\n}"),
        "fn f() {\n    let t = f#\"line {x}\nmore\"#;\n}\n"
    );
}

#[test]
fn single_line_fstr_stays_level_zero() {
    assert_eq!(
        fmt("fn f() {\n    let t = f\"a {x} b\";\n}"),
        "fn f() {\n    let t = f\"a {x} b\";\n}\n"
    );
}

#[test]
fn fstr_escaped_newline_stays_escaped_level_zero() {
    // `f"a\nb"` spells the newline as an escape — the spelling
    // survives; NO promotion to the hash-leveled form
    assert_eq!(
        fmt("fn f() {\n    let t = f\"a\\nb\";\n}"),
        "fn f() {\n    let t = f\"a\\nb\";\n}\n"
    );
}

#[test]
fn fstr_chunk_bytes_stay_verbatim_holes_reprint() {
    // chunk gaps around a hole were empty — the hole's interior
    // whitespace is the live expr's business and gets re-formatted
    assert_eq!(
        fmt("fn f() {\n    let t = f\"{ x + 1 }\";\n}"),
        "fn f() {\n    let t = f\"{x + 1}\";\n}\n"
    );
    // escaped braces beside a hole survive byte-exact (`{{` chunk
    // bytes then the hole's lone opener)
    assert_eq!(
        fmt("fn f() {\n    let x = 1;\n    let t = f\"{{{x}}}\";\n}"),
        "fn f() {\n    let x = 1;\n    let t = f\"{{{x}}}\";\n}\n"
    );
    // adjacent holes, an empty template, and a NESTED f-string in a
    // hole (the grammar admits it — lex_hole routes the `f` ident
    // through the word lane, so the inner literal lexes whole)
    assert_eq!(
        fmt("fn f() {\n    let a = 1;\n    let b = 2;\n    let t = f\"{a}{b}\";\n    let e = f\"\";\n    let n = f\"x{f\"{a}\"}y\";\n}"),
        "fn f() {\n    let a = 1;\n    let b = 2;\n    let t = f\"{a}{b}\";\n    let e = f\"\";\n    let n = f\"x{f\"{a}\"}y\";\n}\n"
    );
}

#[test]
fn a_hole_prints_on_one_line_at_every_level() {
    // law 2: a hole is a single-line zone — the block-carrying atoms
    // print the source's own flat spelling inside the hole, at every
    // hash level (the book: "Holes stay single-line at every level")
    assert_eq!(
        fmt("fn f(n: i32) -> i32 { let s = f\"{when (n) { 1 -> 2, else -> 3 }}\"; s }"),
        "fn f(n: i32) -> i32 {\n    let s = f\"{when (n) { 1 -> 2, else -> 3 }}\";\n    s\n}\n"
    );
    assert_eq!(
        fmt("fn f(n: i32) -> i32 { let s = f#\"{when (n) { 1 -> 2, else -> 3 }}\"#; s }"),
        "fn f(n: i32) -> i32 {\n    let s = f#\"{when (n) { 1 -> 2, else -> 3 }}\"#;\n    s\n}\n"
    );
    assert_eq!(
        fmt("fn f(n: i32) -> i32 { let s = f##\"{when (n) { 1 -> 2, else -> 3 }}\"##; s }"),
        "fn f(n: i32) -> i32 {\n    let s = f##\"{when (n) { 1 -> 2, else -> 3 }}\"##;\n    s\n}\n"
    );
    // the flat spellings are the source's own: block-valued arms and
    // the async mint
    assert_eq!(
        fmt("fn f(n: i32) -> i32 { let s = f\"{when (n) { 1 -> { a + 1 }, else -> { b + 2 } }}\"; s }"),
        "fn f(n: i32) -> i32 {\n    let s = f\"{when (n) { 1 -> { a + 1 }, else -> { b + 2 } }}\";\n    s\n}\n"
    );
    assert_eq!(
        fmt("fn f() -> i32 { let s = f\"{async { a = 1; a }}\"; s }"),
        "fn f() -> i32 {\n    let s = f\"{async { a = 1; a }}\";\n    s\n}\n"
    );
    // a hole wider than the line stays on one line — the LET breaks,
    // the hole does not (103 cols flat, so the rhs breaks; the hole
    // itself never does)
    assert_eq!(
        fmt("fn f(a: i32, b: i32) -> i32 { let s = f\"{a + b + a + b + a + b + a + b + a + b + a + b + a + b + a + b + a + b + a + b + a + b}\"; s }"),
        "fn f(a: i32, b: i32) -> i32 {\n    let s =\n        f\"{a + b + a + b + a + b + a + b + a + b + a + b + a + b + a + b + a + b + a + b + a + b}\";\n    s\n}\n"
    );
}

#[test]
fn multiline_literals_are_idempotent() {
    let src = "fn f() {\n    let s = r#\"a\nb\"#;\n    let t = f#\"line {x}\nmore\"#;\n}";
    let once = fmt(src);
    assert_eq!(fmt(&once), once);
    // the verbatim spellings re-lex to themselves — idempotence holds
    let src = "fn f() {\n    let a = 0xF8FAFCFFu64;\n    let b = 2.50;\n    let c = 1e10f64;\n    let d = \"\\u{41}\";\n    let t = f\"{{{x}}}\";\n}";
    let once = fmt(src);
    assert_eq!(fmt(&once), once);
}

// ---- parens / precedence ----

#[test]
fn grouping_parens_survive_where_flat_would_invert() {
    // `(1 + 2) * 3` — the parens ARE the tree
    assert_eq!(
        fmt("fn f() {\n    let a = (1 + 2) * 3;\n}"),
        "fn f() {\n    let a = (1 + 2) * 3;\n}\n"
    );
    // flat shapes stay bare
    assert_eq!(
        fmt("fn f() {\n    let a = 1 + 2 * 3;\n}"),
        "fn f() {\n    let a = 1 + 2 * 3;\n}\n"
    );
    // same-level rhs keeps its grouping
    assert_eq!(
        fmt("fn f() {\n    let a = 10 - (2 + 3);\n}"),
        "fn f() {\n    let a = 10 - (2 + 3);\n}\n"
    );
    assert_eq!(
        fmt("fn f() {\n    let a = 10 - 2 - 3;\n}"),
        "fn f() {\n    let a = 10 - 2 - 3;\n}\n"
    );
}

#[test]
fn unary_needs_parens_over_a_binary_operand() {
    // `-((1 * 2))` in the source is Unary(Mul) — printing `-1 * 2` would
    // parse to a DIFFERENT tree (Mul(Neg, ..)) — the printer re-parens
    let out = fmt("fn f() {\n    let a = -(1 * 2);\n}");
    assert!(out.contains("-(1 * 2)"), "{out}");
}

#[test]
fn paren_group_on_a_struct_literal_survives_every_postfix_role() {
    // a composite literal leaves the atom frame — the paren group is
    // the only way back into the postfix loop — so every postfix role
    // keeps it: the callee, the field, method and index receivers
    assert_eq!(
        fmt("fn f() -> i32 { let x = (P { a: 1 }).a; x }"),
        "fn f() -> i32 {\n    let x = (P { a: 1 }).a;\n    x\n}\n"
    );
    assert_eq!(
        fmt("fn f() -> i32 { let x = (P { a: 1 }).m(); x }"),
        "fn f() -> i32 {\n    let x = (P { a: 1 }).m();\n    x\n}\n"
    );
    assert_eq!(
        fmt("fn f(i: i32) -> i32 { let x = (P { a: 1 })[i]; x }"),
        "fn f(i: i32) -> i32 {\n    let x = (P { a: 1 })[i];\n    x\n}\n"
    );
    assert_eq!(
        fmt("fn f(a: i32) -> i32 { let x = (P { a: 1 })(a); x }"),
        "fn f(a: i32) -> i32 {\n    let x = (P { a: 1 })(a);\n    x\n}\n"
    );
}

#[test]
fn paren_group_on_array_repeat_and_tuple_survives() {
    // arrays, repeats and tuples leave the atom frame like a struct
    // literal — their groups ride every postfix role
    assert_eq!(
        fmt("fn f(a: i32, b: i32, i: i32) -> i32 { let x = ([a, b])[i]; x }"),
        "fn f(a: i32, b: i32, i: i32) -> i32 {\n    let x = ([a, b])[i];\n    x\n}\n"
    );
    assert_eq!(
        fmt("fn f(a: i32, i: i32) -> i32 { let x = ([a; 2])[i]; x }"),
        "fn f(a: i32, i: i32) -> i32 {\n    let x = ([a; 2])[i];\n    x\n}\n"
    );
    assert_eq!(
        fmt("fn f(a: i32, b: i32) -> i32 { let x = ([a, b]).len(); x }"),
        "fn f(a: i32, b: i32) -> i32 {\n    let x = ([a, b]).len();\n    x\n}\n"
    );
    assert_eq!(
        fmt("fn f(a: i32, b: i32, i: i32) -> i32 { let x = ((a, b))[i]; x }"),
        "fn f(a: i32, b: i32, i: i32) -> i32 {\n    let x = ((a, b))[i];\n    x\n}\n"
    );
}

#[test]
fn paren_group_on_a_lambda_survives() {
    // a lambda's body is a full expression, so the bare spelling
    // swallows the postfix chain or the operator — the group is the
    // lambda's boundary in every constrained position
    assert_eq!(
        fmt("fn f() -> i32 { let x = (x => x)(1); x }"),
        "fn f() -> i32 {\n    let x = (x => x)(1);\n    x\n}\n"
    );
    assert_eq!(
        fmt("fn f() -> i32 { let x = (x => x).a; x }"),
        "fn f() -> i32 {\n    let x = (x => x).a;\n    x\n}\n"
    );
    assert_eq!(
        fmt("fn f() -> i32 { let x = (x => x) + 1; x }"),
        "fn f() -> i32 {\n    let x = (x => x) + 1;\n    x\n}\n"
    );
    assert_eq!(
        fmt("fn f() -> i64 { let x = ((a: i32) => a) as i64; x }"),
        "fn f() -> i64 {\n    let x = ((a: i32) => a) as i64;\n    x\n}\n"
    );
    assert_eq!(
        fmt("fn f(a: bool, b: bool) -> bool { let x = a < (b => b); x }"),
        "fn f(a: bool, b: bool) -> bool {\n    let x = a < (b => b);\n    x\n}\n"
    );
}

#[test]
fn paren_group_on_unary_and_await_survives() {
    // a prefix wraps the whole postfix chain from its atom, so the
    // bare spelling re-nests: `-a.x` is Unary(Field) where `(-a).x`
    // is Field(Unary) — the group is the boundary
    assert_eq!(
        fmt("fn f() -> i32 { let x = (-a).x; x }"),
        "fn f() -> i32 {\n    let x = (-a).x;\n    x\n}\n"
    );
    assert_eq!(
        fmt("fn f() -> bool { let x = (!a).x; x }"),
        "fn f() -> bool {\n    let x = (!a).x;\n    x\n}\n"
    );
    assert_eq!(
        fmt("fn f() -> i32 { let x = (await g()).x; x }"),
        "fn f() -> i32 {\n    let x = (await g()).x;\n    x\n}\n"
    );
}

#[test]
fn level_six_operator_keeps_its_left_group() {
    // level 6 is not associative: the bare spelling either re-nests or
    // is rejected (`a < b < c`), so a level-6 left operand under a
    // level-6 operator keeps its group — the four pairs
    assert_eq!(
        fmt("fn f() -> bool { let x = (a < b) < c; x }"),
        "fn f() -> bool {\n    let x = (a < b) < c;\n    x\n}\n"
    );
    assert_eq!(
        fmt("fn f() -> bool { let x = (a is i32) < c; x }"),
        "fn f() -> bool {\n    let x = (a is i32) < c;\n    x\n}\n"
    );
    assert_eq!(
        fmt("fn f() -> bool { let x = (a < b) is i32; x }"),
        "fn f() -> bool {\n    let x = (a < b) is i32;\n    x\n}\n"
    );
    assert_eq!(
        fmt("fn f() -> bool { let x = (a is i32) is i32; x }"),
        "fn f() -> bool {\n    let x = (a is i32) is i32;\n    x\n}\n"
    );
}

#[test]
fn cast_keeps_its_group_before_a_less_than() {
    // in type position `<` opens generic arguments, so the bare
    // `a as i64 < b` feeds `< b` to the cast's type parse and dies —
    // the group is load-bearing for the parse itself
    assert_eq!(
        fmt("fn f() -> bool { let x = (a as i64) < b; x }"),
        "fn f() -> bool {\n    let x = (a as i64) < b;\n    x\n}\n"
    );
}

#[test]
fn a_receiver_paren_is_kept_only_when_it_carries_meaning() {
    // a receiver chain re-associates left: `(q.b).c` and `q.b.c` are
    // the same tree, and so are `(k[i]).m()` and `k[i].m()` — those
    // groups drop. An operator receiver re-nests bare
    // (`h ^ b as u32 .m()` is not the same chain), so those stay.
    assert_eq!(
        fmt("fn f() -> i32 { let y = (q.b).c; y }"),
        "fn f() -> i32 {\n    let y = q.b.c;\n    y\n}\n"
    );
    assert_eq!(
        fmt("fn f(x: i64) -> i64 { let y = (k[i]).wrapping_add(x); y }"),
        "fn f(x: i64) -> i64 {\n    let y = k[i].wrapping_add(x);\n    y\n}\n"
    );
    assert_eq!(
        fmt("fn f(x: u64) -> u64 { let y = (b as u64).wrapping_add(x); y }"),
        "fn f(x: u64) -> u64 {\n    let y = (b as u64).wrapping_add(x);\n    y\n}\n"
    );
    assert_eq!(
        fmt("fn f() -> i32 { let y = (n - 11).wrapping_mul(5); y }"),
        "fn f() -> i32 {\n    let y = (n - 11).wrapping_mul(5);\n    y\n}\n"
    );
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
    let two = fmt_style(
        src,
        &Style {
            indent_width: 2,
            max_width: 100,
        },
    );
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
    let src = "prelude builtin class Gauge {\nfn tick(self, n: i32);\nfn len(self) -> i32;\n}\n";
    assert_eq!(
        format(src, Mode::Decl, &Style::default()).unwrap(),
        "prelude builtin class Gauge {\n    fn tick(self, n: i32);\n    fn len(self) -> i32;\n}\n"
    );
}

// ---- block tails (the `}`-closed last statement: no `;`) ----

#[test]
fn fn_body_tail_prints_without_semicolon() {
    assert_eq!(fmt("fn f() -> i32 { 5 }"), "fn f() -> i32 {\n    5\n}\n");
}

#[test]
fn tail_after_statements_round_trips() {
    let src = "fn f() -> i32 {\n    let a = 1;\n    a + a\n}\n";
    assert_eq!(fmt(src), src);
}

#[test]
fn tail_comments_ride_the_tail_element() {
    // a leading run drains own-line before the tail; a trailing run
    // rides its line — the tail has the statement hooks, minus the `;`
    let src = "fn f() -> i32 {\n    // leading\n    5 // trail\n}\n";
    assert_eq!(fmt(src), src);
}

#[test]
fn lambda_block_tail_prints_inline() {
    assert_eq!(
        fmt("fn f() -> i32 { let g: fn() -> i32 = fn () -> i32 { 4 }; g }"),
        "fn f() -> i32 {\n    let g: fn() -> i32 = fn () -> i32 { 4 };\n    g\n}\n"
    );
}

#[test]
fn when_arm_block_tails_print_inline() {
    // the arm blocks' tails stay flat (no `;` inside the arm braces)
    assert_eq!(
        fmt("fn f(x: i32) -> i32 { let w = when (x) { 0 -> { 1 }, else -> { 2 } }; w }"),
        "fn f(x: i32) -> i32 {\n    let w =\n        when (x) {\n            0 -> { 1 },\n            else -> { 2 },\n        };\n    w\n}\n"
    );
}

#[test]
fn statement_if_with_tail_arms_prints_multiline() {
    assert_eq!(
        fmt("fn fact(n: i32) -> i32 { if (n <= 1) { 1 } else { n * fact(n - 1) } }"),
        "fn fact(n: i32) -> i32 {\n    if (n <= 1) {\n        1\n    } else {\n        n * fact(n - 1)\n    }\n}\n"
    );
}

// ---- async blocks ----

#[test]
fn an_async_block_keeps_its_tail_through_formatting() {
    // the mint's tail is the block's VALUE — the arm used to print the
    // statements and drop the tail (`async { 1 + 2 }` → `async { }`).
    // The formatted output's dump must hold the whole
    // `Block { stmts: [], tail: Binary }`.
    let out = fmt("fn f() {\n    let x = async { 1 + 2 };\n}");
    let p = parse_with_recover(out, Mode::Impl);
    assert!(
        p.ast().is_clean(),
        "the formatted text must reparse clean: {:?}",
        p.ast().errors()
    );
    let dump = rut_ast::AstPrinter::text_of(p.ast(), p.text(p.span()));
    let body = dump
        .split("AsyncBlock")
        .nth(1)
        .expect("the AsyncBlock must be in the dump");
    let (_, after_stmts) = body
        .split_once("stmts:")
        .expect("the body's Block dumps its stmts");
    let (between, after_tail) = after_stmts
        .split_once("tail:")
        .expect("the tail must survive formatting");
    assert!(
        !between.contains('@'),
        "the async body must hold no statements:\n{dump}"
    );
    assert!(
        after_tail.contains("Binary"),
        "the tail must be the `1 + 2` binary:\n{dump}"
    );
}

#[test]
fn an_async_block_is_idempotent() {
    // the arm never advanced `passed` past the block's `{`, so a second
    // pass inserted an empty line after `async {` (ground truth 9)
    let src = "fn f() {\n    let f = async { a = 1; a };\n}";
    let once = fmt(src);
    assert_eq!(fmt(&once), once, "fmt(fmt(x)) != fmt(x)");
    assert!(
        !once.contains("async {\n\n"),
        "an empty line after `async {{`:\n{once}"
    );
}

// ---- if expressions (the atom beside `when`) ----

#[test]
fn if_expression_in_let_prints_like_the_statement_spelling() {
    // one spelling for both forms; the arms ride the let's flat window
    assert_eq!(
        fmt("fn f(c: bool) -> i32 { let x = if (c) { 1 } else { 2 }; x }"),
        "fn f(c: bool) -> i32 {\n    let x = if (c) { 1 } else { 2 };\n    x\n}\n"
    );
}

#[test]
fn if_expression_chain_and_operand_shapes() {
    // the else-if chain keeps the statement grammar's spelling
    assert_eq!(
        fmt("fn f(a: bool, b: bool) -> i32 { let x = if (a) { 1 } else if (b) { 2 } else { 3 }; x }"),
        "fn f(a: bool, b: bool) -> i32 {\n    let x = if (a) { 1 } else if (b) { 2 } else { 3 };\n    x\n}\n"
    );
    // a binary operand needs no parens (prefix form, no ambiguity)
    assert_eq!(
        fmt("fn f(c: bool) -> i32 { let x = 1 + if (c) { 2 } else { 3 }; x }"),
        "fn f(c: bool) -> i32 {\n    let x = 1 + if (c) { 2 } else { 3 };\n    x\n}\n"
    );
    // as a call argument the argument list's own break law applies
    assert_eq!(
        fmt("fn g(v: i32) -> i32 { v } fn f(c: bool) -> i32 { g(if (c) { 1 } else { 2 }) }"),
        "fn g(v: i32) -> i32 {\n    v\n}\nfn f(c: bool) -> i32 {\n    g(\n        if (c) {\n            1\n        } else {\n            2\n        },\n    )\n}\n"
    );
}

#[test]
fn statement_if_trailing_semicolon_normalizes_away() {
    // the `;` after the statement form only ever discards; the printer
    // prints statement-ifs without one (round-trip stable)
    let once = fmt("fn f(c: bool) -> i32 { if (c) { 1 } else { 2 }; 0 }");
    assert_eq!(
        once,
        "fn f(c: bool) -> i32 {\n    if (c) {\n        1\n    } else {\n        2\n    }\n    0\n}\n"
    );
    assert_eq!(fmt(&once), once, "idempotent");
}

// ---- loop headers + the const head ----

#[test]
fn loop_header_const_survives_parse_fmt_parse() {
    // the const head prints verbatim: `for (const t of xs)` (the
    // per-iteration opt-out) and `const x = ..;` (the binding head) —
    // a field-write target prints bare (`t.x = 1;`: the parser takes
    // whatever operand stood left of `=`, so the bare and the
    // parenthesized spellings are the same `Assign`, and the group
    // drops), and a write through a const binding prints untouched
    // (shallow by law)
    let of_once = fmt("fn f(xs: Vec<i32>) -> nil { for (const t of xs) { t.x = 1; } }");
    assert_eq!(
        of_once,
        "fn f(xs: Vec<i32>) -> nil {\n    for (const t of xs) {\n        t.x = 1;\n    }\n}\n"
    );
    assert_eq!(fmt(&of_once), of_once, "idempotent");

    let head_once = fmt("fn f() -> nil { const x: i32 = 1; x; }");
    assert_eq!(
        head_once,
        "fn f() -> nil {\n    const x: i32 = 1;\n    x;\n}\n"
    );
    assert_eq!(fmt(&head_once), head_once, "idempotent");

    let c_once = fmt("fn f(n: i32) -> nil { for (let i = 0; i < n; i += 1) { } }");
    assert_eq!(
        c_once,
        "fn f(n: i32) -> nil {\n    for (let i = 0; i < n; i += 1) {\n    }\n}\n"
    );
    assert_eq!(fmt(&c_once), c_once, "idempotent");
}

#[test]
fn plain_headers_print_exactly_as_before() {
    // the unflagged spellings are byte-identical to the pre-`const` printer
    assert_eq!(
        fmt("fn f(xs: Vec<i32>) -> nil { for (let t of xs) { } }"),
        "fn f(xs: Vec<i32>) -> nil {\n    for (let t of xs) {\n    }\n}\n"
    );
    assert_eq!(
        fmt("fn f(n: i32) -> nil { for (let i = 0; i < n; i += 1) { } }"),
        "fn f(n: i32) -> nil {\n    for (let i = 0; i < n; i += 1) {\n    }\n}\n"
    );
    assert_eq!(
        fmt("fn f() -> nil { let x = 1; x; }"),
        "fn f() -> nil {\n    let x = 1;\n    x;\n}\n"
    );
}

// ---- refusal ----

#[test]
fn a_source_with_diags_is_refused_never_guessed() {
    let ds = format("fn f( {", Mode::Impl, &Style::default());
    assert!(ds.is_err());
}
