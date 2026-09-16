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
/// primitive elem"). Sub-slot-width primitives pack to their machine
/// width — a `Vec<u8>` costs one byte per element instead of eight —
/// while 8-byte primitives and every reference-typed element keep an
/// 8-byte slot.
///
/// The elements live in a VM-owned block (`heap::blocks`, RFC 0039) at
/// `width`-byte stride; the `kind` says how a stored element reads back
/// into a `Slot`. One fixed-size cell field replaces the ten typed
/// `Vec` variants the enum used to have — the kind tag is what varied,
/// the storage was Rust's.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ArrKind {
    Slots,
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    F32,
    Char,
    Bool,
}

impl ArrKind {
    /// Pick the storage kind for `elem`; unknown/sentinel ids (e.g. the
    /// untyped `TY_ANY`) fall back to slots.
    pub fn of(elem: TypeId, table: &TypeTable) -> ArrKind {
        if (elem as usize) >= table.types.len() {
            return ArrKind::Slots;
        }
        match table.kind(elem) {
            TyKind::Prim(PrimTy::U8) => ArrKind::U8,
            TyKind::Prim(PrimTy::I8) => ArrKind::I8,
            TyKind::Prim(PrimTy::U16) => ArrKind::U16,
            TyKind::Prim(PrimTy::I16) => ArrKind::I16,
            TyKind::Prim(PrimTy::U32) => ArrKind::U32,
            TyKind::Prim(PrimTy::I32) => ArrKind::I32,
            TyKind::Prim(PrimTy::F32) => ArrKind::F32,
            TyKind::Prim(PrimTy::Char) => ArrKind::Char,
            TyKind::Prim(PrimTy::Bool) => ArrKind::Bool,
            _ => ArrKind::Slots,
        }
    }

    /// Bytes per element.
    pub fn width(self) -> usize {
        match self {
            ArrKind::U8 | ArrKind::I8 | ArrKind::Bool => 1,
            ArrKind::U16 | ArrKind::I16 => 2,
            ArrKind::U32 | ArrKind::I32 | ArrKind::F32 | ArrKind::Char => 4,
            ArrKind::Slots => 8,
        }
    }
}

/// The element run of one array cell: a block, a length, an element
/// capacity, and how to read the bytes back. The block is freed by the
/// release path (never by `Drop` glue — freeing needs the `&Arena` the
/// release walk already holds); blocks never move, so element reads are
/// plain offset math off the block pointer.
pub struct ArrData {
    pub(crate) block: *mut u8,
    pub(crate) len: u32,
    /// element capacity (class-rounded via the block header)
    pub(crate) cap: u32,
    pub(crate) kind: ArrKind,
}

impl ArrData {
    /// An empty run with room for `cap` elements — the block arrives
    /// zeroed, which IS the zero-fill for `Array<T>(n)`.
    pub(crate) fn new(kind: ArrKind, block: *mut u8, cap: u32) -> ArrData {
        ArrData { block, len: 0, cap, kind }
    }

