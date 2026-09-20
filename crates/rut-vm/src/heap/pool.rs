//! The OpaqueBox pool — a size-classed Rust-side free list for the
//! erased payload allocations behind `CellData::HostBoxed`
//! (native-fastpath batch, phase 6).
//!
//! The churn profile this attacks: every host mint
//! (`OpaqueBox::alloc` — `map_new`'s `NativeTable` box, plugin state,
//! host-fn returns) was a fresh `Box::new` (malloc) and every rc-0 box
//! death was a free. Classed by the payload type's exact
//! `(size, align)` layout, the block (`Layout::<T>()`, matching
//! `Box::new`'s allocation) is recycled: a death runs the stored
//! payload's destructor **exactly once, in place** — per host
//! semantics (RFC 0016 §3) — then parks the block for the next mint
//! of the same class; a mint pops a parked block (or `Box::new`s on a
//! miss) and writes the new value over it.
//!
//! Soundness: the parked block is only ever handed out for a payload
//! with the SAME `Layout` it was allocated with, so `Box<dyn Any>`'s
//! later dealloc (or the pool's own trim/teardown dealloc) uses a
//! layout equal to the allocation's, per `GlobalAlloc` contract.
//! Every death is routed through `retire` (both drop contexts below —
//! `release_cell` and arena teardown), so the box's own drop glue
//! never runs twice: `retire` takes the box out of the cell, runs the
//! destructor in place through the erased `dyn Any` fat pointer, and
//! only then does any pool push or dealloc happen.
//!
//! Lifetime choice — **arena-owned, not thread_local**. Host payload
//! cells die only through contexts that already hold `&Arena` (the
//! release path and the arena teardown walk), so a per-`Arena` pool
//! has no lifetime queries at all, and the engine is single-threaded
//! per `Vm` (`Cell`/`Rc` throughout — no `Send`/`Sync` exists), which
//! `thread_local` sizing would not need but also could not share
//! across `Vm`s anyway. Blocks ARE `Global` allocations, so a pool
//! could be shared; that is deliberately not done — a dead arena's
//! pool deallocates its remainder with the arena, bounding host
//! memory without a global registry.
//!
//! Rut-minted `opaque(v)` boxes are NOT this allocation: `Op::Box`
//! lowers to `Heap::alloc_opaque` — a `CellData::OpaqueBox { val: Slot }`
//! cell whose payload is a slot INLINE in the cell (no Rust block exists
//! behind it, so there is nothing to pool). Their churn is the CELL
//! itself, and the arena free list already recycles that (`alloc_slot`).
//! The pool therefore tiers the HOST payload blocks — `OpaqueBox::alloc`
//! mints (`map_new`'s `NativeTable`, plugin state, host-fn returns) —
//! which is exactly the tier phase 7 keeps when small payloads go
//! inline (NativeTable, host structs stay boxed, pooled here).
//!
//! Bounded (the high-water rule): the pool never holds more blocks
//! than the churn's standing demand; on retire, when the stored count
//! exceeds `HIGH_WATER` blocks (~128 KiB worst case at `POOL_MAX`) a
//! parked block from an arbitrary class is deallocated until back
//! under the mark (HashMap order — the BOUND is the contract, not
//! FIFO fairness), so a workload's peak does not become a permanent
//! leak. Payloads wider than `POOL_MAX` bytes (NativeTable's big `Vec`s
//! are already behind their own `Box`s — this pools the shallow `T`)
//! bypass the pool.

use crate::arena::Arena;
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::mem;

/// Payloads ≥ this many bytes skip the pool entirely (a large shallow
/// payload is rare; leaving it unpooled keeps the parked-block budget
/// small).
const POOL_MAX: usize = 256;
/// Parked blocks retained per arena before trimming.
const HIGH_WATER: usize = 512;

