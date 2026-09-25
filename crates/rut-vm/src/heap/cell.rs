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
///
/// `Opt(p)` is the primitive-optional store (RFC 0044 §5): a `[?p]`
/// backing holds each element as the raw `p` payload plus a one-byte nil
/// tag — stride `p.width() + 1`, the tag byte last, so one fixed-stride
/// region serves payload and tags and every block-size computation rides
/// `width()` unchanged. No element is a cell handle: nil is the tag (a
/// zeroed block is all-nil, cohering with RFC 0015 §5's nil-is-the-zero
/// word), and a nil store also zeroes the payload so raw-byte views of
/// the block stay deterministic. The release walk contributes no
/// children for these arrays, and reads mint a fresh opt VALUE
/// (value semantics — see `Heap::alloc_opt_value`).
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
    Bool,
    Opt(PrimTy),
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
            TyKind::Prim(PrimTy::Bool) => ArrKind::Bool,
            // the primitive-optional store: `?prim` elements ride the raw
            // payload + nil tag; `?str`/`?record`/`??T` keep cell slots
            TyKind::Opt { elem } => match table.kind(*elem) {
                TyKind::Prim(p) => ArrKind::Opt(*p),
                _ => ArrKind::Slots,
            },
            _ => ArrKind::Slots,
        }
    }

    /// Bytes per element. The opt store strides payload + tag.
    pub fn width(self) -> usize {
        match self {
            ArrKind::U8 | ArrKind::I8 | ArrKind::Bool => 1,
            ArrKind::U16 | ArrKind::I16 => 2,
            ArrKind::U32 | ArrKind::I32 | ArrKind::F32 => 4,
            ArrKind::Slots => 8,
            ArrKind::Opt(p) => p.width() + 1,
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
            ArrKind::Bool => Slot::bool(unsafe { *(p as *const u8) } != 0),
            // the primitive-optional store: the generic slot decode yields
            // the null slot for nil, the RAW payload for some. This raw form
            // serves the block-level paths (raw copies, raw compares); the
            // interpreter's element ops decode through `read_opt_raw` +
            // `Heap::alloc_opt_value` instead, so a proper `?prim` value
            // (a cell) is minted per read and the raw bits never masquerade
            // as a cell handle in a ref-typed register.
            ArrKind::Opt(p) => {
                let w = p.width();
                let e = unsafe { self.block.add(i * self.width()) };
                if unsafe { *e.add(w) } == 0 {
                    Slot::null()
                } else {
                    Self::prim_payload(e, p)
                }
            }
        }
    }

    /// The RAW payload of an `Opt(p)` element at `e` (the element's stride
    /// base), decoded as its prim slot.
    #[inline]
    fn prim_payload(e: *const u8, p: PrimTy) -> Slot {
        let q = e as *const Slot;
        unsafe {
            match p {
                PrimTy::U8 => Slot::int(*e as i64),
                PrimTy::I8 => Slot::int(*(e as *const i8) as i64),
                PrimTy::U16 => Slot::int(std::ptr::read_unaligned(e as *const u16) as i64),
                PrimTy::I16 => Slot::int(std::ptr::read_unaligned(e as *const i16) as i64),
                PrimTy::U32 => Slot::int(std::ptr::read_unaligned(e as *const u32) as i64),
                PrimTy::I32 => Slot::int(std::ptr::read_unaligned(e as *const i32) as i64),
                PrimTy::U64 | PrimTy::I64 => Slot::int(std::ptr::read_unaligned(q as *const i64)),
                PrimTy::F32 => Slot::float(std::ptr::read_unaligned(e as *const f32) as f64),
                PrimTy::F64 => Slot::float(std::ptr::read_unaligned(q as *const f64)),
                PrimTy::Bool => Slot::bool(*e != 0),
            }
        }
    }

    /// The nil tag of element `i` of the primitive-optional store.
    #[inline]
    fn opt_tag(&self, i: usize) -> bool {
        debug_assert!(matches!(self.kind, ArrKind::Opt(_)));
        let w = match self.kind {
            ArrKind::Opt(p) => p.width(),
            _ => 0,
        };
        unsafe { *self.block.add(i * self.width() + w) != 0 }
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
                ArrKind::Bool => *(p as *mut u8) = s.as_bool() as u8,
                // the primitive-optional store ENCODES the incoming proper
                // `?prim` value (cell or null): nil stores tag 0 (payload
                // zeroed — determinism for raw-byte views), some stores the
                // payload's bits plus tag 1. The box itself is never stored;
                // nothing here retains or releases.
                ArrKind::Opt(pt) => {
                    let w = pt.width();
                    let e = self.block.add(i * self.width());
                    if unsafe { s.r.is_null() } {
                        std::ptr::write_bytes(e, 0, w + 1);
                    } else {
                        let raw = cell_of(s).record_get(0).unwrap_or(Slot::int(0));
                        let q = e as *mut Slot;
                        match pt {
                            PrimTy::U8 => *(e) = unsafe { raw.i } as u8,
                            PrimTy::I8 => *(e as *mut i8) = unsafe { raw.i } as i8,
                            PrimTy::U16 => std::ptr::write_unaligned(e as *mut u16, unsafe { raw.i } as u16),
                            PrimTy::I16 => std::ptr::write_unaligned(e as *mut i16, unsafe { raw.i } as i16),
                            PrimTy::U32 => std::ptr::write_unaligned(e as *mut u32, unsafe { raw.i } as u32),
                            PrimTy::I32 => std::ptr::write_unaligned(e as *mut i32, unsafe { raw.i } as i32),
                            PrimTy::U64 | PrimTy::I64 => std::ptr::write_unaligned(q as *mut i64, unsafe { raw.i }),
                            PrimTy::F32 => std::ptr::write_unaligned(e as *mut f32, raw.as_f64() as f32),
                            PrimTy::F64 => std::ptr::write_unaligned(q as *mut f64, raw.as_f64()),
                            PrimTy::Bool => *e = raw.as_bool() as u8,
                        }
                        *e.add(w) = 1;
                    }
                }
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

    /// Direct element read with no bounds check — the caller guarantees
    /// `i < len` (the `seq_get` fast path folds negative + overflow into
    /// ONE unsigned compare before calling this). Never routed for a
    /// primitive-optional store: its raw payload must not masquerade as a
    /// `?prim` value in a register (the element ops decode through
    /// `opt_raw` + `Heap::alloc_opt_value`).
    #[inline]
    pub(crate) unsafe fn read_unchecked(&self, i: usize) -> Slot {
        debug_assert!(!matches!(self.kind, ArrKind::Opt(_)), "read_unchecked on the opt store — decode through opt_raw");
        self.slot_at(i)
    }

    /// Direct element write with no bounds check — same contract as
    /// `read_unchecked`; returns the displaced element. Never routed for a
    /// primitive-optional store (`write_opt`/`write_opt_raw` encode there).
    #[inline]
    pub(crate) unsafe fn write_unchecked(&mut self, i: usize, s: Slot) -> Slot {
        debug_assert!(!matches!(self.kind, ArrKind::Opt(_)), "write_unchecked on the opt store — encode through write_opt");
        let old = self.slot_at(i);
        self.write_slot(i, s);
        old
    }

    pub fn set(&mut self, i: usize, s: Slot) -> Option<Slot> {
        if i >= self.len as usize {
            return None;
        }
        let old = self.slot_at(i);
        self.write_slot(i, s);
        Some(old)
    }

    // ---- the primitive-optional store (`ArrKind::Opt`, RFC 0044 §5) ----
    //
    // The element ops decode/encode through these; the displaced value of a
    // write is raw bits and is intentionally NOT returned as a Slot — a raw
    // store holds no handles, so there is nothing to release.

    /// Raw payload decode: `None` = the element is nil.
    #[inline]
    pub fn opt_raw(&self, i: usize) -> Option<Slot> {
        let p = match self.kind {
            ArrKind::Opt(p) => p,
            _ => return None,
        };
        if !self.opt_tag(i) {
            return None;
        }
        let e = unsafe { self.block.add(i * self.width()) };
        Some(ArrData::prim_payload(e, p))
    }

    /// Encode a proper `?prim` value (cell or null) into element `i`.
    #[inline]
    pub fn write_opt(&mut self, i: usize, v: Slot) {
        debug_assert!(matches!(self.kind, ArrKind::Opt(_)));
        self.write_slot(i, v);
    }

    /// Store a RAW payload into element `i` — the some-tag implied form the
    /// MakeOpt elision bakes into `ArrSet{repr: OptPrimRaw}`.
    #[inline]
    pub fn write_opt_raw(&mut self, i: usize, raw: Slot) {
        let (w, e) = match self.kind {
            ArrKind::Opt(p) => (p.width(), unsafe { self.block.add(i * self.width()) }),
            _ => return,
        };
        let q = e as *mut Slot;
        unsafe {
            match self.kind {
                ArrKind::Opt(PrimTy::U8) => *e = unsafe { raw.i } as u8,
                ArrKind::Opt(PrimTy::I8) => *(e as *mut i8) = unsafe { raw.i } as i8,
                ArrKind::Opt(PrimTy::U16) => std::ptr::write_unaligned(e as *mut u16, unsafe { raw.i } as u16),
                ArrKind::Opt(PrimTy::I16) => std::ptr::write_unaligned(e as *mut i16, unsafe { raw.i } as i16),
                ArrKind::Opt(PrimTy::U32) => std::ptr::write_unaligned(e as *mut u32, unsafe { raw.i } as u32),
                ArrKind::Opt(PrimTy::I32) => std::ptr::write_unaligned(e as *mut i32, unsafe { raw.i } as i32),
                ArrKind::Opt(PrimTy::U64) | ArrKind::Opt(PrimTy::I64) => {
                    std::ptr::write_unaligned(q as *mut i64, unsafe { raw.i })
                }
                ArrKind::Opt(PrimTy::F32) => std::ptr::write_unaligned(e as *mut f32, raw.as_f64() as f32),
                ArrKind::Opt(PrimTy::F64) => std::ptr::write_unaligned(q as *mut f64, raw.as_f64()),
                ArrKind::Opt(PrimTy::Bool) => *e = raw.as_bool() as u8,
                _ => {}
            }
            *e.add(w) = 1;
        }
    }

    /// The array's prim-optional element kind, if it has one.
    pub fn opt_kind(&self) -> Option<PrimTy> {
        match self.kind {
            ArrKind::Opt(p) => Some(p),
            _ => None,
        }
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
    /// str slice view (RFC 0042): a window into an OWNED str cell,
    /// retained by `parent`. `off`/`len` are byte offsets into the
    /// parent's block; codepoint bounds were resolved to bytes at
    /// creation, and view-of-view flattens onto the root, so `parent`
    /// is always an owned `Str`.
    StrView { parent: Slot, off: u32, len: u32, ascii: bool },
    /// array window (RFC 0042 §6): a fixed-length view over a backing
    /// `Array` cell, retained by `parent`. Element `i` of the window is
    /// element `off + i` of the parent — WRITES GO THROUGH (the `*T`
    /// aliasing law): the window is a pointer, not a copy. Fixed-length
    /// by construction; `push`/`pop` on a window trap.
    ArrView { parent: Slot, off: u32, len: u32 },
    /// enum member — immortal singleton per (ty, member)
    Enum { member: u32 },
    /// struct/class instance — the payload as one slot per field
    Record { fields: RefCell<Slots> },
    /// closure value (RFC 0013) — v1 captures by value. The callee's
    /// signature (`params`/`ret`/capture types) is static program data in
    /// `prog.funcs[func]`; the cell carries only the function id and the
    /// per-instance captured values, so it stays small (RFC 0039).
    Closure {
        func: u32,
        captures: Vec<Slot>,
    },
    /// engine stack-trace snapshot (RFC 0036 §2, err-channel phase 2) —
    /// the `StackTrace` builtin class's payload: the RAW captured frames,
    /// innermost first, nothing else. No symbolication data lives in the
    /// cell: `name`/`line`/`col`/`render` resolve lazily, per access,
    /// against the loaded program (the interner + the position table).
    /// One frame is one call site — `(func, pc)`:
    /// 8 bytes, so `len(frames) * 8` is the whole heap charge.
    Trace { frames: Vec<TraceFrame> },
    /// the growable string builder (json-perf phase 2) — the `StrBuf`
    /// builtin class's payload: the engine-owned UTF-8 octets (the same
    /// block-backed `StrVal` shape as `Str`, grown geometrically in
    /// place) plus the tracked codepoint count, so `len` stays O(1) on
    /// non-ASCII builds too. Mutation is ONLY through the builder's
    /// natives (`push`/`push_code`); `finish` copies the octets out to a
    /// fresh immutable `str` cell and the builder keeps its buffer.
    StrBuf { buf: StrVal, chars: u32 },
}

/// One captured frame — RFC 0036 §2's `RawFrame`, rut-only shape: the
/// frame's function id and the CALL-SITE pc (the op that pushed the
/// frame; for the innermost frame, the `CallNat` capture op itself).
/// No name, no source position — capture is a raw walk of `vm.frames`
/// and symbolication pays for the lookups only when someone reads them.
#[derive(Clone, Copy, Debug)]
pub struct TraceFrame {
    pub func: u32,
    pub pc: u32,
}

impl CellVal {
    /// The text as a `&str`. The engine maintains the UTF-8 invariant
    /// itself (see `StrVal`), so this asserts it instead of re-checking it;
    /// byte-level callers should prefer `as_bytes`.
    pub fn as_str(&self) -> &str {
        match &self.data {
            CellData::Str(v) => unsafe { std::str::from_utf8_unchecked(v.bytes()) },
            CellData::StrView { parent, off, len, .. } => {
                // the view retains the parent, so the window is live
                unsafe {
                    std::str::from_utf8_unchecked(&cell_of(*parent).as_bytes()
                        [*off as usize..*off as usize + *len as usize])
                }
            }
            _ => "",
        }
    }
    /// The raw UTF-8 octets of a `str` cell — empty for any other shape.
    pub fn as_bytes(&self) -> &[u8] {
        match &self.data {
            CellData::Str(v) => v.bytes(),
            CellData::StrView { parent, off, len, .. } => {
                let p = cell_of(*parent).as_bytes();
                &p[*off as usize..*off as usize + *len as usize]
            }
            _ => &[],
        }
    }
    /// The number of `char`s: the byte length when all-ASCII (the flag is
    /// computed once at allocation), else a UTF-8 scan.
    pub fn char_len(&self) -> usize {
        match &self.data {
            CellData::Str(v) if v.ascii => v.len as usize,
            CellData::StrView { len, ascii, .. } if *ascii => *len as usize,
            CellData::Str(_) | CellData::StrView { .. } => self.as_str().chars().count(),
            _ => 0,
        }
    }
    /// True when this `str` is all-ASCII, so a char index is a byte index.
    pub fn str_ascii(&self) -> bool {
        match &self.data {
            CellData::Str(v) => v.ascii,
            CellData::StrView { ascii, .. } => *ascii,
            _ => false,
        }
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
    /// The raw octets of a `bytes` cell WITHOUT copying — the block
    /// store's own slice (the borrowed key probe, nmap-borrow-probe);
    /// empty for any other shape. Same lifetime contract as
    /// `as_str`/`as_bytes`: blocks never move, and a crossing's arg
    /// register retains the cell for the call's scope.
    pub fn bytes_view(&self) -> &[u8] {
        match &self.data {
            CellData::Str(v) => v.bytes(),
            CellData::Array { items, .. } if items.borrow().kind == ArrKind::U8 => {
                let d = items.borrow();
                // SAFETY: the block lives as long as the cell (the same
                // contract boundary's `&[u8]` crossing runs on)
                unsafe { std::slice::from_raw_parts(d.block, d.len as usize) }
            }
            _ => &[],
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
