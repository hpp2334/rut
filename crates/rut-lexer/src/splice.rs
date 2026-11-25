//! The token splice — the token stream's edit lane, the `Doc`
//! pipeline's lane (rut-ast's `Doc` is the consumer; nothing above
//! rut-ast touches the token storage). One edit in, one WINDOWED
//! relex out: the window starts at a line boundary that is not inside
//! a construct — a multi-line or unterminated literal, or a comment
//! that spans the line start (the old stream knows the literal
//! extents; a byte scan over the token gaps finds the comments) — an
//! edit inside or at such a construct backs the window to its own
//! start, relexes the fresh text, and splices where the fresh tail
//! CONVERGES with the old suffix (same kinds, spans equal after the
//! length delta). The result is exactly the cold lex of the edited
//! text — the `splice ≡ cold lex` law, property-tested over the
//! corpus (`tests/splice_props.rs`).

use crate::lexer::{lex_normalized, lex_window};
use crate::source::{Edit, Source};
use crate::token::{FPart, Tok, Token};

/// The dirty token range the splice reports: `[start, end)` indexes
/// into the POST-splice stream — the tokens whose content came from
/// the relexed window (everything outside the range is the untouched
/// prefix or the delta-shifted suffix). `window_lo` is the byte the
/// relex window opened at — a line start, or a construct's own start
/// when the line start sat inside it (a multi-line literal or a
/// comment; 0 for a full relex). Together they draw the reuse-unit
/// line for the `Doc` pipeline: tokens before the window are
/// byte-identical text (items built purely from them survive),
/// everything at/after was relexed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenChange {
    /// first dirty token index (into the spliced stream)
    pub start: usize,
    /// one past the last dirty token
    pub end: usize,
    /// the byte the relex window opened at (post-edit coordinates;
    /// always ≤ the edit's start, so old-text spans compare directly)
    pub window_lo: u32,
}

