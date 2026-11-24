//! The finalize pass — orders the classifier's output into the LSP
//! semantic-token contract.

use rut_lexer::span::Span;

use super::legend::TokenType;

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
