//! Module declarations: the root module and the file-form
//! `pub? mod NAME;` — the ONE mod spelling, both modes.

use crate::ast::doc::DocData;
use crate::ast::item::class::{item_vis, ModifierNode};
use crate::ast::item::{ItemRef, Vis};
use crate::slot_alloc;
use crate::Name;
use rut_lexer::span::Span;

/// the root module — the tree's door (`ast.root()`). No name, no doc:
/// it is the container, not a declaration.
#[derive(Clone, Debug)]
pub struct ModuleNode {
    pub span: Span,
    pub items: Vec<ItemRef>,
}

/// `pub? mod NAME;` — a file-module DECLARATION: the body lives in the
/// sibling `NAME/mod.rut`; the loader mounts it.
#[derive(Clone, Debug)]
pub struct ModDeclNode {
    /// starts at the item's FIRST token — the leading modifier when
    /// one is spelled, `mod` otherwise
    pub span: Span,
    /// the modifier chain in source order — `vis()` derives from it
    pub modifiers: Vec<ModifierNode>,
    pub name: Name,
    pub doc: DocData,
}

impl ModDeclNode {
    /// the spelled edge visibility — `Vis::Self_` (bare `mod`) when
    /// unannotated
    pub fn vis(&self) -> Vis {
        item_vis(&self.modifiers)
    }
}

use crate::plain_alloc;
plain_alloc!(ModuleNode);
slot_alloc!(ModDeclNode, ItemRef::ModDecl);
