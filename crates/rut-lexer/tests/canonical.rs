//! The canonical-door law — `lex_canonical` / `canonical`.
//!
//! `lex_canonical` takes the source BY VALUE: a clean (LF-only) input
//! MOVES through unchanged (zero copies — pinned by pointer identity),
//! a dirty one rides the normalize pass and the NORMALIZED text is what
//! comes back. Either way the tokens' spans index exactly the returned
//! text: the caller cannot obtain spans over a text it does not hold.
//! `canonical` is the borrow twin — no tokens, `Cow::Borrowed` when
//! clean, one owned pass when dirty.

use rut_lexer::lexer::{canonical, lex_canonical, lex_normalized};
use rut_lexer::token::Tok;
use std::borrow::Cow;

#[test]
fn clean_input_moves_through_pointer_unchanged() {
    // the zero-copy pin: the returned text IS the input buffer —
    // same allocation (a copy would land at a different address)
    let src = "let x = 1;\nfn f() -> nil {\n    return x;\n}\n".to_string();
    let ptr = src.as_ptr();
    let (text, toks, diags, _) = lex_canonical(src, true);
    assert_eq!(text.as_ptr(), ptr, "a clean input must move, not copy");
    assert!(diags.is_empty());
    assert!(!toks.is_empty());
    // and the spans join the returned text (the pair law)
    let first = &toks[0];
    assert_eq!(&text[first.span.lo as usize..first.span.hi as usize], "let");
}

#[test]
fn dirty_input_returns_the_normalized_text_spans_index() {
    // CRLF in: the normalize pass runs, and the text that comes back is
    // the LF twin — the tokens lex over THAT text, so every span slices
    // the returned String exactly
    let src = "let a = 1;\r\nlet b = a;\r\n".to_string();
    let (text, toks, diags, _) = lex_canonical(src, true);
    assert_eq!(text, "let a = 1;\nlet b = a;\n");
    assert!(diags.is_empty());
    for t in &toks {
        // every span is in-file over the RETURNED text (the pair law)
        // and never covers a `\r` — the text the caller holds IS the
        // text the spans index
        assert!(
            t.span.hi as usize <= text.len(),
            "span out of file: {}..{} over {} bytes",
            t.span.lo,
            t.span.hi,
            text.len()
        );
        let slice = &text[t.span.lo as usize..t.span.hi as usize];
        assert!(!slice.contains('\r'), "no \\r anywhere: {slice:?}");
    }
}

#[test]
fn a_lone_cr_becomes_a_line_break_too() {
    // the dirty mapping pinned at the leaf layer: paired AND lone `\r`
    // both land as `\n`
    let (text, toks, diags, _) = lex_canonical("let a = 1;\rlet b = a;\r".to_string(), true);
    assert_eq!(text, "let a = 1;\nlet b = a;\n");
    assert!(diags.is_empty());
    // the second `let` starts a line — the lone `\r` terminated the first
    let lets: Vec<_> = toks
        .iter()
        .filter(|t| matches!(&t.tok, Tok::Ident(s) if s == "let"))
        .collect();
    assert_eq!(lets.len(), 2);
    let nl = text.as_bytes()[lets[1].span.lo as usize - 1];
    assert_eq!(nl, b'\n', "the lone \\r became a line break");
}

#[test]
fn canonical_borrows_when_clean_and_owns_when_dirty() {
    let clean = "let x = 1;\n";
    assert!(matches!(canonical(clean), Cow::Borrowed(_)));

    let dirty = "let x = 1;\r\n";
    let Cow::Owned(owned) = canonical(dirty) else {
        panic!("a dirty input must come back owned");
    };
    assert_eq!(owned, "let x = 1;\n");
}

#[test]
fn canonical_text_lexes_identically_through_the_borrow_door() {
    // the shared-law pin: lex over canonical()'s output equals
    // lex_canonical's own token stream — one normalization, either door
    let src = "fn f(x: i32) -> i32 {\r\n    return x;\r\n}\r\n";
    let (text, toks, diags, _) = lex_canonical(src.to_string(), true);
    let (toks2, diags2, _) = lex_normalized(&canonical(src), true);
    assert_eq!(text, canonical(src).into_owned());
    assert_eq!(diags.len(), diags2.len());
    assert_eq!(toks.len(), toks2.len());
    for (a, b) in toks.iter().zip(&toks2) {
        assert_eq!(a.span, b.span);
    }
}
