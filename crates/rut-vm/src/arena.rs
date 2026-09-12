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

use crate::heap::{CellVal, HeapAcct};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::mem::MaybeUninit;
use std::rc::Rc;

const ARENA_CHUNK: usize = 1024;

/// Chunked cell storage with a free list. Pointers into a chunk are stable,
/// so a `Slot` can hold a raw `*const CellVal` for the cell's lifetime.
pub(crate) struct Arena {
    free: RefCell<Vec<*mut CellVal>>,
    chunks: RefCell<Vec<Box<[MaybeUninit<CellVal>; ARENA_CHUNK]>>>,
    /// slots used in the last chunk
    bump: Cell<usize>,
}

impl Arena {
    pub(crate) fn new() -> Arena {
        Arena {
            free: RefCell::new(Vec::new()),
            chunks: RefCell::new(Vec::new()),
            bump: Cell::new(ARENA_CHUNK),
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

/// Drop a cell whose strong count reached zero: refund its accounting,
/// run its destructor, and recycle the slot.
pub(crate) fn release_cell(arena: &Arena, acct: &HeapAcct, p: *mut CellVal) {
    unsafe {
        let bytes = (*p).bytes as u64;
        acct.used.set(acct.used.get().saturating_sub(bytes));
        std::ptr::drop_in_place(p);
    }
    arena.free.borrow_mut().push(p);
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
}

impl Clone for OpaqueRef {
    fn clone(&self) -> Self {
        unsafe { Self::bump(self.ptr) };
        OpaqueRef { arena: self.arena.clone(), acct: self.acct.clone(), ptr: self.ptr }
    }
}

impl Drop for OpaqueRef {
    fn drop(&mut self) {
        let n = unsafe { (*self.ptr).refs.get() };
        if n == u32::MAX {
            return; // immortal singleton
        }
        if n <= 1 {
            release_cell(&self.arena, &self.acct, self.ptr as *mut CellVal);
        } else {
            unsafe { (*self.ptr).refs.set(n - 1) };
        }
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
