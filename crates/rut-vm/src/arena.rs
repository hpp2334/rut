//! The self-managed cell arena — RFC 0039's later milestone.
//!
//! `heap.rs` used to mint every cell as its own `Rc<CellVal>`. This module
//! replaces that allocator: cells are carved from fixed-size chunks and a
//! freed slot returns to a free list. The handle stays a stable
//! `*const CellVal`, so `Slot` and `cell_of` are unchanged; references are
//! counted intrusively on the cell (`CellVal::refs`).
//!
//! `OpaqueRef` is the host-facing handle (RFC 0014): it owns one arena
//! reference and holds the shared arena, so it can outlive the `Vm`.

use crate::heap::{Heap, blocks::Blocks, CellData, CellVal, HeapAcct, Slot};
use crate::heap::store::{self, OpaqueEntry, Store};
use rut_core::binary::FuncCode;
use rut_core::types::{TypeTable, TypeId, TyKind};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::mem::MaybeUninit;
use std::rc::Rc;

const ARENA_CHUNK: usize = 1024;

/// What a dying cell must release with itself (RFC 0016 §3: "fields are
/// then released in declaration order (recursively)"). Precomputed from
/// the program's type table + func signatures so the release path — a
/// free fn with no VM access — can walk a cell's ref-typed children
/// without touching tag bytes or per-op type lookups.
pub(crate) struct ReleasePlan {
    /// per type id: is a value of the type a cell handle
    is_ref: Vec<bool>,
    /// per Data type: indices of ref-typed fields
    record_refs: Vec<Vec<u16>>,
    /// per Option/Result type: is the tag-0 / tag-1 payload a cell handle
    /// per func: are its captured slots cell handles
    closure_capture_ref: Vec<Vec<bool>>,
}

impl ReleasePlan {
    pub(crate) fn build(types: &TypeTable, funcs: &[FuncCode]) -> ReleasePlan {
        let is_ref: Vec<bool> = (0..types.types.len())
            .map(|i| types.is_ref(i as u32))
            .collect();
        let of = |t: &u32| is_ref.get(*t as usize).copied().unwrap_or(false);
        let record_refs = types
            .types
            .iter()
            .map(|t| match &t.kind {
                TyKind::Data { fields } => fields
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| of(&f.ty))
                    .map(|(i, _)| i as u16)
                    .collect(),
                // `?T` (RFC 0044): the one payload slot dies with the nullable
                TyKind::Opt { elem } => {
                    if of(elem) {
                        vec![0]
                    } else {
                        Vec::new()
                    }
                }
                _ => Vec::new(),
            })
            .collect();
        let closure_capture_ref = funcs
            .iter()
            .map(|f| f.params.iter().map(of).collect())
            .collect();
        ReleasePlan { is_ref, record_refs, closure_capture_ref }
    }
}

/// Chunked cell storage with a free list. Pointers into a chunk are stable,
/// so a `Slot` can hold a raw `*const CellVal` for the cell's lifetime.
pub(crate) struct Arena {
    free: RefCell<Vec<*mut CellVal>>,
    chunks: RefCell<Vec<Box<[MaybeUninit<CellVal>; ARENA_CHUNK]>>>,
    /// slots used in the last chunk
    bump: Cell<usize>,
    /// which of a dying cell's children die with it (RFC 0016 §3)
    pub(crate) plan: Rc<ReleasePlan>,
    /// `on_drop` callbacks (RFC 0016 §3): cell address -> cleanup closure.
    /// The closure slot is retained for the map's lifetime.
    drop_fns: RefCell<HashMap<usize, Slot>>,
    /// cells whose drop callback is queued: (pinned cell, cleanup). The
    /// interpreter drains these at call boundaries and releases the pin
    /// after the callback runs.
    pending_drops: RefCell<Vec<(*const CellVal, Slot)>>,
    /// the VM-owned block store (RFC 0039): variable-size cell payloads.
    /// Living here — not on `Heap` — because the release path (a free fn
    /// holding only `&Arena`) is what frees a cell's blocks, and because
    /// `OpaqueRef` keeps the arena (hence the store) alive for cells that
    /// outlive the `Vm`.
    pub(crate) blocks: Blocks,
    /// enum member singletons (RFC 0016 §1) — VM-owned state living on
    /// the arena so a `Heap` is a cheap two-`Rc` view (`Heap::view`),
    /// which the release walk builds to run a Host payload's finalize.
    pub(crate) singletons: RefCell<HashMap<(TypeId, u32), *const CellVal>>,
    /// weak-reference lists (RFC 0017 v1): referent slot word -> the
    /// WeakBox cells holding an unretained word to it. The `drop_fns`
    /// shape — lazy, uncharged engine bookkeeping living on the arena
    /// (same two-`Rc` view argument). Referent death nulls every box in
    /// its list BEFORE anything that runs user code; box death
    /// unregisters itself. Keyed by the full slot word (tagged for store
    /// entries — the same keying `drop_fns` uses for both shapes).
    weak_lists: RefCell<HashMap<usize, Vec<*const CellVal>>>,
    /// the Opaque store (nmap-hostvals P2): the ONE home of every rut
    /// `opaque` value — a slab of two-kind entries + free list, rc and
    /// borrow guards on the entry. Same lifetime argument as `blocks`:
    /// `OpaqueRef` keeps the arena (hence the store) alive while a host
    /// holds a box across calls.
    pub(crate) store: Store,
}