/// Splice ONE [`Edit`] into a token stream over the edited [`Source`].
/// `toks` is the pre-edit stream (its spans index the pre-edit text);
/// `src` is the POST-edit source (text + line geometry already
/// maintained); `e` is the change between them. On return `toks` is
/// the full lex of `src.text()` — same shape and order a cold
/// [`lex_normalized`] would have produced — and the dirty range is
/// reported.
pub fn splice_tokens(toks: &mut Vec<Token>, src: &Source, e: &Edit) -> TokenChange {
    let text = src.text();
    let start = e.start as usize;
    let old_end = e.old_end as usize;
    // a structurally impossible edit (or no stream at all): the safe
    // outcome is a full relex — window = the whole file. The check is
    // new-text-shaped: the edit's inserted region must fit (`start` is
    // an old coordinate and may legitimately sit past the new length —
    // a tail deletion does exactly that)
    if toks.is_empty() || start > old_end || start + e.text.len() > text.len() {
        let (fresh, _, _, _) = lex_normalized(text, false);
        let end = fresh.len();
        *toks = fresh;
        return TokenChange {
            start: 0,
            end,
            window_lo: 0,
        };
    }

    // ── the window start ──
    // the last token starting at or before the edit (the edit may cut
    // INTO it), then that line's start — pulled back to the earliest
    // construct holding that position. A token holds it when it spans
    // the position (a multi-line literal) or ENDS at it while open —
    // an unterminated literal runs to EOF, so an edit at or after the
    // last token's end is an edit inside that token. A token start and
    // a comment opener are where a cold lex enters the construct; a
    // position strictly inside one never is.
    let anchor = toks.partition_point(|t| t.span.lo <= e.start);
    let anchor_lo = if anchor > 0 {
        toks[anchor - 1].span.lo as usize
    } else {
        0
    };
    let mut ws = src.line_start_of(anchor_lo as u32) as usize;
    if let Some(t) = toks
        .iter()
        .find(|t| t.span.lo < ws as u32 && (ws as u32) <= t.span.hi)
    {
        ws = t.span.lo as usize;
    }
    // comments are not tokens: the gaps between tokens hold nothing
    // but whitespace and comments (every other byte lexed into the
    // tokens around them), so a byte scan sees every opener — the
    // earliest one holding the line start pulls the window onto itself
    let mut gap_lo = 0usize;
    for t in toks.iter() {
        if gap_lo > ws {
            break;
        }
        if let Some(lo) = comment_holding(text, gap_lo, (t.span.lo as usize).min(text.len()), ws) {
            ws = lo;
            break;
        }
        gap_lo = (t.span.hi as usize).min(text.len());
    }
    if gap_lo <= ws {
        if let Some(lo) = comment_holding(text, gap_lo, text.len(), ws) {
            ws = lo;
        }
    }

    // ── the pieces ──
    // the kept prefix: tokens fully before the window (their text is
    // untouched, their spans still index it)
    let prefix = toks.partition_point(|t| t.span.hi <= ws as u32);
    // the old suffix: tokens starting at or past the edit's end — the
    // bytes they cover are byte-identical in the new text, shifted by
    // the length delta (hole tokens inside an FStr shift with it)
    let suffix_from = toks.partition_point(|t| t.span.lo < e.old_end);
    let delta = e.text.len() as i64 - (old_end - start) as i64;

    // ── the relex ──
    // `doc_start` rides the window's position: a BOM sequence mid-file
    // is content (the cold lex sees it as a word token), only byte 0
    // may skip
    let (mut fresh, _, _) = lex_window(&text[ws..], ws == 0);
    for t in &mut fresh {
        shift_token(t, ws as i64);
    }
    // convergence: the fresh tail must equal the shifted suffix. The
    // window opens at a lex point and the text past the edit is
    // unchanged, so this always holds; verifying (not trusting) keeps
    // the law unconditional — a miss degrades to a full relex, which
    // is still ≡ cold.
    let suffix_len = toks.len() - suffix_from;
    let converged = match fresh.len().checked_sub(suffix_len) {
        Some(k) => {
            let mut expect = toks[suffix_from..].to_vec();
            for t in &mut expect {
                shift_token(t, delta);
            }
            fresh[k..] == expect[..]
        }
        None => false,
    };
    if !converged {
        let (whole, _, _, _) = lex_normalized(text, false);
        let end = whole.len();
        *toks = whole;
        return TokenChange {
            start: 0,
            end,
            window_lo: 0,
        };
    }
    let k = fresh.len() - suffix_len;

    // ── the splice ──
    let mut tail: Vec<Token> = toks.drain(suffix_from..).collect();
    for t in &mut tail {
        shift_token(t, delta);
    }
    toks.truncate(prefix);
    toks.extend_from_slice(&fresh[..k]);
    toks.extend(tail);
    // the fresh tokens land at the truncate point — `prefix`, unless
    // the drained suffix reached UNDER it (the append-at-EOF corner:
    // the old Eof counts as kept prefix and rides the shifted tail at
    // once, so the drain already shortened the stream past `prefix`)
    let landed = prefix.min(suffix_from);
    TokenChange {
        start: landed,
        end: landed + k,
        window_lo: ws as u32,
    }
}

/// Move a token — and the hole tokens riding an `FStr` interior — by
/// `by` bytes (a suffix shift by the edit's length delta, or a window
/// lift onto absolute positions; both recurse the same way).
fn shift_token(t: &mut Token, by: i64) {
    t.span.lo = (t.span.lo as i64 + by) as u32;
    t.span.hi = (t.span.hi as i64 + by) as u32;
    if let Tok::FStr(f) = &mut t.tok {
        for part in &mut f.parts {
            if let FPart::Hole(hole) = part {
                for h in hole {
                    shift_token(h, by);
                }
            }
        }
    }
}

