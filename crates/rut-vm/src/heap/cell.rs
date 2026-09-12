//! Heap cells (RFC 0016): `CellVal`/`CellData`, the inline `Slots` buffer,
//! the packed element store, and the raw `cell`/`cell_of` accessors.
use super::*;

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
    /// closure value (RFC 0013) — v1 captures by value. The callee's
    /// signature (`params`/`ret`/capture types) is static program data in
    /// `prog.funcs[func]`; the cell carries only the function id and the
    /// per-instance captured values, so it stays small (RFC 0039).
    Closure {
        func: u32,
        captures: Vec<Slot>,
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
            CellData::Closure { func, captures } => Some((*func, captures.clone())),
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
/// SAFETY: `s` must hold a live cell pointer.
pub unsafe fn cell<'a>(s: Slot) -> &'a CellVal {
    unsafe { &*s.r }
}

pub fn cell_of(s: Slot) -> &'static CellVal {
    unsafe { &*s.r }
}

/// Every cell is one fixed-size record, so its size is set by the largest
/// `CellData` variant. Guard it at compile time: a bulky variant silently
/// taxes every allocation in the arena (RFC 0039). `Closure` is deliberately
/// minimal — its signature lives in `prog.funcs`, not the cell.
const _: () = assert!(std::mem::size_of::<CellVal>() <= 72);