    pub fn len(&self) -> usize {
        self.len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn width(&self) -> usize {
        self.kind.width()
    }

    #[inline]
    fn slot_at(&self, i: usize) -> Slot {
        let p = unsafe { self.block.add(i * self.width()) } as *const Slot;
        match self.kind {
            ArrKind::Slots => unsafe { std::ptr::read(p) },
            ArrKind::U8 => Slot::int(unsafe { *(p as *const u8) } as i64),
            ArrKind::I8 => Slot::int(unsafe { *(p as *const i8) } as i64),
            ArrKind::U16 => Slot::int(unsafe { std::ptr::read_unaligned(p as *const u16) } as i64),
            ArrKind::I16 => Slot::int(unsafe { std::ptr::read_unaligned(p as *const i16) } as i64),
            ArrKind::U32 => Slot::int(unsafe { std::ptr::read_unaligned(p as *const u32) } as i64),
            ArrKind::I32 => Slot::int(unsafe { std::ptr::read_unaligned(p as *const i32) } as i64),
            ArrKind::F32 => Slot::float(unsafe { std::ptr::read_unaligned(p as *const f32) } as f64),
            ArrKind::Char => Slot::ch(char::from_u32(unsafe { std::ptr::read_unaligned(p as *const u32) }).unwrap_or('\0')),
            ArrKind::Bool => Slot::bool(unsafe { *(p as *const u8) } != 0),
        }
    }

    #[inline]
    fn write_slot(&mut self, i: usize, s: Slot) {
        let p = unsafe { self.block.add(i * self.width()) } as *mut Slot;
        unsafe {
            match self.kind {
                ArrKind::Slots => std::ptr::write(p, s),
                ArrKind::U8 => *(p as *mut u8) = unsafe { s.i } as u8,
                ArrKind::I8 => *(p as *mut i8) = unsafe { s.i } as i8,
                ArrKind::U16 => std::ptr::write_unaligned(p as *mut u16, unsafe { s.i } as u16),
                ArrKind::I16 => std::ptr::write_unaligned(p as *mut i16, unsafe { s.i } as i16),
                ArrKind::U32 => std::ptr::write_unaligned(p as *mut u32, unsafe { s.i } as u32),
                ArrKind::I32 => std::ptr::write_unaligned(p as *mut i32, unsafe { s.i } as i32),
                ArrKind::F32 => std::ptr::write_unaligned(p as *mut f32, unsafe { s.f } as f32),
                ArrKind::Char => std::ptr::write_unaligned(p as *mut u32, s.as_char() as u32),
                ArrKind::Bool => *(p as *mut u8) = s.as_bool() as u8,
            }
        }
    }

    pub fn get(&self, i: usize) -> Option<Slot> {
        if i < self.len as usize {
            Some(self.slot_at(i))
        } else {
            None
        }
    }

    pub fn set(&mut self, i: usize, s: Slot) -> Option<Slot> {
        if i >= self.len as usize {
            return None;
        }
        let old = self.slot_at(i);
        self.write_slot(i, s);
        Some(old)
    }

    /// Append during construction. Growth needs the block store, which
    /// construction paths reach through the `Heap`; element capacity is
    /// exact after `ArrData::new`, so this only fires if callers push
    /// past their own declared capacity.
    pub(crate) fn push(&mut self, s: Slot, blocks: &Blocks) {
        if self.len == self.cap {
            let old_len = self.len as usize;
            self.block = blocks.grow(self.block, old_len * self.width(), (old_len + 1) * self.width());
            self.cap = (blocks.cap_of(self.block) / self.width()) as u32;
        }
        self.write_slot(self.len as usize, s);
        self.len += 1;
    }

    pub fn pop(&mut self) -> Option<Slot> {
        if self.len == 0 {
            return None;
        }
        let v = self.slot_at(self.len as usize - 1);
        self.len -= 1;
        Some(v)
    }

    pub fn to_slots(&self) -> Vec<Slot> {
        (0..self.len as usize).map(|i| self.slot_at(i)).collect()
    }
}

/// A `str` payload: the engine's own UTF-8 octets plus the ASCII-ness,
/// cached once at allocation. `str` is a `char` sequence, but the common
/// case is all-ASCII — and then the char index equals the byte index, so
/// `s[i]` / `for..of` is O(1) instead of re-decoding the UTF-8 prefix
/// every step (RFC 0008).
///
/// The engine owns the UTF-8 invariant rather than a type: the octets are
/// valid UTF-8 by construction (every source is a `&str`, a literal, or a
/// `render`), and appending valid UTF-8 to valid UTF-8 stays valid. So
/// `as_str` asserts the invariant instead of re-checking it, and the
/// byte-level accessors (`as_bytes`, `char_len`) are the primary path.
///
/// The octets live in a VM-owned block (`heap::blocks`, RFC 0039): the
/// cell's slot in the arena stays fixed-size; the variable part is a
/// block the release path frees with the cell. Blocks never move
/// (RFC 0016 OQ-1), so `as_bytes` can hand out `&[u8]` into the store.
pub struct StrVal {
    /// the payload block — `len` octets valid, `cap` octets usable; freed
    /// by the release path (never by `Drop` glue: freeing needs the
    /// `&Arena` the release walk already holds)
    pub(crate) block: *mut u8,
    /// valid octets in the block
    pub(crate) len: u32,
    /// usable octets — cached from the block header so the append gate
    /// stays on the cell's cache line (the block may live pages away)
    pub(crate) cap: u32,
    pub ascii: bool,
}

impl StrVal {
    /// The valid octets. Blocks never move, so the slice is valid for the
    /// cell's lifetime (the same contract as the `&'static CellVal` it
    /// hangs off).
    pub fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.block, self.len as usize) }
    }
    /// In-place append — the caller (Heap::append_bytes) has already
    /// grown the block and owns the accounting.
    #[inline(always)]
    pub(crate) unsafe fn append(&mut self, extra: &[u8]) {
        unsafe {
            std::ptr::copy_nonoverlapping(extra.as_ptr(), self.block.add(self.len as usize), extra.len());
        }
        self.len += extra.len() as u32;
        self.ascii = self.ascii && extra.is_ascii();
    }
}

