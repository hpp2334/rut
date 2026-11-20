//! Access expressions: calls, method calls, field access, indexing.
//!
//! The INCOMPLETE rung: a call's `rparen` and a method's `member` are
//! OPTIONAL slots — the parser emits the partial shape with the absent
//! slot `None` before it would fence, so the tree always has a real
//! node at the typing edge (`f(1, 2` / `recv.`). [`Ast::is_incomplete`]
//! derives the verdict from the absences.

use crate::ast::expr::ExprRef;
use crate::slot_alloc;
use rut_core::IdentId;
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

/// one member ident — the name and its own token span (`.`name`, or
/// the tuple-index token spelled `.0`)
#[derive(Clone, Copy, Debug)]
pub struct MemberIdent {
    pub name: IdentId,
    pub name_span: Span,
}

#[derive(Clone, Debug)]
pub struct MethodNode {
    pub span: Span,
    pub recv: ExprRef,
    /// the member — `None` for the incomplete shape (`recv.` at the
    /// typing edge: the member is being typed)
    pub member: Option<MemberIdent>,
    pub generics: Vec<crate::ast::ty::TyRef>,
    pub args: Vec<ExprRef>,
    /// the `)`'s own span — `None` while the call stays unclosed (the
    /// incomplete shape)
    pub rparen: Option<Span>,
}

impl MethodNode {
    /// the member's name + its own span, when present
    pub fn member(&self) -> Option<(IdentId, Span)> {
        self.member.as_ref().map(|m| (m.name, m.name_span))
    }
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
