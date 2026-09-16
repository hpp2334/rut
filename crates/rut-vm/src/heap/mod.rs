//! The RC heap — RFC 0016: everything except primitives is a heap cell;
//! strong count 0 ⇒ immediate destruction; no collector. v1 implementation:
//! cells are `Rc<CellVal>` with *manual* retain/release at the Slot level
//! (the compiler emits ref-aware ops — §5), riding Rust's allocator with
//! byte accounting and pre-alloc budget checks (RFC 0039's self-managed
//! arena is a later milestone; the observable contract — deterministic
//! destruction, identity, `Trap::OutOfMemory` before any write — holds).

use crate::arena::{release_ref_slot, Arena, ReleasePlan};
use rut_core::types::{PrimTy, TypeId, TypeTable, TyKind};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

pub(crate) mod blocks;
mod cell;
mod hostbox;
mod trap;
mod value;

pub use crate::arena::OpaqueRef;
pub(crate) use blocks::Blocks;

pub use cell::{cell, cell_of, ArrData, ArrKind, CellData, CellVal, Slots, StrVal};
pub use hostbox::OpaqueBox;
pub use trap::{Trap, TrapKind};
pub use value::{Slot, Value};

// ---- accounting (RFC 0040 §1: check BEFORE any write) ----

pub struct HeapAcct {
    pub used: Cell<u64>,
    /// high-water mark of `used` (RFC 0039 accounting) — never decreases,
    /// so a host can read the peak live-heap after a run
    pub peak: Cell<u64>,
    pub limit: Cell<Option<u64>>,
}

// ---- heap ----

pub struct Heap {
    acct: Rc<HeapAcct>,
    arena: Rc<Arena>,
    /// enum member singletons (RFC 0016 §1)
    singletons: RefCell<HashMap<(TypeId, u32), *const CellVal>>,
}

const CELL_OVERHEAD: u64 = 24; // header + Rc box approximation

impl Heap {
    pub(crate) fn new(limit: Option<u64>, plan: ReleasePlan) -> Heap {
        Heap {
            acct: Rc::new(HeapAcct { used: Cell::new(0), peak: Cell::new(0), limit: Cell::new(limit) }),
            arena: Rc::new(Arena::new(Rc::new(plan))),
            singletons: RefCell::new(HashMap::new()),
        }
    }

    pub fn used_bytes(&self) -> u64 {
        self.acct.used.get()
    }
    /// High-water mark of `used_bytes` since the heap was created — the
    /// VM-heap peak (the probe/bencher reads this after a run).
    pub fn peak_bytes(&self) -> u64 {
        self.acct.peak.get()
    }
    pub fn limit(&self) -> Option<u64> {
        self.acct.limit.get()
    }
    pub fn set_limit(&self, limit: Option<u64>) {
        self.acct.limit.set(limit);
    }

    /// Public growth accounting (RFC 0040 §1) — Vec growth charges the
    /// budget before the write; never refunded on shrink (v1 overcounts
    /// rather than undercounts).
    pub fn charge_public(&self, bytes: u64) -> Result<(), Trap> {
        self.charge(bytes)
    }

    fn charge(&self, bytes: u64) -> Result<(), Trap> {
        let a = &*self.acct;
        let next = a.used.get() + bytes;
        if let Some(l) = a.limit.get() {
            if next > l {
                return Err(Trap::new(
                    TrapKind::OutOfMemory,
                    format!("heap budget exceeded: {next} > {l} bytes"),
                ));
            }
        }
        a.used.set(next);
        if next > a.peak.get() {
            a.peak.set(next);
        }
        Ok(())
    }

    fn mint(&self, ty: TypeId, data: CellData, payload_bytes: u64) -> Result<Slot, Trap> {
        let bytes = CELL_OVERHEAD + payload_bytes;
        self.charge(bytes)?;
        let cell = CellVal {
            ty,
            data,
            refs: Cell::new(1),
            bytes: bytes.min(u32::MAX as u64) as u32,
        };
        let p = self.arena.alloc_slot();
        unsafe { p.write(cell) };
        Ok(Slot { r: p as *const CellVal })
    }

