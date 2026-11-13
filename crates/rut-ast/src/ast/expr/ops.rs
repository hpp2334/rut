//! Operators: unary/binary/assignment. `UnOp`/`BinOp` are small DATA
//! enums — payload, not node identity.

use crate::ast::expr::ExprRef;
use crate::slot_alloc;
use rut_lexer::span::Span;

#[derive(Clone, Debug)]
pub struct UnaryNode {
    pub span: Span,
    pub op: UnOp,
    pub expr: ExprRef,
}

#[derive(Clone, Debug)]
pub struct BinaryNode {
    pub span: Span,
    pub op: BinOp,
    pub lhs: ExprRef,
    pub rhs: ExprRef,
}

/// `target = v` / `target op= v`
#[derive(Clone, Debug)]
pub struct AssignNode {
    pub span: Span,
    pub op: Option<BinOp>,
    pub target: ExprRef,
    pub value: ExprRef,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or, // logical short-circuit
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

slot_alloc!(UnaryNode, ExprRef::Unary);
slot_alloc!(BinaryNode, ExprRef::Binary);
slot_alloc!(AssignNode, ExprRef::Assign);
