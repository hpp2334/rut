//! Control expressions: `when`, the if EXPRESSION, `await`, the async
//! block, `is`, `as` — and the when ARM (shared by stmt/expr `when`).

use crate::expr::ExprRef;
use crate::handle::Handle;
use crate::pat::PatRef;
use crate::slot_alloc;
use crate::stmt::ElseBranch;
use crate::ty::TyRef;
use rut_lexer::span::Span;

/// `when ( expr ) { arms }` as an expression
#[derive(Clone, Debug)]
pub struct WhenExprNode {
    pub span: Span,
    pub scrut: ExprRef,
    pub arms: Vec<Handle<WhenArmNode>>,
}

/// one when arm: `pats -> body` (body is block-or-expr)
#[derive(Clone, Debug)]
pub struct WhenArmNode {
    pub span: Span,
    pub pats: Vec<PatRef>,
    pub body: ExprRef,
}

/// `if (cond) { then } else { els }` as an EXPRESSION (`else if`
/// chains hold if-STATEMENT handles, exactly like the statement form)
#[derive(Clone, Debug)]
pub struct IfExprNode {
    pub span: Span,
    pub cond: ExprRef,
    pub then: Handle<crate::expr::BlockNode>,
    pub els: Option<ElseBranch>,
}

#[derive(Clone, Debug)]
pub struct AwaitNode {
    pub span: Span,
    pub expr: ExprRef,
}

/// `async { .. }` — the async primitive: evaluating MINTS the frame;
/// its type is `Future<T>` with `T` from the body's `return`s.
#[derive(Clone, Debug)]
pub struct AsyncBlockNode {
    pub span: Span,
    pub body: Handle<crate::expr::BlockNode>,
}

/// `expr is Type` — relational, non-associative
#[derive(Clone, Debug)]
pub struct IsNode {
    pub span: Span,
    pub expr: ExprRef,
    pub ty: TyRef,
}

/// `expr as T` — the numeric cast
#[derive(Clone, Debug)]
pub struct CastNode {
    pub span: Span,
    pub expr: ExprRef,
    pub ty: TyRef,
}

use crate::plain_alloc;
plain_alloc!(WhenArmNode);
slot_alloc!(WhenExprNode, ExprRef::WhenExpr);
slot_alloc!(IfExprNode, ExprRef::If);
slot_alloc!(AwaitNode, ExprRef::Await);
slot_alloc!(AsyncBlockNode, ExprRef::AsyncBlock);
slot_alloc!(IsNode, ExprRef::Is);
slot_alloc!(CastNode, ExprRef::Cast);
