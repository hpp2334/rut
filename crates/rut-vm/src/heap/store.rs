//! The Opaque store (nmap-hostvals P2) — the ONE home of every rut
//! `opaque` value. TWO kinds, TWO identity worlds:
//!
//! - [`OpaqueEntry::Host`] — a HOST payload (the Rust `TypeId` world):
//!   [`HostOpaque`], a boxed `'static` value behind the
//!   [`HostPayload`] release hook (one per map/logger, never per-op —
//!   the Box is plain Rust malloc, RFC 0023/0026 relaxed call).
//! - [`OpaqueEntry::Rut`] — a RUT value held host-side ([`RutOpaque`],
//!   the RFC 0014 box shape): the payload slot moves into the entry and
//!   its runtime type rides along, so `downcast<T>` reads the cell's
//!   kind and never a Rust `TypeId`.
//!
//! The repr law: the rut-side `opaque` value addresses a store entry
//! DIRECTLY — the arena anchor cell (`CellData::HostBoxed` /
//! `CellData::OpaqueBox`) is gone, one allocation and one indirection
//! fewer per opaque. The slab is chunked like the cell arena (entries
//! never move, a freed slot returns to the free list), and the entry's
//! slot word is the entry pointer TAGGED with bit 0 — a store slot is
//! self-identifying, so the generic rc web (`Heap::retain` /
//! `release_ref_slot`, register/frame/record-field release) routes
//! store slots to the entry's own rc without knowing any types.
//!
//! `rc` and the borrow guard live ON the entry: the BORROW_MUT law
//! (RFC 0023 §2) moved here from the old anchor cell — same guard
//! semantics, new home. Death is rc-0 inside the release walk: the
//! Host payload's `finalize` hook runs BEFORE the Box's own Drop
//! (RFC 0016 §3), the Rut entry's held slot releases with the
//! record-field pattern (children after the parent is freed).

use super::*;
use std::any::Any;
use std::cell::Cell;
use std::marker::PhantomData;
use std::mem;

use crate::arena::OpaqueRef;

/// Exclusive-borrow sentinel (RFC 0023 §2): 0 = free, `BORROW_MUT` = one
/// exclusive borrow live, else the shared-borrow count.
pub(crate) const BORROW_MUT: u32 = u32::MAX;

/// The slot-word tag (bit 0) marking a store entry — every entry pointer
/// is 8-aligned, so the bit is free. A tagged slot word routes retain/
/// release to the entry's rc; an untagged one stays a cell pointer.
pub(crate) const STORE_TAG: usize = 1;

/// The release hook (nmap-hostvals P2; RFC 0016 §3's finalizer arm).
/// Runs at store-entry death — rc-0 inside the release walk — BEFORE the
/// payload's own `Drop`. The no-op default is the P2 posture: payloads
/// opt in. (P5's `NativeTable` fills it: release every held value cell.)
pub trait HostPayload: Any {
    fn finalize(&mut self, _heap: &Heap) {}
}

/// A HOST payload — the Rust `TypeId` world. `type_name` survives for
/// diagnostics (unrecoverable from the erased box), the borrow guard
/// rides the entry, and the finalize hook is captured at mint iff the
/// payload opted into [`HostPayload`].
///
/// The payload itself is `Box<dyn Any>` (directive 9: "HostOpaque can
/// use Box Any if needed"); the optional monomorphized finalize thunk
/// is what makes it a [`HostPayload`] without forcing every minted type
/// to implement the trait — the embedder mints plain `'static` values
/// (`Opaque::alloc(vm, 42i64)`), and a foreign-foreign impl could never
/// be written for those.
pub struct HostOpaque {
    pub(crate) payload: Box<dyn Any>,
    pub(crate) type_name: &'static str,
    pub(crate) finalize: Option<fn(&mut dyn Any, &Heap)>,
}

/// A RUT value held host-side (the RFC 0014 box): identity IS the held
/// slot; the rut-side downcast reads the cell's runtime kind via
/// `val_ty` — never a Rust `TypeId`. `val_ty` rides the entry because a
/// prim payload is raw bits in `slot` (the only type info there is).
pub struct RutOpaque {
    pub(crate) slot: Slot,
    pub(crate) val_ty: TypeId,
}

/// What a rut `opaque` value addresses — the ONE store's two kinds.
pub enum OpaqueEntry {
    /// a HOST payload — the Rust TypeId world
    Host(HostOpaque),
    /// a RUT value held host-side — the RFC 0014 shape
    Rut(RutOpaque),
}

