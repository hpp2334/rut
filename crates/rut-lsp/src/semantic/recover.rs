//! Name recovery + helpers — matching AST names back to `Ident` tokens
//! inside a construct's span, and the finalize pass that orders the
//! classifier's output.

use rut_lexer::span::Span;
use rut_lexer::token::{Tok, Token};

use super::legend::{owned_by_tokens, TokenType};

/// Tokens fully inside `span`, in source order.
pub(crate) fn toks_in<'a>(toks: &'a [Token], span: Span) -> impl Iterator<Item = &'a Token> {
    toks.iter()
        .skip_while(move |t| t.span.lo < span.lo)
        .take_while(move |t| t.span.hi <= span.hi)
}

/// Find the span of the `Ident` token whose text is `name` inside `span` —
/// `last` for trailing positions (method/field access), else the first.
pub(crate) fn find_name(toks: &[Token], span: Span, name: &str, last: bool) -> Option<Span> {
    let mut found = None;
    for t in toks_in(toks, span) {
        if let Tok::Ident(s) = &t.tok {
            if s == name {
                found = Some(t.span);
                if !last {
                    break;
                }
            }
        }
    }
    found
}

pub(crate) fn push_name(
    toks: &[Token],
    span: Span,
    name: &str,
    ty: TokenType,
    out: &mut Vec<(Span, TokenType)>,
    last: bool,
) {
    if owned_by_tokens(name) {
        return; // not a name position
    }
    if let Some(s) = find_name(toks, span, name, last) {
        out.push((s, ty));
    }
}

/// Sort by start; drop empty and overlapping spans. At equal starts the
/// entry pushed LAST wins (the AST pass runs after the token pass, so
/// identifier classes beat literal classes there); stable sort keeps the
/// push order inside a group.
pub(crate) fn finalize(mut raw: Vec<(Span, TokenType)>) -> Vec<(Span, TokenType)> {
    raw.sort_by_key(|(s, _)| s.lo);
    // walk right-to-left so the group's last entry is taken first
    let mut kept: Vec<(Span, TokenType)> = Vec::with_capacity(raw.len());
    let mut i = raw.len();
    while i > 0 {
        // start of the equal-lo group that ends at i-1
        let lo = raw[i - 1].0.lo;
        let mut j = i - 1;
        while j > 0 && raw[j - 1].0.lo == lo {
            j -= 1;
        }
        let (sp, ty) = raw[i - 1]; // last of the group — AST's entry
        if sp.hi > sp.lo {
            let fits = kept.last().map(|(k, _)| sp.hi <= k.lo).unwrap_or(true);
            if fits {
                kept.push((sp, ty));
            }
        }
        i = j;
    }
    kept.reverse();
    kept
}