impl Arena {
    pub(crate) fn new(plan: Rc<ReleasePlan>) -> Arena {
        Arena {
            free: RefCell::new(Vec::new()),
            chunks: RefCell::new(Vec::new()),
            bump: Cell::new(ARENA_CHUNK),
            plan,
            drop_fns: RefCell::new(HashMap::new()),
            pending_drops: RefCell::new(Vec::new()),
            blocks: Blocks::new(),
            singletons: RefCell::new(HashMap::new()),
            weak_lists: RefCell::new(HashMap::new()),
            store: Store::new(),
        }
    }

    /// Register a fresh WeakBox into its referent's weak list (RFC 0017).
    /// `word` is the referent's full slot word.
    pub(crate) fn weak_register(&self, word: usize, box_cell: *const CellVal) {
        self.weak_lists.borrow_mut().entry(word).or_default().push(box_cell);
    }

    /// Referent death (RFC 0017): null every WeakBox in the referent's
    /// list and drop the entry. Runs BEFORE the on_drop pin check and
    /// any payload teardown — `dispose` bodies and queued cleanups that
    /// call `upgrade()` see `nil`, deterministically, no window.
    pub(crate) fn weak_null_list(&self, word: usize) {
        if let Some(boxes) = self.weak_lists.borrow_mut().remove(&word) {
            for b in boxes {
                if let CellData::WeakBox { referent } = unsafe { &(*b).data } {
                    referent.set(Slot::null());
                }
            }
        }
    }

    /// Box death (RFC 0017): a dying WeakBox removes itself from its
    /// referent's list (no-op when already dead — the list entry is
    /// gone). Keeps the later nulling walk off freed cells.
    pub(crate) fn weak_unregister(&self, word: usize, box_cell: *const CellVal) {
        let mut lists = self.weak_lists.borrow_mut();
        if let Some(v) = lists.get_mut(&word) {
            if let Some(i) = v.iter().position(|&b| b == box_cell) {
                v.swap_remove(i);
            }
            if v.is_empty() {
                lists.remove(&word);
            }
        }
    }

    /// Attach a drop callback to a cell (RFC 0016 §3). One per cell — a
    /// second attach is the caller's error to report. The closure slot is
    /// retained for the map's lifetime.
    pub(crate) fn set_drop_fn(&self, p: *const CellVal, cleanup: Slot) -> bool {
        let key = p as usize;
        let mut map = self.drop_fns.borrow_mut();
        if map.contains_key(&key) {
            return false;
        }
        map.insert(key, cleanup);
        true
    }

    /// Take a cell off the drop queue: (pinned cell pointer, cleanup slot).
    /// The caller owns the pin and the cleanup reference — release both
    /// after the callback runs.
    pub(crate) fn take_pending_drop(&self) -> Option<(*const CellVal, Slot)> {
        self.pending_drops.borrow_mut().pop()
    }

    /// Remove and return a cell's registered drop callback (the release
    /// path, when the cell's last reference dies). The taker owns the
    /// retained cleanup slot.
    pub(crate) fn take_drop_fn(&self, key: usize) -> Option<Slot> {
        self.drop_fns.borrow_mut().remove(&key)
    }