    /// A `str` cell from owned text — the boundary path (literals at load,
    /// `render`, `own`, host values). The octets move in; `String` is only
    /// the caller's spelling of "these bytes are valid UTF-8".
    pub fn alloc_str(&self, s: String) -> Result<Slot, Trap> {
        self.alloc_str_bytes(s.into_bytes())
    }

    /// A `str` cell from raw octets — the internal assembly path
    /// (`Concat`, `StrJoin`). The bytes must be valid UTF-8; every caller
    /// builds them out of other valid UTF-8 cells, which is what keeps the
    /// invariant.
    pub fn alloc_str_bytes(&self, bytes: Vec<u8>) -> Result<Slot, Trap> {
        let n = bytes.len() as u64;
        let ascii = bytes.is_ascii();
        let block = self.arena.blocks.alloc(bytes.len());
        let cap = self.arena.blocks.cap_of(block) as u32;
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), block, bytes.len()) };
        let mut d = ArrData::new(ArrKind::U8, block, cap);
        d.len = bytes.len() as u32;
        self.mint(
            rut_core::types::TY_STR,
            CellData::Str(StrVal { block, len: bytes.len() as u32, cap, ascii }),
            n,
        )
    }

    /// A one-char `str` cell — the `{c}` f-string hole. UTF-8 encodes
    /// straight into the block; no intermediate `String` ever exists.
    pub fn alloc_char(&self, c: char) -> Result<Slot, Trap> {
        let mut buf = [0u8; 4];
        let s = c.encode_utf8(&mut buf);
        let n = s.len() as u64;
        let block = self.arena.blocks.alloc(s.len());
        let cap = self.arena.blocks.cap_of(block) as u32;
        unsafe { std::ptr::copy_nonoverlapping(s.as_ptr(), block, s.len()) };
        self.mint(
            rut_core::types::TY_STR,
            CellData::Str(StrVal { block, len: s.len() as u32, cap, ascii: c.is_ascii() }),
            n,
        )
    }

    /// A str slice view (RFC 0042): `len` octets of `parent`'s block at
    /// `off`, retained. O(1) — no octets move. The view cell itself is
    /// tiny and accounted (`CELL_OVERHEAD` + fields); the parent's bytes
    /// stay alive as long as any view does.
    pub fn alloc_str_view(&self, parent: Slot, off: u32, len: u32, ascii: bool) -> Result<Slot, Trap> {
        self.retain(parent);
        let r = self.mint(
            rut_core::types::TY_STR,
            CellData::StrView { parent, off, len, ascii },
            CELL_OVERHEAD + 8,
        );
        match r {
            Ok(s) => Ok(s),
            Err(e) => {
                // the charge failed — give the retain back
                self.release(parent);
                Err(e)
            }
        }
    }

    /// Append `extra` to a `Str` cell **in place**. The caller must
    /// guarantee the cell is uniquely owned (`rc == 1`): no other slot
    /// aliases it, so mutating behind the shared handle is sound. Growth
    /// is geometric and class-rounded inside the block store — appending
    /// one byte at a time touches the allocator O(log n) times, which is
    /// what keeps an accumulator loop like `bytes_decode`'s `out = out + c`
    /// linear instead of copying the whole prefix every step. The charge
    /// is the capacity the cell actually holds; a failed charge leaves the
    /// text untouched (only spare capacity was reserved).
    /// Reserve room for `extra` more octets in a `Str` cell — one growth
    /// decision covering a whole multi-part append (the concat fast path).
    #[inline]
    pub fn reserve_append(&self, s: Slot, extra: usize) -> Result<(), Trap> {
        let p = unsafe { s.r } as *mut CellVal;
        if !matches!(unsafe { &(*p).data }, CellData::Str(_)) {
            return Err(Trap::new(TrapKind::Invalid, "reserve_append on non-str"));
        }
        unsafe {
            let cell = &mut *p;
            let CellData::Str(v) = &mut cell.data else { unreachable!() };
            let need = v.len as usize + extra;
            if need > v.cap as usize {
                v.block = self.arena.blocks.grow(v.block, v.len as usize, need);
                v.cap = self.arena.blocks.cap_of(v.block) as u32;
                let new_bytes = (CELL_OVERHEAD + 8 + v.cap as u64).min(u32::MAX as u64) as u32;
                self.charge((new_bytes as u64).saturating_sub(cell.bytes as u64))?;
                cell.bytes = new_bytes;
            }
        }
        Ok(())
    }

    #[inline]
    pub fn append_bytes(&self, s: Slot, extra: &[u8]) -> Result<(), Trap> {
        let p = unsafe { s.r } as *mut CellVal;
        if !matches!(unsafe { &(*p).data }, CellData::Str(_)) {
            return Err(Trap::new(TrapKind::Invalid, "append_bytes on non-str"));
        }
        unsafe {
            let cell = &mut *p;
            let CellData::Str(v) = &mut cell.data else { unreachable!() };
            let need = v.len as usize + extra.len();
            if need > v.cap as usize {
                v.block = self.arena.blocks.grow(v.block, v.len as usize, need);
                v.cap = self.arena.blocks.cap_of(v.block) as u32;
                let new_bytes = (CELL_OVERHEAD + 8 + v.cap as u64).min(u32::MAX as u64) as u32;
                self.charge((new_bytes as u64).saturating_sub(cell.bytes as u64))?;
                cell.bytes = new_bytes;
            }
            v.append(extra);
        }
        Ok(())
    }

    /// Immutable binary buffer (RFC 0004) — a `u8` array (the `bytes` type
    /// is an array of octets at the engine level).
    pub fn alloc_bytes(&self, b: Vec<u8>) -> Result<Slot, Trap> {
        let n = b.len() as u64;
        let block = self.arena.blocks.alloc(b.len());
        let cap = (self.arena.blocks.cap_of(block)) as u32;
        unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), block, b.len()) };
        let mut d = ArrData::new(ArrKind::U8, block, cap);
        d.len = b.len() as u32;
        self.mint(
            0,
            CellData::Array { elem: rut_core::types::TY_U8, items: RefCell::new(d) },
            n,
        )
    }

    /// A `CellData::Array` with `n` elements pre-filled with `default` — the
    /// `Array<T>(n)` / `bytes_zeroed` path. Builds the packed store at its
    /// final length in one allocation (no `Vec<Slot>` temporary).
    pub fn alloc_array_filled(&self, elem: TypeId, n: usize, default: Slot, table: &TypeTable) -> Result<Slot, Trap> {
        let kind = ArrKind::of(elem, table);
        let w = kind.width();
        let block = self.arena.blocks.alloc(w * n);
        let cap = (self.arena.blocks.cap_of(block) / w) as u32;
        let mut d = ArrData::new(kind, block, cap);
        // the block arrives zeroed (page-carved or boxed zeroed), which is
        // the zero-fill; writing `default` covers non-zero defaults
        for _ in 0..n {
            d.push(default, &self.arena.blocks);
        }
        let bytes = (n as u64) * w as u64;
        self.mint(0, CellData::Array { elem, items: RefCell::new(d) }, bytes)
    }

    pub fn alloc_array(&self, elem: TypeId, items: Vec<Slot>, table: &TypeTable) -> Result<Slot, Trap> {
        let kind = ArrKind::of(elem, table);
        let w = kind.width();
        let n = items.len();
        let block = self.arena.blocks.alloc(w * n);
        let cap = (self.arena.blocks.cap_of(block) / w) as u32;
        let mut d = ArrData::new(kind, block, cap);
        for s in items {
            d.push(s, &self.arena.blocks);
        }
        let bytes = (n as u64) * w as u64;
        self.mint(0, CellData::Array { elem, items: RefCell::new(d) }, bytes)
    }

    pub fn alloc_record(&self, ty: TypeId, fields: Vec<Slot>) -> Result<Slot, Trap> {
        let n = fields.len() as u64;
        self.mint(ty, CellData::Record { fields: RefCell::new(Slots::from_vec(fields)) }, n * 8)
    }

    /// Record with `n` zeroed fields — the `Self { .. }` / struct literal
    /// path (`Op::NewCell`), keeping small payloads inline.
    pub fn alloc_record_zeroed(&self, ty: TypeId, n: usize) -> Result<Slot, Trap> {
        self.mint(ty, CellData::Record { fields: RefCell::new(Slots::zeroed(n)) }, n as u64 * 8)
    }

    /// Deep-copy a VALUE (v1.1 copy-by-value, RFC 0009/0016): records and
    /// arrays clone into fresh cells — nested value fields clone
    /// recursively, `str`/`bytes`/`*T`/closure children share the cell.
    /// The result is a fresh rc-1 cell owned by the caller.
    pub fn clone_val(&self, s: Slot, ty: TypeId, table: &TypeTable) -> Result<Slot, Trap> {
        if unsafe { s.r.is_null() } {
            return Err(Trap::new(TrapKind::NilDeref, "copy of nil"));
        }
        match table.kind(ty).clone() {
            TyKind::Array { elem } => {
                let cell = cell_of(s);
                if let CellData::Array { items, .. } = &cell.data {
                    let src = items.borrow();
                    let mut out = Vec::with_capacity(src.len());
                    for i in 0..src.len() {
                        if let Some(it) = src.get(i) {
                            out.push(self.deep_child(it, elem, table)?);
                        }
                    }
                    drop(src);
                    self.alloc_array(elem, out, table)
                } else {
                    Err(Trap::new(TrapKind::Invalid, "clone: not an array"))
                }
            }
            TyKind::Data { fields } => {
                let cell = cell_of(s);
                if let CellData::Record { fields: src } = &cell.data {
                    let srcb = src.borrow();
                    let mut out = Vec::with_capacity(srcb.len());
                    for (i, f) in fields.iter().enumerate() {
                        if let Some(it) = srcb.get(i) {
                            out.push(self.deep_child(it, f.ty, table)?);
                        }
                    }
                    drop(srcb);
                    self.alloc_record(ty, out)
                } else {
                    Err(Trap::new(TrapKind::Invalid, "clone: not a record"))
                }
            }
            _ => Ok(s),
        }
    }

    /// One cloned child: value children clone recursively, ref-repr
    /// children share the cell (retained for the new parent), primitives
    /// are plain copies.
    fn deep_child(&self, s: Slot, ty: TypeId, table: &TypeTable) -> Result<Slot, Trap> {
        if unsafe { s.r.is_null() } {
            return Ok(s); // capacity padding / nil — copies as nil
        }
        if table.is_value(ty) {
            self.clone_val(s, ty, table)
        } else if table.repr_of(ty).is_ref() {
            self.retain(s);
            Ok(s)
        } else {
            Ok(s)
        }
    }


    pub fn alloc_opaque(&self, val: Slot, val_ty: TypeId) -> Result<Slot, Trap> {
        self.mint(rut_core::types::TY_OPAQUE, CellData::OpaqueBox { val, val_ty }, 8)
    }

    /// A host payload box (RFC 0023/0026): `val` — any `'static` Rust
    /// value — moves into a `Box<dyn Any>` inside the arena behind an
    /// `Opaque` surface; the box's own vtable drops it deterministically
    /// at rc-0 (RFC 0016 §3). The cell accounts the payload's shallow
    /// `size_of::<T>()` (RFC 0040); interior allocations a `T` makes are
    /// the host's own business.
    pub fn alloc_host_box<T: 'static>(&self, val: T) -> Result<Slot, Trap> {
        let payload = Box::new(val) as Box<dyn std::any::Any>;
        let n = (std::mem::size_of::<T>() as u64).max(8);
        self.mint(
            rut_core::types::TY_OPAQUE,
            CellData::HostBoxed { payload, type_name: std::any::type_name::<T>(), borrows: Cell::new(0) },
            n,
        )
    }

    pub fn alloc_closure(&self, func: u32, captures: Vec<Slot>) -> Result<Slot, Trap> {
        let n = captures.len() as u64;
        self.mint(0, CellData::Closure { func, captures }, n * 8)
    }

    /// Enum member — the immortal singleton cell (RFC 0016 §1): the one
    /// place identity quietly behaves as value (RFC 0012 §4).
    pub fn enum_member(&self, ty: TypeId, member: u32) -> Result<Slot, Trap> {
        if let Some(p) = self.singletons.borrow().get(&(ty, member)) {
            return Ok(Slot { r: *p });
        }
        // account once, never on drop — immortal
        self.charge(CELL_OVERHEAD + 8)?;
        let cell = CellVal {
            ty,
            data: CellData::Enum { member },
            refs: Cell::new(u32::MAX),
            bytes: 0,
        };
        let p = self.arena.alloc_slot();
        unsafe { p.write(cell) };
        self.singletons.borrow_mut().insert((ty, member), p as *const CellVal);
        Ok(Slot { r: p as *const CellVal })
    }

    // ---- ref discipline (RFC 0016 §5) ----

    /// rc += 1 (immortal singletons saturate at `u32::MAX`).
    pub fn retain(&self, s: Slot) {
        let p = unsafe { s.r };
        if p.is_null() {
            return;
        }
        unsafe {
            let c = &*p;
            c.refs.set(c.refs.get().saturating_add(1));
        }
    }

    /// Attach a drop callback to a cell (RFC 0016 §3). Traps on `nil` or a
    /// second attach. The cleanup slot is retained for the map's lifetime.
    pub fn set_drop_fn(&self, obj: Slot, cleanup: Slot) -> Result<(), Trap> {
        let p = unsafe { obj.r };
        if p.is_null() {
            return Err(Trap::new(TrapKind::NilDeref, "on_drop on nil"));
        }
        if unsafe { cleanup.r.is_null() } {
            return Err(Trap::new(TrapKind::Invalid, "on_drop: nil cleanup"));
        }
        self.retain(cleanup);
        if !self.arena.set_drop_fn(p, cleanup) {
            self.release(cleanup);
            return Err(Trap::new(
                TrapKind::Invalid,
                "on_drop already attached to this pointer (RFC 0016 §3)",
            ));
        }
        Ok(())
    }

    /// Pop one queued drop callback: `(pinned cell, cleanup)`. The caller
    /// runs `cleanup(cell)` and then releases both.
    pub fn take_pending_drop(&self) -> Option<(Slot, Slot)> {
        self.arena.take_pending_drop().map(|(p, cleanup)| (Slot { r: p }, cleanup))
    }

    /// rc -= 1; at zero the cell is dropped, its slot recycled, and its
    /// ref-typed children collected and released recursively (RFC 0016 §3).
    pub fn release(&self, s: Slot) {
        release_ref_slot(&self.arena, &self.acct, s);
    }

    /// Build an owning host handle for an `Opaque` cell (retains once).
    pub fn opaque_handle(&self, p: *const CellVal) -> OpaqueRef {
        OpaqueRef::new(&self.arena, &self.acct, p)
    }

    /// The owning variant for a cell minted this instant (RFC 0023/0026):
    /// the handle takes over the mint reference instead of adding one, so
    /// `mint -> handle -> Value` accounts exactly one reference. The
    /// embedder equivalent of `OpaqueBox::alloc` — for host fns that
    /// build a box from rut-shaped parts (`alloc_str`/`alloc_opaque`)
    /// and hand it straight back.
    pub fn opaque_handle_take(&self, p: *const CellVal) -> OpaqueRef {
        OpaqueRef::owning(&self.arena, &self.acct, p)
    }

    /// The `(payload, payload type)` inside an `Opaque` handle (RFC 0014).
    pub fn opaque_inner(&self, h: &OpaqueRef) -> Option<(Slot, TypeId)> {
        cell_of(Slot { r: h.ptr() }).as_opaque()
    }

    /// Deep release of a slot by static type — used when dropping frames.
    pub fn release_typed(&self, s: Slot, ty: TypeId, table: &TypeTable) {
        if table.is_ref(ty) {
            self.release(s);
        }
    }

    /// Clone a value deeply (`own(x)`, RFC 0011 §1): shallow for handles —
    /// primitive fields copied, handle fields shared.
    pub fn own(&self, s: Slot, ty: TypeId, table: &TypeTable) -> Result<Slot, Trap> {
        match table.kind(ty).clone() {
            TyKind::Prim(_) | TyKind::Unit | TyKind::Fn { .. } | TyKind::Ptr { .. } => Ok(s),
            TyKind::Str => {
                let cell = cell_of(s);
                self.alloc_str_bytes(cell.as_bytes().to_vec())
            }
            TyKind::Bytes => {
                let cell = cell_of(s);
                self.alloc_bytes(cell.bytes_copy())
            }
            TyKind::Array { elem } => {
                let cell = cell_of(s);
                if let CellData::Array { items, .. } = &cell.data {
                    let src = items.borrow();
                    let mut out = Vec::with_capacity(src.len());
                    for i in 0..src.len() {
                        if let Some(it) = src.get(i) {
                            out.push(self.clone_slot(it, elem, table)?);
                        }
                    }
                    drop(src);
                    self.alloc_array(elem, out, table)
                } else {
                    Err(Trap::new(TrapKind::Invalid, "own: not an array"))
                }
            }
            TyKind::Data { fields } => {
                let cell = cell_of(s);
                if let CellData::Record { fields: src } = &cell.data {
                    let srcb = src.borrow();
                    let mut out = Vec::with_capacity(srcb.len());
                    for (i, f) in fields.iter().enumerate() {
                        if let Some(it) = srcb.get(i) {
                            out.push(self.clone_slot(it, f.ty, table)?);
                        }
                    }
                    drop(srcb);
                    self.alloc_record(ty, out)
                } else {
                    Err(Trap::new(TrapKind::Invalid, "own: not a record"))
                }
            }
            TyKind::Opaque => {
                // box once, share the inner handle (RFC 0014). A host
                // payload box has no inner rut value to re-box (RFC 0023):
                // own is a plain reference share — the box's identity and
                // its payload Drop timing are unchanged.
                let cell = cell_of(s);
                if matches!(cell.data, CellData::HostBoxed { .. }) {
                    self.retain(s);
                    return Ok(s);
                }
                let Some((val, val_ty)) = cell.as_opaque() else {
                    return Err(Trap::new(TrapKind::Invalid, "own: not a box"));
                };
                let inner = self.clone_slot(val, val_ty, table)?;
                self.alloc_opaque(inner, val_ty)
            }
            TyKind::Enum { .. } | TyKind::TraitObj { .. } => {
                // singletons & trait refs alias one cell — own() must mint a
                // new identity; for enums that would break singleton `==`,
                // so enums share (values, RFC 0006); trait objects have no
                // standalone own semantics in v1 beyond the cell handle
                Ok(s)
            }
        }
    }

    fn clone_slot(&self, s: Slot, ty: TypeId, table: &TypeTable) -> Result<Slot, Trap> {
        if table.is_ref(ty) {
            self.retain(s);
        }
        Ok(s)
    }
}
