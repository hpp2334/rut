//! `.d.rut` surface declarations: `host fn` / `host struct` and the
//! `builtin` family (fn / class / primitive / impl).

use crate::doc::DocData;
use crate::item::{FieldRef, ItemRef, MethodRef, ParamRef, Vis};
use crate::slot_alloc;
use crate::ty::TyRef;
use rut_core::IdentId;
use rut_lexer::span::Span;

/// Who implements a surface declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Linkage {
    /// `host` — the embedding Rust (a registered `NativeModule`)
    Host,
    /// `builtin` — the engine itself, compiler-lowered (core only).
    /// `ambient: true` is `prelude builtin` (binds without `use`);
    /// `false` is `pub builtin` (import-gated).
    Builtin { ambient: bool },
}

/// `host fn name(params) -> T;` / `host async fn ..` / `builtin fn ..`
/// — a bodiless signature over the crossing set (or compiler-lowered).
#[derive(Clone, Debug)]
pub struct SurfaceFnNode {
    pub span: Span,
    pub vis: Vis,
    pub linkage: Linkage,
    pub is_async: bool,
    pub name: IdentId,
    pub name_span: Span,
    pub generics: Vec<IdentId>,
    pub params: Vec<ParamRef>,
    pub ret: Option<TyRef>,
    pub doc: DocData,
}

/// `host struct Name { fields }` — a flat record whose every field is
/// a crossing type; the host constructs and reads it.
#[derive(Clone, Debug)]
pub struct SurfaceStructNode {
    pub span: Span,
    pub vis: Vis,
    pub name: IdentId,
    pub name_span: Span,
    pub fields: Vec<FieldRef>,
    pub doc: DocData,
}

/// `builtin Name<..> { methods }` — an engine builtin TYPE's member
/// contract (`Option`, `Result`, `Future`, …).
#[derive(Clone, Debug)]
pub struct BuiltinTyNode {
    pub span: Span,
    pub vis: Vis,
    /// `true` — `prelude builtin` (ambient); `false` — `pub builtin`
    pub ambient: bool,
    pub name: IdentId,
    pub name_span: Span,
    pub generics: Vec<IdentId>,
    pub members: Vec<MethodRef>,
    pub doc: DocData,
}

/// `builtin primitive <name> { .. }` — a boot primitive's surface
/// statement (`str`/`bytes`/`opaque`): NOT a class.
#[derive(Clone, Debug)]
pub struct BuiltinPrimitiveNode {
    pub span: Span,
    pub ambient: bool,
    pub name: IdentId,
    pub name_span: Span,
    pub members: Vec<MethodRef>,
    pub doc: DocData,
}

/// `builtin impl i32 { .. }` — numeric methods ON a primitive type,
/// lowered inline at the call site.
#[derive(Clone, Debug)]
pub struct BuiltinImplNode {
    pub span: Span,
    pub vis: Vis,
    /// the receiver primitive's name
    pub prim: IdentId,
    pub name_span: Span,
    pub methods: Vec<MethodRef>,
}

slot_alloc!(SurfaceFnNode, ItemRef::SurfaceFn);
slot_alloc!(SurfaceStructNode, ItemRef::SurfaceStruct);
slot_alloc!(BuiltinTyNode, ItemRef::BuiltinTy);
slot_alloc!(BuiltinPrimitiveNode, ItemRef::BuiltinPrimitive);
slot_alloc!(BuiltinImplNode, ItemRef::BuiltinImpl);
