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
/// the cell (RFC 0015 §4), larger ones spill to the heap.
/// This removes the per-record `Vec` allocation for the common small
/// struct.
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
    /// Drain all slots (release-path helper: the release walk reads a
    /// record's ref-typed fields out before the cell dies). Leaves the
    /// container empty.
    pub fn take_slots(&mut self) -> Vec<Slot> {
        match self {
            Slots::Heap(v) => std::mem::take(v),
            Slots::Inline { len, data } => {
                let n = (*len).min(INLINE_SLOTS as u8) as usize;
                *len = 0;
                data[..n].to_vec()
            }
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

    /// Drain all slots (release-path helper: the release walk reads them
    /// out before the container dies). Leaves the container empty. Only
    /// the `Slots` variant stores cell handles; prim-packed variants have
    /// no children.
    pub fn take_slots(&mut self) -> Vec<Slot> {
        match self {
            Packed::Slots(v) => std::mem::take(v),
            _ => Vec::new(),
        }
    }
}

/// A `str` payload: the engine's own UTF-8 octets plus the ASCII-ness,
/// cached once at allocation. `str` is a `char` sequence, but the common
/// case is all-ASCII — and then the char index equals the byte index, so
/// `s[i]` / `for..of` is O(1) instead of re-decoding the UTF-8 prefix
/// every step (RFC 0008).
///
/// The engine owns the UTF-8 invariant rather than a type: `bytes` is
/// valid UTF-8 by construction (every source is a `&str`, a literal, or a
/// `render`), and appending valid UTF-8 to valid UTF-8 stays valid. So
/// `as_str` asserts the invariant instead of re-checking it, and the
/// byte-level accessors (`as_bytes`, `char_len`) are the primary path.
pub struct StrVal {
    pub bytes: Vec<u8>,
    pub ascii: bool,
}

/// A type-erased Rust payload behind a host-constructed `Opaque` box
/// (RFC 0023/0026). The host news the box with any `'static` type
/// (`OpaqueBox::alloc`) and borrows it back call-scoped
/// (`OpaqueBox::with`/`with_mut`). Erasure is a one-fn vtable — a drop
/// pointer plus a `TypeId` token — no `dyn` in the heap; the payload's
/// `Drop` runs when the cell's rc hits 0 (`release_cell` drops the
/// `CellVal`, which drops this), and its shallow `size_of::<T>()` is
/// what the cell accounts against the heap budget (RFC 0040 — interior
/// allocations a `T` makes are the host's own business).
pub struct HostPayload {
    ptr: *mut u8,                      // a leaked `Box<T>`, `T: 'static`
    drop_fn: unsafe fn(*mut u8),
    /// the payload's Rust type — `OpaqueBox::from_handle` compares this
    /// before any pointer moves, so a wrong-type borrow is a checked
    /// error, never UB
    ty: std::any::TypeId,
    /// for diagnostics (`host box holds \`MyMap\`, not \`Canvas\``)
    type_name: &'static str,
    /// borrow guard (RFC 0023 §2): 0 = free, `BORROW_MUT` = one exclusive
    /// borrow live, else the shared-borrow count. Rut-side ops never
    /// touch the payload, so only the host accessors check it.
    borrows: Cell<u32>,
}

pub(crate) const BORROW_MUT: u32 = u32::MAX;

impl HostPayload {
    pub fn new<T: 'static>(val: T) -> HostPayload {
        HostPayload {
            ptr: Box::into_raw(Box::new(val)) as *mut u8,
            drop_fn: |p: *mut u8| unsafe {
                drop(Box::from_raw(p as *mut T));
            },
            ty: std::any::TypeId::of::<T>(),
            type_name: std::any::type_name::<T>(),
            borrows: Cell::new(0),
        }
    }

    /// Does this payload hold a `T`? The `TypeId` token compare is what
    /// makes a wrong-type borrow a checked error instead of UB.
    pub fn is<T: 'static>(&self) -> bool {
        self.ty == std::any::TypeId::of::<T>()
    }

    pub fn type_name(&self) -> &'static str {
        self.type_name
    }

    pub(crate) fn borrows(&self) -> u32 {
        self.borrows.get()
    }

    pub(crate) fn begin_shared(&self) {
        self.borrows.set(self.borrows.get() + 1);
    }

    pub(crate) fn begin_mut(&self) {
        self.borrows.set(BORROW_MUT);
    }

    pub(crate) fn end_borrow(&self) {
        self.borrows.set(if self.borrows.get() == BORROW_MUT { 0 } else { self.borrows.get() - 1 });
    }

    /// The checked exclusive borrow. `with`/`with_mut` must have
    /// verified `ty` and taken the guard already — this is the raw deref.
    pub(crate) unsafe fn deref<T>(&self) -> &mut T {
        unsafe { &mut *(self.ptr as *mut T) }
    }
}

impl Drop for HostPayload {
    fn drop(&mut self) {
        unsafe { (self.drop_fn)(self.ptr) };
    }
}