/// The comment holding `ws` inside one gap of the old stream — its
/// opener's offset. A gap holds nothing but whitespace and comments
/// (every other byte lexed into the tokens around it), so a byte scan
/// sees every opener. `//` runs to its newline — a `/*` inside it is
/// comment text, not an opener, and a line comment never spans a line
/// start. `/*` runs to its `*/`, or to the end of the text when open:
/// an open comment holds every position past its opener, the
/// edit-at-EOF inside it included.
fn comment_holding(text: &str, lo: usize, hi: usize, ws: usize) -> Option<usize> {
    let b = text.as_bytes();
    let mut i = lo;
    while i < hi {
        match b[i] {
            b'/' if i + 1 < hi && b[i + 1] == b'/' => {
                while i < hi && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < hi && b[i + 1] == b'*' => {
                let opener = i;
                let mut j = i + 2;
                let mut closed = false;
                while j + 1 < hi {
                    if b[j] == b'*' && b[j + 1] == b'/' {
                        j += 2;
                        closed = true;
                        break;
                    }
                    j += 1;
                }
                let holds = if closed {
                    opener < ws && ws < j
                } else {
                    opener < ws && ws <= hi
                };
                if holds {
                    return Some(opener);
                }
                i = if closed { j } else { hi };
            }
            _ => i += 1,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::splice_tokens;
    use crate::lexer::lex_normalized;
    use crate::source::{Edit, Source};
    use crate::splice::TokenChange;
    use crate::token::{FPart, Tok, Token};

    /// the lex lane's assembly: text + table + tokens from one scan
    fn src_toks(text: &str) -> (Source, Vec<Token>) {
        let (toks, _, _, lines) = lex_normalized(text, false);
        (Source::new(text.to_string(), lines), toks)
    }

    /// the pipeline order: the edit diffs, the Source is built over the
    /// old table and edited, THEN the splice sees it
    fn apply_edit(text: &str, new: &str, toks: &mut Vec<Token>) -> TokenChange {
        let e = Edit::between(text, new);
        let (_, _, _, lines) = lex_normalized(text, false);
        let mut src = Source::new(text.to_string(), lines);
        src.edit(&e);
        splice_tokens(toks, &src, &e)
    }

    #[test]
    fn a_small_mid_file_edit_relexes_a_small_window() {
        let text = "let a = 1;\nlet b = 2;\nlet c = 3;\n";
        let (src, mut toks) = src_toks(text);
        let n_before = toks.len();
        let change = apply_edit(text, "let a = 1;\nlet bb = 2;\nlet c = 3;\n", &mut toks);
        let (_, cold) = src_toks("let a = 1;\nlet bb = 2;\nlet c = 3;\n");
        assert_eq!(toks, cold, "spliced ≡ cold");
        // the dirty range is a window, not the whole stream: line 1's
        // tokens stay put, the edit's line and the shifted suffix follow
        assert!(change.start > 0, "the untouched prefix stays put");
        assert!(
            change.end - change.start < n_before,
            "a one-char insert relexes a window, not the file: {change:?}"
        );
        let _ = src;
    }

    #[test]
    fn an_edit_inside_a_literal_reanchors_to_the_literal() {
        let text = "let a = 1;\nlet s = r#\"\nline one\nline two\n\"#;\nlet n = 2;\n";
        let new = "let a = 1;\nlet s = r#\"\nline one\nline TWO\n\"#;\nlet n = 2;\n";
        let (_, mut toks) = src_toks(text);
        let change = apply_edit(text, new, &mut toks);
        let (_, cold) = src_toks(new);
        assert_eq!(toks, cold, "spliced ≡ cold across a multi-line raw string");
        // the line boundary past the edit sits INSIDE the raw string
        // (the window would re-lex literal bytes as code), so the
        // window opened at the literal's own start: line 1's tokens
        // stay, the `let s =` head of the literal's line is dirty
        // with it
        assert_eq!(change.start, 5, "{change:?}");
        let first_dirty = &toks[change.start];
        assert_eq!(&first_dirty.tok, &Tok::Ident("let".into()));
        assert_eq!(first_dirty.span.lo, 11);
    }

    #[test]
    fn an_fstr_suffix_shifts_its_holes_with_it() {
        // the edit sits BEFORE the f-string, so the FStr token rides the
        // shifted suffix — its hole tokens must move with it
        let text = "let n = 2;\nlet s = f\"x{a}y\";\n";
        let new = "let q = 0;\nlet n = 2;\nlet s = f\"x{a}y\";\n";
        let (_, mut toks) = src_toks(text);
        apply_edit(text, new, &mut toks);
        let (_, cold) = src_toks(new);
        assert_eq!(toks, cold, "spliced ≡ cold");
        let (span, hole_lo) = toks
            .iter()
            .find_map(|t: &Token| match &t.tok {
                Tok::FStr(f) => {
                    let hole = f.parts.iter().find_map(|p| match p {
                        FPart::Hole(h) => h.iter().find(|t| t.tok != Tok::Eof),
                        FPart::Lit(_) => None,
                    });
                    Some((t.span, hole.map(|h| h.span)))
                }
                _ => None,
            })
            .expect("the f-string token survives");
        assert_eq!(span.lo, 30, "the token shifted by the inserted 11 bytes");
        assert_eq!(hole_lo.map(|s| s.lo), Some(34), "the hole shifted with it");
    }

    #[test]
    fn appending_at_eof_converges_through_eof() {
        let text = "let a = 1;\n";
        let new = "let a = 1;\nlet b = 2;\n";
        let (_, mut toks) = src_toks(text);
        apply_edit(text, new, &mut toks);
        let (_, cold) = src_toks(new);
        assert_eq!(toks, cold);
        assert_eq!(toks.last().unwrap().tok, Tok::Eof);
    }

    #[test]
    fn deleting_everything_leaves_the_eof_token() {
        let text = "let a = 1;\nlet b = 2;\n";
        let (_, mut toks) = src_toks(text);
        let e = Edit::between(text, "");
        let (_, _, _, lines) = lex_normalized(text, false);
        let mut src = Source::new(text.to_string(), lines);
        src.edit(&e);
        splice_tokens(&mut toks, &src, &e);
        assert_eq!(src.text(), "");
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].tok, Tok::Eof);
        assert_eq!(toks[0].span.lo, 0);
        assert_eq!(toks[0].span.hi, 0);
    }

    #[test]
    fn deleting_the_tail_keeps_the_window_open() {
        // old_end sits past the new text's length here — the sanity
        // check must not misread that as an impossible edit
        let text = "let a = 1;\nlet b = 2;\nlet c = 3;\n";
        let new = "let a = 1;\nlet b = 2;\nlet c";
        let (_, mut toks) = src_toks(text);
        let change = apply_edit(text, new, &mut toks);
        let (_, cold) = src_toks(new);
        assert_eq!(toks, cold);
        assert!(
            change.end < toks.len(),
            "the shifted Eof suffix survived: {change:?}"
        );
    }

    #[test]
    fn an_unterminated_literal_after_the_edit_stays_cold_equal() {
        // the relexed window produces a diagnostic (dropped — the Doc
        // pipeline re-derives diagnostics); the TOKEN stream must still
        // equal the cold lex
        let text = "let a = \"ok\";\nlet b = 2;\n";
        let new = "let a = \"ok\nlet b = 2;\n";
        let (_, mut toks) = src_toks(text);
        apply_edit(text, new, &mut toks);
        let (_, cold) = src_toks(new);
        assert_eq!(toks, cold);
    }

    // ── law 3's pins: the re-lex window opens where a cold lex opens ──

    #[test]
    fn a_window_never_opens_inside_a_block_comment() {
        // ground truth 11, both literals. (1) an append at EOF after
        // an unterminated block comment: the old line start at the edit
        // sits INSIDE the comment — the window must pull back to the
        // comment's opener, or the `}` lexes as code
        let text = "let a = 1;\n/* never closed\n";
        let new = "let a = 1;\n/* never closed\n}\n";
        let (_, mut toks) = src_toks(text);
        let change = apply_edit(text, new, &mut toks);
        let (_, cold) = src_toks(new);
        assert_eq!(toks, cold, "spliced ≡ cold across the open comment");
        assert!(
            change.window_lo <= 11,
            "the window opened inside the comment: {change:?}"
        );
        // (2) a rename whose line start sits inside a CLOSED multi-line
        // comment — same law, the comment's bytes are no code
        let text = "let a = 1;\n/* multi\n   line */ fn foo() {}\nlet x = 1;\n";
        let new = "let a = 1;\n/* multi\n   line */ fn foobar() {}\nlet x = 1;\n";
        let (_, mut toks) = src_toks(text);
        let change = apply_edit(text, new, &mut toks);
        let (_, cold) = src_toks(new);
        assert_eq!(toks, cold, "spliced ≡ cold across the closed comment");
        assert!(
            change.window_lo <= 11,
            "the window opened inside the comment: {change:?}"
        );
    }

    #[test]
    fn an_edit_at_eof_relexes_an_open_construct() {
        // ground truth 11: an edit at/after the last token's end is an
        // edit INSIDE the construct when the construct is still open —
        // the unterminated comment and each literal opener relex from
        // their own start
        let cases = [
            ("let a = 1;\n/* never closed\n", 11u32),
            ("let a = 1;\nr#\"unterminated\n", 11),
            ("let a = 1;\nr##\"unterminated\n", 11),
            ("let a = 1;\nf#\"unterminated\n", 11),
        ];
        for (text, open_lo) in cases {
            let mut new = text.to_string();
            new.push_str("}\n");
            let (_, mut toks) = src_toks(text);
            let change = apply_edit(text, &new, &mut toks);
            let (_, cold) = src_toks(&new);
            assert_eq!(toks, cold, "spliced ≡ cold over {text:?}");
            assert!(
                change.window_lo <= open_lo,
                "the window opened inside the open construct: {change:?} over {text:?}"
            );
        }
    }
}
