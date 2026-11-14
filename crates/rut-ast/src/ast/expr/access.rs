//! Access expressions: calls, method calls, field access, indexing.

use crate::ast::expr::ExprRef;
use crate::slot_alloc;
use rut_core::IdentId;
use rut_lexer::span::Span;

#[derive(Clone, Debug)]
pub struct CallNode {
    pub span: Span,
    pub callee: ExprRef,
    pub args: Vec<ExprRef>,
}

#[derive(Clone, Debug)]
pub struct MethodNode {
    pub span: Span,
    pub recv: ExprRef,
    pub name: IdentId,
    /// the member ident token's span (`.name` — or the tuple-index
    /// token's, spelled `.0`)
    pub name_span: Span,
    pub generics: Vec<crate::ast::ty::TyRef>,
    pub args: Vec<ExprRef>,
}

#[derive(Clone, Debug)]
pub struct FieldNode {
    pub span: Span,
    pub recv: ExprRef,
    pub name: IdentId,
    /// the member ident token's span (`.name` — or the tuple-index
    /// token's, spelled `.0`)
    pub name_span: Span,
}

#[derive(Clone, Debug)]
pub struct IndexNode {
    pub span: Span,
    pub recv: ExprRef,
    pub idx: ExprRef,
}

slot_alloc!(CallNode, ExprRef::Call);
slot_alloc!(MethodNode, ExprRef::Method);
slot_alloc!(FieldNode, ExprRef::Field);
slot_alloc!(IndexNode, ExprRef::Index);
