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

use crate::heap::{CellData, CellVal, HeapAcct, Slot};
use rut_core::binary::FuncCode;
use rut_core::types::{TypeTable, TyKind};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
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
    sum_payload_ref: Vec<[bool; 2]>,
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
                _ => Vec::new(),
            })
            .collect();
        let sum_payload_ref = types
            .types
            .iter()
            .map(|t| match &t.kind {
                TyKind::Option { elem } => [of(elem), false],
                TyKind::Result { ok, err } => [of(ok), of(err)],
                _ => [false, false],
            })
            .collect();
        let closure_capture_ref = funcs
            .iter()
            .map(|f| f.params.iter().map(of).collect())
            .collect();
        ReleasePlan { is_ref, record_refs, sum_payload_ref, closure_capture_ref }
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
}

impl Arena {
    pub(crate) fn new(plan: Rc<ReleasePlan>) -> Arena {
        Arena {
            free: RefCell::new(Vec::new()),
            chunks: RefCell::new(Vec::new()),
            bump: Cell::new(ARENA_CHUNK),
            plan,
        }
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
        if chunks.is_empty() {
            return;
        }
        let total = (chunks.len() - 1) * ARENA_CHUNK + self.bump.get();
        for (ci, chunk) in chunks.iter().enumerate() {
            let base = chunk.as_ptr() as *mut CellVal;
            for i in 0..ARENA_CHUNK {
                if ci * ARENA_CHUNK + i >= total {
                    break;
                }
                let p = unsafe { base.add(i) };
                if !free.contains(&p) {
                    unsafe { std::ptr::drop_in_place(p) };
                }
            }
        }
    }
}

/// One reference gone. At rc-0 the cell dies — and its ref-typed children
/// die with it (RFC 0016 §3): `release_cell` collects them and recurses,
/// so a record's `str`/`Opaque`/vec fields no longer pin their children
/// until VM end.
#[inline(always)]
pub(crate) fn release_ref_slot(arena: &Arena, acct: &HeapAcct, s: Slot) {
    let p = unsafe { s.r };
    if p.is_null() {
        return;
    }
    unsafe {
        let c = &*p;
        let n = c.refs.get();
        if n == u32::MAX {
            return; // immortal singleton
        }
        if n <= 1 {
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
        CellData::Sum { tag, payload } => {
            if let Some(p) = payload {
                let refs = plan
                    .sum_payload_ref
                    .get(c.ty as usize)
                    .copied()
                    .unwrap_or([false, false]);
                if refs[*tag as usize] {
                    out.push(*p);
                }
            }
        }
        CellData::Array { elem, items } => {
            // the ELEMENT type decides: a `Vec<str>`'s slots are cell handles
            if plan.is_ref.get(*elem as usize).copied().unwrap_or(false) {
                out.extend(items.borrow_mut().take_slots());
            }
        }
        // a user box stores the inner handle — its release is the box's
        // own (RFC 0014: box death drops the boxed value's reference)
        CellData::OpaqueBox { val, val_ty } => {
            if plan.is_ref.get(*val_ty as usize).copied().unwrap_or(false) {
                out.push(*val);
            }
        }
        CellData::Closure { func, captures } => {
            if let Some(flags) = plan.closure_capture_ref.get(*func as usize) {
                for (&cap, &is_ref) in captures.iter().zip(flags.iter()) {
                    if is_ref {
                        out.push(cap);
                    }
                }
            }
        }
        // a host payload box has no rut-typed children — the Rust payload
        // is dropped with the cell through its own Drop (RFC 0023/0026)
        CellData::HostBoxed { .. } | CellData::Enum { .. } | CellData::Str(_) => {}
    }
    out
}

/// Drop a cell whose strong count reached zero: refund its accounting,
/// release its ref-typed children (see `collect_ref_children`), run its
/// destructor, and recycle the slot.
#[inline(always)]
pub(crate) fn release_cell(arena: &Arena, acct: &HeapAcct, p: *mut CellVal) {
    // children are collected first and released after the parent is
    // freed, so a release cascade can never observe the dying cell.
    // Childless kinds (the churn case: strs, singletons, host boxes)
    // never build the child vec at all.
    let children = unsafe {
        match (*p).data {
            CellData::Str(_) | CellData::Enum { .. } | CellData::HostBoxed { .. } => None,
            _ => Some(collect_ref_children(&*p, &arena.plan)),
        }
    };
    unsafe {
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

/// A host-held `Opaque` handle (RFC 0014): owns one arena reference.
pub struct OpaqueRef {
    arena: Rc<Arena>,
    acct: Rc<HeapAcct>,
    ptr: *const CellVal,
}

impl OpaqueRef {
    pub fn ptr(&self) -> *const CellVal {
        self.ptr
    }
    unsafe fn bump(ptr: *const CellVal) {
        let c = unsafe { &*ptr };
        c.refs.set(c.refs.get().saturating_add(1));
    }
    /// Wrap `ptr`, retaining once (the caller transfers ownership).
    pub(crate) fn new(arena: &Rc<Arena>, acct: &Rc<HeapAcct>, ptr: *const CellVal) -> OpaqueRef {
        unsafe { Self::bump(ptr) };
        OpaqueRef { arena: arena.clone(), acct: acct.clone(), ptr }
    }

    /// Wrap a cell JUST minted (`refs` == 1): the handle takes over the
    /// mint reference instead of adding one. `OpaqueBox::alloc` and the
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