    /// Hand out a slot (free list first, then bump within the last chunk).
    pub(crate) fn alloc_slot(&self) -> *mut CellVal {
        if let Some(p) = self.free.borrow_mut().pop() {
            return p;
        }
        let mut chunks = self.chunks.borrow_mut();
        let mut bump = self.bump.get();
        if bump >= ARENA_CHUNK {
            chunks.push(Box::new([const { MaybeUninit::uninit() }; ARENA_CHUNK]));
            bump = 0;
        }
        let idx = chunks.len() - 1;
        let p = unsafe { (chunks[idx].as_mut_ptr() as *mut CellVal).add(bump) };
        self.bump.set(bump + 1);
        p
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        // drop every initialised cell that is not already on the free list
        // (frees its `String`/`Vec` payload); recycled slots are uninitialised
        let free: HashSet<*mut CellVal> = self.free.borrow().iter().copied().collect();
        let chunks = self.chunks.borrow();
        if !chunks.is_empty() {
            let total = (chunks.len() - 1) * ARENA_CHUNK + self.bump.get();
            for (ci, chunk) in chunks.iter().enumerate() {
                let base = chunk.as_ptr() as *mut CellVal;
                for i in 0..ARENA_CHUNK {
                    if ci * ARENA_CHUNK + i >= total {
                        break;
                    }
                    let p = unsafe { base.add(i) };
                    if !free.contains(&p) {
                        // free the cell's payload block(s) first — freeing needs
                        // the &Arena that Drop glue would not have (same rule as
                        // the release path below)
                        let cell = unsafe { &mut *p };
                        match &mut cell.data {
                            CellData::Str(sv) => self.blocks.free(sv.block),
                            CellData::Array { items, .. } => self.blocks.free(items.borrow().block),
                            _ => {}
                        }
                        unsafe { std::ptr::drop_in_place(p) };
                    }
                }
            }
        }
        // the Opaque store's own teardown: entries still live here had no
        // legal owner (an `OpaqueRef` would have kept this arena alive) —
        // they are the trap-torn-frame survivors, and their payload
        // `Box`es drop here. The debug balance check rides with it.
        self.store.teardown();
    }
}

/// One reference gone. At rc-0 the cell dies — and its ref-typed children
/// die with it (RFC 0016 §3): `release_cell` collects them and recurses,
/// so a record's `str`/`Opaque`/vec fields no longer pin their children
/// until VM end. A store slot (the tagged word, nmap-hostvals P2) routes
/// to the entry's own rc (`release_entry`) — the same law, new home.
#[inline(always)]
pub(crate) fn release_ref_slot(arena: &Rc<Arena>, acct: &Rc<HeapAcct>, s: Slot) {
    let p = unsafe { s.r };
    if p.is_null() {
        return;
    }
    if store::is_entry(s) {
        unsafe {
            let e = store::untag_entry(p);
            let c = &*e;
            let n = c.refs.get();
            if n <= 1 {
                // weak nulling first (RFC 0017): an opaque box's death
                // nulls its weak list before the pin check and before
                // `release_entry`'s finalize — nothing that runs user
                // code observes a live weak to a dying entry
                arena.weak_null_list(p as usize);
                // on_drop (RFC 0016 §3): the same pin-and-queue the cell
                // path runs — the key is the tagged word either way
                if let Some(cleanup) = arena.take_drop_fn(p as usize) {
                    c.refs.set(1);
                    arena.pending_drops.borrow_mut().push((p, cleanup));
                    return;
                }
                release_entry(arena, acct, e);
            } else {
                c.refs.set(n - 1);
            }
        }
        return;
    }
    unsafe {
        let c = &*p;
        let n = c.refs.get();
        if n == u32::MAX {
            return; // immortal singleton
        }
        if n <= 1 {
            // weak nulling first (RFC 0017): the referent's boxes go
            // dead before dispose/on_drop/user code can run
            arena.weak_null_list(p as usize);
            // on_drop (RFC 0016 §3): a registered callback pins the cell
            // (refs stay 1) and queues it; the interpreter runs the
            // callback at a call boundary and releases the pin afterwards.
            if let Some(cleanup) = arena.take_drop_fn(p as usize) {
                c.refs.set(1);
                arena.pending_drops.borrow_mut().push((p, cleanup));
                return;
            }
            release_cell(arena, acct, p as *mut CellVal);
        } else {
            c.refs.set(n - 1);
        }
    }
}

