//! `pub? use pkg::A::B::{C, D};` — the import (re-export) edge.

use crate::ast::doc::DocData;
use crate::ast::item::{ItemRef, Vis};
use crate::slot_alloc;
use rut_core::IdentId;
use rut_lexer::span::Span;

/// `pub? use pkg::A::B::{C, D};` / `use pkg::A::B::C;` — `path[0]` is
/// the package, the middle segments name modules, and the leaf names
/// are the imports. The flat forms (`use pkg::{A, B};` / `use pkg::A;`)
/// degenerate to `path = [pkg]`. `vis` is the re-export edge: a `pub
/// use`'s imported names join the module's public surface (a private
/// `use` imports only). `name_span` is the PACKAGE ident's span (the
/// head — definition targets key on it); `name_spans` are the leaf
/// idents' spans, parallel to `names`.
#[derive(Clone, Debug)]
pub struct UseNode {
    pub span: Span,
    pub vis: Vis,
    pub path: Vec<IdentId>,
    pub name_span: Span,
    pub names: Vec<IdentId>,
    pub name_spans: Vec<Span>,
    pub doc: DocData,
}

slot_alloc!(UseNode, ItemRef::Use);
