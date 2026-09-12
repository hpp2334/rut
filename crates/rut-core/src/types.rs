//! RutType descriptors — RFC 0015: every value's type exists at runtime;
//! kind + width + field tables drive layout, `is`, and serialization.
//! Type ids are program-global (single-module link in v1; RFC 0035 §1
//! rebase lands with multi-module linking).

pub type TypeId = u32;

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
    /// Slot width in bytes (RFC 0015 §5 — all slots are 8 bytes; this is
    /// the *semantic* width used by truncating ops).
    pub fn width(self) -> u32 {
        match self {
            PrimTy::U8 | PrimTy::I8 => 1,
            PrimTy::U16 | PrimTy::I16 => 2,
            PrimTy::U32 | PrimTy::I32 | PrimTy::F32 => 4,
            _ => 8,
        }
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
    /// repr-C offset of the field inside the payload block (RFC 0015 §4)
    pub offset: u32,
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
    /// growable buffer cell; flat for primitive elem, handle slots otherwise
    Vec { elem: TypeId },
    /// heap array cell — runtime length, non-growable (RFC 0005). The
    /// growable `Vec<T>` is a rut class over it (`std:collection`).
    Array { elem: TypeId },
    /// named-int set (RFC 0006); members are immortal singleton cells
    Enum { members: Vec<(String, i64)> },
    /// builtin sum (RFC 0005): tag 0 = some/ok, 1 = none/err
    Option { elem: TypeId },
    Result { ok: TypeId, err: TypeId },
    /// dataclass or class payload cell — same repr-C block either way
    /// (RFC 0009/0010); construction rules differ, layout does not
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
    /// repr-C value size (what own() clones; RFC 0015 §3)
    pub size: u32,
    pub align: u32,
}

#[derive(Clone, Debug, Default)]
pub struct TypeTable {
    pub types: Vec<RutType>,
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
    /// Boot table: primitives + string + Opaque at fixed ids.
    pub fn boot() -> TypeTable {
        let mut t = TypeTable { types: Vec::new() };
        let mut push = |name: &str, kind: TyKind, size: u32, align: u32| {
            t.types.push(RutType {
                name: name.to_string(),
                kind,
                size,
                align,
            });
        };
        push("unit", TyKind::Unit, 0, 1);
        push("u8", TyKind::Prim(PrimTy::U8), 1, 1);
        push("u16", TyKind::Prim(PrimTy::U16), 2, 2);
        push("u32", TyKind::Prim(PrimTy::U32), 4, 4);
        push("u64", TyKind::Prim(PrimTy::U64), 8, 8);
        push("i8", TyKind::Prim(PrimTy::I8), 1, 1);
        push("i16", TyKind::Prim(PrimTy::I16), 2, 2);
        push("i32", TyKind::Prim(PrimTy::I32), 4, 4);
        push("i64", TyKind::Prim(PrimTy::I64), 8, 8);
        push("f32", TyKind::Prim(PrimTy::F32), 4, 4);
        push("f64", TyKind::Prim(PrimTy::F64), 8, 8);
        push("bool", TyKind::Prim(PrimTy::Bool), 1, 1);
        push("char", TyKind::Prim(PrimTy::Char), 4, 4);
        push("string", TyKind::Str, 8, 8);
        push("Opaque", TyKind::Opaque, 8, 8);
        push("bytes", TyKind::Bytes, 8, 8);
        t
    }

    pub fn intern(&mut self, ty: RutType) -> TypeId {
        // structural interning for anonymous instantiations (Vec<T>, Option<T>...)
        for (i, t) in self.types.iter().enumerate() {
            if t.name == ty.name && t.kind == ty.kind {
                return i as TypeId;
            }
        }
        let id = self.types.len() as TypeId;
        self.types.push(ty);
        id
    }

    pub fn kind(&self, id: TypeId) -> &TyKind {
        &self.types[id as usize].kind
    }
    pub fn name(&self, id: TypeId) -> &str {
        &self.types[id as usize].name
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
