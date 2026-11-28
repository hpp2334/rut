//! Access expressions: calls, method calls, field access, indexing.
//!
//! The INCOMPLETE rung: a call's `rparen` and a method's `member` are
//! OPTIONAL slots — the parser emits the partial shape with the absent
//! slot `None` before it would fence, so the tree always has a real
//! node at the typing edge (`f(1, 2` / `recv.`). [`Ast::is_incomplete`]
//! derives the verdict from the absences.

use crate::ast::expr::ExprRef;
use crate::slot_alloc;
use crate::Name;
use rut_lexer::span::Span;

#[derive(Clone, Debug)]
pub struct CallNode {
    pub span: Span,
    pub callee: ExprRef,
    pub args: Vec<ExprRef>,
    /// the `)`'s own span — `None` while the call stays unclosed (the
    /// incomplete shape)
    pub rparen: Option<Span>,
}

#[derive(Clone, Debug)]
pub struct MethodNode {
    pub span: Span,
    pub recv: ExprRef,
    /// the member — `None` for the incomplete shape (`recv.` at the
    /// typing edge: the member is being typed)
    pub member: Option<Name>,
    pub generics: Vec<crate::ast::ty::TyRef>,
    pub args: Vec<ExprRef>,
    /// the `)`'s own span — `None` while the call stays unclosed (the
    /// incomplete shape)
    pub rparen: Option<Span>,
}

#[derive(Clone, Debug)]
pub struct FieldNode {
    pub span: Span,
    pub recv: ExprRef,
    /// the member ident (`.name` — or the tuple-index token, spelled
    /// `.0`) and its own span
    pub name: Name,
}

#[derive(Clone, Debug)]
pub struct IndexNode {
    pub span: Span,
    pub recv: ExprRef,
    pub idx: ExprRef,
}

/// the postfix non-null assert `e!` — one `?` layer peeled per `!`
/// (the explicit-deref law's spelling). The span covers the whole form
/// (the operand's first token through the `!`); the trap on nil lands
/// at this site.
#[derive(Clone, Debug)]
pub struct AssertNode {
    pub span: Span,
    pub expr: ExprRef,
}

slot_alloc!(CallNode, ExprRef::Call);
slot_alloc!(MethodNode, ExprRef::Method);
slot_alloc!(FieldNode, ExprRef::Field);
slot_alloc!(IndexNode, ExprRef::Index);
slot_alloc!(AssertNode, ExprRef::Assert);
