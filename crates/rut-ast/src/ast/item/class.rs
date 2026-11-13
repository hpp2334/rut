//! Structs, classes, interfaces — and their member satellites: field
//! and method declarations. Field/method lists are SLOTS: a broken
//! member region is an `ErrorNode` beside the survivors.

use crate::ast::doc::DocData;
use crate::ast::error::ErrorNode;
use crate::ast::expr::{BlockNode, ExprRef};
use crate::ast::handle::Handle;
use crate::ast::item::{ItemRef, Vis};
use crate::ast::ty::TyRef;
use crate::slot_alloc;
use rut_core::IdentId;
use rut_lexer::span::Span;

/// `struct Name<..> { fields }` — fields only; methods live in `impl`.
#[derive(Clone, Debug)]
pub struct StructNode {
    pub span: Span,
    /// the modifier chain in source order — `vis()` derives from it
    pub modifiers: Vec<ModifierNode>,
    pub name: IdentId,
    pub name_span: Span,
    pub generics: Vec<IdentId>,
    pub fields: Vec<FieldRef>,
    pub methods: Vec<MethodRef>,
    pub doc: DocData,
}

impl StructNode {
    /// the spelled item visibility — `Vis::Self_` when unannotated
    pub fn vis(&self) -> Vis {
        item_vis(&self.modifiers)
    }
}

/// `class Name<..> { fields; methods }` — or the positional newtype
/// `class Name(Wrapped);` (the desugared one-`inner`-field shape).
#[derive(Clone, Debug)]
pub struct ClassNode {
    pub span: Span,
    /// the modifier chain in source order — `vis()` derives from it
    pub modifiers: Vec<ModifierNode>,
    pub name: IdentId,
    pub name_span: Span,
    pub generics: Vec<IdentId>,
    /// admission-only bounds on the class's own generics
    /// (`class HashMap<K requires Hashable, V>`)
    pub requires: Vec<(IdentId, TyRef)>,
    /// the positional one-field decl (`class JsonI64(i64);`) — arms the
    /// call construction at the checker
    pub newtype: bool,
    pub fields: Vec<FieldRef>,
    pub methods: Vec<MethodRef>,
    pub doc: DocData,
}

impl ClassNode {
    /// the spelled item visibility — `Vis::Self_` when unannotated
    pub fn vis(&self) -> Vis {
        item_vis(&self.modifiers)
    }
}

/// `interface Name<..> { fn sig(..) -> T; .. }` — signatures only;
/// satisfaction is structural.
#[derive(Clone, Debug)]
pub struct InterfaceNode {
    pub span: Span,
    /// the modifier chain in source order — `vis()` derives from it
    pub modifiers: Vec<ModifierNode>,
    pub name: IdentId,
    pub name_span: Span,
    pub generics: Vec<IdentId>,
    pub methods: Vec<MethodRef>,
    pub doc: DocData,
}

impl InterfaceNode {
    /// the spelled item visibility — `Vis::Self_` when unannotated
    pub fn vis(&self) -> Vis {
        item_vis(&self.modifiers)
    }
}

/// one modifier — a SPANNED child node riding the decl in source
/// order; the decl's flags are the DERIVED view (the node's
/// accessors), never the storage
#[derive(Clone, Debug)]
pub struct ModifierNode {
    pub span: Span,
    pub kind: ModKind,
}

impl ModifierNode {
    /// the `pub(..)` visibility this modifier spells, if any
    pub fn vis(&self) -> Option<Vis> {
        match self.kind {
            ModKind::Pub(v) => Some(v),
            _ => None,
        }
    }
}

/// the spelled item visibility — `Vis::Self_` (module-private) when
/// unannotated; the item accessors' shared derivation
pub(crate) fn item_vis(modifiers: &[ModifierNode]) -> Vis {
    modifiers
        .iter()
        .find_map(ModifierNode::vis)
        .unwrap_or(Vis::Self_)
}

/// spelled `async`? — the item accessors' shared derivation
pub(crate) fn item_is_async(modifiers: &[ModifierNode]) -> bool {
    modifiers.iter().any(|m| matches!(m.kind, ModKind::Async))
}

/// the item modifier set. Member kinds (`static`/`marker`) land on the
/// decls inside type bodies; the item kinds (`entry`/`host`/`builtin`)
/// belong to the module-level item decls and land with them.
#[derive(Clone, Debug)]
pub enum ModKind {
    /// `pub`, `pub(super)`, `pub(pkg)` — the span covers the parens
    Pub(Vis),
    /// `async`
    Async,
    /// `static`
    Static,
    /// `[disposal]` etc. — the span covers the brackets
    Marker(IdentId),
    /// `entry fn` — the host-callable declaration surface
    Entry,
    /// `host` — the native linkage (the embedding Rust)
    Host,
    /// `builtin` with its ambient gate — the span covers the whole
    /// spelling (`prelude builtin` ambient / `pub builtin` import-gated)
    Builtin { ambient: bool },
}

