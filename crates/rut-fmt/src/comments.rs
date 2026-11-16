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
