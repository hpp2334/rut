//! RutType descriptors — every value's type exists at runtime;
//! kind + field tables drive `is`, dispatch, and serialization.
//! Type ids are program-global (single-module link in v1; rebase lands with multi-module linking).

pub type TypeId = u32;

use crate::id::{local_of, pack, scope_of, ScopeId, BOOT_SCOPE};
use crate::sym::{self, IdentId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimTy {
    U8, U16, U32, U64,
    I8, I16, I32, I64,
    F32, F64,
    Bool,
}

impl PrimTy {
    pub fn is_int(self) -> bool {
        !matches!(self, PrimTy::F32 | PrimTy::F64 | PrimTy::Bool)
    }
    pub fn is_float(self) -> bool {
        matches!(self, PrimTy::F32 | PrimTy::F64)
    }
    pub fn is_unsigned(self) -> bool {
        matches!(self, PrimTy::U8 | PrimTy::U16 | PrimTy::U32 | PrimTy::U64)
    }
    pub fn name(self) -> &'static str {
        match self {
            PrimTy::U8 => "u8", PrimTy::U16 => "u16", PrimTy::U32 => "u32", PrimTy::U64 => "u64",
            PrimTy::I8 => "i8", PrimTy::I16 => "i16", PrimTy::I32 => "i32", PrimTy::I64 => "i64",
            PrimTy::F32 => "f32", PrimTy::F64 => "f64",
            PrimTy::Bool => "bool",
        }
    }
    /// Stable wire code for the typed bytecode — the VM no
    /// longer needs the type table to execute a scalar op.
    pub fn to_u8(self) -> u8 {
        match self {
            PrimTy::U8 => 0, PrimTy::U16 => 1, PrimTy::U32 => 2, PrimTy::U64 => 3,
            PrimTy::I8 => 4, PrimTy::I16 => 5, PrimTy::I32 => 6, PrimTy::I64 => 7,
            PrimTy::F32 => 8, PrimTy::F64 => 9, PrimTy::Bool => 10,
        }
    }
    pub fn from_u8(b: u8) -> Option<PrimTy> {
        Some(match b {
            0 => PrimTy::U8, 1 => PrimTy::U16, 2 => PrimTy::U32, 3 => PrimTy::U64,
            4 => PrimTy::I8, 5 => PrimTy::I16, 6 => PrimTy::I32, 7 => PrimTy::I64,
            8 => PrimTy::F32, 9 => PrimTy::F64, 10 => PrimTy::Bool,
            // 11 is RETIRED (the char exorcism): a stale artifact's char
            // prim tag lands in the `_ => None` arm and fails decode loudly
            _ => return None,
        })
    }

    /// Machine payload width in bytes — the primitive-optional element
    /// store's payload region rides this, as does the
    /// packed-array kind table in the VM heap.
    pub fn width(self) -> usize {
        match self {
            PrimTy::U8 | PrimTy::I8 | PrimTy::Bool => 1,
            PrimTy::U16 | PrimTy::I16 => 2,
            PrimTy::U32 | PrimTy::I32 | PrimTy::F32 => 4,
            PrimTy::U64 | PrimTy::I64 | PrimTy::F64 => 8,
        }
    }
}

/// Compact runtime representation of a value, resolved from its static
/// type at compile time and baked into ops ("the full table is
/// mechanical"). `Prim` carries the scalar machine kind, `Ref` marks a cell
/// handle needing retain/release, and `Any` is the conservative fallback
/// (untyped registers, nil, fn values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repr {
    Prim(PrimTy),
    Ref,
    Any,
    /// Array-element representation of a `[?prim]` backing (the
    /// primitive-optional store): the element lives in the block as a raw
    /// payload plus a nil tag — never as a cell. `ArrGet` yields a FRESH opt
    /// value (a minted one-slot cell the destination register owns);
    /// `ArrSet` takes a proper `?prim` value (cell or null) and encodes it
    /// into the raw store. No slot of this repr is retained or released by
    /// the element ops (the primitive-store tier).
    OptPrim(PrimTy),
    /// `ArrSet` form after the MakeOpt elision: `val` holds the RAW payload
    /// (the some-tag is implied by the op). The peephole folds
    /// `MakeOpt + ArrSet{OptPrim}` into this when the box feeds only the
    /// store — the box's identity is unobservable for a primitive payload
    /// (`?T == ?T` compares payloads).
    OptPrimRaw(PrimTy),
    /// `ArrGet` form after the deref fold: yields the RAW payload directly,
    /// with the nil tag trapping `NilDeref` — exactly the behavior of the
    /// `ArrGet + GetF{field 0}` pair it replaces, minus the mint.
    OptPrimLoad(PrimTy),
}

