//! `use pkg::A::B::{C, D};` — the import edge.

use crate::doc::DocData;
use crate::item::ItemRef;
use crate::slot_alloc;
use rut_core::IdentId;
use rut_lexer::span::Span;

/// `use pkg::A::B::{C, D};` / `use pkg::A::B::C;` — `path[0]` is the
/// package, the middle segments name modules, and the leaf names are
/// the imports. The flat forms (`use pkg::{A, B};` / `use pkg::A;`)
/// degenerate to `path = [pkg]`. `name_span` is the PACKAGE ident's
/// span (the head — definition targets key on it); `name_spans` are
/// the leaf idents' spans, parallel to `names`.
#[derive(Clone, Debug)]
pub struct UseNode {
    pub span: Span,
    pub path: Vec<IdentId>,
    pub name_span: Span,
    pub names: Vec<IdentId>,
    pub name_spans: Vec<Span>,
    pub doc: DocData,
}

slot_alloc!(UseNode, ItemRef::Use);