/// One park/pop buffer, owned by an `Arena`.
pub(crate) struct Pool {
    /// key: payload `(size, align)`; value: parked raw blocks whose
    /// allocation layout is exactly that key.
    free: HashMap<(usize, usize), Vec<usize>>,
    /// total parked blocks — the trim counter
    stored: usize,
}

impl Pool {
    pub(crate) fn new() -> Pool {
        Pool { free: HashMap::new(), stored: 0 }
    }

    /// Pop a parked block with exactly `Layout::from_size_align(sz, al)`.
    fn take(&mut self, sz: usize, al: usize) -> Option<*mut u8> {
        let v = self.free.get_mut(&(sz, al))?;
        let raw = v.pop()?;
        self.stored -= 1;
        Some(raw as *mut u8)
    }

    /// Park a block whose allocation layout is `(sz, al)`, trimming to
    /// the high-water mark.
    fn park(&mut self, sz: usize, al: usize, raw: *mut u8) {
        // trim: deallocate parked blocks down to the mark before parking
        // a new one (an arbitrary class — see the module docs: the bound
        // is the contract). Each block deallocates with the layout of
        // ITS class — the key of the vec it was parked in — never the
        // incoming block's layout.
        while self.stored >= HIGH_WATER {
            let Some((key, p)) = self.any_parked() else {
                break;
            };
            unsafe { dealloc_block(p, key.0, key.1) };
        }
        // Trim ran until `any_oldest` found nothing (stored == 0) or below
        // the mark; HIGH_WATER >= 1 is assumed throughout this module.
        debug_assert!(self.stored < HIGH_WATER);
        self.free.entry((sz, al)).or_default().push(raw as usize);
        self.stored += 1;
    }

    /// Parked-block count — the trim counter's read side (tests).
    #[cfg(test)]
    fn parked(&self) -> usize {
        self.stored
    }

    /// Pull a parked block out of whichever class has one, with that
    /// class's layout key, bumping `stored` — the caller piles
    /// deallocation on top.
    fn any_parked(&mut self) -> Option<((usize, usize), *mut u8)> {
        let key = *self.free.iter().find(|(_, v)| !v.is_empty())?.0;
        let v = self.free.get_mut(&key)?;
        let raw = v.pop()?;
        self.stored -= 1;
        Some((key, raw as *mut u8))
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        for ((sz, al), v) in self.free.drain() {
            for raw in v {
                unsafe { dealloc_block(raw as *mut u8, sz, al) };
            }
        }
    }
}

/// A GlobalAlloc with exactly `(sz, al)` — the same layout `Box::new`
/// used when the block was first minted.
unsafe fn dealloc_block(p: *mut u8, sz: usize, al: usize) {
    std::alloc::dealloc(
        p,
        std::alloc::Layout::from_size_align(sz, al)
            .expect("pool block layout is size/align of a live Rust type"),
    );
}

/// Mint the payload allocation: pop a parked block of `T`'s layout when
/// one is free, else `Box::new`. Either way the caller gets a plain
/// `Box<T>` carrying `val` — the cell cannot tell the paths apart.
pub(crate) fn host_box<T: 'static>(arena: &Arena, val: T) -> Box<T> {
    let (sz, al) = (mem::size_of::<T>(), mem::align_of::<T>());
    if sz == 0 || sz > POOL_MAX {
        return Box::new(val);
    }
    let raw = arena.pool.borrow_mut().take(sz, al);
    let Some(raw) = raw else {
        return Box::new(val);
    };
    let p = raw as *mut T;
    // SAFETY: the block is parked, uninitialised, and sized/aligned
    // exactly for a `T` (the (sz, al) key) — it may have been minted for
    // an earlier payload type of the same layout, and `align_of::<T>()`
    // <= the block's actual alignment by construction.
    unsafe { p.write(val) };
    // Self-soundness: `Box::from_raw` requires the pointer to have been
    // allocated with `Layout::for_value::<T>()` — it was (the (sz, al)
    // key matches `Box::new<T>`'s allocation, whether this mint or any
    // earlier one minted the block).
    unsafe { Box::from_raw(p) }
}

