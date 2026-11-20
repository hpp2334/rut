//! Types and the type slot. `TypeKind`-era names carry the `Ty` prefix
//! (rut-core's type-table kind owns `TyKind`).

use crate::ast::error::ErrorNode;
use crate::ast::expr::PathSeg;
use crate::ast::handle::{AnyHandle, Handle};
use crate::slot_alloc;
use rut_lexer::span::Span;

#[derive(Clone, Copy, Debug)]
pub enum TyRef {
    Path(Handle<TyPathNode>),
    Self_(Handle<SelfTyNode>),
    Fn(Handle<TyFnNode>),
    Opt(Handle<TyOptNode>),
    Array(Handle<TyArrayNode>),
    Tuple(Handle<TyTupleNode>),
    Union(Handle<TyUnionNode>),
    Error(Handle<ErrorNode>),
}

impl TyRef {
    pub fn span(self, ctx: &crate::Ast) -> Span {
        crate::slot_span!(
            self,
            ctx,
            TyRef::Path,
            TyRef::Self_,
            TyRef::Fn,
            TyRef::Opt,
            TyRef::Array,
            TyRef::Tuple,
            TyRef::Union,
            TyRef::Error,
        )
    }
    /// the universal address — the tree index's walk currency
    pub fn any(self) -> AnyHandle {
        crate::slot_any!(
            self,
            TyRef::Path,
            TyRef::Self_,
            TyRef::Fn,
            TyRef::Opt,
            TyRef::Array,
            TyRef::Tuple,
            TyRef::Union,
            TyRef::Error,
        )
    }
    pub fn is_error(self) -> bool {
        matches!(self, TyRef::Error(_))
    }
    /// the `Self` type spelling — the enclosing type's own name (the
    /// tree-driven kind; no text comparison downstream)
    pub fn is_self(self) -> bool {
        matches!(self, TyRef::Self_(_))
    }
    pub fn as_path(self) -> Option<Handle<TyPathNode>> {
        match self {
            TyRef::Path(h) => Some(h),
            _ => None,
        }
    }
}

/// path type — a trait name in type position is the object type
#[derive(Clone, Debug)]
pub struct TyPathNode {
    pub span: Span,
    pub segs: Vec<PathSeg>,
}

/// `Self` — the enclosing type's own name in type position (`-> Self`,
/// `?Self`, `impl Self`, the `Self { .. }` literal's head). One token,
/// no segments.
#[derive(Clone, Debug)]
pub struct SelfTyNode {
    pub span: Span,
}

/// fn type: `fn(Store, P) -> R` — params are bare types; an omitted
/// return was already `nil`-synthesized at the parse
#[derive(Clone, Debug)]
pub struct TyFnNode {
    pub span: Span,
    pub params: Vec<TyRef>,
    pub ret: TyRef,
}

/// `?T` — the nullable, prefix-only
#[derive(Clone, Debug)]
pub struct TyOptNode {
    pub span: Span,
    pub inner: TyRef,
}

/// `[T]` — the fixed-length heap array
#[derive(Clone, Debug)]
pub struct TyArrayNode {
    pub span: Span,
    pub elem: TyRef,
}

/// tuple type `(A, B, ..)`
#[derive(Clone, Debug)]
pub struct TyTupleNode {
    pub span: Span,
    pub elems: Vec<TyRef>,
}

/// `A | B` — bound-only union (requires bounds and union aliases)
#[derive(Clone, Debug)]
pub struct TyUnionNode {
    pub span: Span,
    pub elems: Vec<TyRef>,
}

slot_alloc!(TyPathNode, TyRef::Path);
slot_alloc!(SelfTyNode, TyRef::Self_);
slot_alloc!(TyFnNode, TyRef::Fn);
slot_alloc!(TyOptNode, TyRef::Opt);
slot_alloc!(TyArrayNode, TyRef::Array);
slot_alloc!(TyTupleNode, TyRef::Tuple);
slot_alloc!(TyUnionNode, TyRef::Union);