/// struct/class field: `pub(..)`? `static`? name: ty (= init)?
#[derive(Clone, Debug)]
pub struct FieldDeclNode {
    pub span: Span,
    /// the modifier chain in source order — `vis()`/`is_static()`
    /// derive from it
    pub modifiers: Vec<ModifierNode>,
    pub name: IdentId,
    pub name_span: Span,
    pub ty: TyRef,
    pub init: Option<ExprRef>,
    pub doc: DocData,
}

impl FieldDeclNode {
    /// the spelled member visibility, if any
    pub fn vis(&self) -> Option<Vis> {
        self.modifiers.iter().find_map(|m| match m.kind {
            ModKind::Pub(v) => Some(v),
            _ => None,
        })
    }
    /// spelled `static`?
    pub fn is_static(&self) -> bool {
        self.modifiers
            .iter()
            .any(|m| matches!(m.kind, ModKind::Static))
    }
}

/// `pub(..)`? `[marker]`? `async`? fn name<..>(self, ..) -> T { .. }`
#[derive(Clone, Debug)]
pub struct MethodDeclNode {
    /// starts at the member's FIRST token — the leading modifier when
    /// one is spelled, `fn` otherwise
    pub span: Span,
    /// the modifier chain in source order — `vis()`/`is_async()`/
    /// `marker()` derive from it
    pub modifiers: Vec<ModifierNode>,
    pub name: IdentId,
    pub name_span: Span,
    pub generics: Vec<IdentId>,
    /// inline admission-only bounds on the method's own generics
    pub bounds: Vec<(IdentId, TyRef)>,
    pub params: Vec<crate::ast::item::ParamRef>,
    pub ret: Option<TyRef>,
    /// `None` in interfaces / surface classes / diagnosed bodiless forms
    pub body: Option<Handle<BlockNode>>,
    pub doc: DocData,
}

impl MethodDeclNode {
    /// the spelled member visibility, if any
    pub fn vis(&self) -> Option<Vis> {
        self.modifiers.iter().find_map(|m| match m.kind {
            ModKind::Pub(v) => Some(v),
            _ => None,
        })
    }
    /// spelled `async`?
    pub fn is_async(&self) -> bool {
        self.modifiers
            .iter()
            .any(|m| matches!(m.kind, ModKind::Async))
    }
    /// the designated surface the member fills (`[disposal]` /
    /// `[iterable]` / `[constructor]`) — validated by the checker
    pub fn marker(&self) -> Option<IdentId> {
        self.modifiers.iter().find_map(|m| match m.kind {
            ModKind::Marker(w) => Some(w),
            _ => None,
        })
    }
}

/// the field slot — a field or an ErrorNode in its place
#[derive(Clone, Copy, Debug)]
pub enum FieldRef {
    Field(Handle<FieldDeclNode>),
    Error(Handle<ErrorNode>),
}

impl FieldRef {
    pub fn span(self, ctx: &crate::Ast) -> Span {
        match self {
            FieldRef::Field(h) => h.span(ctx),
            FieldRef::Error(h) => h.span(ctx),
        }
    }
    pub fn is_error(self) -> bool {
        matches!(self, FieldRef::Error(_))
    }
    pub fn as_field(self) -> Option<Handle<FieldDeclNode>> {
        match self {
            FieldRef::Field(h) => Some(h),
            FieldRef::Error(_) => None,
        }
    }
}

/// the method slot — a method or an ErrorNode in its place
#[derive(Clone, Copy, Debug)]
pub enum MethodRef {
    Method(Handle<MethodDeclNode>),
    Error(Handle<ErrorNode>),
}

impl MethodRef {
    pub fn span(self, ctx: &crate::Ast) -> Span {
        match self {
            MethodRef::Method(h) => h.span(ctx),
            MethodRef::Error(h) => h.span(ctx),
        }
    }
    pub fn is_error(self) -> bool {
        matches!(self, MethodRef::Error(_))
    }
    pub fn as_method(self) -> Option<Handle<MethodDeclNode>> {
        match self {
            MethodRef::Method(h) => Some(h),
            MethodRef::Error(_) => None,
        }
    }
}

slot_alloc!(FieldDeclNode, FieldRef::Field);
slot_alloc!(MethodDeclNode, MethodRef::Method);
slot_alloc!(StructNode, ItemRef::Struct);
slot_alloc!(ClassNode, ItemRef::Class);
slot_alloc!(InterfaceNode, ItemRef::Interface);
