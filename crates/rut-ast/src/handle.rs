//! Handles, arenas, and the NodeStore law.
//!
//! Every node kind is its own struct stored in its own private arena on
//! [`crate::Ast`]. A `Handle<T>` is a typed index into T's arena — Copy,
//! zero-cost, and the only way to reference a node. The grammar-blind
//! walk surface is the SLOT enums (`ItemRef`, `StmtRef`, `ExprRef`, …),
//! each a small tagged-handle enum with an `Error` variant — an
//! `ErrorNode` may occupy ANY child position (the uniform recovery law,
//! D10).

use rut_lexer::span::Span;
use std::marker::PhantomData;

/// A typed index into one per-kind arena. Zero-sized marker; `Copy`.
pub struct Handle<T> {
    pub(crate) idx: u32,
    _ty: PhantomData<fn() -> T>,
}

impl<T> Handle<T> {
    pub(crate) fn new(idx: u32) -> Self {
        Handle {
            idx,
            _ty: PhantomData,
        }
    }
    /// DEBUG/dump identity — the raw arena index (kind implied by T).
    pub fn index(self) -> u32 {
        self.idx
    }
}
impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Handle<T> {}
impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.idx == other.idx
    }
}
impl<T> Eq for Handle<T> {}
impl<T> std::hash::Hash for Handle<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.idx.hash(state);
    }
}
impl<T> std::fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.idx)
    }
}

/// One per-kind arena — a plain `Vec` newtype (flat drop, one
/// allocation per kind, exact per-kind slot sizing: no node pays for
/// another kind's payload).
pub struct Arena<T>(pub(crate) Vec<T>);

impl<T> Arena<T> {
    pub(crate) fn empty() -> Self {
        Arena(Vec::new())
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.0.iter()
    }
}
impl<T> IntoIterator for Arena<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
impl<'a, T> IntoIterator for &'a Arena<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

/// every node carries its own span (D6) — the field is public on the
/// node; this trait exposes it to generic code (Handle::span, the
/// slot enums' span dispatch).
pub trait HasSpan {
    fn span(&self) -> Span;
}

/// Maps a node kind to its arena on the store — the one trait every
/// node implements, wiring `allocate`/`get`.
pub trait NodeStore: HasSpan + Sized {
    fn arena(ast: &crate::Ast) -> &Arena<Self>;
    fn arena_mut(ast: &mut crate::Ast) -> &mut Arena<Self>;
}

impl<T: NodeStore> Handle<T> {
    /// the typed read — bounds-checked by the arena
    pub fn get(self, ctx: &crate::Ast) -> &T {
        &NodeStore::arena(ctx).0[self.idx as usize]
    }
    /// the node's own span (spans live IN the node — D6)
    pub fn span(self, ctx: &crate::Ast) -> Span {
        self.get(ctx).span()
    }
}

/// slot-returning construction: `XNode { .. }.allocate(&mut ast)`
/// lands the node in its arena and hands back its slot. Every node
/// knows its own slot — one call, slot out.
#[macro_export]
macro_rules! slot_alloc {
    ($ty:ty, $slot:ident :: $var:ident) => {
        impl $ty {
            pub fn allocate(self, ctx: &mut $crate::Ast) -> $slot {
                $slot::$var($crate::Ast::alloc(ctx, self))
            }
        }
    };
}

/// handle-returning construction for closed slots (BlockNode bodies,
/// WhenArmNode arms — positions that hold exactly one kind and never
/// an Error).
#[macro_export]
macro_rules! plain_alloc {
    ($ty:ty) => {
        impl $ty {
            pub fn alloc_plain(self, ctx: &mut $crate::Ast) -> $crate::handle::Handle<$ty> {
                ctx.alloc(self)
            }
        }
    };
}

/// generate a slot enum's `span(ctx)` — one arm per variant, each a
/// typed handle read (or-patterns can't unify distinct handle types)
#[macro_export]
macro_rules! slot_span {
    ($self:expr, $ctx:expr, $( $slot:ident :: $var:ident ),* $(,)?) => {
        match $self {
            $( $slot::$var(h) => h.span($ctx), )*
        }
    };
}
