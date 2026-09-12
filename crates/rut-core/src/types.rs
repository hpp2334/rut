//! RutType descriptors — RFC 0015: every value's type exists at runtime;
//! kind + field tables drive `is`, dispatch, and serialization.
//! Type ids are program-global (single-module link in v1; RFC 0035 §1
//! rebase lands with multi-module linking).

pub type TypeId = u32;

use crate::id::{local_of, pack, scope_of, ScopeId, BOOT_SCOPE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimTy {
    U8, U16, U32, U64,
    I8, I16, I32, I64,
    F32, F64,
    Bool, Char,
}

impl PrimTy {
    pub fn is_int(self) -> bool {
        !matches!(self, PrimTy::F32 | PrimTy::F64 | PrimTy::Bool | PrimTy::Char)
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
            PrimTy::Bool => "bool", PrimTy::Char => "char",
        }
    }
    /// Stable wire code for the typed bytecode (RFC 0032) — the VM no
    /// longer needs the type table to execute a scalar op.
    pub fn to_u8(self) -> u8 {
        match self {
            PrimTy::U8 => 0, PrimTy::U16 => 1, PrimTy::U32 => 2, PrimTy::U64 => 3,
            PrimTy::I8 => 4, PrimTy::I16 => 5, PrimTy::I32 => 6, PrimTy::I64 => 7,
            PrimTy::F32 => 8, PrimTy::F64 => 9, PrimTy::Bool => 10, PrimTy::Char => 11,
        }
    }
    pub fn from_u8(b: u8) -> Option<PrimTy> {
        Some(match b {
            0 => PrimTy::U8, 1 => PrimTy::U16, 2 => PrimTy::U32, 3 => PrimTy::U64,
            4 => PrimTy::I8, 5 => PrimTy::I16, 6 => PrimTy::I32, 7 => PrimTy::I64,
            8 => PrimTy::F32, 9 => PrimTy::F64, 10 => PrimTy::Bool, 11 => PrimTy::Char,
            _ => return None,
        })
    }
}

/// Compact runtime representation of a value, resolved from its static
/// type at compile time and baked into ops (RFC 0032 "the full table is
/// mechanical"). `Prim` carries the scalar machine kind, `Ref` marks a cell
/// handle needing retain/release, and `Any` is the conservative fallback
/// (untyped registers, unit, fn values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repr {
    Prim(PrimTy),
    Ref,
    Any,
}

impl Repr {
    /// Wire codes for the non-primitive variants (primitive codes come from
    /// `PrimTy::to_u8`).
    pub const REF_CODE: u8 = 12;
    pub const ANY_CODE: u8 = 13;

    pub fn to_u8(self) -> u8 {
        match self {
            Repr::Prim(p) => p.to_u8(),
            Repr::Ref => Self::REF_CODE,
            Repr::Any => Self::ANY_CODE,
        }
    }

    pub fn from_u8(b: u8) -> Option<Repr> {
        match b {
            Self::REF_CODE => Some(Repr::Ref),
            Self::ANY_CODE => Some(Repr::Any),
            _ => PrimTy::from_u8(b).map(Repr::Prim),
        }
    }

