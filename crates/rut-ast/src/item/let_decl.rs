//! Module-scope `let` — a real definition.

use crate::doc::DocData;
use crate::expr::ExprRef;
use crate::item::{ItemRef, Vis};
use crate::slot_alloc;
use crate::ty::TyRef;
use rut_core::IdentId;
use rut_lexer::span::Span;

#[derive(Clone, Debug)]
pub struct ModuleLetNode {
    pub span: Span,
    pub vis: Vis,
    pub name: IdentId,
    pub name_span: Span,
    pub ty: Option<TyRef>,
    pub init: ExprRef,
    pub doc: DocData,
}

slot_alloc!(ModuleLetNode, ItemRef::ModuleLet);