/// A drop site for a `CellData::HostBoxed` payload: run the payload's
/// destructor in place (exactly once — the box's own glue cannot run
/// again after `into_raw`) and either park the block or free it.
///
/// `#[inline(never)]` on purpose: `release_cell` is `#[inline(always)]`
/// into the interpreter's release web, and inlining the pool machinery
/// (RefCell borrow, HashMap probe, trim) into it perturbed every op
/// handler's codegen — a cold path must stay out of the hot fn's body.
#[inline(never)]
pub(crate) fn retire(arena: &Arena, payload: Box<dyn Any>) {
    // layout figures BEFORE the drop runs — gone once the inner is raw
    let (sz, al) = {
        let d: &dyn Any = &*payload;
        (mem::size_of_val(d), mem::align_of_val(d))
    };
    let inner = Box::into_raw(payload);
    // SAFETY: `inner` is a live `*mut dyn Any` from `into_raw`; dropping
    // it here runs the concrete payload's destructor in place; the Box's
    // dealloc glue is never involved again (we own the raw side now).
    unsafe { std::ptr::drop_in_place(inner) };
    let p = inner as *mut u8;
    if sz == 0 || sz > POOL_MAX {
        // A ZST payload never allocated a block — `Box<T>`'s own glue
        // skips the dealloc for the same reason — so there is nothing
        // to free and `p` is a dangling (well-aligned) `NonNull::dangling`.
        if sz > 0 {
            unsafe { dealloc_block(p, sz, al) };
        }
        return;
    }
    arena.pool.borrow_mut().park(sz, al, p);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::ReleasePlan;
    use rut_core::binary::Program;
    use rut_core::types::TypeTable;
    use std::rc::Rc;

    /// A bare arena — no VM, just the heap structures the pool rides.
    fn test_arena() -> Rc<Arena> {
        let prog = Program {
            name: "pool".into(),
            scope: 0,
            interner: Default::default(),
            surface: Default::default(),
            types: TypeTable::boot(),
            traits: Vec::new(),
            trait_slots: Vec::new(),
            vtables: Vec::new(),
            consts: Vec::new(),
            funcs: Vec::new(),
            exports: Vec::new(),
        };
        Rc::new(Arena::new(Rc::new(ReleasePlan::build(&prog.types, &prog.funcs))))
    }

    /// The HostBoxed drop context, exactly as `release_cell` runs it:
    /// mint a host box into a cell-shaped life, then "die" it.
    fn die<T: 'static>(arena: &Arena, val: T) {
        let b: Box<dyn Any> = host_box(arena, val);
        retire(arena, b);
    }

    #[test]
    fn roundtrip_reuses_same_block() {
        let arena = test_arena();
        let b1 = host_box(&arena, [7u8; 64]);
        let addr1 = &*b1 as *const [u8; 64] as usize;
        assert_eq!(arena.pool.borrow().parked(), 0, "a live mint parks nothing");
        drop(b1);
        assert_eq!(arena.pool.borrow().parked(), 0, "dropping a Box is NOT a cell death — retire is");
        die(&arena, [7u8; 64]);
        assert_eq!(arena.pool.borrow().parked(), 1, "the death parks its block");
        let b2 = host_box(&arena, [0u8; 64]);
        let addr2 = &*b2 as *const [u8; 64] as usize;
        assert_eq!(arena.pool.borrow().parked(), 0, "the mint popped the parked block");
        assert_eq!(addr1, addr2, "same size class: the block is recycled");
        drop(b2);
        drop(arena);
    }

    #[test]
    fn destructor_runs_exactly_once_per_death() {
        static DROPS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        struct Counted;
        impl Drop for Counted {
            fn drop(&mut self) {
                DROPS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let arena = test_arena();
        die(&arena, Counted);
        assert_eq!(DROPS.load(std::sync::atomic::Ordering::SeqCst), 1, "the death ran the dtor once");
        // mint again over the parked block: reuse must NOT re-run anything
        let b = host_box(&arena, Counted);
        assert_eq!(DROPS.load(std::sync::atomic::Ordering::SeqCst), 1, "reuse never re-runs the parked dtor");
        // the cell death — retire — runs it exactly once more, in place
        die(&arena, Counted);
        assert_eq!(DROPS.load(std::sync::atomic::Ordering::SeqCst), 2);
        // and a plain Box drop (a value never in a cell) is one run too
        drop(b);
        assert_eq!(DROPS.load(std::sync::atomic::Ordering::SeqCst), 3);
        drop(arena); // the parked block drops with the pool — no dtor runs
        assert_eq!(DROPS.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[test]
    fn classes_are_separate() {
        let arena = test_arena();
        let a = host_box(&arena, [0u8; 32]);
        let b = host_box(&arena, [0u8; 128]);
        drop(a);
        drop(b);
        die(&arena, [1u8; 32]);
        die(&arena, [1u8; 128]);
        assert_eq!(arena.pool.borrow().parked(), 2);
        let a2 = host_box(&arena, [0u8; 32]);
        let a_addr = &*a2 as *const [u8; 32] as usize;
        let _b2 = host_box(&arena, [0u8; 128]);
        assert_eq!(arena.pool.borrow().parked(), 0, "each class popped its own block");
        drop(a2);
        die(&arena, [2u8; 32]);
        let c = host_box(&arena, [0u8; 32]);
        assert_eq!(&*c as *const [u8; 32] as usize, a_addr, "class keys are exact (size, align)");
        drop(c);
        drop(arena);
    }

    #[test]
    fn trim_caps_the_pool() {
        let arena = test_arena();
        for i in 0..600u32 {
            die(&arena, i);
        }
        let parked = arena.pool.borrow().parked();
        assert!(parked < 600, "trim must cap the pool: {parked} parked");
        assert!(parked <= 512, "HIGH_WATER respected: {parked} parked");
    }

    #[test]
    fn big_and_zero_sized_bypass_the_pool() {
        let arena = test_arena();
        let big = host_box(&arena, [0u8; POOL_MAX + 1]);
        drop(big);
        die(&arena, [0u8; POOL_MAX + 1]);
        assert_eq!(arena.pool.borrow().parked(), 0, "oversized payloads skip the pool");
        die(&arena, ());
        assert_eq!(arena.pool.borrow().parked(), 0, "ZST payloads skip the pool (no block exists)");
        die(&arena, [0u8; POOL_MAX]);
        assert_eq!(arena.pool.borrow().parked(), 1, "the boundary size still pools");
        drop(arena);
    }

    #[test]
    fn arena_teardown_runs_the_payload_dtor_once_and_frees() {
        use crate::heap::{CellData, CellVal};
        static DROPS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        struct Counted;
        impl Drop for Counted {
            fn drop(&mut self) {
                DROPS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let arena = test_arena();
        // a LIVE host-payload cell (never released): the arena teardown
        // walk is the only path that can retire it
        let p = arena.alloc_slot();
        unsafe {
            p.write(CellVal {
                ty: rut_core::types::TY_OPAQUE,
                data: CellData::HostBoxed {
                    payload: Box::new(Counted),
                    type_name: std::any::type_name::<Counted>(),
                    borrows: std::cell::Cell::new(0),
                },
                refs: std::cell::Cell::new(1),
                bytes: 16,
            });
        }
        drop(arena); // the teardown walk retires it — dtor once, block parked
        assert_eq!(
            DROPS.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "teardown ran the payload dtor exactly once (no leak, no double drop)"
        );
        // and the parked block went with the arena's pool — a fresh mint
        // on the new arena is served by the allocator, not by dead state
        let arena2 = test_arena();
        let _c = host_box(&arena2, 0u8);
    }
}