/// One slab element: the entry + its rc + the borrow guard. `bytes` is
/// the RFC 0040 charge the mint made, refunded at death.
pub(crate) struct EntryCell {
    pub(crate) e: OpaqueEntry,
    pub(crate) refs: Cell<u32>,
    pub(crate) borrows: Cell<u32>,
    pub(crate) bytes: u32,
}

/// Tag a slab pointer into its slot-word form.
pub(crate) fn tag_entry(p: *mut EntryCell) -> *const CellVal {
    (p as usize | STORE_TAG) as *const CellVal
}

/// The untagged slab pointer behind a slot word. Only legal on tagged
/// words (`is_entry`).
pub(crate) fn untag_entry(p: *const CellVal) -> *mut EntryCell {
    (p as usize & !STORE_TAG) as *mut EntryCell
}

/// Is this slot word a store entry (vs an arena cell pointer)?
#[inline(always)]
pub(crate) fn is_entry(s: Slot) -> bool {
    unsafe { (s.r as usize) & STORE_TAG == STORE_TAG }
}

/// The entry behind a tagged slot word — `None` for any cell slot.
#[inline(always)]
pub(crate) fn store_entry(s: Slot) -> Option<&'static EntryCell> {
    if !is_entry(s) {
        return None;
    }
    Some(unsafe { &*(untag_entry(s.r) as *const EntryCell) })
}

/// The VM-side slab: fixed-size chunks (entries never move) + a free
/// list, shaped like the cell arena. Counters feed the debug VM-exit
/// balance check (§0.8 i).
pub(crate) struct Store {
    free: std::cell::RefCell<Vec<*mut EntryCell>>,
    chunks: std::cell::RefCell<Vec<Box<[mem::MaybeUninit<EntryCell>; STORE_CHUNK]>>>,
    bump: Cell<usize>,
    live: Cell<usize>,
    charged: Cell<u64>,
    inserts: Cell<u64>,
    deaths: Cell<u64>,
}

const STORE_CHUNK: usize = 1024;

impl Store {
    pub(crate) fn new() -> Store {
        Store {
            free: std::cell::RefCell::new(Vec::new()),
            chunks: std::cell::RefCell::new(Vec::new()),
            bump: Cell::new(STORE_CHUNK),
            live: Cell::new(0),
            charged: Cell::new(0),
            inserts: Cell::new(0),
            deaths: Cell::new(0),
        }
    }

    /// Insert an entry owning one reference. The caller did the RFC 0040
    /// charge (`bytes` rides the entry for the death refund).
    pub(crate) fn insert(&self, e: OpaqueEntry, bytes: u64) -> Slot {
        let p = {
            let popped = self.free.borrow_mut().pop();
            match popped {
                Some(p) => p,
                None => {
                    let mut chunks = self.chunks.borrow_mut();
                    let mut bump = self.bump.get();
                    if bump >= STORE_CHUNK {
                        chunks.push(Box::new([const { mem::MaybeUninit::uninit() }; STORE_CHUNK]));
                        bump = 0;
                    }
                    let idx = chunks.len() - 1;
                    let p = unsafe { (chunks[idx].as_mut_ptr() as *mut EntryCell).add(bump) };
                    self.bump.set(bump + 1);
                    p
                }
            }
        };
        unsafe {
            p.write(EntryCell {
                e,
                refs: Cell::new(1),
                borrows: Cell::new(0),
                bytes: bytes.min(u32::MAX as u64) as u32,
            });
        }
        self.live.set(self.live.get() + 1);
        self.charged.set(self.charged.get() + bytes);
        self.inserts.set(self.inserts.get() + 1);
        Slot { r: tag_entry(p) }
    }

    /// Death bookkeeping — the caller took the payload out; the slot
    /// returns to the free list and the charge is refunded by the
    /// release walk (`release_entry`, arena.rs).
    pub(crate) fn note_death(&self, p: *mut EntryCell, bytes: u64) {
        self.free.borrow_mut().push(p);
        self.live.set(self.live.get().saturating_sub(1));
        self.charged.set(self.charged.get().saturating_sub(bytes));
        self.deaths.set(self.deaths.get() + 1);
    }

    /// Live entries — the host-facing leak probe (unit tests).
    #[cfg(test)]
    pub fn live(&self) -> usize {
        self.live.get()
    }

