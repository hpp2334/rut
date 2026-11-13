//! The pair artifact's LEAF LAYER — the unique survivors of the
//! archived `parse_opts` gate (the fail-fast/recover/artifact laws
//! themselves are pinned by `entry_pair.rs`). `Parsed.src` is the
//! normalized text every span indexes; the token + comment layer is
//! one deterministic call away from it — `lex_normalized(&src,
//! collect)` — and never consults the tree.
//!
//! Ported from `tests/port-pending/parse_opts.rs` (v1's `parse_opts`
//! facade); the rest of that file's laws moved onto the entry pair.

use rut_ast::parser::{parse_with_recover, Mode, ParseOpts};
use rut_lexer::lexer::lex_normalized;
use rut_lexer::token::Tok;

#[test]
fn the_leaf_layer_is_one_call_away_and_deterministic() {
    let src = "fn main() -> nil {\n    let x = 1;\n}\n";
    let p = parse_with_recover(src.to_string(), Mode::Impl, ParseOpts::default());
    let (toks_a, diags, comments) = lex_normalized(p.src(), true);
    let (toks_b, _, _) = lex_normalized(p.src(), false);
    assert!(diags.is_empty());
    assert!(comments.is_empty(), "no comments in the fixture");
    assert_eq!(
        toks_a.iter().map(|t| t.span).collect::<Vec<_>>(),
        toks_b.iter().map(|t| t.span).collect::<Vec<_>>(),
        "collecting changes nothing about the token stream"
    );
    // the tokens slice the artifact's text — ident spellings round-trip
    let ident_spans: Vec<&str> = toks_a
        .iter()
        .filter(|t| matches!(t.tok, Tok::Ident(_)))
        .map(|t| &p.src()[t.span.lo as usize..t.span.hi as usize])
        .collect();
    for want in ["fn", "main", "nil", "let", "x"] {
        assert!(ident_spans.contains(&want), "{want}: {ident_spans:?}");
    }
}

#[test]
fn comments_collect_with_the_rustc_doc_convention() {
    let src = concat!(
        "/// doc line\n",                                          // doc
        "//! inner doc\n",                                         // doc (inner)
        "// plain\n",                                              // plain
        "//// four slashes\n",                                     // plain (rustc: not doc)
        "/** doc block */\n",                                      // doc
        "/**/                        // plain empty + trailing\n", // the
        "/*! inner block */\n",                                    // doc (inner)
        "/* plain block */\n",                                     // plain
        "fn main() -> nil {\n}\n",
    );
    let p = parse_with_recover(src.to_string(), Mode::Impl, ParseOpts::default());
    let (_, diags, comments) = lex_normalized(p.src(), true);
    assert!(diags.is_empty(), "{diags:?}");
    assert_eq!(comments.len(), 9, "{comments:#?}");
    let doc_flags: Vec<bool> = comments.iter().map(|c| c.doc).collect();
    assert_eq!(
        doc_flags,
        vec![true, true, false, false, true, false, false, true, false],
        "{comments:#?}"
    );
    // text: the marker stripped, spacing left for the renderer
    assert_eq!(comments[0].text, " doc line");
    assert_eq!(comments[2].text, " plain");
    assert_eq!(comments[4].text, " doc block ");
    assert_eq!(
        comments[5].text, "",
        "the empty block comment has no interior"
    );
    // spans are in byte order and slice to the marker
    assert!(comments.windows(2).all(|w| w[0].span.hi <= w[1].span.lo));
    assert_eq!(
        &p.src()[comments[0].span.lo as usize..comments[0].span.hi as usize],
        "/// doc line"
    );
}

#[test]
fn trailing_spaces_trim_off_line_comment_spans_only() {
    let p = parse_with_recover(
        "// trailing spaces   \nfn main() -> nil {\n}\n/* keep  */\n".to_string(),
        Mode::Impl,
        ParseOpts::default(),
    );
    let (_, _, comments) = lex_normalized(p.src(), true);
    assert_eq!(comments.len(), 2);
    // the line comment's hi stops at the last non-space (the fmt
    // printable-span law); the block keeps its closing `*/`
    assert_eq!(
        &p.src()[comments[0].span.lo as usize..comments[0].span.hi as usize],
        "// trailing spaces"
    );
    assert_eq!(
        &p.src()[comments[1].span.lo as usize..comments[1].span.hi as usize],
        "/* keep  */"
    );
}

#[test]
fn fstring_hole_interiors_are_not_document_trivia() {
    // a comment inside an f-string hole is FStr TOKEN interior — the
    // bytes belong to the token, not to the document trivia table
    let src = "fn main() -> nil {\n    let s = f\"{ x /* hole */ }\";\n}\n";
    let p = parse_with_recover(src.to_string(), Mode::Impl, ParseOpts::default());
    let (_, _, comments) = lex_normalized(p.src(), true);
    assert!(
        comments.is_empty(),
        "hole interiors never collect: {comments:?}"
    );
}

#[test]
fn an_unterminated_block_comment_collects_and_diagnoses() {
    let src = "fn main() -> nil {\n}\n/* never closed";
    let p = parse_with_recover(src.to_string(), Mode::Impl, ParseOpts::default());
    let (_, diags, comments) = lex_normalized(p.src(), true);
    assert!(
        diags
            .iter()
            .any(|d| d.msg.contains("unterminated block comment")),
        "{diags:?}"
    );
    // the span still collects (to EOF) — printable trivia beats lost bytes
    assert_eq!(comments.len(), 1, "{comments:?}");
    assert_eq!(comments[0].span.hi as usize, p.src().len());
    assert_eq!(comments[0].text, " never closed");
}
