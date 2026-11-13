//! `pub(..)? type Name = Target;` — the transparent alias (or the
//! bound-only union alias `A | B`). Non-generic by law.

use crate::ast::doc::DocData;
use crate::ast::item::class::{item_vis, ModifierNode};
use crate::ast::item::ItemRef;
use crate::ast::ty::TyRef;
use crate::slot_alloc;
use rut_core::IdentId;
use rut_lexer::span::Span;

/// `type X = A;` / `type X = A | B;` — `target` may be a plain type or
/// a `TyUnion` (bound-only, never a value type).
#[derive(Clone, Debug)]
pub struct AliasNode {
    /// starts at the item's FIRST token — the leading modifier when
    /// one is spelled, `type` otherwise
    pub span: Span,
    /// the modifier chain in source order — `vis()` derives from it
    pub modifiers: Vec<ModifierNode>,
    pub name: IdentId,
    pub name_span: Span,
    pub target: TyRef,
    pub doc: DocData,
}

impl AliasNode {
    /// the spelled item visibility — `Vis::Self_` when unannotated
    pub fn vis(&self) -> crate::ast::item::Vis {
        item_vis(&self.modifiers)
    }
}

slot_alloc!(AliasNode, ItemRef::Alias);
