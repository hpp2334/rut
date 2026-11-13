//! Free functions and the parameter vocabulary shared by fns,
//! methods, and lambdas. Param lists are SLOTS: a broken parameter is
//! a `ParamRef::Error` beside the survivors — the fn allocates anyway
//! (the recovery ladder's PART tier).

use crate::ast::doc::DocData;
use crate::ast::error::ErrorNode;
use crate::ast::expr::BlockNode;
use crate::ast::handle::Handle;
use crate::ast::item::class::{item_is_async, item_vis, ModKind, ModifierNode};
use crate::ast::item::{ItemRef, Vis};
use crate::ast::ty::TyRef;
use crate::slot_alloc;
use rut_core::IdentId;
use rut_lexer::span::Span;

/// `pub(..)? async? entry? fn Name<..>(params) -> T { body }`
#[derive(Clone, Debug)]
pub struct FnNode {
    /// starts at the item's FIRST token — the leading modifier when
    /// one is spelled, `fn` otherwise
    pub span: Span,
    /// the modifier chain in source order — `vis()`/`is_async()`/
    /// `entry()` derive from it
    pub modifiers: Vec<ModifierNode>,
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

impl FnNode {
    /// the spelled item visibility — `Vis::Self_` when unannotated
    pub fn vis(&self) -> Vis {
        item_vis(&self.modifiers)
    }
    /// spelled `async`?
    pub fn is_async(&self) -> bool {
        item_is_async(&self.modifiers)
    }
    /// spelled `entry`? — the host-callable surface
    pub fn entry(&self) -> bool {
        self.modifiers
            .iter()
            .any(|m| matches!(m.kind, ModKind::Entry))
    }
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