    /// True when a slot of this representation is a retained cell handle.
    pub fn is_ref(self) -> bool {
        matches!(self, Repr::Ref)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldInfo {
    pub name: String,
    pub ty: TypeId,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TyKind {
    Unit,
    Prim(PrimTy),
    /// immutable UTF-8 string cell
    Str,
    /// immutable raw byte buffer cell (RFC 0004 — the language's binary
    /// data type); contiguous, content-compared, COW-shared like `Str`
    Bytes,
    /// heap array cell — runtime length, non-growable (RFC 0005). The
    /// growable `Vec<T>` is a rut class over it (`std:collection`).
    Array { elem: TypeId },
    /// named-int set (RFC 0006); members are immortal singleton cells
    Enum { members: Vec<(String, i64)> },
    /// builtin sum (RFC 0005): tag 0 = some/ok, 1 = none/err
    Option { elem: TypeId },
    Result { ok: TypeId, err: TypeId },
    /// dataclass or class record cell — fields stored as one slot each
    /// (RFC 0009/0010); construction rules differ, representation does not
    Data { fields: Vec<FieldInfo> },
    /// `dyn I` — unsized object; the slot stores the cell handle and the
    /// cell's own type reaches the vtable (RFC 0015 §6)
    TraitObj { trait_id: u32 },
    /// erasure box (RFC 0014)
    Opaque,
    /// fn(P..) -> R — a closure value { func, captures } in one slot
    Fn { params: Vec<TypeId>, ret: TypeId },
}

#[derive(Clone, Debug)]
pub struct RutType {
    pub name: String,
    pub kind: TyKind,
}

#[derive(Clone, Debug, Default)]
pub struct TypeTable {
    pub types: Vec<RutType>,
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

pub const TY_UNIT: TypeId = 0;
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
pub const TY_CHAR: TypeId = 12;
pub const TY_STR: TypeId = 13;
pub const TY_OPAQUE: TypeId = 14;
/// immutable binary buffer (RFC 0004) — appended after `Opaque`; fixed ids
/// are wire-stable and must never be reordered
pub const TY_BYTES: TypeId = 15;

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
        let mut push = |name: &str, kind: TyKind| {
            t.types.push(RutType {
                name: name.to_string(),
                kind,
            });
        };
        push("unit", TyKind::Unit);
        push("u8", TyKind::Prim(PrimTy::U8));
        push("u16", TyKind::Prim(PrimTy::U16));
        push("u32", TyKind::Prim(PrimTy::U32));
        push("u64", TyKind::Prim(PrimTy::U64));
        push("i8", TyKind::Prim(PrimTy::I8));
        push("i16", TyKind::Prim(PrimTy::I16));
        push("i32", TyKind::Prim(PrimTy::I32));
        push("i64", TyKind::Prim(PrimTy::I64));
        push("f32", TyKind::Prim(PrimTy::F32));
        push("f64", TyKind::Prim(PrimTy::F64));
        push("bool", TyKind::Prim(PrimTy::Bool));
        push("char", TyKind::Prim(PrimTy::Char));
        push("string", TyKind::Str);
        push("Opaque", TyKind::Opaque);
        push("bytes", TyKind::Bytes);
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
    /// tables: imported blocks keep the scope they were declared under.
    #[inline]
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

    /// Which scope's block contains a non-boot dense index (the greatest
    /// registered block start `<= dense`).
    fn scope_of_dense(&self, dense: u32) -> ScopeId {
        if dense < self.boot_len {
            return BOOT_SCOPE;
        }
        let mut best = self.scope;
        let mut best_base = 0u32;
        for s in 0..self.scope_base.len() as u32 {
            if s == BOOT_SCOPE as u32 {
                continue;
            }
            let b = self.scope_base[s as usize];
            if b <= dense && b >= best_base {
                best = s as ScopeId;
                best_base = b;
            }
        }
        best
    }

    /// Import another module's type descriptors. `blocks` gives each scope's
    /// block start as a local offset inside `descs`; the own block is moved to
    /// the end so later [`TypeTable::intern`] calls append after the imports.
    /// Must run before any own type is interned.
    pub fn import_block(&mut self, descs: Vec<crate::types::RutType>, blocks: &[(ScopeId, u32)]) {
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

    pub fn intern(&mut self, ty: RutType) -> TypeId {
        // structural interning for anonymous instantiations (Vec<T>, Option<T>...)
        for (i, t) in self.types.iter().enumerate() {
            if t.name == ty.name && t.kind == ty.kind {
                return self.id_for(i as u32);
            }
        }
        let d = self.types.len() as u32;
        self.types.push(ty);
        self.id_for(d)
    }

    /// The descriptor at `id`.
    #[inline]
    pub fn type_at(&self, id: TypeId) -> &RutType {
        &self.types[self.dense(id) as usize]
    }
    pub fn kind(&self, id: TypeId) -> &TyKind {
        &self.types[self.dense(id) as usize].kind
    }
    pub fn name(&self, id: TypeId) -> &str {
        &self.types[self.dense(id) as usize].name
    }
    pub fn is_prim(&self, id: TypeId) -> Option<PrimTy> {
        match self.kind(id) {
            TyKind::Prim(p) => Some(*p),
            _ => None,
        }
    }
    /// True when slots of this type are cell handles (RFC 0016 §1).
    pub fn is_ref(&self, id: TypeId) -> bool {
        !matches!(
            self.kind(id),
            TyKind::Unit | TyKind::Prim(_) | TyKind::Fn { .. }
        )
    }

    /// The baked runtime representation of a type (see [`Repr`]).
    pub fn repr_of(&self, id: TypeId) -> Repr {
        match self.kind(id) {
            TyKind::Prim(p) => Repr::Prim(*p),
            // unit is a zero slot; fn values are closure cells the compiler
            // owns (v1 captures by value), so neither takes RC traffic
            TyKind::Unit | TyKind::Fn { .. } => Repr::Any,
            _ => Repr::Ref,
        }
    }
}
