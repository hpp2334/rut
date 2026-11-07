//! Comment reattachment — the policy over the lexer's side table.
//! Comments are collected at the LEXER (`lex_collect`, the skip the
//! scanner performs anyway promoted to data — main-scan comments only;
//! f-string hole interiors are `FStr` token interiors, not document
//! trivia); this module no longer scans bytes. It classifies each
//! collected comment for the printer:
//!
//! - a run starting on the same source line as the PRECEDING token's
//!   end is **trailing** — printed after that element's final token on
//!   the same output line;
//! - otherwise it is **own-line** — printed on its own line at the
//!   current indent, before the element it precedes.
//!
//! Runs arrive in source order; the printer consumes them strictly
//! positionally (`passed` cursor), so comment ORDER is never reordered.

use rut_lexer::token::{Comment, Token};

/// One comment (or a consecutive block-comment) as reattached.
#[derive(Clone, Debug)]
pub struct CommentRun {
    /// byte start of the comment MARKER (`//` or `/*`) in the normalized source
    pub lo: u32,
    /// byte end — past the comment's last char, BEFORE a line `//`'s
    /// newline; block comments end after the closing `*/`
    pub hi: u32,
    /// true when the comment starts on the same source line the
    /// PRECEDING lexed token ended on (a trailing comment);
    /// the file's first gap has no preceding token — always false there
    pub trailing: bool,
}

fn line_of(line_starts: &[u32], off: u32) -> usize {
    line_starts.partition_point(|&s| s <= off) - 1
}

/// Reattach the artifact's collected comments (`ast.comments`) as
/// printer runs. `toks` must be the artifact's own token stream —
/// spans index the same (normalized) `src`.
pub fn runs_from_comments(comments: &[Comment], toks: &[Token], src: &str) -> Vec<CommentRun> {
    let bytes = src.as_bytes();
    // line starts (the rut-lsp LineIndex law, inlined — the fmt crate
    // keeps no LSP dependency)
    let mut line_starts: Vec<u32> = vec![0];
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            line_starts.push(i as u32 + 1);
        }
    }

    let mut runs = Vec::new();
    // gap identity: the number of tokens fully before the comment — two
    // comments between the same token pair share a gap. Only the FIRST
    // run in a gap can be trailing (deterministic; later comments on
    // that line fall own-line and reattach cleanly on their own lines).
    let mut last_gap = usize::MAX;
    for c in comments {
        let gap = toks.partition_point(|t| t.span.hi <= c.span.lo);
        let first = gap != last_gap;
        last_gap = gap;
        let trailing = first
            && gap > 0
            && line_of(&line_starts, toks[gap - 1].span.hi) == line_of(&line_starts, c.span.lo);
        runs.push(CommentRun {
            lo: c.span.lo,
            hi: c.span.hi,
            trailing,
        });
    }
    runs
}

/// Scan all comments in the source, in byte order. `toks` must be the
/// lexed tokens of the SAME (normalized) source — spans index it.
pub fn scan_comment_runs(src: &str, toks: &[Token]) -> Vec<CommentRun> {
    let bytes = src.as_bytes();
    // line starts (the rut-lsp LineIndex law, inlined — the fmt crate
    // keeps no LSP dependency)
    let mut line_starts: Vec<u32> = vec![0];
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            line_starts.push(i as u32 + 1);
        }
    }

    let mut runs = Vec::new();
    let scan_gap = |gap: (usize, usize), prev_hi: Option<u32>, runs: &mut Vec<CommentRun>| {
        if gap.0 >= gap.1 {
            return;
        }
        let mut i = gap.0;
        // classify trailing once per gap: the FIRST comment in the gap
        // starting on the prev token's line; later comments in the same
        // gap fall own-line (deterministic; comment runs after a
        // trailing comment reattach cleanly on their own lines)
        let mut trailing_anchor = prev_hi.map(|hi| line_of(&line_starts, hi));
        let mut first = true;
        while i < gap.1 {
            // skip to the next comment start
            while i < gap.1 && bytes[i] != b'/' {
                i += 1;
            }
            if i >= gap.1 {
                break;
            }
            let is_line = bytes.get(i + 1) == Some(&b'/');
            let is_block = bytes.get(i + 1) == Some(&b'*');
            if !is_line && !is_block {
                i += 1;
                continue;
            }
            let lo = i;
            let hi;
            if is_line {
                let mut j = i;
                while j < gap.1 && bytes[j] != b'\n' {
                    j += 1;
                }
                // trim the comment's own trailing spaces off the
                // printable text (the pad re-adds one deterministic one)
                let mut end = j;
                while end > lo && bytes[end - 1] == b' ' {
                    end -= 1;
                }
                hi = end;
                i = j; // the newline is left in place for line bookkeeping
            } else {
                // block comment: to the closing `*/`
                let mut j = i + 2;
                let mut closed = false;
                while j + 1 < bytes.len() {
                    if bytes[j] == b'*' && bytes[j + 1] == b'/' {
                        j += 2;
                        closed = true;
                        break;
                    }
                    j += 1;
                }
                if !closed {
                    hi = gap.1; // the lexer already diagnosed the real one; take what is here
                } else {
                    hi = j;
                }
                i = j;
            }
            // `trailing` only for the first run in a same-line gap; a
            // line comment right after a block comment on that line
            // still counts (it started on the prev token's line)
            let trailing = first
                && trailing_anchor
                    .map(|a| a == line_of(&line_starts, lo as u32))
                    .unwrap_or(false);
            runs.push(CommentRun {
                lo: lo as u32,
                hi: hi as u32,
                trailing,
            });
            first = false;
            trailing_anchor = Some(line_of(&line_starts, lo as u32 + (hi - lo) as u32));
        }
    };

    let mut prev_hi: Option<u32> = None;
    for t in toks {
        let lo = t.span.lo as usize;
        scan_gap((prev_hi.map_or(0, |h| h as usize), lo), prev_hi, &mut runs);
        prev_hi = Some(t.span.hi);
    }
    // the tail gap (comments after the final real token, before/under Eof)
    if let Some(h) = prev_hi {
        scan_gap((h as usize, bytes.len()), prev_hi, &mut runs);
    }
    runs.sort_by_key(|r| r.lo);
    runs
}