/// The cell's ref-typed child slots, in declaration order. The slots are
/// copied out, not taken — the parent is dead memory the moment
/// `drop_in_place` runs, and `Slots`/`Packed` have no drop glue of their
/// own, so nothing double-releases.
#[inline(always)]
unsafe fn collect_ref_children(c: &CellVal, plan: &ReleasePlan) -> Vec<Slot> {
    let mut out = Vec::new();
    match &c.data {
        CellData::Record { fields } => {
            let fb = fields.borrow();
            for &i in plan
                .record_refs
                .get(c.ty as usize)
                .into_iter()
                .flatten()
            {
                if let Some(s) = fb.get(i as usize) {
                    out.push(s);
                }
            }
        }
        CellData::Array { elem, items } => {
            // the ELEMENT type decides: a `Vec<str>`'s slots are cell handles
            // (slot-width elements). Handles are COPIED out — the block is
            // freed by the caller right after. The primitive-optional store
            // (`ArrKind::Opt`) holds raw payloads + nil tags — no handles —
            // so it contributes no children (reading its bytes as slots
            // would hand raw payload bits to the release walk).
            let d = items.borrow();
            if !matches!(d.kind, crate::heap::ArrKind::Opt(_))
                && plan.is_ref.get(*elem as usize).copied().unwrap_or(false)
            {
                out.extend(d.to_slots());
            }
        }
        // a user box stores the inner handle — its release is the box's
        // own (RFC 0014: box death drops the boxed value's reference).
        // That anchor cell is GONE at P2 (the rut-value box lives as a
        // store entry now — `release_entry` walks its held slot); a rut
        // `opaque` in a record field / array element is a store slot and
        // routes through `release_ref_slot`'s entry path by its tag.
        // a str view retains the window's parent (RFC 0042)
        CellData::StrView { parent, .. } => out.push(*parent),
        // a weak box holds an UNRETAINED referent word (RFC 0017) — never
        // a child; its unregister runs in `release_cell` before this walk
        CellData::WeakBox { .. } => {}
        // an array window retains its backing array (RFC 0042 §6)
        CellData::ArrView { parent, .. } => out.push(*parent),
        CellData::Closure { func, captures } => {
            if let Some(flags) = plan.closure_capture_ref.get(*func as usize) {
                for (&cap, &is_ref) in captures.iter().zip(flags.iter()) {
                    if is_ref {
                        out.push(cap);
                    }
                }
            }
        }
        // a trace cell's frames are plain (func, pc) words, never handles;
        // a builder's block-backed octets are likewise never handles
        CellData::Enum { .. } | CellData::Str(_)
        | CellData::Trace { .. } | CellData::StrBuf { .. } => {}
    }
    out
}

/// Drop a cell whose strong count reached zero: refund its accounting,
/// release its ref-typed children (see `collect_ref_children`), run its
/// destructor, and recycle the slot.
#[inline(always)]
pub(crate) fn release_cell(arena: &Rc<Arena>, acct: &Rc<HeapAcct>, p: *mut CellVal) {
    // children are collected first and released after the parent is
    // freed, so a release cascade can never observe the dying cell.
    // Childless kinds (the churn case: strs, singletons) never build
    // the child vec at all.
    // box death (RFC 0017): a dying WeakBox removes itself from its
    // referent's list before its slot is recycled — the later nulling
    // walk must never touch freed cells. (No block to free, no children:
    // the referent word is deliberately not a Slot child.)
    if let CellData::WeakBox { referent } = unsafe { &(*p).data } {
        let word = referent.get();
        if unsafe { !word.r.is_null() } {
            arena.weak_unregister(unsafe { word.r } as usize, p);
        }
    }
    let children = unsafe {
        match (*p).data {
            CellData::Str(_) | CellData::Enum { .. }
            | CellData::Trace { .. } | CellData::StrBuf { .. }
            | CellData::WeakBox { .. } => None,
            _ => Some(collect_ref_children(&*p, &arena.plan)),
        }
    };
    unsafe {
        // payload blocks die with the cell, explicitly — freeing needs the
        // &Arena this walk holds (payloads carry no Drop glue). Children
        // were collected above, so a ref-typed array's handles are already
        // out before its block goes.
        match &mut (*p).data {
            CellData::Str(sv) => arena.blocks.free(sv.block),
            CellData::StrBuf { buf, .. } => arena.blocks.free(buf.block),
            CellData::Array { items, .. } => arena.blocks.free(items.borrow().block),
            _ => {}
        }
        let bytes = (*p).bytes as u64;
        acct.used.set(acct.used.get().saturating_sub(bytes));
        std::ptr::drop_in_place(p);
    }
    arena.free.borrow_mut().push(p);
    if let Some(children) = children {
        for s in children {
            release_ref_slot(arena, acct, s);
        }
    }
}

