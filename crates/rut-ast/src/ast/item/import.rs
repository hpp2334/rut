//! `pub? use pkg::A::B::{C, D};` — the import (re-export) edge.

use crate::ast::doc::DocData;
use crate::ast::item::class::{item_vis, ModifierNode};
use crate::ast::item::ItemRef;
use crate::slot_alloc;
use crate::Name;
use rut_lexer::span::Span;

/// `pub? use pkg::A::B::{C, D};` / `use pkg::A::B::C;` — `path[0]` is
/// the package, the middle segments name modules, and the leaf names
/// are the imports. The flat forms (`use pkg::{A, B};` / `use pkg::A;`)
/// degenerate to `path = [pkg]`. `vis()` is the re-export edge: a `pub
/// use`'s imported names join the module's public surface (a private
/// `use` imports only). `name_span` is the PACKAGE ident's span (the
/// head — definition targets key on it); `path` and `names` carry each
/// ident's own span (the [`Name`] pairing).
#[derive(Clone, Debug)]
pub struct UseNode {
    /// starts at the item's FIRST token — the leading modifier when
    /// one is spelled, `use` otherwise
    pub span: Span,
    /// the modifier chain in source order — `vis()` derives from it
    pub modifiers: Vec<ModifierNode>,
    pub path: Vec<Name>,
    pub name_span: Span,
    pub names: Vec<Name>,
    /// the `{..}` group's span, braces included — `None` for the flat
    /// leaf (`use pkg::A;`) and for the incomplete shape (`use pkg::`
    /// at the typing edge: the group is being typed)
    pub group: Option<Span>,
    /// the glob leaf — the `*`'s own span when the use spells `pkg::*`
    /// (flat) or its brace list carries a star (`{ A, * }`). `None` for
    /// ordinary uses. A glob binds the walked path's pub surface (the
    /// expansion lives in the compile/editor lanes, not the tree); a
    /// glob leaf is a COMPLETE leaf.
    pub glob: Option<Span>,
    pub doc: DocData,
}

impl UseNode {
    /// the spelled re-export visibility — `Vis::Self_` (a private
    /// import) when unannotated
    pub fn vis(&self) -> crate::ast::item::Vis {
        item_vis(&self.modifiers)
    }
}

slot_alloc!(UseNode, ItemRef::Use);
