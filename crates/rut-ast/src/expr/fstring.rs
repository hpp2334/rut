//! f-strings: the hole tiling. Parts are literal text or expression
//! holes — the holes are expression SLOTS (an unparseable hole is an
//! `Error` slot, not a failed string).

use crate::expr::ExprRef;
use crate::slot_alloc;
use rut_lexer::span::Span;

#[derive(Clone, Debug)]
pub enum FPartAst {
    Lit(String),
    Hole(ExprRef),
}

#[derive(Clone, Debug)]
pub struct FStrNode {
    pub span: Span,
    pub parts: Vec<FPartAst>,
}

slot_alloc!(FStrNode, ExprRef::FStr);
