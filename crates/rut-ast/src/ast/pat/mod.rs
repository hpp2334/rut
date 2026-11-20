//! Patterns and the pattern slot (literal/path forms).

use crate::ast::error::ErrorNode;
use crate::ast::expr::{ExprRef, PathSeg};
use crate::ast::handle::{AnyHandle, Handle};
use crate::slot_alloc;
use rut_lexer::span::Span;

#[derive(Clone, Copy, Debug)]
pub enum PatRef {
    Lit(Handle<PatLitNode>),
    Path(Handle<PatPathNode>),
    Wild(Handle<PatWildNode>),
    Else(Handle<PatElseNode>),
    Error(Handle<ErrorNode>),
}

impl PatRef {
    pub fn span(self, ctx: &crate::Ast) -> Span {
        crate::slot_span!(
            self,
            ctx,
            PatRef::Lit,
            PatRef::Path,
            PatRef::Wild,
            PatRef::Else,
            PatRef::Error,
        )
    }
    /// the universal address — the tree index's walk currency
    pub fn any(self) -> AnyHandle {
        crate::slot_any!(
            self,
            PatRef::Lit,
            PatRef::Path,
            PatRef::Wild,
            PatRef::Else,
            PatRef::Error,
        )
    }
    pub fn is_error(self) -> bool {
        matches!(self, PatRef::Error(_))
    }
}

/// literal pattern — an expression node, literal only
#[derive(Clone, Debug)]
pub struct PatLitNode {
    pub span: Span,
    pub expr: ExprRef,
}

/// enum-member path: `Color.Red`
#[derive(Clone, Debug)]
pub struct PatPathNode {
    pub span: Span,
    pub segs: Vec<PathSeg>,
}

#[derive(Clone, Debug)]
pub struct PatWildNode {
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct PatElseNode {
    pub span: Span,
}

slot_alloc!(PatLitNode, PatRef::Lit);
slot_alloc!(PatPathNode, PatRef::Path);
slot_alloc!(PatWildNode, PatRef::Wild);
slot_alloc!(PatElseNode, PatRef::Else);
