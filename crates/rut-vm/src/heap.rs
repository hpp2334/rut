//! The RC heap — RFC 0016: everything except primitives is a heap cell;
//! strong count 0 ⇒ immediate destruction; no collector. v1 implementation:
//! cells are `Rc<CellVal>` with *manual* retain/release at the Slot level
//! (the compiler emits ref-aware ops — §5), riding Rust's allocator with
//! byte accounting and pre-alloc budget checks (RFC 0039's self-managed
//! arena is a later milestone; the observable contract — deterministic
//! destruction, identity, `Trap::OutOfMemory` before any write — holds).

use rut_core::types::{TypeId, TypeTable, TyKind};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::mem;
use std::rc::Rc;

// ---- traps (RFC 0034 §2) ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrapKind {
    OutOfFuel,
    OutOfMemory,
    Interrupted,
    Overflow,
    DivByZero,
    UnwrapNone,
    IndexOutOfBounds,
    Assert,
    Panic,
    BadUnbox,
    Invalid,
}

#[derive(Clone, Debug)]
pub struct Trap {
    pub kind: TrapKind,
    pub msg: String,
}

impl Trap {
    pub fn new(kind: TrapKind, msg: impl Into<String>) -> Trap {
        Trap { kind, msg: msg.into() }
    }
    pub fn name(&self) -> String {
        match self.kind {
            TrapKind::OutOfFuel => "OutOfFuel".into(),
            TrapKind::OutOfMemory => "OutOfMemory".into(),
            TrapKind::Interrupted => "Interrupted".into(),
            TrapKind::Overflow => "Overflow".into(),
            TrapKind::DivByZero => "DivByZero".into(),
            TrapKind::UnwrapNone => "UnwrapNone".into(),
            TrapKind::IndexOutOfBounds => "IndexOutOfBounds".into(),
            TrapKind::Assert => "Assert".into(),
            TrapKind::Panic => "Panic".into(),
            TrapKind::BadUnbox => "BadUnbox".into(),
            TrapKind::Invalid => "Invalid".into(),
        }
    }
}

// ---- the tagged Value exists only at the host boundary (RFC 0023) ----

#[derive(Clone, Debug)]
pub enum Value {
    Unit,
    I64(i64),
    F64(f64),
    Bool(bool),
    Char(char),
    Str(String),
}

// ---- slots (RFC 0015 §5): untagged 8 bytes; bytecode is typed ----

#[derive(Clone, Copy)]
pub union Slot {
    pub i: i64,
    pub f: f64,
    pub b: bool,
    pub c: char,
    pub r: Option<*const CellVal>,
}

impl Slot {
    pub fn int(v: i64) -> Slot {
        Slot { i: v }
    }
    pub fn float(v: f64) -> Slot {
        Slot { f: v }
    }
    /// NOTE: every constructor writes the FULL 8 bytes — unions leave
    /// stale bytes otherwise, and ops must be able to read `.i` from any
    /// slot (RFC 0015 §5 untagged discipline).
    pub fn bool(v: bool) -> Slot {
        Slot { i: v as i64 }
    }
    pub fn ch(v: char) -> Slot {
        Slot { i: v as u32 as i64 }
    }
    pub fn as_bool(&self) -> bool {
        unsafe { self.i != 0 }
    }
    pub fn as_char(&self) -> char {
        char::from_u32(unsafe { self.i } as u32).unwrap_or('\0')
    }
    pub fn null() -> Slot {
        Slot { r: None }
    }
    /// SAFETY: caller guarantees the slot is a ref slot (verifier-checked).
    pub unsafe fn get_ref(&self) -> Option<*const CellVal> {
        unsafe { self.r }
    }
    pub fn same_ref(a: Slot, b: Slot) -> bool {
        unsafe { a.r == b.r } // cell identity (RFC 0012 §4)
    }
}

impl std::fmt::Debug for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "slot({:#x})", unsafe { self.i as u64 })
    }
}

// ---- cells ----

pub struct CellVal {
    pub ty: TypeId,
    pub data: CellData,
    acct: Option<Rc<HeapAcct>>,
    bytes: u32,
}

