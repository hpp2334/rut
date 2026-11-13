//! Module-scope `let` — a real definition.

use crate::ast::doc::DocData;
use crate::ast::expr::ExprRef;
use crate::ast::item::class::{item_vis, ModifierNode};
use crate::ast::item::{ItemRef, Vis};
use crate::ast::ty::TyRef;
use crate::slot_alloc;
use rut_core::IdentId;
use rut_lexer::span::Span;

#[derive(Clone, Debug)]
pub struct ModuleLetNode {
    /// starts at the item's FIRST token — the leading modifier when
    /// one is spelled, `let` otherwise
    pub span: Span,
    /// the modifier chain in source order — `vis()` derives from it
    pub modifiers: Vec<ModifierNode>,
    pub name: IdentId,
    pub name_span: Span,
    pub ty: Option<TyRef>,
    pub init: ExprRef,
    pub doc: DocData,
}

impl ModuleLetNode {
    /// the spelled item visibility — `Vis::Self_` when unannotated
    pub fn vis(&self) -> Vis {
        item_vis(&self.modifiers)
    }
}

slot_alloc!(ModuleLetNode, ItemRef::ModuleLet);
