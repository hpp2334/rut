//! The name vocabulary — one value per recorded name.
//!
//! A name in the tree is the interner id AND the token's own span,
//! paired. [`Name`] is that pairing as one value: the decl nodes'
//! `name` field, the method call's member, the path segment's head —
//! every former `name` + `name_span` sibling pair or anonymous id/span
//! tuple reads through it (`n.name.ident` / `n.name.span`).

use rut_lexer::span::Span;

/// one name — the interner id and the token's own span, one value
#[derive(Clone, Copy, Debug)]
pub struct Name {
    pub ident: crate::IdentId,
    pub span: Span,
}
