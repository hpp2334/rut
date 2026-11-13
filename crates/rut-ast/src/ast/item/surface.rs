//! `.d.rut` surface declarations: `host fn` / `host struct` and the
//! `builtin` family (fn / class / primitive / impl).

use super::class::{item_is_async, item_vis, ModKind, ModifierNode};
use crate::ast::doc::DocData;
use crate::ast::item::{FieldRef, ItemRef, MethodRef, ParamRef, Vis};
use crate::ast::ty::TyRef;
use crate::slot_alloc;
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

/// the spelled surface linkage — the surface accessors' shared
/// derivation (`Linkage::Host` when no linkage modifier is present)
pub(crate) fn item_linkage(modifiers: &[ModifierNode]) -> Linkage {
    for m in modifiers {
        match m.kind {
            ModKind::Host => return Linkage::Host,
            ModKind::Builtin { ambient } => return Linkage::Builtin { ambient },
            _ => {}
        }
    }
    Linkage::Host
}

/// the spelled ambient gate — `prelude builtin` (ambient); the surface
/// accessors' shared derivation (`false` = a plain or import-gated decl)
pub(crate) fn item_ambient(modifiers: &[ModifierNode]) -> bool {
    modifiers
        .iter()
        .any(|m| matches!(m.kind, ModKind::Builtin { ambient: true }))
}

/// `host fn name(params) -> T;` / `host async fn ..` / `builtin fn ..`
/// — a bodiless signature over the crossing set (or compiler-lowered).
#[derive(Clone, Debug)]
pub struct SurfaceFnNode {
    /// starts at the item's FIRST token — the leading modifier when
    /// one is spelled, the linkage keyword otherwise
    pub span: Span,
    /// the modifier chain in source order — `vis()`/`linkage()`/
    /// `is_async()` derive from it
    pub modifiers: Vec<ModifierNode>,
    pub name: IdentId,
    pub name_span: Span,
    pub generics: Vec<IdentId>,
    pub params: Vec<ParamRef>,
    pub ret: Option<TyRef>,
    pub doc: DocData,
}

impl SurfaceFnNode {
    /// the spelled item visibility — `Vis::Self_` when unannotated
    pub fn vis(&self) -> Vis {
        item_vis(&self.modifiers)
    }
    /// the spelled linkage — `host` or the `builtin` gate
    pub fn linkage(&self) -> Linkage {
        item_linkage(&self.modifiers)
    }
    /// spelled `async`?
    pub fn is_async(&self) -> bool {
        item_is_async(&self.modifiers)
    }
}

/// `host struct Name { fields }` — a flat record whose every field is
/// a crossing type; the host constructs and reads it.
#[derive(Clone, Debug)]
pub struct SurfaceStructNode {
    /// starts at the item's FIRST token — the leading modifier when
    /// one is spelled, the linkage keyword otherwise
    pub span: Span,
    /// the `{..}` region, braces included — the head slice's right cut
    pub body_span: Span,
    /// the modifier chain in source order — `vis()`/`linkage()` derive
    /// from it
    pub modifiers: Vec<ModifierNode>,
    pub name: IdentId,
    pub name_span: Span,
    pub fields: Vec<FieldRef>,
    pub doc: DocData,
}

impl SurfaceStructNode {
    /// the spelled item visibility — `Vis::Self_` when unannotated
    pub fn vis(&self) -> Vis {
        item_vis(&self.modifiers)
    }
    /// the spelled linkage — `host` or the `builtin` gate
    pub fn linkage(&self) -> Linkage {
        item_linkage(&self.modifiers)
    }
}

/// `builtin Name<..> { methods }` — an engine builtin TYPE's member
/// contract (`Option`, `Result`, `Future`, …).
#[derive(Clone, Debug)]
pub struct BuiltinTyNode {
    /// starts at the item's FIRST token — the leading modifier when
    /// one is spelled, the linkage keyword otherwise
    pub span: Span,
    /// the `{..}` region, braces included — the head slice's right cut
    pub body_span: Span,
    /// the modifier chain in source order — `vis()`/`ambient()` derive
    /// from it
    pub modifiers: Vec<ModifierNode>,
    pub name: IdentId,
    pub name_span: Span,
    pub generics: Vec<IdentId>,
    pub members: Vec<MethodRef>,
    pub doc: DocData,
}

impl BuiltinTyNode {
    /// the spelled item visibility — `Vis::Self_` when unannotated
    pub fn vis(&self) -> Vis {
        item_vis(&self.modifiers)
    }
    /// `prelude builtin` (ambient) — binds without `use`
    pub fn ambient(&self) -> bool {
        item_ambient(&self.modifiers)
    }
}

/// `builtin primitive <name> { .. }` — a boot primitive's surface
/// statement (`str`/`bytes`/`opaque`): NOT a class.
#[derive(Clone, Debug)]
pub struct BuiltinPrimitiveNode {
    /// starts at the item's FIRST token — the leading modifier when
    /// one is spelled, the linkage keyword otherwise
    pub span: Span,
    /// the `{..}` region, braces included — the head slice's right cut
    pub body_span: Span,
    /// the modifier chain in source order — `vis()`/`ambient()` derive
    /// from it
    pub modifiers: Vec<ModifierNode>,
    pub name: IdentId,
    pub name_span: Span,
    pub members: Vec<MethodRef>,
    pub doc: DocData,
}

impl BuiltinPrimitiveNode {
    /// the spelled item visibility — `Vis::Self_` when unannotated
    pub fn vis(&self) -> Vis {
        item_vis(&self.modifiers)
    }
    /// `prelude builtin` (ambient) — binds without `use`
    pub fn ambient(&self) -> bool {
        item_ambient(&self.modifiers)
    }
}

/// `builtin impl i32 { .. }` — numeric methods ON a primitive type,
/// lowered inline at the call site. No visibility: no `pub impl`
/// spelling exists (the `pub` of a `pub builtin impl` rides the
/// modifier chain unread).
#[derive(Clone, Debug)]
pub struct BuiltinImplNode {
    /// starts at the item's FIRST token — the leading modifier when
    /// one is spelled, the linkage keyword otherwise
    pub span: Span,
    /// the modifier chain in source order
    pub modifiers: Vec<ModifierNode>,
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