pub enum CellData {
    Str(StrVal),
    Array { elem: TypeId, items: RefCell<Packed> },
    /// enum member — immortal singleton per (ty, member)
    Enum { member: u32 },
    /// Option/Result: tag 0 = some/ok, 1 = none/err
    Sum { tag: u32, payload: Option<Slot> },
    /// struct/class instance — the payload as one slot per field
    Record { fields: RefCell<Slots> },
    /// Opaque box (RFC 0014): the value + its runtime type
    OpaqueBox { val: Slot, val_ty: TypeId },
    /// host payload box (RFC 0023/0026) — any `'static` Rust value behind
    /// the same `Opaque` surface; rut sees only the box (`downcast<T>` is
    /// `None`, `o is Opaque` is `true`), the host borrows it typed. The
    /// descriptor is boxed to keep `CellVal` inside its size pin below;
    /// this is the cold host boundary, not a hot-loop cell.
    HostBoxed { payload: Box<HostPayload> },
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
    /// The text as a `&str`. The engine maintains the UTF-8 invariant
    /// itself (see `StrVal`), so this asserts it instead of re-checking it;
    /// byte-level callers should prefer `as_bytes`.
    pub fn as_str(&self) -> &str {
        match &self.data {
            CellData::Str(v) => unsafe { std::str::from_utf8_unchecked(&v.bytes) },
            _ => "",
        }
    }
    /// The raw UTF-8 octets of a `str` cell — empty for any other shape.
    pub fn as_bytes(&self) -> &[u8] {
        match &self.data {
            CellData::Str(v) => &v.bytes,
            _ => &[],
        }
    }
    /// The number of `char`s: the byte length when all-ASCII (the flag is
    /// computed once at allocation), else a UTF-8 scan.
    pub fn char_len(&self) -> usize {
        match &self.data {
            CellData::Str(v) if v.ascii => v.bytes.len(),
            CellData::Str(_) => self.as_str().chars().count(),
            _ => 0,
        }
    }
    /// True when this `str` is all-ASCII, so a char index is a byte index.
    pub fn str_ascii(&self) -> bool {
        matches!(&self.data, CellData::Str(v) if v.ascii)
    }
    /// The raw octets of a `bytes` cell — `bytes` is a `u8` array at the
    /// engine level (RFC 0004); empty for any other shape.
    pub fn bytes_copy(&self) -> Vec<u8> {
        self.seq_bytes_copy().unwrap_or_default()
    }
    /// Content equality of two sequences (RFC 0012 §4: `bytes` compares by
    /// content — it is a `u8` array). Element-wise, shallow.
    pub fn array_eq(&self, other: &CellVal) -> bool {
        let (a, b) = match (&self.data, &other.data) {
            (CellData::Array { items: a, .. }, CellData::Array { items: b, .. }) => {
                (a.borrow(), b.borrow())
            }
            _ => return false,
        };
        if a.len() != b.len() {
            return false;
        }
        match (&*a, &*b) {
            (Packed::U8(x), Packed::U8(y)) => x == y,
            (Packed::I8(x), Packed::I8(y)) => x == y,
            (Packed::U16(x), Packed::U16(y)) => x == y,
            (Packed::I16(x), Packed::I16(y)) => x == y,
            (Packed::U32(x), Packed::U32(y)) => x == y,
            (Packed::I32(x), Packed::I32(y)) => x == y,
            (Packed::F32(x), Packed::F32(y)) => x == y,
            (Packed::Char(x), Packed::Char(y)) => x == y,
            (Packed::Bool(x), Packed::Bool(y)) => x == y,
            (Packed::Slots(x), Packed::Slots(y)) => {
                x.iter().zip(y.iter()).all(|(p, q)| Slot::same_ref(*p, *q))
            }
            _ => false,
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
            CellData::Array { items, .. } => Some(items.borrow().len()),
            _ => None,
        }
    }
    pub fn seq_items_copy(&self) -> Option<Vec<Slot>> {
        match &self.data {
            CellData::Array { items, .. } => Some(items.borrow().to_slots()),
            _ => None,
        }
    }
    pub fn elem_ty(&self) -> Option<TypeId> {
        match &self.data {
            CellData::Array { elem, .. } => Some(*elem),
            _ => None,
        }
    }
    /// Copy a `Vec<u8>`/`Array<u8>` cell's packed payload to a flat byte
    /// vector (the `bytes` conversion path); avoids widening through `Slot`.
    pub fn seq_bytes_copy(&self) -> Option<Vec<u8>> {
        match &self.data {
            CellData::Array { items, .. } => {
                let b = items.borrow();
                Some(match &*b {
                    Packed::U8(v) => v.clone(),
                    other => (0..other.len())
                        .map(|i| other.get(i).map(|s| unsafe { s.i } as u8).unwrap_or(0))
                        .collect(),
                })
            }
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
