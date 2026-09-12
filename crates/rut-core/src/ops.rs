//! Typed bytecode ops — RFC 0032 (LIR). Registers are u16 indices into a
//! per-frame `Vec<Slot>`; every register has a static type in the function
//! signature; the verifier re-checks at load (RFC 0033 §2).
//!
//! Op families carry their `TypeId` operand where the type selects the
//! exact opcode (e.g. `Arith{ty: i32}` is `i32add`) — the binary encoding
//! materializes (family × prim) opcode bytes, same information (RFC 0032
//! "the full table is mechanical").

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
    StrLen,
    VecNew,    // Vec<T>() empty
    VecZeroed, // Vec<T>(n)
    VecFrom,   // Vec.from(Array<T, N>)
    VecLen,
    VecPush,
    VecPop,
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

    /// trapping arithmetic (RFC 0004 §3); `prim` is the resolved operand
    /// type — the VM never consults the type table to execute it
    Arith { op: ArithOp, prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    /// wrapping escapes: &+ &- &* (RFC 0004 §3)
    Wrap { op: ArithOp, prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    Bit { op: BitOp, prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    Cmp { op: CmpOp, prim: PrimTy, dst: Reg, a: Reg, b: Reg },
    Not { dst: Reg, a: Reg },          // bool !
    Neg { prim: PrimTy, dst: Reg, a: Reg }, // trapping negate
    /// string content compare (RFC 0012 §4) — used for ==/!= on string
    StrCmp { eq: bool, dst: Reg, a: Reg, b: Reg },
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
    /// `repr` is the field's baked representation (removes the runtime
    /// field-type lookup + `is_ref`); `field` is the declaration index.
    GetF { dst: Reg, obj: Reg, field: u32, repr: Repr },
    SetF { obj: Reg, field: u32, val: Reg, repr: Repr },
    /// `own(x)` payload copy (RFC 0011 §1): data payload memcpy with
    /// handle-field retains; buffers clone; strings clone
    Own { dst: Reg, src: Reg, ty: TypeId },

    ArrNew { dst: Reg, ty: TypeId, len: Reg, repr: Repr }, // Vec<T>(n) zeroed
    ArrLit { dst: Reg, ty: TypeId, elems: Vec<Reg> }, // fixed Array<T, N>
    ArrGet { dst: Reg, arr: Reg, idx: Reg, repr: Repr },       // bounds trap
    ArrSet { arr: Reg, idx: Reg, val: Reg, repr: Repr },

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

    /// fuel-check no-op back-edge marker (RFC 0040 §2: loop back-edges are
    /// natural checkpoints) — emitted at loop heads
    LoopHead,
}
