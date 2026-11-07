//! `pub(..)? type Name = Target;` — the transparent alias (or the
//! bound-only union alias `A | B`). Non-generic by law.

use crate::doc::DocData;
use crate::item::ItemRef;
use crate::slot_alloc;
use crate::ty::TyRef;
use rut_core::IdentId;
use rut_lexer::span::Span;

/// `type X = A;` / `type X = A | B;` — `target` may be a plain type or
/// a `TyUnion` (bound-only, never a value type).
#[derive(Clone, Debug)]
pub struct AliasNode {
    pub span: Span,
    pub vis: crate::item::Vis,
    pub name: IdentId,
    pub name_span: Span,
    pub target: TyRef,
    pub doc: DocData,
}

slot_alloc!(AliasNode, ItemRef::Alias);
