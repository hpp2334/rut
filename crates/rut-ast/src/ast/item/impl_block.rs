//! `impl T { .. }` — the inherent impl: braced bodies, methods only.
//! Generic binders are DECLARED after `impl` (`impl<T> Vec<T>`).

use crate::ast::item::{ItemRef, MethodRef};
use crate::ast::ty::TyRef;
use crate::slot_alloc;
use rut_core::IdentId;
use rut_lexer::span::Span;

#[derive(Clone, Debug)]
pub struct ImplNode {
    pub span: Span,
    /// the declared generic binders — the definition site
    pub generics: Vec<IdentId>,
    /// bounds accepted for grammar uniformity; the collector ignores
    /// them (admission is the fn/class machinery's business)
    pub bounds: Vec<(IdentId, TyRef)>,
    pub target: TyRef,
    pub methods: Vec<MethodRef>,
}

slot_alloc!(ImplNode, ItemRef::Impl);