/// Drop a store entry whose rc reached zero (nmap-hostvals P2): the
/// payload is taken out first (the dying entry is dead memory the moment
/// the slot is recycled), the charge refunded, the slot freed — and only
/// THEN the nested work, the record-field pattern (RFC 0016 §3): the
/// Host payload's `finalize` hook runs before the Box's own Drop; a Rut
/// entry's held slot releases through the same walk (an entry holding a
/// rut record recurses through it).
#[inline(always)]
pub(crate) fn release_entry(arena: &Rc<Arena>, acct: &Rc<HeapAcct>, p: *mut store::EntryCell) {
    debug_assert_eq!(
        unsafe { (*p).borrows.get() },
        0,
        "entry death under a live borrow — a with_mut view holds an rc"
    );
    let taken = unsafe {
        std::mem::replace(
            &mut (*p).e,
            OpaqueEntry::Rut(store::RutOpaque { slot: Slot::null(), val_ty: 0 }),
        )
    };
    let bytes = unsafe { (*p).bytes } as u64;
    arena.store.note_death(p, bytes);
    acct.used.set(acct.used.get().saturating_sub(bytes));
    match taken {
        OpaqueEntry::Host(h) => {
            // the finalize hook first (§0.8 i), the Box's own Drop second;
            // the hook allocates/releases through a two-Rc `Heap` view —
            // the same arena and budget, never a second machine
            if let (Some(f), mut host) = (h.finalize, h) {
                let heap = Heap::view(arena, acct);
                f(host.payload.as_mut(), &heap);
                drop(host);
            }
        }
        OpaqueEntry::Rut(r) => {
            // the held value cell dies with the entry (a prim payload is
            // raw bits — nothing to release; the plan's is_ref table knows)
            if arena.plan.is_ref.get(r.val_ty as usize).copied().unwrap_or(false) {
                release_ref_slot(arena, acct, r.slot);
            }
        }
    }
}

/// A host-held `Opaque` handle (RFC 0014; re-based on the Opaque store
/// at nmap-hostvals P2): owns one store-entry reference. `ptr` is the
/// TAGGED slot word — the same word a rut `opaque` slot carries — so the
/// handle round-trips through `Slot` and the generic rc web routes it.
pub struct OpaqueRef {
    arena: Rc<Arena>,
    acct: Rc<HeapAcct>,
    ptr: *const CellVal,
}

impl OpaqueRef {
    /// The tagged slot word (build a `Slot` from it directly). Only the
    /// tag-aware accessors may deref it — see `heap::store`.
    pub fn ptr(&self) -> *const CellVal {
        self.ptr
    }
    /// The untagged slab pointer — the crate's own entry accessors only.
    pub(crate) fn entry_ptr(&self) -> *mut store::EntryCell {
        store::untag_entry(self.ptr)
    }
    unsafe fn bump(ptr: *const CellVal) {
        let c = unsafe { &*store::untag_entry(ptr) };
        c.refs.set(c.refs.get().saturating_add(1));
    }
    /// Wrap a tagged slot word, retaining once (the caller transfers
    /// ownership).
    pub(crate) fn new(arena: &Rc<Arena>, acct: &Rc<HeapAcct>, ptr: *const CellVal) -> OpaqueRef {
        debug_assert!(store::is_entry(Slot { r: ptr }), "opaque_handle on a non-store slot");
        unsafe { Self::bump(ptr) };
        OpaqueRef { arena: arena.clone(), acct: acct.clone(), ptr }
    }

    /// Wrap an entry JUST minted (`refs` == 1): the handle takes over the
    /// mint reference instead of adding one. `Opaque::alloc` and the
    /// host-fn return path use this — mint, then hand straight to the
    /// boundary — so the box's first crossing owns exactly one reference.
    pub(crate) fn owning(arena: &Rc<Arena>, acct: &Rc<HeapAcct>, ptr: *const CellVal) -> OpaqueRef {
        OpaqueRef { arena: arena.clone(), acct: acct.clone(), ptr }
    }
}

impl Clone for OpaqueRef {
    fn clone(&self) -> Self {
        unsafe { Self::bump(self.ptr) };
        OpaqueRef { arena: self.arena.clone(), acct: self.acct.clone(), ptr: self.ptr }
    }
}

impl Drop for OpaqueRef {
    fn drop(&mut self) {
        release_ref_slot(
            &self.arena,
            &self.acct,
            Slot { r: self.ptr },
        );
    }
}

impl PartialEq for OpaqueRef {
    fn eq(&self, other: &Self) -> bool {
        self.ptr == other.ptr
    }
}

impl std::fmt::Debug for OpaqueRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Opaque(<cell>)")
    }
}
