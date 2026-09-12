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

pub use crate::arena::OpaqueRef;

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

#[derive(Clone)]
pub enum Value {
    Unit,
    I64(i64),
    F64(f64),
    Bool(bool),
    Char(char),
    Str(String),
    /// `Vec<u8>` buffer crossing (RFC 0023 §2)
    Bytes(Vec<u8>),
    /// `Option<T>` — payload converted when `Some`
    Opt(Option<Box<Value>>),
    /// `Result<T, E>` — payloads converted in both arms
    Res(Result<Box<Value>, Box<Value>>),
    /// an `Opaque` box (RFC 0014) — the one cell the host may hold and
    /// pass back; the handle owns one arena reference
    Opaque(OpaqueRef),
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Unit, Value::Unit) => true,
            (Value::I64(a), Value::I64(b)) => a == b,
            (Value::F64(a), Value::F64(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Char(a), Value::Char(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bytes(a), Value::Bytes(b)) => a == b,
            (Value::Opt(a), Value::Opt(b)) => a == b,
            (Value::Res(a), Value::Res(b)) => a == b,
            // boxes compare by identity — the payload's type is erased
            (Value::Opaque(a), Value::Opaque(b)) => a == b,
            _ => false,
        }
    }
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Unit => write!(f, "Unit"),
            Value::I64(v) => write!(f, "I64({v})"),
            Value::F64(v) => write!(f, "F64({v})"),
            Value::Bool(v) => write!(f, "Bool({v})"),
            Value::Char(v) => write!(f, "Char({v:?})"),
            Value::Str(v) => write!(f, "Str({v:?})"),
            Value::Bytes(v) => write!(f, "Bytes(len {})", v.len()),
            Value::Opt(None) => write!(f, "Opt(None)"),
            Value::Opt(Some(v)) => write!(f, "Opt(Some({v:?}))"),
            Value::Res(Ok(v)) => write!(f, "Res(Ok({v:?}))"),
            Value::Res(Err(v)) => write!(f, "Res(Err({v:?}))"),
            Value::Opaque(_) => write!(f, "Opaque(<cell>)"),
        }
    }
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
    /// intrusive strong count (RFC 0039 self-managed arena); `u32::MAX`
    /// marks an immortal enum singleton
    pub(crate) refs: Cell<u32>,
    /// accounted bytes, refunded on release
    pub(crate) bytes: u32,
}

/// Payload slots for a record/array cell: small payloads live inline in
/// the cell (RFC 0015 §4 "inline blocks"), larger ones spill to the heap.
/// This removes the per-record `Vec` allocation for the common small
/// dataclass.
const INLINE_SLOTS: usize = 4;

pub enum Slots {
    Inline { len: u8, data: [Slot; INLINE_SLOTS] },
    Heap(Vec<Slot>),
}

impl Slots {
    pub fn zeroed(n: usize) -> Slots {
        if n <= INLINE_SLOTS {
            Slots::Inline { len: n as u8, data: [Slot::null(); INLINE_SLOTS] }
        } else {
            Slots::Heap(vec![Slot::null(); n])
        }
    }
    pub fn from_vec(v: Vec<Slot>) -> Slots {
        if v.len() <= INLINE_SLOTS {
            let mut data = [Slot::null(); INLINE_SLOTS];
            let len = v.len() as u8;
            for (i, s) in v.into_iter().enumerate() {
                data[i] = s;
            }
            Slots::Inline { len, data }
        } else {
            Slots::Heap(v)
        }
    }
    pub fn len(&self) -> usize {
        match self {
            Slots::Inline { len, .. } => *len as usize,
            Slots::Heap(v) => v.len(),
        }
    }
    pub fn get(&self, i: usize) -> Option<Slot> {
        match self {
            Slots::Inline { len, data } => {
                if i < *len as usize {
                    Some(data[i])
                } else {
                    None
                }
            }
            Slots::Heap(v) => v.get(i).copied(),
        }
    }
    pub fn set(&mut self, i: usize, v: Slot) -> Option<Slot> {
        match self {
            Slots::Inline { len, data } => {
                if i < *len as usize {
                    let old = data[i];
                    data[i] = v;
                    Some(old)
                } else {
                    None
                }
            }
            Slots::Heap(items) => {
                let old = *items.get(i)?;
                items[i] = v;
                Some(old)
            }
        }
    }
}

