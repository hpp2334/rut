//! The one-call embedder API — normalize → lex → best-effort parse →
//! classify, then comments (which the lexer skips as gaps) are scanned
//! back in from the unclaimed runs between token spans. Broken source
//! degrades to token-level classes — never a wrong color.

use rut_lexer::lexer::{lex, normalize};
use rut_lexer::span::Span;
use rut_lexer::token::Token;
use rut_parser::{parse, Mode};

use crate::recover::finalize;
use crate::{classify, TokenType};

/// Classify a whole source text in one call. Spans are byte ranges into
/// the normalized (LF-only) source; the run is sorted and non-overlapping,
/// and together with the token spans it tiles the source — every byte is
/// either inside a span or between two (whitespace).
pub fn classify_source(src: &str, mode: Mode) -> Vec<(Span, TokenType)> {
    let src = normalize(src);
    let (toks, _) = lex(&src);
    // best-effort: the parser's recovery AST — diags are not this API's
    // concern, and a partial AST simply means fewer name classes
    let (ast, _) = parse(&src, mode);
    let mut out = classify(&toks, &ast);
    scan_gaps(&src, &toks, &mut out);
    finalize(out)
}

/// Scan the unclaimed runs between token spans (plus the run before the
/// first token and after the last) for `//` line comments and `/* */`
/// block comments — the same shapes the lexer skips (`lexer.rs`). An
/// unterminated block comment clamps at the gap's end: the lexer already
/// diagnosed it, and the coloring stays sane.
fn scan_gaps(src: &str, toks: &[Token], out: &mut Vec<(Span, TokenType)>) {
    let b = src.as_bytes();
    let end = src.len() as u32;
    let mut gap_lo = 0u32;
    for t in toks {
        scan_gap(b, Span::new(gap_lo, t.span.lo), out);
        gap_lo = t.span.hi;
    }
    scan_gap(b, Span::new(gap_lo, end), out);
}

fn scan_gap(b: &[u8], gap: Span, out: &mut Vec<(Span, TokenType)>) {
    let lo_end = gap.lo as usize;
    let hi_end = gap.hi as usize;
    let mut i = lo_end;
    while i < hi_end {
        if b[i] == b'/' && i + 1 < hi_end && b[i + 1] == b'/' {
            let lo = i as u32;
            while i < hi_end && b[i] != b'\n' {
                i += 1;
            }
            out.push((Span::new(lo, i as u32), TokenType::Comment));
        } else if b[i] == b'/' && i + 1 < hi_end && b[i + 1] == b'*' {
            let lo = i as u32;
            i = (i + 2).min(hi_end);
            while i + 1 < hi_end && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(hi_end);
            out.push((Span::new(lo, i as u32), TokenType::Comment));
        } else {
            i += 1;
        }
    }
}
