//! The RC heap — RFC 0016: everything except primitives is a heap cell;
//! strong count 0 ⇒ immediate destruction; no collector. v1 implementation:
//! cells are `Rc<CellVal>` with *manual* retain/release at the Slot level
//! (the compiler emits ref-aware ops — §5), riding Rust's allocator with
//! byte accounting and pre-alloc budget checks (RFC 0039's self-managed
//! arena is a later milestone; the observable contract — deterministic
//! destruction, identity, `Trap::OutOfMemory` before any write — holds).

use crate::arena::{release_cell, Arena};
use rut_core::types::{PrimTy, TypeId, TypeTable, TyKind};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

mod cell;
mod trap;
mod value;

pub use crate::arena::OpaqueRef;
pub use cell::{cell, cell_of, CellData, CellVal, Packed, Slots};
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
    pub fn new(limit: Option<u64>) -> Heap {
        Heap {
            acct: Rc::new(HeapAcct { used: Cell::new(0), peak: Cell::new(0), limit: Cell::new(limit) }),
            arena: Rc::new(Arena::new()),
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

    pub fn alloc_str(&self, s: String) -> Result<Slot, Trap> {
        let n = s.len() as u64;
        self.mint(rut_core::types::TY_STR, CellData::Str(s), n)
    }

    pub fn alloc_vec(&self, elem: TypeId, cap: usize, table: &TypeTable) -> Result<Slot, Trap> {
        let p = Packed::for_elem(elem, table, cap);
        let bytes = (cap as u64) * p.elem_width();
        self.mint(
            0, // Vec cells carry the elem kind in CellData; the *slot's*
               // static type knows the instantiation
            CellData::Vec { elem, items: RefCell::new(p) },
            bytes,
        )
    }

    pub fn alloc_array(&self, elem: TypeId, items: Vec<Slot>, table: &TypeTable) -> Result<Slot, Trap> {
        let p = Packed::from_slots(elem, table, items);
        let bytes = (p.len() as u64) * p.elem_width();
        self.mint(0, CellData::Array { elem, items: RefCell::new(p) }, bytes)
    }

    /// Vec from an already-packed payload (the `own` deep-clone path).
    fn alloc_vec_packed(&self, elem: TypeId, p: Packed) -> Result<Slot, Trap> {
        let bytes = (p.len() as u64) * p.elem_width();
        self.mint(0, CellData::Vec { elem, items: RefCell::new(p) }, bytes)
    }

    pub fn alloc_record(&self, ty: TypeId, fields: Vec<Slot>) -> Result<Slot, Trap> {
        let n = fields.len() as u64;
        self.mint(ty, CellData::Record { fields: RefCell::new(Slots::from_vec(fields)) }, n * 8)
    }

    /// Record with `n` zeroed fields — the `Self { .. }` / dataclass literal
    /// path (`Op::NewCell`), keeping small payloads inline.
    pub fn alloc_record_zeroed(&self, ty: TypeId, n: usize) -> Result<Slot, Trap> {
        self.mint(ty, CellData::Record { fields: RefCell::new(Slots::zeroed(n)) }, n as u64 * 8)
    }

    pub fn alloc_sum(&self, ty: TypeId, tag: u32, payload: Option<Slot>) -> Result<Slot, Trap> {
        self.mint(ty, CellData::Sum { tag, payload }, 8)
    }

    pub fn alloc_opaque(&self, val: Slot, val_ty: TypeId) -> Result<Slot, Trap> {
        self.mint(rut_core::types::TY_OPAQUE, CellData::OpaqueBox { val, val_ty }, 8)
    }

    pub fn alloc_closure(
        &self,
        func: u32,
        params: Vec<TypeId>,
        ret: TypeId,
        captures: Vec<Slot>,
        cap_tys: Vec<TypeId>,
    ) -> Result<Slot, Trap> {
        let n = captures.len() as u64;
        self.mint(
            0,
            CellData::Closure { func, params, ret, captures, cap_tys },
            n * 8,
        )
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

    /// rc -= 1; at zero the cell is dropped and its slot recycled. Note the
    /// current ref discipline does not recursively release a cell's child
    /// slots on drop (matching the previous `Rc` behaviour).
    pub fn release(&self, s: Slot) {
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
                release_cell(&self.arena, &self.acct, p as *mut CellVal);
            } else {
                c.refs.set(n - 1);
            }
        }
    }

    /// Build an owning host handle for an `Opaque` cell (retains once).
    pub fn opaque_handle(&self, p: *const CellVal) -> OpaqueRef {
        OpaqueRef::new(&self.arena, &self.acct, p)
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
            TyKind::Prim(_) | TyKind::Unit | TyKind::Fn { .. } => Ok(s),
            TyKind::Str => {
                let cell = cell_of(s);
                self.alloc_str(cell.as_str().to_string())
            }
            TyKind::Vec { elem } => {
                let cell = cell_of(s);
                if let CellData::Vec { items, .. } = &cell.data {
                    let src = items.borrow();
                    let mut out = Vec::with_capacity(src.len());
                    for i in 0..src.len() {
                        if let Some(it) = src.get(i) {
                            out.push(self.clone_slot(it, elem, table)?);
                        }
                    }
                    drop(src);
                    self.alloc_vec_packed(elem, Packed::from_slots(elem, table, out))
                } else {
                    Err(Trap::new(TrapKind::Invalid, "own: not a vec"))
                }
            }
            TyKind::Array { elem, len } => {
                let cell = cell_of(s);
                if let CellData::Array { items, .. } = &cell.data {
                    let mut out = Vec::with_capacity(len as usize);
                    let src = items.borrow();
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
            TyKind::Option { elem } => {
                let Some((tag, payload)) = cell_of(s).as_sum() else {
                    return Err(Trap::new(TrapKind::Invalid, "own: not a sum"));
                };
                match (tag, payload) {
                    (1, _) => self.alloc_sum(ty, 1, None),
                    (0, Some(p)) => {
                        let v = self.clone_slot(p, elem, table)?;
                        self.alloc_sum(ty, 0, Some(v))
                    }
                    _ => Err(Trap::new(TrapKind::Invalid, "own: bad sum")),
                }
            }
            TyKind::Result { ok, err } => {
                let Some((tag, payload)) = cell_of(s).as_sum() else {
                    return Err(Trap::new(TrapKind::Invalid, "own: not a sum"));
                };
                match (tag, payload) {
                    (0, Some(p)) => {
                        let v = self.clone_slot(p, ok, table)?;
                        self.alloc_sum(ty, 0, Some(v))
                    }
                    (1, Some(p)) => {
                        let v = self.clone_slot(p, err, table)?;
                        self.alloc_sum(ty, 1, Some(v))
                    }
                    _ => Err(Trap::new(TrapKind::Invalid, "own: bad sum")),
                }
            }
            TyKind::Opaque => {
                // box once, share the inner handle (RFC 0014)
                let Some((val, val_ty)) = cell_of(s).as_opaque() else {
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
