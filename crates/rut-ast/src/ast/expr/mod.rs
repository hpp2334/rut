//! Expressions and the expression slot. `BlockNode` is an expression
//! (fn/loop/if bodies, block arms); `PathSeg` is the shared
//! path-segment vocabulary (expr paths, type paths, pattern paths).

mod access;
mod control;
mod fstring;
mod lit;
mod ops;

pub use access::{CallNode, FieldNode, IndexNode, MethodNode};
pub use control::{
    AsyncBlockNode, AwaitNode, CastNode, IfExprNode, IsNode, WhenArmNode, WhenExprNode,
};
pub use fstring::{FPartAst, FStrNode};
pub use lit::{Lit, LitNode};
pub use ops::{AssignNode, BinOp, BinaryNode, UnOp, UnaryNode};

use crate::ast::error::ErrorNode;
use crate::ast::handle::Handle;
use rut_lexer::span::Span;

/// one dotted-path segment, generic args included — shared by expr
/// paths, type paths, and pattern paths.
#[derive(Clone, Debug)]
pub struct PathSeg {
    pub name: rut_core::IdentId,
    /// generic arguments — type slots (incl. const-generic `TyConst`)
    pub generics: Vec<crate::ast::ty::TyRef>,
}

/// the expression walk surface — a tagged handle per kind + the
/// recovery slot (an ErrorNode may occupy any expr position).
#[derive(Clone, Copy, Debug)]
pub enum ExprRef {
    Block(Handle<BlockNode>),
    Lit(Handle<LitNode>),
    Path(Handle<PathNode>),
    Call(Handle<CallNode>),
    Method(Handle<MethodNode>),
    Field(Handle<FieldNode>),
    Index(Handle<IndexNode>),
    Unary(Handle<UnaryNode>),
    Binary(Handle<BinaryNode>),
    Assign(Handle<AssignNode>),
    Lambda(Handle<LambdaNode>),
    Try(Handle<TryNode>),
    FStr(Handle<FStrNode>),
    Struct(Handle<StructLitNode>),
    Tuple(Handle<TupleNode>),
    ArrayLit(Handle<ArrayLitNode>),
    ArrayRepeat(Handle<ArrayRepeatNode>),
    WhenExpr(Handle<WhenExprNode>),
    If(Handle<IfExprNode>),
    Await(Handle<AwaitNode>),
    AsyncBlock(Handle<AsyncBlockNode>),
    Is(Handle<IsNode>),
    Cast(Handle<CastNode>),
    Error(Handle<ErrorNode>),
}

impl ExprRef {
    pub fn span(self, ctx: &crate::Ast) -> Span {
        crate::slot_span!(
            self,
            ctx,
            ExprRef::Block,
            ExprRef::Lit,
            ExprRef::Path,
            ExprRef::Call,
            ExprRef::Method,
            ExprRef::Field,
            ExprRef::Index,
            ExprRef::Unary,
            ExprRef::Binary,
            ExprRef::Assign,
            ExprRef::Lambda,
            ExprRef::Try,
            ExprRef::FStr,
            ExprRef::Struct,
            ExprRef::Tuple,
            ExprRef::ArrayLit,
            ExprRef::ArrayRepeat,
            ExprRef::WhenExpr,
            ExprRef::If,
            ExprRef::Await,
            ExprRef::AsyncBlock,
            ExprRef::Is,
            ExprRef::Cast,
            ExprRef::Error,
        )
    }
    pub fn is_error(self) -> bool {
        matches!(self, ExprRef::Error(_))
    }
    pub fn as_block(self) -> Option<Handle<BlockNode>> {
        match self {
            ExprRef::Block(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_path(self) -> Option<Handle<PathNode>> {
        match self {
            ExprRef::Path(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_field(self) -> Option<Handle<FieldNode>> {
        match self {
            ExprRef::Field(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_index(self) -> Option<Handle<IndexNode>> {
        match self {
            ExprRef::Index(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_call(self) -> Option<Handle<CallNode>> {
        match self {
            ExprRef::Call(h) => Some(h),
            _ => None,
        }
    }
}

// ---- the nodes that live nowhere more specific ----

/// block expression — fn bodies, loop bodies, block arms/lambda bodies.
/// Rust-style tail value: the last statement WITHOUT `;` is the
/// block's value; `None` ⇒ nil.
#[derive(Clone, Debug)]
pub struct BlockNode {
    pub span: Span,
    pub stmts: Vec<crate::ast::stmt::StmtRef>,
    pub tail: Option<ExprRef>,
}

/// dotted path, possibly with generic args on any segment: `primes`,
/// `Color.Red`, `Vec.from`, `MyMap<K, V>.new`, `Self`
#[derive(Clone, Debug)]
pub struct PathNode {
    pub span: Span,
    pub segs: Vec<PathSeg>,
}

/// tuple expression `(a, b, ..)` — a record with numeric fields
/// (`()` is rejected — the empty value is `nil`)
#[derive(Clone, Debug)]
pub struct TupleNode {
    pub span: Span,
    pub elems: Vec<ExprRef>,
}

/// struct / `Self { .. }` literal — classes have no instance literal
#[derive(Clone, Debug)]
pub struct StructLitNode {
    pub span: Span,
    pub ty: crate::ast::ty::TyRef,
    pub fields: Vec<(rut_core::IdentId, ExprRef)>,
}

/// `[e1, .., en] : [T]`
#[derive(Clone, Debug)]
pub struct ArrayLitNode {
    pub span: Span,
    pub elems: Vec<ExprRef>,
}

/// `[v; n] : [T]` — the repeat construction
#[derive(Clone, Debug)]
pub struct ArrayRepeatNode {
    pub span: Span,
    pub value: ExprRef,
    pub count: ExprRef,
}

/// `( params ) (-> Type)? => expr | block` / `fn (..) { .. }`
#[derive(Clone, Debug)]
pub struct LambdaNode {
    pub span: Span,
    pub params: Vec<crate::ast::item::ParamRef>,
    pub ret: Option<crate::ast::ty::TyRef>,
    pub body: ExprRef,
}

/// postfix `?`
#[derive(Clone, Debug)]
pub struct TryNode {
    pub span: Span,
    pub expr: ExprRef,
}

impl Handle<BlockNode> {
    pub fn stmts(self, ctx: &crate::Ast) -> &[crate::ast::stmt::StmtRef] {
        &self.get(ctx).stmts
    }
}

use crate::{plain_alloc, slot_alloc};
plain_alloc!(BlockNode);
slot_alloc!(PathNode, ExprRef::Path);
slot_alloc!(TupleNode, ExprRef::Tuple);
slot_alloc!(StructLitNode, ExprRef::Struct);
slot_alloc!(ArrayLitNode, ExprRef::ArrayLit);
slot_alloc!(ArrayRepeatNode, ExprRef::ArrayRepeat);
slot_alloc!(LambdaNode, ExprRef::Lambda);
slot_alloc!(TryNode, ExprRef::Try);
