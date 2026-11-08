//! Free functions and the parameter vocabulary shared by fns,
//! methods, and lambdas. Param lists are SLOTS: a broken parameter is
//! a `ParamRef::Error` beside the survivors — the fn allocates anyway
//! (the recovery ladder's PART tier).

use crate::doc::DocData;
use crate::error::ErrorNode;
use crate::expr::BlockNode;
use crate::handle::Handle;
use crate::item::{ItemRef, Vis};
use crate::slot_alloc;
use crate::ty::TyRef;
use rut_core::IdentId;
use rut_lexer::span::Span;

/// `pub(..)? async? entry? fn Name<..>(params) -> T { body }`
#[derive(Clone, Debug)]
pub struct FnNode {
    pub span: Span,
    pub vis: Vis,
    pub is_async: bool,
    /// `entry fn` — host-callable: lands in the binary's entry table
    pub entry: bool,
    pub name: IdentId,
    pub name_span: Span,
    pub generics: Vec<IdentId>,
    /// inline admission-only bounds (`fn f<T requires A | B>(..)`)
    pub bounds: Vec<(IdentId, TyRef)>,
    pub params: Vec<ParamRef>,
    pub ret: Option<TyRef>,
    pub body: Handle<BlockNode>,
    pub doc: DocData,
}

/// `mut`? name: ty — or a type-less lambda param `(x)`
#[derive(Clone, Debug)]
pub struct ParamNode {
    pub span: Span,
    pub is_mut: bool,
    pub name: IdentId,
    pub name_span: Span,
    pub ty: Option<TyRef>,
}

/// `self` / `mut self`
#[derive(Clone, Debug)]
pub struct SelfParamNode {
    pub span: Span,
    pub is_mut: bool,
}

/// the parameter slot — Param, SelfParam, or an ErrorNode in place
#[derive(Clone, Copy, Debug)]
pub enum ParamRef {
    Param(Handle<ParamNode>),
    SelfParam(Handle<SelfParamNode>),
    Error(Handle<ErrorNode>),
}

impl ParamRef {
    pub fn span(self, ctx: &crate::Ast) -> Span {
        match self {
            ParamRef::Param(h) => h.span(ctx),
            ParamRef::SelfParam(h) => h.span(ctx),
            ParamRef::Error(h) => h.span(ctx),
        }
    }
    pub fn is_error(self) -> bool {
        matches!(self, ParamRef::Error(_))
    }
    pub fn as_param(self) -> Option<Handle<ParamNode>> {
        match self {
            ParamRef::Param(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_self_param(self) -> Option<Handle<SelfParamNode>> {
        match self {
            ParamRef::SelfParam(h) => Some(h),
            _ => None,
        }
    }
}

slot_alloc!(ParamNode, ParamRef::Param);
slot_alloc!(SelfParamNode, ParamRef::SelfParam);
slot_alloc!(FnNode, ItemRef::Fn);