/// Compact element storage for a sequence cell (RFC 0015 §4: "flat for
/// primitive elem"). Sub-slot-width primitives are packed to their machine
/// width — a `Vec<u8>` costs one byte per element instead of eight — while
/// 8-byte primitives and every reference-typed element keep an 8-byte slot.
pub enum Packed {
    Slots(Vec<Slot>),
    U8(Vec<u8>),
    I8(Vec<i8>),
    U16(Vec<u16>),
    I16(Vec<i16>),
    U32(Vec<u32>),
    I32(Vec<i32>),
    F32(Vec<f32>),
    Char(Vec<u32>),
    Bool(Vec<u8>),
}

impl Packed {
    /// Pick the storage kind for `elem`; unknown/sentinel ids (e.g. the
    /// untyped `TY_ANY`) fall back to slots.
    pub fn for_elem(elem: TypeId, table: &TypeTable, cap: usize) -> Packed {
        if (elem as usize) >= table.types.len() {
            return Packed::Slots(Vec::with_capacity(cap));
        }
        match table.kind(elem) {
            TyKind::Prim(PrimTy::U8) => Packed::U8(Vec::with_capacity(cap)),
            TyKind::Prim(PrimTy::I8) => Packed::I8(Vec::with_capacity(cap)),
            TyKind::Prim(PrimTy::U16) => Packed::U16(Vec::with_capacity(cap)),
            TyKind::Prim(PrimTy::I16) => Packed::I16(Vec::with_capacity(cap)),
            TyKind::Prim(PrimTy::U32) => Packed::U32(Vec::with_capacity(cap)),
            TyKind::Prim(PrimTy::I32) => Packed::I32(Vec::with_capacity(cap)),
            TyKind::Prim(PrimTy::F32) => Packed::F32(Vec::with_capacity(cap)),
            TyKind::Prim(PrimTy::Char) => Packed::Char(Vec::with_capacity(cap)),
            TyKind::Prim(PrimTy::Bool) => Packed::Bool(Vec::with_capacity(cap)),
            _ => Packed::Slots(Vec::with_capacity(cap)),
        }
    }

    pub fn from_slots(elem: TypeId, table: &TypeTable, slots: Vec<Slot>) -> Packed {
        let mut p = Packed::for_elem(elem, table, slots.len());
        for s in slots {
            p.push(s);
        }
        p
    }

