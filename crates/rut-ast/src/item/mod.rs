//! Items and the item slot. One struct per kind; data (span,
//! name_span, doc) directly inside (D1); `ItemRef` is the small
//! tagged-handle walk surface with its `Error` variant (D2).

mod class;
mod enum_decl;
mod func;
mod impl_block;
mod import;
mod let_decl;
mod module_decl;
mod surface;
mod type_alias;

pub use class::{
    ClassNode, FieldDeclNode, FieldRef, InterfaceNode, MethodDeclNode, MethodRef, ModKind,
    ModifierNode, StructNode,
};
pub use enum_decl::{EnumMemberNode, EnumNode, MemberRef};
pub use func::{FnNode, ParamNode, ParamRef, SelfParamNode};
pub use impl_block::ImplNode;
pub use import::UseNode;
pub use let_decl::ModuleLetNode;
pub use module_decl::{ModDeclNode, ModInlineNode, ModuleNode};
pub use surface::{
    BuiltinImplNode, BuiltinPrimitiveNode, BuiltinTyNode, Linkage, SurfaceFnNode, SurfaceStructNode,
};
pub use type_alias::AliasNode;

use crate::error::ErrorNode;
use crate::handle::Handle;
use rut_lexer::span::Span;

/// the item walk surface — a tagged handle per kind + the recovery
/// slot. Dispatch once, then hold a typed handle.
#[derive(Clone, Copy, Debug)]
pub enum ItemRef {
    Module(Handle<ModuleNode>),
    ModDecl(Handle<ModDeclNode>),
    ModInline(Handle<ModInlineNode>),
    Use(Handle<UseNode>),
    Alias(Handle<AliasNode>),
    ModuleLet(Handle<ModuleLetNode>),
    Enum(Handle<EnumNode>),
    Struct(Handle<StructNode>),
    Class(Handle<ClassNode>),
    Interface(Handle<InterfaceNode>),
    Impl(Handle<ImplNode>),
    Fn(Handle<FnNode>),
    SurfaceFn(Handle<SurfaceFnNode>),
    SurfaceStruct(Handle<SurfaceStructNode>),
    BuiltinTy(Handle<BuiltinTyNode>),
    BuiltinPrimitive(Handle<BuiltinPrimitiveNode>),
    BuiltinImpl(Handle<BuiltinImplNode>),
    Error(Handle<ErrorNode>),
}

impl ItemRef {
    pub fn span(self, ctx: &crate::Ast) -> Span {
        crate::slot_span!(
            self,
            ctx,
            ItemRef::Module,
            ItemRef::ModDecl,
            ItemRef::ModInline,
            ItemRef::Use,
            ItemRef::Alias,
            ItemRef::ModuleLet,
            ItemRef::Enum,
            ItemRef::Struct,
            ItemRef::Class,
            ItemRef::Interface,
            ItemRef::Impl,
            ItemRef::Fn,
            ItemRef::SurfaceFn,
            ItemRef::SurfaceStruct,
            ItemRef::BuiltinTy,
            ItemRef::BuiltinPrimitive,
            ItemRef::BuiltinImpl,
            ItemRef::Error,
        )
    }
    pub fn is_error(self) -> bool {
        matches!(self, ItemRef::Error(_))
    }
    // checked narrowing — the `as_xxx` law (D5)
    pub fn as_module(self) -> Option<Handle<ModuleNode>> {
        match self {
            ItemRef::Module(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_mod_decl(self) -> Option<Handle<ModDeclNode>> {
        match self {
            ItemRef::ModDecl(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_mod_inline(self) -> Option<Handle<ModInlineNode>> {
        match self {
            ItemRef::ModInline(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_use(self) -> Option<Handle<UseNode>> {
        match self {
            ItemRef::Use(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_alias(self) -> Option<Handle<AliasNode>> {
        match self {
            ItemRef::Alias(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_module_let(self) -> Option<Handle<ModuleLetNode>> {
        match self {
            ItemRef::ModuleLet(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_enum(self) -> Option<Handle<EnumNode>> {
        match self {
            ItemRef::Enum(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_struct(self) -> Option<Handle<StructNode>> {
        match self {
            ItemRef::Struct(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_class(self) -> Option<Handle<ClassNode>> {
        match self {
            ItemRef::Class(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_interface(self) -> Option<Handle<InterfaceNode>> {
        match self {
            ItemRef::Interface(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_impl(self) -> Option<Handle<ImplNode>> {
        match self {
            ItemRef::Impl(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_fn(self) -> Option<Handle<FnNode>> {
        match self {
            ItemRef::Fn(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_surface_fn(self) -> Option<Handle<SurfaceFnNode>> {
        match self {
            ItemRef::SurfaceFn(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_surface_struct(self) -> Option<Handle<SurfaceStructNode>> {
        match self {
            ItemRef::SurfaceStruct(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_builtin_ty(self) -> Option<Handle<BuiltinTyNode>> {
        match self {
            ItemRef::BuiltinTy(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_builtin_primitive(self) -> Option<Handle<BuiltinPrimitiveNode>> {
        match self {
            ItemRef::BuiltinPrimitive(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_builtin_impl(self) -> Option<Handle<BuiltinImplNode>> {
        match self {
            ItemRef::BuiltinImpl(h) => Some(h),
            _ => None,
        }
    }
}

/// module-item visibility — `pub` scopes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vis {
    /// `pub` — usable from anywhere
    Pub,
    /// `pub(super)` — the parent module's subtree
    Super,
    /// `pub(pkg)` — package-wide, no further
    Pkg,
    /// unannotated module items **and members** — module-private
    /// (the default; the `pub(self)` spelling is gone)
    Self_,
}

impl Handle<ModuleNode> {
    /// the module's items — the most-walked list in the tree
    pub fn items(self, ctx: &crate::Ast) -> &[ItemRef] {
        &self.get(ctx).items
    }
}
