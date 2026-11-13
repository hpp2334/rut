//! The literal fold (D3): ONE `LitNode` for every literal — the small
//! `Lit` tag is DATA, not node identity. No six near-empty arenas.

use crate::ast::expr::ExprRef;
use crate::slot_alloc;
use rut_lexer::span::Span;
use rut_lexer::token::{FloatSuffix, IntSuffix};

#[derive(Clone, Debug)]
pub struct LitNode {
    pub span: Span,
    pub lit: Lit,
}

#[derive(Clone, Debug)]
pub enum Lit {
    Int(u64, Option<IntSuffix>),
    Float(u64 /*f64 bits*/, Option<FloatSuffix>),
    Str(String),
    RawStr(String),
    Bool(bool),
    /// `nil` — the null pointer literal
    Nil,
}

slot_alloc!(LitNode, ExprRef::Lit);