    /// Teardown: drop every entry still live (the trap-torn-frame / leak
    /// survivors — no legal owner exists, or the `Rc`s would have kept
    /// the arena alive). The finalize hook is the release walk's and the
    /// walk is over; the payload `Box`es drop here. The debug balance
    /// check pins the counters: every insert was refunded exactly once.
    pub(crate) fn teardown(&self) {
        let free: std::collections::HashSet<*mut EntryCell> =
            self.free.borrow().iter().copied().collect();
        let chunks = self.chunks.borrow();
        let mut freed = 0u64;
        let mut refund = 0u64;
        for (ci, chunk) in chunks.iter().enumerate() {
            let total = if ci + 1 == chunks.len() { self.bump.get() } else { STORE_CHUNK };
            let base = chunk.as_ptr() as *mut EntryCell;
            for i in 0..total {
                let p = unsafe { base.add(i) };
                if free.contains(&p) {
                    continue;
                }
                freed += 1;
                refund += unsafe { (*p).bytes } as u64;
                let taken = mem::replace(
                    unsafe { &mut (*p).e },
                    OpaqueEntry::Rut(RutOpaque { slot: Slot::null(), val_ty: 0 }),
                );
                drop(taken);
            }
        }
        // the balance law (§0.8 i): every minted entry died exactly once
        // (walk or teardown), and the charges outstanding are exactly the
        // entries the teardown just freed — nothing was double-counted or
        // lost on the way.
        debug_assert_eq!(
            self.inserts.get(),
            self.deaths.get() + freed,
            "store balance: every entry minted must die exactly once (walk or teardown)"
        );
        debug_assert_eq!(
            self.charged.get(),
            refund,
            "store balance: the outstanding charges must be exactly the live entries' own"
        );
        self.charged.set(0);
        self.live.set(0);
    }
}

/// The host's typed view over a store entry (RFC 0023/0026): `alloc`
/// news any `'static` Rust value into the store (no finalize hook —
/// the plain-embedder mint), `alloc_hosted` news a [`HostPayload`] and
/// wires its hook, and `from_handle` re-decodes a handle with the
/// checked-borrow law (`payload.is::<T>()` — a wrong `T` is a checked
/// error naming both sides, never UB; a rut-value box answering a Host
/// decode names the recovery). `with`/`with_mut` borrow the payload for
/// the closure's duration — the guards live on the store entry (same
/// BORROW_MUT law, new home), and `with_mut` flows the VM because the
/// store lives behind it: the exclusive borrow's closure reaches the
/// heap for in-crossing rc work (P5's crossings), no stale captures.
pub struct Opaque<T: 'static> {
    handle: OpaqueRef,
    _marker: PhantomData<fn() -> T>,
}

