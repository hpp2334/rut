//! Handles, arenas, and the NodeStore law.
//!
//! Every node kind is its own struct stored in its own private arena on
//! [`crate::Ast`]. A `Handle<T>` is a typed index into T's arena — Copy,
//! zero-cost, and the only way to reference a node. The grammar-blind
//! walk surface is the SLOT enums (`ItemRef`, `StmtRef`, `ExprRef`, …),
//! each a small tagged-handle enum with an `Error` variant — an
//! `ErrorNode` may occupy ANY child position (the uniform recovery law,
//! D10).
//!
//! The arena is a free-list SLOT MAP: a handle is stable for the tree's
//! lifetime, a removal punches a hole, and the next allocation fills
//! the oldest hole first. The grammar-blind ADDRESS is
//! [`AnyHandle`] — `{ kind, idx }`, the pair the tree index and the
//! printer speak — with a kind-checked `downcast` back to a typed
//! handle (never a handle into the wrong arena).

use rut_lexer::span::Span;
use std::marker::PhantomData;

/// the arena kind id — stamped by the `arenas!` macro (one running
/// index over the inventory; dense, unique by construction)
pub type NodeKindId = u16;

/// The kind identity — `const KIND` (the arena's id) and `const NAME`
/// (the printable label) stamped by the `arenas!` macro beside every
/// arena. One inventory: a new kind cannot lack an id or a name — the
/// macro's arity forces both.
pub trait NodeKind {
    const KIND: NodeKindId;
    const NAME: &'static str;
}

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

/// One per-kind arena — a free-list SLOT MAP (stable handles, holes on
/// removal): the slot vec plus the free list of punched indices. `alloc`
/// fills the oldest hole first; reads return `None` for holes (the
/// printer prints an explicit `dead` row, never a wrong-arena read).
/// Exact per-kind slot sizing: no node pays for another kind's payload.
pub struct Arena<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
}

enum Slot<T> {
    Full(T),
    #[allow(dead_code)] // punched by remove — the edit lane's holes
    Hole,
}

impl<T> Arena<T> {
    pub(crate) fn empty() -> Self {
        Arena {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }
    /// land a node — the oldest hole's index when one exists, else the
    /// next fresh slot (the index IS the handle: stable for the
    /// tree's lifetime)
    pub(crate) fn insert(&mut self, node: T) -> u32 {
        match self.free.pop() {
            Some(idx) => {
                self.slots[idx as usize] = Slot::Full(node);
                idx
            }
            None => {
                let idx = self.slots.len() as u32;
                self.slots.push(Slot::Full(node));
                idx
            }
        }
    }
    /// remove a node — the slot becomes a hole the next alloc reuses.
    /// Defined NOW (the store law: holes bounded by dead nodes); the
    /// edit lane (Phase 3) is the caller.
    #[allow(dead_code)] // the edit lane's door — Phase 3 calls it
    pub(crate) fn remove(&mut self, idx: u32) -> Option<T> {
        match self.slots.get_mut(idx as usize)? {
            Slot::Full(_) => {
                self.free.push(idx);
                Some(
                    match std::mem::replace(&mut self.slots[idx as usize], Slot::Hole) {
                        Slot::Full(n) => n,
                        Slot::Hole => unreachable!("matched Full above"),
                    },
                )
            }
            Slot::Hole => None,
        }
    }
    /// the node at `idx`, if the slot is live (`None` = dead hole)
    pub(crate) fn get(&self, idx: u32) -> Option<&T> {
        match self.slots.get(idx as usize)? {
            Slot::Full(n) => Some(n),
            Slot::Hole => None,
        }
    }
    /// live-node count (holes excluded)
    pub fn len(&self) -> usize {
        self.slots.len() - self.free.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// the live nodes, slot order
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.slots.iter().filter_map(|s| match s {
            Slot::Full(n) => Some(n),
            Slot::Hole => None,
        })
    }
}
impl<T> IntoIterator for Arena<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        self.slots
            .into_iter()
            .filter_map(|s| match s {
                Slot::Full(n) => Some(n),
                Slot::Hole => None,
            })
            .collect::<Vec<_>>()
            .into_iter()
    }
}
impl<'a, T> IntoIterator for &'a Arena<T> {
    type Item = &'a T;
    type IntoIter = std::vec::IntoIter<&'a T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter().collect::<Vec<_>>().into_iter()
    }
}