/// Wire-code bases for the opt-prim element forms: `base + PrimTy::to_u8`.
impl Repr {
    /// Wire codes for the non-primitive variants (primitive codes come from
    /// `PrimTy::to_u8`).
    pub const REF_CODE: u8 = 12;
    pub const ANY_CODE: u8 = 13;
    const OPT_PRIM_BASE: u8 = 14;
    const OPT_PRIM_RAW_BASE: u8 = 26;
    const OPT_PRIM_LOAD_BASE: u8 = 38;

    pub fn to_u8(self) -> u8 {
        match self {
            Repr::Prim(p) => p.to_u8(),
            Repr::Ref => Self::REF_CODE,
            Repr::Any => Self::ANY_CODE,
            Repr::OptPrim(p) => Self::OPT_PRIM_BASE + p.to_u8(),
            Repr::OptPrimRaw(p) => Self::OPT_PRIM_RAW_BASE + p.to_u8(),
            Repr::OptPrimLoad(p) => Self::OPT_PRIM_LOAD_BASE + p.to_u8(),
        }
    }

    pub fn from_u8(b: u8) -> Option<Repr> {
        match b {
            Self::REF_CODE => Some(Repr::Ref),
            Self::ANY_CODE => Some(Repr::Any),
            _ if (Self::OPT_PRIM_BASE..Self::OPT_PRIM_RAW_BASE).contains(&b) => {
                PrimTy::from_u8(b - Self::OPT_PRIM_BASE).map(Repr::OptPrim)
            }
            _ if (Self::OPT_PRIM_RAW_BASE..Self::OPT_PRIM_LOAD_BASE).contains(&b) => {
                PrimTy::from_u8(b - Self::OPT_PRIM_RAW_BASE).map(Repr::OptPrimRaw)
            }
            _ if b >= Self::OPT_PRIM_LOAD_BASE => {
                PrimTy::from_u8(b - Self::OPT_PRIM_LOAD_BASE).map(Repr::OptPrimLoad)
            }
            _ => PrimTy::from_u8(b).map(Repr::Prim),
        }
    }

    /// True when a slot of this representation is a retained cell handle.
    /// The opt-prim element forms are NOT: the raw store holds payloads and
    /// tags, and the ops that carry these reprs own their (non-)rc discipline
    /// explicitly.
    pub fn is_ref(self) -> bool {
        matches!(self, Repr::Ref)
    }

    /// The primitive payload of an opt-prim element form, if this is one.
    pub fn opt_prim(self) -> Option<PrimTy> {
        match self {
            Repr::OptPrim(p) | Repr::OptPrimRaw(p) | Repr::OptPrimLoad(p) => Some(p),
            _ => None,
        }
    }
}