impl<T: 'static> Opaque<T> {
    /// News the box: `val` moves into the store immediately, its shallow
    /// `size_of::<T>()` charged to the heap budget (RFC 0040). No
    /// finalize hook — see [`Opaque::alloc_hosted`].
    pub fn alloc(vm: &mut crate::interp::Vm, val: T) -> Result<Opaque<T>, Trap> {
        let slot = vm.heap.alloc_host_box(val, None)?;
        let handle = vm.heap.opaque_handle_take(unsafe { slot.r });
        Ok(Opaque { handle, _marker: PhantomData })
    }

    /// Same check over a bare handle. The payload's own `Any` vtable
    /// carries the type token — `is::<T>` is the check, so a wrong-type
    /// borrow is a checked error, never UB. `type_name` (kept in the
    /// entry, unrecoverable from the erased box) names what it holds.
    pub fn from_handle(h: &OpaqueRef) -> Result<Opaque<T>, Trap> {
        let cell = unsafe { &*h.entry_ptr() };
        let OpaqueEntry::Host(host) = &cell.e else {
            return Err(Trap::new(
                TrapKind::Invalid,
                "the box holds a rut value, not a host payload — recover it with `downcast<T>` on the rut side (RFC 0014)",
            ));
        };
        if !host.payload.is::<T>() {
            return Err(Trap::new(
                TrapKind::Invalid,
                format!("host box holds `{}`, not `{}`", host.type_name, std::any::type_name::<T>()),
            ));
        }
        Ok(Opaque { handle: h.clone(), _marker: PhantomData })
    }

    /// Shared borrow for the closure's duration. Nested `with`s stack;
    /// an active `with_mut` excludes them. The guard lives on the store
    /// entry (reachable through the view's own handle — the rc it holds
    /// pins the entry for the borrow's scope), so the shared borrow needs
    /// no VM; `with_mut` is the shape that flows it (P5's in-crossing rc
    /// law runs its heap work inside the exclusive borrow).
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> Result<R, Trap> {
        let (payload, borrows) = self.boxed();
        if borrows.get() == BORROW_MUT {
            return Err(Trap::new(
                TrapKind::Invalid,
                "host box is `&mut`-borrowed by an outer host call (RFC 0023 §2)",
            ));
        }
        borrows.set(borrows.get() + 1);
        let out = f(payload
            .downcast_ref::<T>()
            .expect("Opaque<T> holds a T — checked at from_handle/alloc"));
        borrows.set(borrows.get() - 1);
        Ok(out)
    }

    /// Exclusive borrow for the closure's duration. A re-entrant
    /// `vm.call` reaching another host fn that borrows the same box
    /// traps `borrowed by host` instead of aliasing (RFC 0023 §2).
    pub fn with_mut<R>(
        &self,
        vm: &mut crate::interp::Vm,
        f: impl FnOnce(&mut crate::interp::Vm, &mut T) -> R,
    ) -> Result<R, Trap> {
        let (payload, borrows) = self.boxed_mut();
        if borrows.get() != 0 {
            return Err(Trap::new(
                TrapKind::Invalid,
                "host box is borrowed by an outer host call (RFC 0023 §2)",
            ));
        }
        borrows.set(BORROW_MUT);
        let out = f(vm, payload
            .downcast_mut::<T>()
            .expect("Opaque<T> holds a T — checked at from_handle/alloc"));
        borrows.set(0);
        Ok(out)
    }

    /// The erased handle (RFC 0023): the box's `Opaque` identity, for
    /// passing the box through typed `call`/host-fn boundaries.
    pub fn handle(&self) -> &OpaqueRef {
        &self.handle
    }

    /// The erased payload and its borrow guard (always a Host entry —
    /// `from_handle`/`alloc` constructed it).
    fn boxed(&self) -> (&dyn Any, &Cell<u32>) {
        let cell = unsafe { &*self.handle.entry_ptr() };
        let OpaqueEntry::Host(host) = &cell.e else {
            unreachable!("Opaque<T> is always a Host entry");
        };
        (&*host.payload, &cell.borrows)
    }

    /// The exclusive variant: reaches the payload mutably through the
    /// entry pointer. Sound under the guard `with_mut` holds — no other
    /// borrow of the entry is live, and rut-side ops never touch host
    /// payloads — the same argument the previous raw-payload design
    /// made; the view's own handle pins the rc for the borrow's scope.
    fn boxed_mut(&self) -> (&mut dyn Any, &Cell<u32>) {
        let cell = unsafe { &mut *self.handle.entry_ptr() };
        let OpaqueEntry::Host(host) = &mut cell.e else {
            unreachable!("Opaque<T> is always a Host entry");
        };
        (&mut *host.payload, &cell.borrows)
    }
}

impl<T: 'static> Clone for Opaque<T> {
    fn clone(&self) -> Self {
        Opaque { handle: self.handle.clone(), _marker: PhantomData }
    }
}