pub enum CellData {
    Str(StrVal),
    Array { elem: TypeId, items: RefCell<ArrData> },
    /// enum member — immortal singleton per (ty, member)
    Enum { member: u32 },
    /// struct/class instance — the payload as one slot per field
    Record { fields: RefCell<Slots> },
    /// Opaque box (RFC 0014): the value + its runtime type
    OpaqueBox { val: Slot, val_ty: TypeId },
    /// host payload box (RFC 0023/0026) — any `'static` Rust value behind
    /// the same `Opaque` surface; rut sees only the box (`downcast<T>` is
    /// `None`, `o is Opaque` is `true`), the host borrows it typed. The
    /// box is the Rust concept it is: `dyn Any` erases the payload (its
    /// own vtable drops it when the cell dies), `type_name` survives for
    /// diagnostics (unrecoverable from the erased box), and the borrow
    /// guard (RFC 0023 §2) rides in the cell. Boxed to keep `CellVal`
    /// inside its size pin below; this is the cold host boundary, not a
    /// hot-loop cell.
    HostBoxed { payload: Box<dyn std::any::Any>, type_name: &'static str, borrows: Cell<u32> },
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
            CellData::Str(v) => unsafe { std::str::from_utf8_unchecked(v.bytes()) },
            _ => "",
        }
    }
    /// The raw UTF-8 octets of a `str` cell — empty for any other shape.
    pub fn as_bytes(&self) -> &[u8] {
        match &self.data {
            CellData::Str(v) => v.bytes(),
            _ => &[],
        }
    }
    /// The number of `char`s: the byte length when all-ASCII (the flag is
    /// computed once at allocation), else a UTF-8 scan.
    pub fn char_len(&self) -> usize {
        match &self.data {
            CellData::Str(v) if v.ascii => v.len as usize,
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
        if a.len() != b.len() || a.kind != b.kind {
            return false;
        }
        match a.kind {
            // same kind, same width: narrow kinds compare their packed
            // bytes; the slot kind compares handles/64-bit raw
            ArrKind::Slots => (0..a.len as usize).all(|i| {
                let (x, y) = (a.slot_at(i), b.slot_at(i));
                Slot::same_ref(x, y)
            }),
            _ => {
                let w = a.width();
                unsafe {
                    std::slice::from_raw_parts(a.block, a.len as usize * w)
                        == std::slice::from_raw_parts(b.block, b.len as usize * w)
                }
            }
        }
    }
    /// Option/Result payload: (tag 0=some/ok 1=none/err, payload)
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
                Some(match b.kind {
                    ArrKind::U8 => unsafe {
                        std::slice::from_raw_parts(b.block, b.len as usize).to_vec()
                    },
                    _ => (0..b.len())
                        .map(|i| b.get(i as usize).map(|s| unsafe { s.i } as u8).unwrap_or(0))
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