pub enum CellData {
    Str(String),
    Vec { elem: TypeId, items: RefCell<Vec<Slot>> },
    Array { elem: TypeId, items: RefCell<Vec<Slot>> },
    /// enum member — immortal singleton per (ty, member)
    Enum { member: u32 },
    /// Option/Result: tag 0 = some/ok, 1 = none/err
    Sum { tag: u32, payload: Option<Slot> },
    /// dataclass/class instance — the repr-C payload as slots
    Record { fields: RefCell<Vec<Slot>> },
    /// Opaque box (RFC 0014): the value + its runtime type
    OpaqueBox { val: Slot, val_ty: TypeId },
    /// closure value (RFC 0013) — v1 captures by value
    Closure {
        func: u32,
        params: Vec<TypeId>,
        ret: TypeId,
        captures: Vec<Slot>,
        cap_tys: Vec<TypeId>,
    },
}

impl Drop for CellVal {
    fn drop(&mut self) {
        if let Some(a) = &self.acct {
            a.used.set(a.used.get().saturating_sub(self.bytes as u64));
        }
    }
}

impl CellVal {
    pub fn as_str(&self) -> &str {
        match &self.data {
            CellData::Str(s) => s,
            _ => "",
        }
    }
    /// Option/Result payload: (tag 0=some/ok 1=none/err, payload)
    pub fn as_sum(&self) -> Option<(u32, Option<Slot>)> {
        match &self.data {
            CellData::Sum { tag, payload } => Some((*tag, *payload)),
            _ => None,
        }
    }
    pub fn as_enum_member(&self) -> Option<u32> {
        match &self.data {
            CellData::Enum { member } => Some(*member),
            _ => None,
        }
    }
    pub fn as_opaque(&self) -> Option<(Slot, TypeId)> {
        match &self.data {
            CellData::OpaqueBox { val, val_ty } => Some((*val, *val_ty)),
            _ => None,
        }
    }
    pub fn as_closure(&self) -> Option<(u32, Vec<Slot>)> {
        match &self.data {
            CellData::Closure { func, captures, .. } => Some((*func, captures.clone())),
            _ => None,
        }
    }
    pub fn seq_len(&self) -> Option<usize> {
        match &self.data {
            CellData::Vec { items, .. } => Some(items.borrow().len()),
            CellData::Array { items, .. } => Some(items.borrow().len()),
            _ => None,
        }
    }
    pub fn seq_items_copy(&self) -> Option<Vec<Slot>> {
        match &self.data {
            CellData::Vec { items, .. } => Some(items.borrow().clone()),
            CellData::Array { items, .. } => Some(items.borrow().clone()),
            _ => None,
        }
    }
    pub fn elem_ty(&self) -> Option<TypeId> {
        match &self.data {
            CellData::Vec { elem, .. } | CellData::Array { elem, .. } => Some(*elem),
            _ => None,
        }
    }
    pub fn record_get(&self, field: u32) -> Option<Slot> {
        match &self.data {
            CellData::Record { fields } => fields.borrow().get(field as usize).copied(),
            _ => None,
        }
    }
    pub fn record_set(&self, field: u32, v: Slot) -> Option<Slot> {
        match &self.data {
            CellData::Record { fields } => {
                let mut fb = fields.borrow_mut();
                let old = *fb.get(field as usize)?;
                fb[field as usize] = v;
                Some(old)
            }
            _ => None,
        }
    }
}

// ---- accounting (RFC 0040 §1: check BEFORE any write) ----

pub struct HeapAcct {
    pub used: Cell<u64>,
    pub limit: Cell<Option<u64>>,
}

// ---- heap ----

pub struct Heap {
    acct: Rc<HeapAcct>,
    /// enum member singletons (RFC 0016 §1)
    singletons: RefCell<HashMap<(TypeId, u32), *const CellVal>>,
}

const CELL_OVERHEAD: u64 = 24; // header + Rc box approximation

impl Heap {
    pub fn new(limit: Option<u64>) -> Heap {
        Heap {
            acct: Rc::new(HeapAcct { used: Cell::new(0), limit: Cell::new(limit) }),
            singletons: RefCell::new(HashMap::new()),
        }
    }

    pub fn used_bytes(&self) -> u64 {
        self.acct.used.get()
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
        Ok(())
    }