impl<T: HostPayload> Opaque<T> {
    /// News a box whose payload opted into the release hook: `finalize`
    /// runs at entry death — rc-0 inside the release walk — before the
    /// payload's own `Drop` (RFC 0016 §3). One per map/logger, never
    /// per-op.
    pub fn alloc_hosted(vm: &mut crate::interp::Vm, val: T) -> Result<Opaque<T>, Trap> {
        fn finalize_thunk<P: HostPayload>(p: &mut dyn Any, heap: &Heap) {
            p.downcast_mut::<P>()
                .expect("finalize thunk runs on its own payload")
                .finalize(heap);
        }
        let slot = vm.heap.alloc_host_box(val, Some(finalize_thunk::<T> as fn(&mut dyn Any, &Heap)))?;
        let handle = vm.heap.opaque_handle_take(unsafe { slot.r });
        Ok(Opaque { handle, _marker: PhantomData })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    /// A bare Vm: the store laws here ride the boot types table only.
    fn bare_vm() -> crate::interp::Vm {
        let mut prog = rut_core::binary::Program::default();
        prog.types = rut_core::types::TypeTable::boot();
        crate::interp::Vm::new(
            Rc::new(prog),
            &crate::interp::Limits::default(),
            crate::interp::HostHooks::default(),
            crate::interp::HostRegistry::new(),
        )
        .expect("bare vm")
    }

    struct Counted {
        fin: Rc<Cell<bool>>,
        dropped: Rc<Cell<bool>>,
    }
    impl HostPayload for Counted {
        fn finalize(&mut self, _heap: &Heap) {
            self.fin.set(true);
        }
    }
    impl Drop for Counted {
        fn drop(&mut self) {
            // the ORDER law (RFC 0016 §3, §0.8 i): finalize ran first
            assert!(self.fin.get(), "finalize must run before the Box's own Drop");
            self.dropped.set(true);
        }
    }

    #[test]
    fn finalize_runs_before_the_box_drop_at_rc0() {
        let mut vm = bare_vm();
        let fin = Rc::new(Cell::new(false));
        let dropped = Rc::new(Cell::new(false));
        let b = Opaque::alloc_hosted(&mut vm, Counted { fin: fin.clone(), dropped: dropped.clone() })
            .expect("alloc_hosted");
        assert!(!fin.get() && !dropped.get(), "nothing runs while the box lives");
        drop(b);
        assert!(fin.get(), "the finalize hook ran at entry death");
        assert!(dropped.get(), "the payload's Drop ran with it");
        assert_eq!(vm.heap.arena_store().live(), 0, "the entry is gone");
    }

    #[test]
    fn plain_alloc_still_drops_deterministically_without_a_hook() {
        let mut vm = bare_vm();
        let dropped = Rc::new(Cell::new(false));
        struct NoHook(Rc<Cell<bool>>);
        impl Drop for NoHook {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }
        let d = dropped.clone();
        let b = Opaque::alloc(&mut vm, NoHook(d)).expect("alloc");
        drop(b);
        assert!(dropped.get(), "plain alloc: the Box's own Drop is the destructor");
        assert_eq!(vm.heap.arena_store().live(), 0);
    }

    #[test]
    fn the_view_holds_an_rc_and_clones_share_the_entry() {
        let mut vm = bare_vm();
        let b = Opaque::alloc(&mut vm, vec![1u8, 2, 3]).expect("alloc");
        let c = b.clone();
        assert_eq!(b.handle(), c.handle(), "identity: the same entry");
        drop(b);
        assert_eq!(vm.heap.arena_store().live(), 1, "c still pins the entry");
        drop(c);
        assert_eq!(vm.heap.arena_store().live(), 0, "rc-0 frees the entry");
    }

    #[test]
    fn from_handle_laws_naming_both_sides_and_the_rut_recovery() {
        let mut vm = bare_vm();
        let host = Opaque::alloc(&mut vm, 7i64).expect("alloc");
        // a wrong T: the checked error naming what it holds and what was asked
        let err = Opaque::<u64>::from_handle(host.handle()).err().expect("wrong T rejected");
        assert!(err.msg.contains("i64") && err.msg.contains("u64"), "{}", err.msg);
        // a rut-value entry answering a Host decode: the RFC 0014 recovery text
        let s = vm.heap.alloc_str("k".into()).unwrap();
        let rut = vm.heap.alloc_opaque(s, rut_core::types::TY_STR).unwrap();
        let h = vm.heap.opaque_handle_take(unsafe { rut.r });
        let err = Opaque::<u64>::from_handle(&h).err().expect("rut box rejected");
        assert!(
            err.msg.contains("rut value") && err.msg.contains("downcast"),
            "{}",
            err.msg
        );
    }

    #[test]
    fn the_borrow_guard_lives_on_the_entry() {
        let mut vm = bare_vm();
        let b = Opaque::alloc(&mut vm, 41i64).expect("alloc");
        b.with_mut(&mut vm, |vm, v| {
            *v += 1;
            // nested shared borrow: excluded while &mut is out
            assert!(b.with(|_| {}).is_err(), "shared under mut must trap");
            let again = b.with_mut(vm, |_, _| {});
            assert!(again.unwrap_err().msg.contains("borrowed by an outer host call"));
        })
        .expect("the exclusive borrow runs");
        assert_eq!(b.with(|v| *v).unwrap(), 42, "the mutation stands");
        b.with(|_| {}).expect("the guard clears when the closure returns");
    }

    #[test]
    fn rut_entry_death_releases_the_held_cell() {
        let mut vm = bare_vm();
        let base = vm.heap_usage();
        // mint a str cell + a rut box over it — the entry holds the cell
        let s = vm.heap.alloc_str("hello".into()).unwrap();
        let boxed = vm.heap.alloc_opaque(s, rut_core::types::TY_STR).unwrap();
        let h = vm.heap.opaque_handle_take(unsafe { boxed.r });
        let minted = vm.heap_usage() - base;
        assert_eq!(vm.heap.arena_store().live(), 1);
        drop(h);
        assert_eq!(vm.heap.arena_store().live(), 0, "the entry died");
        assert_eq!(
            vm.heap_usage(),
            base,
            "the held str cell released with the entry (the record-field pattern)"
        );
        assert!(minted > 0);
    }
}
