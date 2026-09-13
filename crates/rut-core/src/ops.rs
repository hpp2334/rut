//! Typed bytecode ops — RFC 0032 (LIR). Registers are u16 indices into a
//! per-frame `Vec<Slot>`; every register has a static type in the function
//! signature; the verifier re-checks at load (RFC 0033 §2).
//!
//! Scalar operations are one opcode *per operation*, with the kind (int vs
//! float) in the opcode and the width in the `prim` operand — `addf`/`addi`,
//! not a generic `arith{op, ty}` the VM has to switch on. The frontend
//! resolves `prim`, so there is no `specialize` pass and no runtime `op`
//! selector (RFC 0032 "the opcode selects the type").

use crate::types::{PrimTy, Repr, TypeId};

pub type Reg = u16;
pub type Label = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArithOp {
    Add, Sub, Mul, Div, Mod,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq, Ne, Lt, Gt, Le, Ge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BitOp {
    And, Or, Xor, Shl, Shr,
    /// wrapping `&<<` (RFC 0004 §3): left shift truncated to the operand
    /// width — never traps, unlike `Shl`
    WrapShl,
}

/// Internal natives reached via `CallNat` — RFC 0032 §1.1 R2: things rut
/// spells with a name are natives, never ops (str/concat, Vec's named API,
/// the host print sink).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nat {
    /// host print sink (RFC 0028: uninstalled sink is a silent no-op)
    Print,
    /// per-type formatting (the `f""` desugaring, RFC 0007 §2)
    Str,
    /// str.concat(parts...)
    Concat,
    StrLen,      // `for..of`/`Iter::len`/`string_len` — the char count
    ArrLen,    // Array<T>.len() — the heap array's runtime length
    // ---- bytes (RFC 0004) — the immutable binary buffer ----
    BytesNew,    // bytes(n) zeroed
    BytesFrom,   // bytes.from(Array<u8>) — one copy
    BytesLen,
    StrEncode,   // string.encode() -> bytes (UTF-8)
    BytesDecode, // bytes.decode() -> string (UTF-8, lossy)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// untyped move (verifier: non-ref type)
    Mov { dst: Reg, src: Reg },
    /// ref move: retain new, release old (RFC 0016 §5)
    MovRef { dst: Reg, src: Reg },
    /// const-pool load
    Const { dst: Reg, k: u32 },
    /// raw scalar const (i64 bits / f64 bits / bool / char) — folded form
    ConstRaw { dst: Reg, bits: u64 },

    Not { dst: Reg, a: Reg },          // bool !
    /// Scalar arithmetic/bitwise/compare are one opcode *per operation*,
    /// with the kind (int vs float) in the opcode and the width in `prim`
    /// (RFC 0032 "the opcode selects the type"). The VM never consults the
    /// type table, and there is no runtime `op` selector.
    AddF { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    SubF { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    MulF { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    DivF { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    ModF { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    NegF { prim: PrimTy, dst: Reg, a: Reg },
    EqF { dst: Reg, a: Reg, b: Reg },
    NeF { dst: Reg, a: Reg, b: Reg },
    LtF { dst: Reg, a: Reg, b: Reg },
    GtF { dst: Reg, a: Reg, b: Reg },
    LeF { dst: Reg, a: Reg, b: Reg },
    GeF { dst: Reg, a: Reg, b: Reg },
    AddI { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    SubI { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    MulI { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    DivI { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    ModI { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    WAddI { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    WSubI { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    WMulI { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    WDivI { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    WModI { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    AndI { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    OrI { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    XorI { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    ShlI { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    ShrI { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    WrapShlI { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    /// int compare — `prim` selects signed vs unsigned ordering; bool/char
    /// compare through here as well (their slot is an integer)
    EqI { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    NeI { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    LtI { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    GtI { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    LeI { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    GeI { prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    NegI { prim: PrimTy, dst: Reg, a: Reg },
    /// string content compare (RFC 0012 §4) — used for ==/!= on string
    StrCmp { eq: bool, dst: Reg, a: Reg, b: Reg },
    /// bytes content compare (RFC 0004) — used for ==/!= on bytes
    BytesCmp { eq: bool, dst: Reg, a: Reg, b: Reg },
    /// cell identity compare (RFC 0012 §4) — used for ==/!= on ref types
    RefEq { eq: bool, dst: Reg, a: Reg, b: Reg },

    Jmp { target: Label },
    Br { cond: Reg, then_t: Label, else_t: Label },
    /// `when` on enums / downcast chains (RFC 0032 §1)
    BrTable { idx: Reg, table: Vec<Label>, default: Label },

    /// direct call (free fns + class methods + closure-entry direct calls)
    Call { func: u32, args: Vec<Reg>, dst: Option<Reg> },
    /// direct method call — recv arrives as the callee's param 0
    CallM { func: u32, recv: Reg, args: Vec<Reg>, dst: Option<Reg> },
    /// trait vtable call — ALWAYS dynamic for trait-declared members
    /// (RFC 0012 §1: no devirtualization); `slot` is a global trait-method
    /// slot id (RFC 0015 §6)
    CallI { slot: u32, recv: Reg, args: Vec<Reg>, dst: Option<Reg> },
    /// native module call (RFC 0032 §1.1 R2)
    CallNat { nat: Nat, recv: Option<Reg>, args: Vec<Reg>, dst: Option<Reg> },
    /// call through an fn-typed value (closures — RFC 0013)
    CallFn { fval: Reg, args: Vec<Reg>, dst: Option<Reg> },
    Ret { val: Option<Reg> },

    /// mint a value cell (the `Self { .. }` / dataclass literal;
    /// RFC 0032 §1) — fields follow via SetF
    NewCell { dst: Reg, ty: TypeId },
    /// fused record literal: allocate and initialize every field in one op
    /// (`vals[i]` is field `i`, in declaration order). Replaces the
    /// `NewCell` + N×`SetF` + `MovRef` sequence (RFC 0009).
    MakeRecord { dst: Reg, ty: TypeId, vals: Vec<Reg> },
    /// `repr` is the field's baked representation (removes the runtime
    /// field-type lookup + `is_ref`); `field` is the declaration index.
    GetF { dst: Reg, obj: Reg, field: u32, repr: Repr },
    SetF { obj: Reg, field: u32, val: Reg, repr: Repr },
    /// `own(x)` payload copy (RFC 0011 §1): data payload memcpy with
    /// handle-field retains; buffers clone; strings clone
    Own { dst: Reg, src: Reg, ty: TypeId },

    ArrNew { dst: Reg, ty: TypeId, len: Reg, repr: Repr }, // Array<T>(n) zeroed
    ArrLit { dst: Reg, ty: TypeId, elems: Vec<Reg> }, // fixed Array<T, N>
    ArrGet { dst: Reg, arr: Reg, idx: Reg, repr: Repr },       // bounds trap
    ArrSet { arr: Reg, idx: Reg, val: Reg, repr: Repr },
    /// fused `obj.field[idx]` / `obj.field[idx] = val` — `field` is a known
    /// `Array<T>` handle, read as a *borrow* (no retire/release): the owner
    /// record keeps it alive for the duration of the access. This is the
    /// `Vec<T>` class's element path (`impl Slice<T>`), so indexing a
    /// std:collection sequence costs one op, not a field read per element.
    ArrGetF { dst: Reg, obj: Reg, field: u32, idx: Reg, repr: Repr },
    ArrSetF { obj: Reg, field: u32, idx: Reg, val: Reg, repr: Repr },

    /// enum member value (immortal singleton cell, RFC 0016 §1)
    EnumNew { dst: Reg, ty: TypeId, member: u32 },
    OptSome { dst: Reg, ty: TypeId, val: Reg },
    OptNone { dst: Reg, ty: TypeId },
    ResOk { dst: Reg, ty: TypeId, val: Reg },
    ResErr { dst: Reg, ty: TypeId, val: Reg },
    /// Option.is_some / Result.is_ok (tag 0 check); `want_err` flips
    SumIs { dst: Reg, v: Reg, want_err: bool },
    /// `.value` / `.error` — traps on the wrong tag (RFC 0005)
    Unwrap { dst: Reg, v: Reg, want_err: bool },
    UnwrapOr { dst: Reg, v: Reg, default: Reg },
    Expect { dst: Reg, v: Reg, msg: Reg },

    /// read a handle's runtime TypeId → u32 (pure load; RFC 0032 §1)
    TidOf { dst: Reg, obj: Reg },
    /// `x is T` — exact test; sees through Opaque boxes (RFC 0014)
    IsType { dst: Reg, obj: Reg, want: TypeId },
    /// `x is I` — capability probe: descriptor impls scan (RFC 0015 §6)
    IsTrait { dst: Reg, obj: Reg, want: u32 },
    /// extract an Opaque box's payload as the statically known T — traps
    /// on TypeId mismatch; the compiler guards (RFC 0032 §1.1)
    Unbox { dst: Reg, box_: Reg, ty: TypeId },
    /// Opaque.new(v) — box mint (internal native in RFC terms; an op here
    /// because it needs no name resolution)
    Box { dst: Reg, val: Reg, ty: TypeId },

    /// closure literal: { func, captures } (RFC 0013 §1 — v1 captures by
    /// value; by-ref capture lands with coroutine frames, RFC 0018 §4)
    MakeClosure { dst: Reg, func: u32, captures: Vec<Reg> },

    /// panic(msg) / assert(cond, msg?) — RFC 0034 §2
    Panic { msg: Reg },
    Assert { cond: Reg, msg: Option<Reg> },

    /// explicit numeric conversion `i32(x)` etc — always a call, never an
    /// operator (RFC 0007 §1); narrowing traps when the value doesn't fit
    Conv { dst: Reg, src: Reg, from: PrimTy, to: PrimTy },
    /// string codepoint read (for-of strings, RFC 0008 §1); bounds trap
    StrCharAt { dst: Reg, s: Reg, idx: Reg },
    /// byte read (indexing / for-of on bytes); bounds trap
    BytesGet { dst: Reg, s: Reg, idx: Reg },

    /// fuel-check no-op back-edge marker (RFC 0040 §2: loop back-edges are
    /// natural checkpoints) — emitted at loop heads
    LoopHead,
}

// ---- specialized scalar-op constructors (RFC 0032) ----
//
// The frontend resolves `prim` before emitting, so it names the exact opcode
// directly — no separate `specialize` pass and no runtime `op` selector.
// `is_float` picks the int/float family; `prim` carries the width.

/// Trapping arithmetic (`+ - * / %`) for int or float `prim`.
pub fn arith(op: ArithOp, prim: PrimTy, dst: Reg, a: Reg, b: Reg) -> Op {
    if prim.is_float() {
        match op {
            ArithOp::Add => Op::AddF { prim, dst, a, b },
            ArithOp::Sub => Op::SubF { prim, dst, a, b },
            ArithOp::Mul => Op::MulF { prim, dst, a, b },
            ArithOp::Div => Op::DivF { prim, dst, a, b },
            ArithOp::Mod => Op::ModF { prim, dst, a, b },
        }
    } else {
        match op {
            ArithOp::Add => Op::AddI { prim, dst, a, b },
            ArithOp::Sub => Op::SubI { prim, dst, a, b },
            ArithOp::Mul => Op::MulI { prim, dst, a, b },
            ArithOp::Div => Op::DivI { prim, dst, a, b },
            ArithOp::Mod => Op::ModI { prim, dst, a, b },
        }
    }
}

/// Wrapping arithmetic (`&+ &- &* &/ &%`) — a no-op for floats.
pub fn wrap_arith(op: ArithOp, prim: PrimTy, dst: Reg, a: Reg, b: Reg) -> Op {
    if prim.is_float() {
        return arith(op, prim, dst, a, b);
    }
    match op {
        ArithOp::Add => Op::WAddI { prim, dst, a, b },
        ArithOp::Sub => Op::WSubI { prim, dst, a, b },
        ArithOp::Mul => Op::WMulI { prim, dst, a, b },
        ArithOp::Div => Op::WDivI { prim, dst, a, b },
        ArithOp::Mod => Op::WModI { prim, dst, a, b },
    }
}

pub fn bitop(op: BitOp, prim: PrimTy, dst: Reg, a: Reg, b: Reg) -> Op {
    match op {
        BitOp::And => Op::AndI { prim, dst, a, b },
        BitOp::Or => Op::OrI { prim, dst, a, b },
        BitOp::Xor => Op::XorI { prim, dst, a, b },
        BitOp::Shl => Op::ShlI { prim, dst, a, b },
        BitOp::Shr => Op::ShrI { prim, dst, a, b },
        BitOp::WrapShl => Op::WrapShlI { prim, dst, a, b },
    }
}

/// Compare. Floats compare in the f64 slot (no width); ints carry `prim` so
/// unsigned widths order unsigned; bool/char compare as their integer slot.
pub fn cmpop(op: CmpOp, prim: PrimTy, dst: Reg, a: Reg, b: Reg) -> Op {
    if prim.is_float() {
        match op {
            CmpOp::Eq => Op::EqF { dst, a, b },
            CmpOp::Ne => Op::NeF { dst, a, b },
            CmpOp::Lt => Op::LtF { dst, a, b },
            CmpOp::Gt => Op::GtF { dst, a, b },
            CmpOp::Le => Op::LeF { dst, a, b },
            CmpOp::Ge => Op::GeF { dst, a, b },
        }
    } else {
        match op {
            CmpOp::Eq => Op::EqI { prim, dst, a, b },
            CmpOp::Ne => Op::NeI { prim, dst, a, b },
            CmpOp::Lt => Op::LtI { prim, dst, a, b },
            CmpOp::Gt => Op::GtI { prim, dst, a, b },
            CmpOp::Le => Op::LeI { prim, dst, a, b },
            CmpOp::Ge => Op::GeI { prim, dst, a, b },
        }
    }
}

pub fn negop(prim: PrimTy, dst: Reg, a: Reg) -> Op {
    if prim.is_float() {
        Op::NegF { prim, dst, a }
    } else {
        Op::NegI { prim, dst, a }
    }
}
