//! Trivia verdicts — doc chains on the tree (D7).
//!
//! `CommentData` is ONE comment's verdict: its span (marker INCLUDED,
//! trailing whitespace trimmed off line comments — the printable-span
//! law) and the rustc-convention `doc` flag (`///` `//!` `/**` `/*!`;
//! `////` and `/**/` are plain). `DocData` is the attached chain, a
//! field on every decl-shaped node, populated at ALLOCATION time when
//! `include_comments` is set — the grammar never consults comments;
//! tree shape never depends on them.
//!
//! SPANS, never text: the interior is a pure function of (span, src),
//! sliced on demand — the artifact law. `lines(src)` yields borrowed
//! views; wire types own their copies at their own boundary.

use rut_lexer::span::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommentData {
    /// the comment's full span — marker included; line comments trim
    /// trailing whitespace off `hi`; unterminated blocks clamp at EOF
    pub span: Span,
    /// the rustc doc convention: `///` `//!` `/**` `/*!` mark doc
    /// comments; plain `//` rows ride the chain too (the Go gate —
    /// byte-identical to the hover behavior this replaces); the flag
    /// stays available for a future render-side rustc gate
    pub doc: bool,
}

impl CommentData {
    /// the comment's interior — marker stripped, a VIEW into src
    pub fn text<'a>(&self, src: &'a str) -> &'a str {
        let whole = &src[self.span.lo as usize..self.span.hi as usize];
        let rest = if let Some(r) = whole.strip_prefix("///") {
            r
        } else if let Some(r) = whole.strip_prefix("//!") {
            r
        } else if let Some(r) = whole.strip_prefix("/**") {
            r.strip_suffix("*/").unwrap_or(r)
        } else if let Some(r) = whole.strip_prefix("/*!") {
            r.strip_suffix("*/").unwrap_or(r)
        } else if let Some(r) = whole.strip_prefix("//") {
            r
        } else if let Some(r) = whole.strip_prefix("/*") {
            r.strip_suffix("*/").unwrap_or(r)
        } else {
            whole
        };
        rest.trim_start()
    }
}

/// the doc chain attached to a decl — spans only, in source order.
/// Empty ⇒ no docs. Populated by the parse lane when
/// `ParseOpts::include_comments` is set; `DocData::default()` otherwise.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocData(pub Vec<CommentData>);

impl DocData {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// the rendered doc lines — borrowed views into src, zero copies
    pub fn lines<'a>(&'a self, src: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.0.iter().map(move |c| c.text(src))
    }
}
