//! The AST facade's own gate — `parse_opts` + `Parsed` + the
//! self-contained artifact law. The parser's grammar tests live in the
//! other files here; these pin the FACADE contract:
//!
//! - fail-fast (the default): the first diag stops the parse — no
//!   tree, one row; `recover` buys the total tree + every row
//! - the artifact is self-contained: `ast.src` is the normalized text
//!   its spans index, and NOTHING derivable is stored beside it
//! - the leaf layer is one deterministic call away: comments + tokens
//!   via `lex_normalized(&ast.src, collect)`

use rut_lexer::lexer::{lex_normalized, normalize};
use rut_parser::{parse, parse_opts, Mode, ParseOpts};

#[test]
fn fail_fast_is_the_default_and_stops_at_the_first_diag() {
    // clean source: a tree, no diags
    let p = parse_opts("fn main() -> nil {\n}\n", Mode::Impl, ParseOpts::default());
    assert!(p.ast.is_some(), "a clean parse yields the tree");
    assert!(p.diags.is_empty(), "{:?}", p.diags);

    // broken source, DEFAULT opts: ast None, exactly ONE diag
    let p = parse_opts(
        "fn main( -> nil {\n    let = ;\n}\n",
        Mode::Impl,
        ParseOpts::default(),
    );
    assert!(p.ast.is_none(), "fail-fast carries no tree");
    assert_eq!(p.diags.len(), 1, "exactly the first diag: {:?}", p.diags);
}

#[test]
fn recover_yields_the_total_tree_and_every_diag() {
    let src = "fn main( -> nil {\n    let = ;\n}\n";
    let p = parse_opts(src, Mode::Impl, ParseOpts { recover: true });
    let ast = p.ast.expect("the recovering parse always yields a tree");
    assert!(
        !p.diags.is_empty(),
        "recovering keeps every diag: {:?}",
        p.diags.len()
    );
    // the legacy entry is exactly this shape (the phase-3 retirement
    // keeps behavior: same diags, same total tree)
    let (legacy_ast, legacy_diags) = parse(src, Mode::Impl);
    assert_eq!(
        legacy_diags.len(),
        p.diags.len(),
        "the legacy entry is exactly {{recover: true}}"
    );
    assert_eq!(
        legacy_diags
            .iter()
            .map(|d| (d.span, d.msg.clone()))
            .collect::<Vec<_>>(),
        p.diags
            .iter()
            .map(|d| (d.span, d.msg.clone()))
            .collect::<Vec<_>>(),
        "same diag rows, same order"
    );
    assert_eq!(legacy_ast.root, ast.root);
}

#[test]
fn the_artifact_is_self_contained_and_normalized() {
    // CRLF input: spans index the NORMALIZED text the artifact carries
    let p = parse_opts(
        "fn main() -> nil {\r}\r\n",
        Mode::Impl,
        ParseOpts { recover: true },
    );
    let ast = p.ast.expect("the recovering parse always yields a tree");
    assert_eq!(ast.src, "fn main() -> nil {\n}\n", "src is CRLF-normalized");
    assert_eq!(ast.src, normalize("fn main() -> nil {\r}\r\n"));
    // every node span slices the artifact's own text
    for n in &ast.nodes {
        let (lo, hi) = (n.span.lo as usize, n.span.hi as usize);
        assert!(
            lo <= hi && hi <= ast.src.len(),
            "span {}..{} indexes ast.src",
            lo,
            hi
        );
    }
}

#[test]
fn the_leaf_layer_is_one_call_away_and_deterministic() {
    let src = "fn main() -> nil {\n    let x = 1;\n}\n";
    let p = parse_opts(src, Mode::Impl, ParseOpts::default());
    let ast = p.ast.unwrap();
    let (toks_a, diags, comments) = lex_normalized(&ast.src, true);
    let (toks_b, _, _) = lex_normalized(&ast.src, false);
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
        .filter(|t| matches!(t.tok, rut_lexer::token::Tok::Ident(_)))
        .map(|t| &ast.src[t.span.lo as usize..t.span.hi as usize])
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
    let p = parse_opts(src, Mode::Impl, ParseOpts::default());
    let ast = p.ast.unwrap();
    let (_, diags, comments) = lex_normalized(&ast.src, true);
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
        &ast.src[comments[0].span.lo as usize..comments[0].span.hi as usize],
        "/// doc line"
    );
}

#[test]
fn trailing_spaces_trim_off_line_comment_spans_only() {
    let p = parse_opts(
        "// trailing spaces   \nfn main() -> nil {\n}\n/* keep  */\n",
        Mode::Impl,
        ParseOpts::default(),
    );
    let ast = p.ast.unwrap();
    let (_, _, comments) = lex_normalized(&ast.src, true);
    assert_eq!(comments.len(), 2);
    // the line comment's hi stops at the last non-space (the fmt
    // printable-span law); the block keeps its closing `*/`
    assert_eq!(
        &ast.src[comments[0].span.lo as usize..comments[0].span.hi as usize],
        "// trailing spaces"
    );
    assert_eq!(
        &ast.src[comments[1].span.lo as usize..comments[1].span.hi as usize],
        "/* keep  */"
    );
}

#[test]
fn fstring_hole_interiors_are_not_document_trivia() {
    // a comment inside an f-string hole is FStr TOKEN interior — the
    // bytes belong to the token, not to the document trivia table
    let src = "fn main() -> nil {\n    let s = f\"{ x /* hole */ }\";\n}\n";
    let p = parse_opts(src, Mode::Impl, ParseOpts::default());
    let ast = p.ast.unwrap();
    let (_, _, comments) = lex_normalized(&ast.src, true);
    assert!(
        comments.is_empty(),
        "hole interiors never collect: {comments:?}"
    );
}

#[test]
fn an_unterminated_block_comment_collects_and_diagnoses() {
    let src = "fn main() -> nil {\n}\n/* never closed";
    let p = parse_opts(src, Mode::Impl, ParseOpts { recover: true });
    let ast = p.ast.unwrap();
    let (_, diags, comments) = lex_normalized(&ast.src, true);
    assert!(
        diags
            .iter()
            .any(|d| d.msg.contains("unterminated block comment")),
        "{diags:?}"
    );
    // the span still collects (to EOF) — printable trivia beats lost bytes
    assert_eq!(comments.len(), 1, "{comments:?}");
    assert_eq!(comments[0].span.hi as usize, ast.src.len());
    assert_eq!(comments[0].text, " never closed");
}