/// The grammar-blind node address — `{ kind, idx }`, one pair that
/// names a node in ANY arena. The tree index (`node_at`/`parent`/
/// `ancestors`) and the printer speak it; the typed world is one
/// kind-checked `downcast` away.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AnyHandle {
    pub kind: NodeKindId,
    pub idx: u32,
}

impl AnyHandle {
    /// the typed handle — `None` unless the kind matches T's arena
    /// (the kind check comes FIRST: never a handle into the wrong
    /// arena)
    pub fn downcast<T: NodeKind>(self) -> Option<Handle<T>> {
        if self.kind == T::KIND {
            Some(Handle::new(self.idx))
        } else {
            None
        }
    }
    /// is this address into T's arena?
    pub fn is<T: NodeKind>(self) -> bool {
        self.kind == T::KIND
    }
    /// the node's span (the store dispatches the kind) — panics on a
    /// stale (dead-slot) address; probing callers ride `Ast`'s
    /// `span_of_any` first
    pub fn span(self, ast: &crate::Ast) -> Span {
        ast.span_of(self)
    }
    /// the kind's printable name — the `arenas!` macro's single
    /// inventory (the same table the printer's labels read)
    pub fn kind_name(&self) -> &'static str {
        crate::ast::store::kind_name_of(self.kind)
    }
}

impl<T: NodeKind> Handle<T> {
    /// the universal address — the handle's arena kind plus its slot
    pub fn any(self) -> AnyHandle {
        AnyHandle {
            kind: T::KIND,
            idx: self.idx,
        }
    }
}

impl<T: NodeKind> From<Handle<T>> for AnyHandle {
    fn from(h: Handle<T>) -> AnyHandle {
        h.any()
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
    /// the typed read — `None` only for a stale index (a dead hole or
    /// past the arena); the capsule law says callers hold live
    /// handles, so this panics with a named message rather than a
    /// bare index panic
    pub fn get(self, ctx: &crate::Ast) -> &T {
        NodeStore::arena(ctx).get(self.idx).unwrap_or_else(|| {
            panic!(
                "stale handle — arena slot #{} is empty or out of range (the tree was edited)",
                self.idx
            )
        })
    }
    /// the node's own span (spans live IN the node — D6)
    pub fn span(self, ctx: &crate::Ast) -> Span {
        self.get(ctx).span()
    }
}

/// slot-returning construction: `XNode { .. }.allocate(&mut ast)`
/// lands the node in its arena and hands back its slot. Every node
/// knows its own slot — one call, slot out.
///
/// The generated method stays `pub(crate)` on purpose: it rides a
/// publicly re-exported type, so visibility is the method's own
/// marker, not the (sealed) module's.
macro_rules! slot_alloc {
    ($ty:ty, $slot:ident :: $var:ident) => {
        impl $ty {
            pub(crate) fn allocate(self, ctx: &mut $crate::Ast) -> $slot {
                $slot::$var($crate::Ast::alloc(ctx, self))
            }
        }
    };
}
pub(crate) use slot_alloc;

/// handle-returning construction for closed slots (BlockNode bodies,
/// WhenArmNode arms — positions that hold exactly one kind and never
/// an Error).
///
/// Same laws as `slot_alloc`: the macro's own re-export chain keeps
/// `pub(crate)` (a macro_rules definition is module-private, so `pub`
/// is not a legal spelling), and the generated method keeps its own
/// `pub(crate)` — visibility rides the re-exported type, not the
/// sealed module.
macro_rules! plain_alloc {
    ($ty:ty) => {
        impl $ty {
            pub(crate) fn alloc_plain(
                self,
                ctx: &mut $crate::Ast,
            ) -> $crate::ast::handle::Handle<$ty> {
                ctx.alloc(self)
            }
        }
    };
}
pub(crate) use plain_alloc;

/// generate a slot enum's `span(ctx)` — one arm per variant, each a
/// typed handle read (or-patterns can't unify distinct handle types).
///
/// Same laws as the alloc macros: the re-export chain stays
/// `pub(crate)` (a macro_rules definition is module-private, so `pub`
/// is not a legal spelling), and the `span` methods it expands into
/// keep their own visibility marker — it rides the re-exported type,
/// not the sealed module.
macro_rules! slot_span {
    ($self:expr, $ctx:expr, $( $slot:ident :: $var:ident ),* $(,)?) => {
        match $self {
            $( $slot::$var(h) => h.span($ctx), )*
        }
    };
}
pub(crate) use slot_span;

/// generate a slot enum's `any()` — one arm per variant, each the
/// typed handle's universal address (the tree index's walk currency).
/// Same macro laws as `slot_span` (the re-export chain stays
/// `pub(crate)`; the generated method keeps its own marker).
macro_rules! slot_any {
    ($self:expr, $( $slot:ident :: $var:ident ),* $(,)?) => {
        match $self {
            $( $slot::$var(h) => h.any(), )*
        }
    };
}
pub(crate) use slot_any;

#[cfg(test)]
mod tests {
    use super::{AnyHandle, Arena, Handle, NodeKind, NodeKindId};

