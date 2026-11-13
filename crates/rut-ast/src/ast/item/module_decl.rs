//! Module declarations: the root module, the file-form `mod NAME;`,
//! and the inline declaration module (Decl mode).

use crate::ast::doc::DocData;
use crate::ast::handle::Handle;
use crate::ast::item::class::{item_vis, ModifierNode};
use crate::ast::item::{ItemRef, Vis};
use crate::slot_alloc;
use rut_core::IdentId;
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
    pub name: IdentId,
    pub name_span: Span,
    pub doc: DocData,
}

/// `pub? mod NAME { <declarations> }` — the INLINE module form,
/// declaration files only: a `.d.rut` nests its declarations in the
/// grammar. Declarations only, recursively.
#[derive(Clone, Debug)]
pub struct ModInlineNode {
    /// starts at the item's FIRST token — the leading modifier when
    /// one is spelled, `mod` otherwise
    pub span: Span,
    /// the modifier chain in source order — `vis()` derives from it
    pub modifiers: Vec<ModifierNode>,
    pub name: IdentId,
    pub name_span: Span,
    pub items: Vec<ItemRef>,
    pub doc: DocData,
}

impl ModDeclNode {
    /// the spelled edge visibility — `Vis::Self_` (bare `mod`) when
    /// unannotated
    pub fn vis(&self) -> Vis {
        item_vis(&self.modifiers)
    }
}

impl ModInlineNode {
    /// the spelled edge visibility — `Vis::Self_` (bare `mod`) when
    /// unannotated
    pub fn vis(&self) -> Vis {
        item_vis(&self.modifiers)
    }
}

impl Handle<ModInlineNode> {
    pub fn items(self, ctx: &crate::Ast) -> &[ItemRef] {
        &self.get(ctx).items
    }
}

use crate::plain_alloc;
plain_alloc!(ModuleNode);
slot_alloc!(ModDeclNode, ItemRef::ModDecl);
slot_alloc!(ModInlineNode, ItemRef::ModInline);
