//! Patterns and the pattern slot (constructor/binding forms).

use crate::error::ErrorNode;
use crate::expr::{ExprRef, PathSeg};
use crate::handle::Handle;
use crate::slot_alloc;
use rut_core::IdentId;
use rut_lexer::span::Span;

#[derive(Clone, Copy, Debug)]
pub enum PatRef {
    Lit(Handle<PatLitNode>),
    Path(Handle<PatPathNode>),
    Ctor(Handle<PatCtorNode>),
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
            PatRef::Ctor,
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

/// constructor pattern: `Ok(cfg)`, `Option<T>.Some(x)`, `Some(_)` —
/// args are binding names (`None` = `_`)
#[derive(Clone, Debug)]
pub struct PatCtorNode {
    pub span: Span,
    pub segs: Vec<PathSeg>,
    pub args: Vec<Option<IdentId>>,
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
slot_alloc!(PatCtorNode, PatRef::Ctor);
slot_alloc!(PatWildNode, PatRef::Wild);
slot_alloc!(PatElseNode, PatRef::Else);
