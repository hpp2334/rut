//! Statements and the statement slot. `ExprStmt` is folded: a bare
//! expression statement is `StmtRef::Expr(ExprRef)` — the slot embeds
//! the expr slot (whose `Error` variant is the broken-stmt recovery).

use crate::ast::error::ErrorNode;
use crate::ast::expr::{BlockNode, ExprRef};
use crate::ast::handle::{AnyHandle, Handle};
use crate::ast::ty::TyRef;
use crate::slot_alloc;
use rut_core::IdentId;
use rut_lexer::span::Span;

/// the statement walk surface
#[derive(Clone, Copy, Debug)]
pub enum StmtRef {
    Let(Handle<LetStmtNode>),
    If(Handle<IfNode>),
    While(Handle<WhileNode>),
    ForOf(Handle<ForOfNode>),
    ForC(Handle<ForCNode>),
    Return(Handle<ReturnNode>),
    Break(Handle<BreakNode>),
    Continue(Handle<ContinueNode>),
    When(Handle<WhenStmtNode>),
    /// a bare expression statement — the expr slot embedded directly
    Expr(ExprRef),
    Error(Handle<ErrorNode>),
}

impl StmtRef {
    pub fn span(self, ctx: &crate::Ast) -> Span {
        crate::slot_span!(
            self,
            ctx,
            StmtRef::Let,
            StmtRef::If,
            StmtRef::While,
            StmtRef::ForOf,
            StmtRef::ForC,
            StmtRef::Return,
            StmtRef::Break,
            StmtRef::Continue,
            StmtRef::When,
            StmtRef::Error,
            StmtRef::Expr,
        ) // the Expr arm binds an ExprRef — which has the same
          // `span(ctx)` shape, so one macro covers both
    }
    pub fn is_error(self) -> bool {
        matches!(self, StmtRef::Error(_) | StmtRef::Expr(ExprRef::Error(_)))
    }
    pub fn as_expr(self) -> Option<ExprRef> {
        match self {
            StmtRef::Expr(e) => Some(e),
            _ => None,
        }
    }
    pub fn as_let(self) -> Option<Handle<LetStmtNode>> {
        match self {
            StmtRef::Let(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_if(self) -> Option<Handle<IfNode>> {
        match self {
            StmtRef::If(h) => Some(h),
            _ => None,
        }
    }
    /// the universal address — the tree index's walk currency
    pub fn any(self) -> AnyHandle {
        crate::slot_any!(
            self,
            StmtRef::Let,
            StmtRef::If,
            StmtRef::While,
            StmtRef::ForOf,
            StmtRef::ForC,
            StmtRef::Return,
            StmtRef::Break,
            StmtRef::Continue,
            StmtRef::When,
            StmtRef::Error,
            StmtRef::Expr,
        )
    }
}

/// `else if` chains re-enter the if statement; plain `else` is a block
#[derive(Clone, Copy, Debug)]
pub enum ElseBranch {
    If(Handle<IfNode>),
    Block(Handle<BlockNode>),
}

impl ElseBranch {
    /// the universal address — the tree index's walk currency
    pub fn any(self) -> AnyHandle {
        crate::slot_any!(self, ElseBranch::If, ElseBranch::Block)
    }
}

#[derive(Clone, Debug)]
pub struct LetStmtNode {
    pub span: Span,
    /// the head spelled `const` — a shallow opt-out: reassignment of
    /// the binding itself errors, writes through it (`x.f = ..`) are
    /// allowed
    pub is_const: bool,
    pub name: IdentId,
    pub name_span: Span,
    /// `let (a, b) = ..` — tuple destructuring: each member's ident +
    /// its own token span (name_span is names\[0]'s; the per-member
    /// spans are the bindings pass's decl targets — no token recovery)
    pub destructure: Option<Vec<(IdentId, Span)>>,
    pub ty: Option<TyRef>,
    pub init: ExprRef,
}

#[derive(Clone, Debug)]
pub struct IfNode {
    pub span: Span,
    pub cond: ExprRef,
    pub then: Handle<BlockNode>,
    pub els: Option<ElseBranch>,
}

#[derive(Clone, Debug)]
pub struct WhileNode {
    pub span: Span,
    pub cond: ExprRef,
    pub body: Handle<BlockNode>,
}

#[derive(Clone, Debug)]
pub struct ForOfNode {
    pub span: Span,
    pub var: IdentId,
    pub name_span: Span,
    /// the header spelled `const` — a fresh per-iteration binding that
    /// the body may not reassign
    pub is_const: bool,
    pub iter: ExprRef,
    pub body: Handle<BlockNode>,
}

#[derive(Clone, Debug)]
pub struct ForCNode {
    pub span: Span,
    pub var: IdentId,
    pub name_span: Span,
    /// `const` in a for-c header is refused at parse (the induction
    /// variable is genuinely reassigned) — there is no flag to carry
    pub init: ExprRef,
    pub cond: ExprRef,
    pub update: ExprRef,
    pub body: Handle<BlockNode>,
}

#[derive(Clone, Debug)]
pub struct ReturnNode {
    pub span: Span,
    pub value: Option<ExprRef>,
}

#[derive(Clone, Debug)]
pub struct BreakNode {
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ContinueNode {
    pub span: Span,
}

/// `when ( expr ) { arms }` as a statement
#[derive(Clone, Debug)]
pub struct WhenStmtNode {
    pub span: Span,
    pub scrut: ExprRef,
    pub arms: Vec<Handle<crate::ast::expr::WhenArmNode>>,
}

slot_alloc!(LetStmtNode, StmtRef::Let);
use crate::plain_alloc;
plain_alloc!(IfNode);
slot_alloc!(WhileNode, StmtRef::While);
slot_alloc!(ForOfNode, StmtRef::ForOf);
slot_alloc!(ForCNode, StmtRef::ForC);
slot_alloc!(ReturnNode, StmtRef::Return);
slot_alloc!(BreakNode, StmtRef::Break);
slot_alloc!(ContinueNode, StmtRef::Continue);
slot_alloc!(WhenStmtNode, StmtRef::When);