    fn mint(&self, ty: TypeId, data: CellData, payload_bytes: u64) -> Result<Slot, Trap> {
        let bytes = CELL_OVERHEAD + payload_bytes;
        self.charge(bytes)?;
        let cell = CellVal {
            ty,
            data,
            acct: Some(self.acct.clone()),
            bytes: bytes.min(u32::MAX as u64) as u32,
        };
        Ok(Slot { r: Some(Rc::into_raw(Rc::new(cell))) })
    }

    pub fn alloc_str(&self, s: String) -> Result<Slot, Trap> {
        let n = s.len() as u64;
        self.mint(rut_core::types::TY_STR, CellData::Str(s), n)
    }

    pub fn alloc_vec(&self, elem: TypeId, cap: usize) -> Result<Slot, Trap> {
        self.mint(
            0, // Vec cells carry the elem kind in CellData; the *slot's*
               // static type knows the instantiation
            CellData::Vec { elem, items: RefCell::new(Vec::with_capacity(cap)) },
            (cap as u64) * 8,
        )
    }

    pub fn alloc_array(&self, elem: TypeId, items: Vec<Slot>) -> Result<Slot, Trap> {
        let n = items.len() as u64;
        self.mint(0, CellData::Array { elem, items: RefCell::new(items) }, n * 8)
    }

    pub fn alloc_record(&self, ty: TypeId, fields: Vec<Slot>) -> Result<Slot, Trap> {
        let n = fields.len() as u64;
        self.mint(ty, CellData::Record { fields: RefCell::new(fields) }, n * 8)
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
            return Ok(Slot { r: Some(*p) });
        }
        // account once, never on drop — immortal
        self.charge(CELL_OVERHEAD + 8)?;
        let cell = CellVal {
            ty,
            data: CellData::Enum { member },
            acct: None,
            bytes: 0,
        };
        let raw = Rc::into_raw(Rc::new(cell));
        self.singletons.borrow_mut().insert((ty, member), raw);
        // keep it alive forever: leak one Rc reference
        mem::forget(unsafe { Rc::from_raw(raw) }.clone());
        Ok(Slot { r: Some(raw) })
    }

    // ---- ref discipline (RFC 0016 §5) ----

    /// rc += 1 (immortal singletons have no counter pressure: their Rc is
    /// pinned, so clone never frees)
    pub fn retain(&self, s: Slot) {
        let Some(p) = (unsafe { s.r }) else { return };
        unsafe {
            let rc = Rc::from_raw(p);
            let dup = rc.clone();
            mem::forget(rc);
            mem::forget(dup);
        }
    }

    /// rc -= 1; at zero the cell drops (fields release recursively —
    /// deterministic destructors, RFC 0016 §3)
    pub fn release(&self, s: Slot) {
        let Some(p) = (unsafe { s.r }) else { return };
        unsafe {
            drop(Rc::from_raw(p));
        }
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
                    for it in src.iter() {
                        out.push(self.clone_slot(*it, elem, table)?);
                    }
                    self.alloc_vec(elem, out.len()).map(|v| {
                        if let CellData::Vec { items, .. } = &cell_of(v).data {
                            *items.borrow_mut() = out;
                        }
                        v
                    })
                } else {
                    Err(Trap::new(TrapKind::Invalid, "own: not a vec"))
                }
            }
            TyKind::Array { elem, len } => {
                let cell = cell_of(s);
                if let CellData::Array { items, .. } = &cell.data {
                    let mut out = Vec::with_capacity(len as usize);
                    for it in items.borrow().iter() {
                        out.push(self.clone_slot(*it, elem, table)?);
                    }
                    self.alloc_array(elem, out)
                } else {
                    Err(Trap::new(TrapKind::Invalid, "own: not an array"))
                }
            }
            TyKind::Data { fields } => {
                let cell = cell_of(s);
                if let CellData::Record { fields: src } = &cell.data {
                    let mut out = Vec::with_capacity(fields.len());
                    for (it, f) in src.borrow().iter().zip(fields.iter()) {
                        out.push(self.clone_slot(*it, f.ty, table)?);
                    }
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

/// SAFETY: `s` must hold a live cell pointer.
pub unsafe fn cell<'a>(s: Slot) -> &'a CellVal {
    unsafe { &*s.r.unwrap_unchecked() }
}

pub fn cell_of(s: Slot) -> &'static CellVal {
    unsafe { &*(s.r.unwrap() as *const CellVal) }
}