/// The element representation an ARRAY op bakes for `elem` (the
/// primitive-store tier): a `[?prim]` backing stores raw payloads + nil
/// tags, so its element ops carry the `OptPrim` form; every other element
/// keeps the plain `repr_of` (reference payloads stay cell-backed slots).
/// The verifier checks array-op reprs against THIS resolution, and the
/// deref-fold/elision peepholes may further refine GETs to `OptPrimLoad` and
/// SETs to `OptPrimRaw` (same payload prim — the verifier accepts the
/// refinement, never a payload change).
pub fn arr_elem_repr(table: &TypeTable, elem: TypeId) -> Repr {
    if let TyKind::Opt { elem: inner } = table.kind(elem) {
        if let TyKind::Prim(p) = table.kind(*inner) {
            return Repr::OptPrim(*p);
        }
    }
    table.repr_of(elem)
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldInfo {
    pub name: IdentId,
    pub ty: TypeId,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TyKind {
    Nil,
    Prim(PrimTy),
    /// immutable UTF-8 string cell
    Str,
    /// immutable raw byte buffer cell (the language's binary
    /// data type); contiguous, content-compared, COW-shared like `Str`
    Bytes,
    /// heap array cell — runtime length, non-growable. The
    /// growable `Vec<T>` is a rut class over it (`pouch`).
    Array { elem: TypeId },
    /// named-int set; members are immortal singleton cells
    Enum { members: Vec<(IdentId, i64)> },
    /// struct or class record cell — fields stored as one slot each;
    /// construction rules differ, representation does not
    Data { fields: Vec<FieldInfo> },
    /// trait object (`i: I`) — unsized object; the slot stores the cell handle and the
    /// cell's own type reaches the vtable
    TraitObj { trait_id: u32 },
    /// erasure box
    Opaque,
    /// engine stack-trace snapshot — the
    /// `StackTrace` builtin class's runtime type: a stateful engine
    /// snapshot with methods, never a value spelling (that is why this is
    /// a `builtin class` and not a `builtin primitive`). The cell carries
    /// the RAW captured frames; symbolication is lazy, per member access.
    Trace,
    /// the disposal drain's minted context cell — the `DisposalContext`
    /// builtin class's runtime type. The engine mints one per `dispose`
    /// call; a stateless engine cell today (empty surface), shaped to
    /// grow without touching the trait's signature. A ref-repr cell like
    /// `Trace`: assignment shares it.
    DisposalContext,
    /// `?T` — a nil-able cell; `nil` is the null slot.
    /// Same one-slot box the old `*T` pointer was: `T → ?T` boxes, `?T → T`
    /// reads field 0 (nil check on use)
    Opt { elem: TypeId },
    /// weak reference — the `Weak<T>` builtin class's
    /// runtime type: a WeakBox side cell holding an UNRETAINED slot word
    /// to the referent, nulled when the referent dies. Generic like
    /// `Array { elem }`: interned per instantiation (`mk_weak`), no boot
    /// row. A ref type by the `is_ref` law (the box is a cell — the
    /// referent it holds is not).
    Weak { elem: TypeId },
    /// fn(P..) -> R — a closure value { func, captures } in one slot
    Fn { params: Vec<TypeId>, ret: TypeId },
}

#[derive(Clone, Debug, PartialEq)]
pub struct RutType {
    pub name: IdentId,
    pub kind: TyKind,
}

#[derive(Clone, Debug, Default)]
pub struct TypeTable {
    pub types: Vec<RutType>,
    /// When set, NEW rows intern into the OWN scope directly
    /// (`pack(scope, dense - own_base)`), never attributed to a foreign
    /// block that happens to sit above the own block (a reseed's seed
    /// rows do exactly that). The fresh compile leaves it off — its own
    /// block is the topmost anyway.
    pub own_attribution: bool,
    /// packed-id mode (compiler): ids are `(scope, local)`; `scope_base[s]`
    /// is the dense index where scope `s`'s block starts
    pub scope_base: Vec<u32>,
    /// the scope this table interns into (packed mode)
    pub scope: ScopeId,
    /// number of shared boot types (the dense prefix)
    pub boot_len: u32,
    /// `true` while ids are `(scope, local)` (compiler); `false` when ids
    /// are dense indices (post-link / VM)
    pub packed: bool,
}

pub const TY_NIL: TypeId = 0;
pub const TY_U8: TypeId = 1;
pub const TY_U16: TypeId = 2;
pub const TY_U32: TypeId = 3;
pub const TY_U64: TypeId = 4;
pub const TY_I8: TypeId = 5;
pub const TY_I16: TypeId = 6;
pub const TY_I32: TypeId = 7;
pub const TY_I64: TypeId = 8;
pub const TY_F32: TypeId = 9;
pub const TY_F64: TypeId = 10;
pub const TY_BOOL: TypeId = 11;
/// RESERVED (the char exorcism): the boot row at this id is a `Nil` shell —
/// the enumerated `char` kind is gone, and the id is kept only so the
/// wire-stable boot ids above it never move.
pub const TY_CHAR: TypeId = 12;
pub const TY_STR: TypeId = 13;
pub const TY_OPAQUE: TypeId = 14;
/// immutable binary buffer — appended after `Opaque`; fixed ids
/// are wire-stable and must never be reordered
pub const TY_BYTES: TypeId = 15;
/// the `StackTrace` snapshot — appended
/// after `Bytes`; fixed ids are wire-stable and must never be reordered
pub const TY_STACK_TRACE: TypeId = 16;
/// RESERVED (the host strbuild pkg): the growable string builder's boot
/// row — the engine surface is GONE (the builder is a host package over
/// `strbuild_host::*`), but the ROW stays so every id above it keeps its
/// wire-stable boot position (the fixed ids must never be reordered);
/// nothing reaches it — the resolver's removed-core check fires on the
/// NAME long before any id could.
pub const TY_STRBUF: TypeId = 17;
/// the `?str` answer lane (the legal-host-returns phase): the boot row
/// `Opt { elem: TY_STR }` a `.d.rut` `-> ?str` return maps to — the
/// fixed id the registry's `Option<String>` binding verifies against.
/// Appended after the RESERVED row 18 (the retired `any` lane, below);
/// fixed ids are wire-stable and must never be reordered.
pub const TY_OPT_STR: TypeId = 19;
/// the `?bytes` answer lane (the legal-host-returns phase): `Opt { elem: TY_BYTES }`.
pub const TY_OPT_BYTES: TypeId = 20;
/// the `?opaque` answer lane (the legal-host-returns phase): `Opt { elem: TY_OPAQUE }`.
pub const TY_OPT_OPAQUE: TypeId = 21;
/// the `DisposalContext` builtin class (the disposal surface) — appended
/// after the host-answer optionals; fixed ids are wire-stable and must
/// never be reordered
pub const TY_DISPOSAL_CONTEXT: TypeId = 22;
/// A host payload box's runtime type as `TidOf` reports it:
/// the payload is Rust, so no rut type describes it. Type ids are type-table
/// indices, which can never reach this value — `downcast<T>` therefore
/// compares false for every `T` and yields `nil`, never a trap.
pub const HOST_BOX_TID: TypeId = u32::MAX;

impl TypeTable {
    /// Boot table (dense ids): primitives + string + Opaque/Bytes at fixed ids.
    pub fn boot() -> TypeTable {
        Self::boot_impl(false, BOOT_SCOPE)
    }

    /// Boot table whose ids are `(scope, local)` for a compiler module.
    pub fn boot_scoped(scope: ScopeId) -> TypeTable {
        Self::boot_impl(true, scope)
    }

    fn boot_impl(packed: bool, scope: ScopeId) -> TypeTable {
        let mut t = TypeTable { packed, scope, ..Default::default() };
        let mut push = |name: IdentId, kind: TyKind| {
            t.types.push(RutType { name, kind });
        };
        // boot names are well-known symbols — fixed ids, no interner needed
        push(sym::NIL, TyKind::Nil);
        push(sym::U8, TyKind::Prim(PrimTy::U8));
        push(sym::U16, TyKind::Prim(PrimTy::U16));
        push(sym::U32, TyKind::Prim(PrimTy::U32));
        push(sym::U64, TyKind::Prim(PrimTy::U64));
        push(sym::I8, TyKind::Prim(PrimTy::I8));
        push(sym::I16, TyKind::Prim(PrimTy::I16));
        push(sym::I32, TyKind::Prim(PrimTy::I32));
        push(sym::I64, TyKind::Prim(PrimTy::I64));
        push(sym::F32, TyKind::Prim(PrimTy::F32));
        push(sym::F64, TyKind::Prim(PrimTy::F64));
        push(sym::BOOL, TyKind::Prim(PrimTy::Bool));
        // RESERVED (the char exorcism): `char` is gone — the
        // enumerated kind died with `PrimTy::Char`. The ROW stays so every
        // id above it keeps its wire-stable boot position (the fixed ids
        // must never be reordered); nothing reaches it — the resolver's
        // removed-core check fires on the NAME long before any id could.
        push(sym::CHAR, TyKind::Nil);
        push(sym::STR, TyKind::Str);
        push(sym::OPAQUE, TyKind::Opaque);
        push(sym::BYTES, TyKind::Bytes);
        push(sym::STACK_TRACE, TyKind::Trace);
        // RESERVED (the host strbuild pkg, the TY_CHAR convention): the
        // builder's engine surface is GONE — the kind is deleted and the
        // removed-core check diagnoses the NAME. The ROW stays so every
        // id above it keeps its wire-stable boot position; nothing
        // reaches it. The name keeps its well-known symbol (the
        // removed-name lookup spells it).
        push(sym::STRBUF, TyKind::Nil);
        // RESERVED (the any-lane removal, the TY_CHAR convention): the
        // `any` crossing row (nmap-hostvals P3) is GONE from the surface
        // — the lexer refuses the spelling in every mode, the decl
        // grammar has no `any` arm, and nothing maps to this id. The ROW
        // stays so every id above it keeps its wire-stable boot position
        // (the fixed ids must never be reordered); nothing reaches it.
        // The name rides the shell convention (sym::NIL).
        push(sym::NIL, TyKind::Nil);
        // the host-answer optionals (the legal-host-returns phase): REAL
        // crossing rows — `?str`/`?bytes`/`?opaque` cross back
        // nil-flattened (the optionals law, the read direction
        // always promised). The decl surface's `-> ?T` spellings map to
        // the CONSTS above (the driver's answer table), and the registry's
        // `Option<T>` SIG shares them, so the boot join verifies the
        // pair by identity. The row names ride the shell convention (the
        // elem's own symbol — the reserved-row convention); a diagnostic
        // that needs the `?` shape names it from the CONST.
        push(sym::STR, TyKind::Opt { elem: TY_STR });
        push(sym::BYTES, TyKind::Opt { elem: TY_BYTES });
        push(sym::OPAQUE, TyKind::Opt { elem: TY_OPAQUE });
        // the disposal drain's context cell (the disposal surface): the
        // engine mints one per `dispose` call; nothing constructs it
        push(sym::DISPOSAL_CONTEXT, TyKind::DisposalContext);
        t.boot_len = t.types.len() as u32;
        t.scope_base = vec![0; scope as usize + 1];
        if packed {
            t.scope_base[scope as usize] = t.boot_len;
        }
        t
    }

    /// Dense index of an id (decodes `(scope, local)` in packed mode).
    #[inline]
    pub fn dense(&self, id: TypeId) -> u32 {
        if !self.packed {
            return id;
        }
        let base = self.scope_base.get(scope_of(id) as usize).copied().unwrap_or(0);
        base + local_of(id)
    }

    /// Pack a dense index into this table's id space. Handles multi-scope
    /// tables: used blocks keep the scope they were declared under.
    #[inline]
    /// The packed id for a dense index (the pub twin of [`Self::id_for`],
    /// for the driver's seed-region registration).
    pub fn id_for_pub(&self, dense: u32) -> TypeId {
        self.id_for(dense)
    }

    /// [`Self::id_for_pub`] without the trap: `None` when the dense
    /// index sits below its scope's registered base (a row stranded by a
    /// later base move — the seed attach re-registers requester scopes).
    pub fn id_for_pub_checked(&self, dense: u32) -> Option<TypeId> {
        if dense < self.boot_len {
            return Some(pack(BOOT_SCOPE, dense));
        }
        let s = self.scope_of_dense(dense);
        let base = self.scope_base.get(s as usize).copied().unwrap_or(0);
        if base > dense {
            return None;
        }
        Some(pack(s, dense - base))
    }

    fn id_for(&self, dense: u32) -> TypeId {
        if !self.packed {
            return dense;
        }
        if dense < self.boot_len {
            return pack(BOOT_SCOPE, dense);
        }
        let s = self.scope_of_dense(dense);
        let base = self.scope_base.get(s as usize).copied().unwrap_or(0);
        pack(s, dense - base)
    }

    /// Which scope's block contains a non-boot dense index: the greatest
    /// registered block start `<= dense`. The OWN block wins a tie at the
    /// top base (a used block registered empty ties the own block's
    /// base, and the rows interned after it are this table's own); a
    /// query below the own block's start still answers with the foreign
    /// block it lands in.
    fn scope_of_dense(&self, dense: u32) -> ScopeId {
        if dense < self.boot_len {
            return BOOT_SCOPE;
        }
        let mut best: Option<(u32, ScopeId)> = None;
        let mut own_at_best = false;
        for s in 0..self.scope_base.len() as u32 {
            if s == BOOT_SCOPE as u32 {
                continue;
            }
            let b = self.scope_base[s as usize];
            if b > dense {
                continue;
            }
            match best {
                Some((bb, _)) if b < bb => {}
                Some((bb, _)) if b == bb => own_at_best |= s as ScopeId == self.scope,
                _ => {
                    best = Some((b, s as ScopeId));
                    own_at_best = s as ScopeId == self.scope;
                }
            }
        }
        match best {
            Some((_, s)) if own_at_best => self.scope,
            Some((_, s)) => s,
            None => self.scope,
        }
    }

    /// Use another module's type descriptors. `blocks` gives each scope's
    /// block start as a local offset inside `descs`; the own block is moved to
    /// the end so later [`TypeTable::intern`] calls append after the uses.
    /// Must run before any own type is interned.
    pub fn use_block(&mut self, descs: Vec<crate::types::RutType>, blocks: &[(ScopeId, u32)]) {
        if !self.packed {
            return;
        }
        let start = self.types.len() as u32;
        self.types.extend(descs);
        for (s, off) in blocks {
            let need = *s as usize + 1;
            if self.scope_base.len() < need {
                self.scope_base.resize(need, 0);
            }
            self.scope_base[*s as usize] = start + *off;
        }
        let need = self.scope as usize + 1;
        if self.scope_base.len() < need {
            self.scope_base.resize(need, 0);
        }
        self.scope_base[self.scope as usize] = self.types.len() as u32;
    }

    /// [`Self::use_block`] without the own-base tail: the seed rows a
    /// RESEED registers append after the unit's own rows already exist
    /// (the roots have compiled), so the own block's base must stay
    /// where it is — the owner's own ids spell through it.
    pub fn use_seed_rows(&mut self, descs: Vec<crate::types::RutType>, blocks: &[(ScopeId, u32)]) {
        if !self.packed {
            return;
        }
        let start = self.types.len() as u32;
        self.types.extend(descs);
        for (s, off) in blocks {
            let need = *s as usize + 1;
            if self.scope_base.len() < need {
                self.scope_base.resize(need, 0);
            }
            self.scope_base[*s as usize] = start + *off;
        }
    }

    /// Does this dense index sit in a REGISTERED block (the boot prefix,
    /// a used block, the seed region, or the own block)? A host module's
    /// surface appends rows WITHOUT a scope registration (`scope_base`
    /// stays 0 there), and a spelling that deduped onto such a row would
    /// mint an id resolving through the 0-base trap — a different row in
    /// every other table, and nothing at link.
    fn dense_is_registered(&self, i: u32) -> bool {
        if i < self.boot_len {
            return true;
        }
        for (s, &b) in self.scope_base.iter().enumerate() {
            if s == BOOT_SCOPE as usize || b == 0 || b > i {
                continue;
            }
            return true;
        }
        false
    }

    pub fn intern(&mut self, ty: RutType) -> TypeId {
        // structural interning for anonymous instantiations (Vec<T>, fn(..)...)
        // — names are IdentIds, so the dedup key compare is an integer compare
        let own_base = self
            .scope_base
            .get(self.scope as usize)
            .copied()
            .unwrap_or(self.boot_len);
        for (i, t) in self.types.iter().enumerate() {
            if t.name == ty.name && t.kind == ty.kind && self.dense_is_registered(i as u32) {
                // under own attribution, a hit ABOVE the own block (a
                // reseed's seed region) re-spells into the own scope:
                // the row is this unit's compilation's, and the linked
                // table appends everything from the own base upward as
                // this unit's block
                if self.own_attribution && i as u32 >= own_base {
                    return pack(self.scope, i as u32 - own_base);
                }
                return self.id_for(i as u32);
            }
        }
        let d = self.types.len() as u32;
        self.types.push(ty);
        if self.own_attribution {
            return pack(self.scope, d - own_base);
        }
        self.id_for(d)
    }

    /// Intern into THIS table's own block, never deduplicating against a
    /// used block's row: a `#param` placeholder (the generic templates'
    /// field leaves) must live in the own scope, because the template's
    /// field descriptors reference it and the unit's own compilation —
    /// splice composition included — must own the row. A used block
    /// happens to carry an equal-looking placeholder (the exporter's own
    /// template), and a plain structural hit would graft the template's
    /// fields onto a foreign block the program may never link.
    pub fn intern_own(&mut self, ty: RutType) -> TypeId {
        let own_start = self
            .scope_base
            .get(self.scope as usize)
            .copied()
            .unwrap_or(self.boot_len) as usize;
        for (i, t) in self.types.iter().enumerate().skip(own_start) {
            if t.name == ty.name && t.kind == ty.kind {
                return self.id_for(i as u32);
            }
        }
        let d = self.types.len() as u32;
        self.types.push(ty);
        if self.own_attribution {
            let own_base = self
                .scope_base
                .get(self.scope as usize)
                .copied()
                .unwrap_or(self.boot_len);
            return pack(self.scope, d - own_base);
        }
        self.id_for(d)
    }

    /// The id of the FIRST row named `name` (the engine singleton
    /// lookup — `RunContext`): None when the unit has not minted it.
    pub fn dense_id_of_name(&self, name: IdentId) -> Option<TypeId> {
        self.types
            .iter()
            .position(|t| t.name == name)
            .map(|i| self.id_for(i as u32))
    }

    /// The descriptor at `id`.
    #[inline]
    pub fn type_at(&self, id: TypeId) -> &RutType {
        &self.types[self.dense(id) as usize]
    }
    pub fn kind(&self, id: TypeId) -> &TyKind {
        &self.types[self.dense(id) as usize].kind
    }
    pub fn is_prim(&self, id: TypeId) -> Option<PrimTy> {
        match self.kind(id) {
            TyKind::Prim(p) => Some(*p),
            _ => None,
        }
    }
    /// True when slots of this type are cell handles.
    pub fn is_ref(&self, id: TypeId) -> bool {
        !matches!(
            self.kind(id),
            TyKind::Nil | TyKind::Prim(_) | TyKind::Fn { .. }
        )
    }

    /// True when the type is a MUTABLE VALUE. Nothing is (the
    /// by-reference regime): every cell type — records, arrays, enums,
    /// `str`/`bytes`, closures, `?T` — shares its cell on assignment and
    /// parameter passing; only primitives and `fn` values copy (immediate
    /// slots). Kept as a predicate because the VM's deep-copy paths
    /// (`own`) still branch on it, and the answer is now uniformly "no".
    pub fn is_value(&self, _id: TypeId) -> bool {
        false
    }

    /// The baked runtime representation of a type (see [`Repr`]).
    pub fn repr_of(&self, id: TypeId) -> Repr {
        match self.kind(id) {
            TyKind::Prim(p) => Repr::Prim(*p),
            // nil is a zero slot; fn values are closure cells the compiler
            // owns (v1 captures by value), so neither takes RC traffic
            TyKind::Nil | TyKind::Fn { .. } => Repr::Any,
            _ => Repr::Ref,
        }
    }

    /// The crossing rule: the value shapes a
    /// host may hold and pass back. `entry fn` is the host-callable surface,
    /// so **host functions obey the same rule** — primitives, `str`, `nil`,
    /// `bytes`, `?T` over crossable, anonymous tuples of crossable types,
    /// and `opaque`. Named records (user structs/classes) never cross. The
    /// row carries named-ness, so the caller's interner rides along: a
    /// minted tuple row's name is its `(T, ..)` spelling (the `mk_tuple`
    /// convention), a declared struct/class row has a real ident.
    pub fn crosses_boundary(&self, names: &sym::Interner, id: TypeId) -> bool {
        match self.kind(id) {
            TyKind::Nil | TyKind::Prim(_) | TyKind::Str | TyKind::Bytes | TyKind::Opaque => true,
            // `?T` crosses nil-flattened when T crosses (the
            // promise: optionals cross as the tuples they are always
            // spelled as — the implementation only predates the text).
            // No err carve-out here: the nullable's element answers the
            // rule, the tuple it rides is just a tuple.
            TyKind::Opt { elem } => self.crosses_boundary(names, *elem),
            // ANONYMOUS tuples cross field-by-field — the row name is
            // the minted `(T, ..)` spelling. A named record of perfectly
            // crossable fields still does not cross: the arm never
            // recurses into it.
            TyKind::Data { fields } => {
                names.name(self.type_at(id).name).starts_with('(')
                    && fields.iter().all(|f| self.crosses_boundary(names, f.ty))
            }
            _ => false,
        }
    }
}