    // stand-in kinds for the tests — ids beyond the real inventory
    // (the arenas! stamp owns 0..)
    struct Item;
    struct Other;
    impl NodeKind for Item {
        const KIND: NodeKindId = 250;
        const NAME: &'static str = "Item";
    }
    impl NodeKind for Other {
        const KIND: NodeKindId = 251;
        const NAME: &'static str = "Other";
    }

    #[test]
    fn arena_alloc_appends_and_reads() {
        let mut a: Arena<&'static str> = Arena::empty();
        let i0 = a.insert("zero");
        let i1 = a.insert("one");
        assert_eq!((i0, i1), (0, 1));
        assert_eq!(a.get(0), Some(&"zero"));
        assert_eq!(a.get(1), Some(&"one"));
        assert_eq!(a.get(2), None);
        assert_eq!(a.len(), 2);
    }

    #[test]
    fn removal_punches_a_hole_and_alloc_fills_it_first() {
        let mut a: Arena<u32> = Arena::empty();
        let i0 = a.insert(10);
        let i1 = a.insert(11);
        let i2 = a.insert(12);
        assert_eq!(a.remove(i1), Some(11));
        assert_eq!(a.get(i1), None, "the slot is a dead hole");
        assert_eq!(a.len(), 2, "holes are not live");
        assert_eq!(a.remove(i1), None, "double removal finds no node");
        // the oldest hole fills first — the index (and so the handle)
        // of the survivor is unchanged
        let reused = a.insert(111);
        assert_eq!(reused, i1);
        assert_eq!(a.get(i0), Some(&10));
        assert_eq!(a.get(i1), Some(&111));
        assert_eq!(a.get(i2), Some(&12));
        assert_eq!(a.len(), 3);
        // holes exhausted: back to appending fresh slots
        let fresh = a.insert(13);
        assert_eq!(fresh, i2 + 1);
    }

    #[test]
    fn iter_and_into_iter_skip_holes() {
        let mut a: Arena<u32> = Arena::empty();
        a.insert(0);
        let hole = a.insert(1);
        a.insert(2);
        a.remove(hole);
        let live: Vec<u32> = a.iter().copied().collect();
        assert_eq!(live, vec![0, 2]);
        let owned: Vec<u32> = a.into_iter().collect();
        assert_eq!(owned, vec![0, 2]);
    }

    #[test]
    fn ast_is_send_sync() {
        // the index is plain data — the capsule stays Send + Sync
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<crate::Ast>();
        assert_send_sync::<AnyHandle>();
    }

    #[test]
    fn any_handle_downcasts_by_kind_first() {
        let hi = Handle::<Item>::new(3);
        let ho = Handle::<Other>::new(3);
        let ai = hi.any();
        let ao = ho.any();
        assert_eq!(ai.downcast::<Item>(), Some(hi));
        assert_eq!(ai.downcast::<Other>(), None, "never the wrong arena");
        assert!(ai.is::<Item>());
        assert!(!ai.is::<Other>());
        assert_ne!(ai, ao, "same idx, different kind — different addresses");
        assert_eq!(AnyHandle::from(hi), ai);
        // kind names come from the real inventory (the arenas! stamp)
        let real = Handle::<crate::ast::item::ModuleNode>::new(0);
        assert_eq!(real.any().kind_name(), "Module");
        // a bogus id names itself honestly
        assert_eq!(AnyHandle { kind: 999, idx: 0 }.kind_name(), "unknown");
    }
}