    /// Bytes of accounting/actual storage per element.
    pub fn elem_width(&self) -> u64 {
        match self {
            Packed::Slots(_) => 8,
            Packed::U8(_) | Packed::I8(_) | Packed::Bool(_) => 1,
            Packed::U16(_) | Packed::I16(_) => 2,
            Packed::U32(_) | Packed::I32(_) | Packed::F32(_) | Packed::Char(_) => 4,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Packed::Slots(v) => v.len(),
            Packed::U8(v) => v.len(),
            Packed::I8(v) => v.len(),
            Packed::U16(v) => v.len(),
            Packed::I16(v) => v.len(),
            Packed::U32(v) => v.len(),
            Packed::I32(v) => v.len(),
            Packed::F32(v) => v.len(),
            Packed::Char(v) => v.len(),
            Packed::Bool(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Reconstruct a slot (the VM's untagged 8-byte value) from an element.
    pub fn get(&self, i: usize) -> Option<Slot> {
        Some(match self {
            Packed::Slots(v) => *v.get(i)?,
            Packed::U8(v) => Slot::int(*v.get(i)? as i64),
            Packed::I8(v) => Slot::int(*v.get(i)? as i64),
            Packed::U16(v) => Slot::int(*v.get(i)? as i64),
            Packed::I16(v) => Slot::int(*v.get(i)? as i64),
            Packed::U32(v) => Slot::int(*v.get(i)? as i64),
            Packed::I32(v) => Slot::int(*v.get(i)? as i64),
            Packed::F32(v) => Slot::float(*v.get(i)? as f64),
            Packed::Char(v) => Slot::ch(char::from_u32(*v.get(i)?).unwrap_or('\0')),
            Packed::Bool(v) => Slot::bool(*v.get(i)? != 0),
        })
    }

    pub fn set(&mut self, i: usize, s: Slot) -> Option<Slot> {
        let old = self.get(i)?;
        match self {
            Packed::Slots(v) => v[i] = s,
            Packed::U8(v) => v[i] = unsafe { s.i } as u8,
            Packed::I8(v) => v[i] = unsafe { s.i } as i8,
            Packed::U16(v) => v[i] = unsafe { s.i } as u16,
            Packed::I16(v) => v[i] = unsafe { s.i } as i16,
            Packed::U32(v) => v[i] = unsafe { s.i } as u32,
            Packed::I32(v) => v[i] = unsafe { s.i } as i32,
            Packed::F32(v) => v[i] = unsafe { s.f } as f32,
            Packed::Char(v) => v[i] = s.as_char() as u32,
            Packed::Bool(v) => v[i] = s.as_bool() as u8,
        }
        Some(old)
    }

    pub fn push(&mut self, s: Slot) {
        match self {
            Packed::Slots(v) => v.push(s),
            Packed::U8(v) => v.push(unsafe { s.i } as u8),
            Packed::I8(v) => v.push(unsafe { s.i } as i8),
            Packed::U16(v) => v.push(unsafe { s.i } as u16),
            Packed::I16(v) => v.push(unsafe { s.i } as i16),
            Packed::U32(v) => v.push(unsafe { s.i } as u32),
            Packed::I32(v) => v.push(unsafe { s.i } as i32),
            Packed::F32(v) => v.push(unsafe { s.f } as f32),
            Packed::Char(v) => v.push(s.as_char() as u32),
            Packed::Bool(v) => v.push(s.as_bool() as u8),
        }
    }

    pub fn pop(&mut self) -> Option<Slot> {
        let n = self.len();
        if n == 0 {
            return None;
        }
        let v = self.get(n - 1);
        match self {
            Packed::Slots(v) => {
                v.pop();
            }
            Packed::U8(v) => {
                v.pop();
            }
            Packed::I8(v) => {
                v.pop();
            }
            Packed::U16(v) => {
                v.pop();
            }
            Packed::I16(v) => {
                v.pop();
            }
            Packed::U32(v) => {
                v.pop();
            }
            Packed::I32(v) => {
                v.pop();
            }
            Packed::F32(v) => {
                v.pop();
            }
            Packed::Char(v) => {
                v.pop();
            }
            Packed::Bool(v) => {
                v.pop();
            }
        }
        v
    }

    pub fn to_slots(&self) -> Vec<Slot> {
        (0..self.len()).map(|i| self.get(i).unwrap()).collect()
    }
}

pub enum CellData {
    Str(String),
    Vec { elem: TypeId, items: RefCell<Packed> },
    Array { elem: TypeId, items: RefCell<Packed> },
    /// enum member — immortal singleton per (ty, member)
    Enum { member: u32 },
    /// Option/Result: tag 0 = some/ok, 1 = none/err
    Sum { tag: u32, payload: Option<Slot> },
    /// dataclass/class instance — the repr-C payload as slots
    Record { fields: RefCell<Slots> },
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
            CellData::Vec { items, .. } => Some(items.borrow().to_slots()),
            CellData::Array { items, .. } => Some(items.borrow().to_slots()),
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
            CellData::Record { fields } => fields.borrow().get(field as usize),
            _ => None,
        }
    }
    pub fn record_set(&self, field: u32, v: Slot) -> Option<Slot> {
        match &self.data {
            CellData::Record { fields } => fields.borrow_mut().set(field as usize, v),
            _ => None,
        }
    }
}

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
        Ok(Slot { r: Some(p as *const CellVal) })
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
            return Ok(Slot { r: Some(*p) });
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
        Ok(Slot { r: Some(p as *const CellVal) })
    }

    // ---- ref discipline (RFC 0016 §5) ----

    /// rc += 1 (immortal singletons saturate at `u32::MAX`).
    pub fn retain(&self, s: Slot) {
        let Some(p) = (unsafe { s.r }) else { return };
        unsafe {
            let c = &*p;
            c.refs.set(c.refs.get().saturating_add(1));
        }
    }

    /// rc -= 1; at zero the cell is dropped and its slot recycled. Note the
    /// current ref discipline does not recursively release a cell's child
    /// slots on drop (matching the previous `Rc` behaviour).
    pub fn release(&self, s: Slot) {
        let Some(p) = (unsafe { s.r }) else { return };
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

/// SAFETY: `s` must hold a live cell pointer.
pub unsafe fn cell<'a>(s: Slot) -> &'a CellVal {
    unsafe { &*s.r.unwrap_unchecked() }
}

pub fn cell_of(s: Slot) -> &'static CellVal {
    unsafe { &*(s.r.unwrap() as *const CellVal) }
}
