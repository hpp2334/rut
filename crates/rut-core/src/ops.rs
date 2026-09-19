//! Typed bytecode ops — RFC 0032 (LIR). Registers are u16 indices into a
//! per-frame `Vec<Slot>`; every register has a static type in the function
//! signature; the verifier re-checks at load (RFC 0033 §2).
//!
//! ## Operand pools (RFC 0032 §"narrow ops")
//!
//! Ops are exactly 24 bytes (a deliberate pin — see `Op::Pad`): no
//! variant owns a heap allocation, and the stride stays off the pow2
//! collision path in the dispatch loop. The
//! variadic operand lists — call arguments, record/array/capture element
//! registers, branch tables — live in **per-function pools**
//! (`FuncCode::argv` for `Reg` lists, `FuncCode::labels` for `Label`
//! tables); the op carries an `(off, argc)` span into its owning
//! function's pool. Pools are append-only and deduplicated at build
//! time, so repeated argument shapes share one entry, and the empty
//! list is always the span `(0, 0)` (never stored). `argc` is u16
//! because a list can name at most as many registers as the u16
//! register file holds; the emitter rejects literals beyond that.
//!
//! Scalar operations are one opcode *per operation*, with the kind (int vs
//! float) in the opcode and the width in the `prim` operand — `addf`/`addi`,
//! not a generic `arith{op, ty}` the VM has to switch on. The frontend
//! resolves `prim`, so there is no `specialize` pass and no runtime `op`
//! selector (RFC 0032 "the opcode selects the type").

use crate::types::{PrimTy, Repr, TypeId};

pub type Reg = u16;
pub type Label = u32;

/// Sentinel for "no register" in optional operands (call `dst`, native
/// `recv`): a function's register file can never reach index u16::MAX
/// (the emitter caps it), so the value is unambiguous. Mandatory
/// operands never carry it — the load verifier rejects any register
/// index ≥ the file size, which `NOREG` always is.
pub const NOREG: Reg = u16::MAX;

/// `Option<Reg>` → sentinel (for building ops).
pub fn opt_reg(o: Option<Reg>) -> Reg {
    o.unwrap_or(NOREG)
}

/// Sentinel → `Option<Reg>` (for consuming ops).
pub fn reg_opt(r: Reg) -> Option<Reg> {
    if r == NOREG { None } else { Some(r) }
}

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
    /// wrapping `calc::wrapping_shl` (RFC 0004 §3): left shift
    /// truncated to the operand width — never traps, unlike `Shl`
    WrapShl,
}

/// Compiler-lowered native methods (RFC 0032 §1.1 R2): the operations
/// `core`'s `builtin impl <int>` blocks declare and the frontend expands
/// inline at the method call (`x.wrapping_add(y)`) instead of calling —
/// wrapping, saturating and checked integer arithmetic. The id travels in
/// the module surface (no `FuncCode`); `rut-lir` owns the lowering,
/// keyed off the receiver's primitive width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intrinsic {
    WrappingAdd, WrappingSub, WrappingMul, WrappingShl,
    SaturatingAdd, SaturatingSub, SaturatingMul,
    CheckedAdd, CheckedSub, CheckedMul,
}

/// Internal natives reached via `CallNat` — RFC 0032 §1.1 R2: things rut
/// spells with a name are natives, never ops (str/concat, Vec's named API,
/// the host print sink).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nat {
    /// per-type formatting (the `f""` desugaring, RFC 0007 §2)
    Str,
    /// str.concat(parts...)
    Concat,
    StrLen,    // s.len() — the codepoint count
    ArrLen,    // Array<T>.len()/bytes.len() — the heap sequence's runtime length
    /// join every element of an `Array<str>` (one sizing pass, one alloc)
    StrJoin,
    /// `s.slice(from, to)` — an O(1) view into the string's octets
    /// (RFC 0042): codepoint-indexed bounds, byte offsets inside
    StrSlice,
    /// `v.slice(from, to)` — an O(1) array window (RFC 0042 §6):
    /// recv = the backing array, args = [from, to, live_len]; the result
    /// is an `ArrView` cell the caller boxes as `*Vec<T>`
    ArrSlice,
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
    ArrayCmp { eq: bool, dst: Reg, a: Reg, b: Reg },
    /// cell identity compare (RFC 0012 §4) — used for ==/!= on ref types
    RefEq { eq: bool, dst: Reg, a: Reg, b: Reg },

    Jmp { target: Label },
    Br { cond: Reg, then_t: Label, else_t: Label },
    /// `when` on enums / downcast chains (RFC 0032 §1) — arms in the
    /// function's `labels` pool: `labels[table_off..table_off + count]`
    BrTable { idx: Reg, table_off: u32, count: u16, default: Label },

    /// direct call (free fns + class methods + closure-entry direct calls)
    /// — argument registers in the function's `argv` pool; `dst == NOREG`
    /// discards
    Call { func: u32, argv_off: u32, argc: u16, dst: Reg },
    /// direct method call — the receiver is `argv[0]`: it arrives as the
    /// callee's param 0, so the pool entry is exactly the callee's
    /// parameter list (a uniform copy loop, no special-cased slot)
    CallM { func: u32, argv_off: u32, argc: u16, dst: Reg },
    /// trait vtable call — the vtable form of the two-rule dispatch law
    /// (RFC 0012 §1): origins multiple, the call consults the value's
    /// descriptor; `slot` is a global trait-method
    /// slot id (RFC 0015 §6); the receiver is `argv[0]`
    CallI { slot: u32, argv_off: u32, argc: u16, dst: Reg },
    /// native module call (RFC 0032 §1.1 R2) — `recv == NOREG` for free
    /// functions
    CallNat { nat: Nat, recv: Reg, argv_off: u32, argc: u16, dst: Reg },
    /// call through an fn-typed value (closures — RFC 0013)
    CallFn { fval: Reg, argv_off: u32, argc: u16, dst: Reg },
    Ret { val: Option<Reg> },

    /// mint a value cell (the `Self { .. }` / struct literal;
    /// RFC 0032 §1) — fields follow via SetF
    NewCell { dst: Reg, ty: TypeId },
    /// fused record literal: allocate and initialize every field in one op
    /// (`argv[argv_off + i]` is field `i`, in declaration order). Replaces
    /// the `NewCell` + N×`SetF` + `MovRef` sequence (RFC 0009).
    MakeRecord { dst: Reg, ty: TypeId, argv_off: u32, argc: u16 },
    /// `repr` is the field's baked representation (removes the runtime
    /// field-type lookup + `is_ref`); `field` is the declaration index.
    GetF { dst: Reg, obj: Reg, field: u32, repr: Repr },
    SetF { obj: Reg, field: u32, val: Reg, repr: Repr },
    /// `own(x)` payload copy (RFC 0011 §1): data payload memcpy with
    /// handle-field retains; buffers clone; strings clone
    Own { dst: Reg, src: Reg, ty: TypeId },
    /// copy-by-value (RFC 0009/0016 v1.1): deep-copy the record/array in
    /// `src` into a fresh cell — nested value fields clone recursively,
    /// `str`/`bytes`/`*T`/closure children share. `ty` is the value type.
    CloneVal { dst: Reg, src: Reg, ty: TypeId },
    /// ownership transfer (the move-elided `CloneVal`): `dst` takes over
    /// `src`'s reference (MovRef semantics — release dst's old cell) and
    /// `src` is KILLED (nulled) so the frame-exit release of its own
    /// reference no-ops. The compiler emits this only where the move is
    /// provably unobservable (rut-lir `moveval`: dead source + sole
    /// ownership, or the field-rebind discipline) — the v1.1 copy law
    /// (two bindings never alias observably) holds at every fired site.
    MoveVal { dst: Reg, src: Reg },
    /// `make_ptr(v)` (RFC 0005): box `v` into a fresh one-slot cell —
    /// the result is a nil-able `*T` (`ty` is the pointer type)
    MakePtr { dst: Reg, src: Reg, ty: TypeId },
    /// `on_drop(p, cleanup)` (RFC 0016 §3): run `cleanup(p)` when p's
    /// cell refcount reaches zero
    OnDrop { obj: Reg, cleanup: Reg },
    /// structural `==`/`!=` on values (RFC 0009/0016 v1.1): recursive,
    /// field-by-field; `str`/`bytes` by content; `*T` by identity
    ValEq { dst: Reg, a: Reg, b: Reg, ty: TypeId, eq: bool },

    ArrNew { dst: Reg, ty: TypeId, len: Reg, repr: Repr }, // Array<T>(n) zeroed
    ArrLit { dst: Reg, ty: TypeId, argv_off: u32, argc: u16 }, // fixed Array<T, N>
    ArrGet { dst: Reg, arr: Reg, idx: Reg, repr: Repr },       // bounds trap
    ArrSet { arr: Reg, idx: Reg, val: Reg, repr: Repr },
    /// fused `obj.field[idx]` / `obj.field[idx] = val` — `field` is a known
    /// `Array<T>` handle, read as a *borrow* (no retire/release): the owner
    /// record keeps it alive for the duration of the access. This is the
    /// `Vec<T>` class's element path (`impl Slice<T>`), so indexing a
    /// pouch sequence costs one op, not a field read per element.
    ArrGetF { dst: Reg, obj: Reg, field: u32, idx: Reg, repr: Repr },
    ArrSetF { obj: Reg, field: u32, idx: Reg, val: Reg, repr: Repr },

    /// `for (let v of xs)` element reference (RFC 0012 §6): box the element
    /// at `idx` into a fresh one-slot cell of the pointer type `ty` —
    /// ref-typed elements alias the stored slot, scalars box a copy
    ArrGetRef { dst: Reg, arr: Reg, idx: Reg, ty: TypeId },

    /// enum member value (immortal singleton cell, RFC 0016 §1)
    EnumNew { dst: Reg, ty: TypeId, member: u32 },

    /// read a handle's runtime TypeId → u32 (pure load; RFC 0032 §1)
    TidOf { dst: Reg, obj: Reg },
    /// `x is T` — exact test; sees through Opaque boxes (RFC 0014)
    IsType { dst: Reg, obj: Reg, want: TypeId },
    /// `x is I` — capability probe: descriptor impls scan (RFC 0015 §6)
    IsTrait { dst: Reg, obj: Reg, want: u32 },
    /// extract an Opaque box's payload as the statically known T — traps
    /// on TypeId mismatch; the compiler guards (RFC 0032 §1.1)
    Unbox { dst: Reg, box_: Reg, ty: TypeId },
    /// opaque(v) — box mint (internal native in RFC terms; an op here
    /// because it needs no name resolution)
    Box { dst: Reg, val: Reg, ty: TypeId },

    /// closure literal: { func, captures } (RFC 0013 §1 — v1 captures by
    /// value; by-ref capture lands with coroutine frames, RFC 0018 §4) —
    /// capture registers in the function's `argv` pool
    MakeClosure { dst: Reg, func: u32, argv_off: u32, argc: u16 },

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
    /// Layout pin — never constructed. The enum is deliberately 24 bytes,
    /// not the natural 16: in the threaded dispatch loop the op-stream
    /// loads alias against the register-file stores when the op stride is
    /// a small power of two (measured on this µarch: 16/32-byte ops cost
    /// intloop +13%; 24/40/64 are clean). 24 keeps the pooled form 40%
    /// narrower than the Vec-carrying layout it replaces while staying
    /// off the collision stride. The size assert below pins it.
    #[allow(dead_code)]
    Pad { p: [u32; 5] },
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

/// Wrapping arithmetic (`calc::wrapping_add/sub/mul`) — a no-op for
/// floats. `&/`/`&%` were never spellable, so there is no wrapping div/rem.
pub fn wrap_arith(op: ArithOp, prim: PrimTy, dst: Reg, a: Reg, b: Reg) -> Op {
    if prim.is_float() {
        return arith(op, prim, dst, a, b);
    }
    match op {
        ArithOp::Add => Op::WAddI { prim, dst, a, b },
        ArithOp::Sub => Op::WSubI { prim, dst, a, b },
        ArithOp::Mul => Op::WMulI { prim, dst, a, b },
        ArithOp::Div | ArithOp::Mod => unreachable!("no wrapping division (RFC 0004 §3)"),
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

// ---- operand-pool spans ----

impl Op {
    /// The `(off, argc)` span this op reads from its function's `argv`
    /// pool, if it has one.
    pub fn argv_span(&self) -> Option<(u32, u16)> {
        Some(match self {
            Op::Call { argv_off, argc, .. }
            | Op::CallM { argv_off, argc, .. }
            | Op::CallI { argv_off, argc, .. }
            | Op::CallNat { argv_off, argc, .. }
            | Op::CallFn { argv_off, argc, .. }
            | Op::MakeRecord { argv_off, argc, .. }
            | Op::ArrLit { argv_off, argc, .. }
            | Op::MakeClosure { argv_off, argc, .. } => (*argv_off, *argc),
            _ => return None,
        })
    }

    /// Mutable span — for the rewriter that re-interns a remapped list.
    pub fn argv_span_mut(&mut self) -> Option<(&mut u32, &mut u16)> {
        Some(match self {
            Op::Call { argv_off, argc, .. }
            | Op::CallM { argv_off, argc, .. }
            | Op::CallI { argv_off, argc, .. }
            | Op::CallNat { argv_off, argc, .. }
            | Op::CallFn { argv_off, argc, .. }
            | Op::MakeRecord { argv_off, argc, .. }
            | Op::ArrLit { argv_off, argc, .. }
            | Op::MakeClosure { argv_off, argc, .. } => (argv_off, argc),
            _ => return None,
        })
    }

    /// The `(off, count)` span this op reads from its function's `labels`
    /// pool (`BrTable` only).
    pub fn label_span(&self) -> Option<(u32, u16)> {
        match self {
            Op::BrTable { table_off, count, .. } => Some((*table_off, *count)),
            _ => None,
        }
    }
}

/// The narrow-op law: no variant owns a heap allocation, so the dispatch
/// stream stays dense (RFC 0032).
/// The narrow-op law: pooled operands, no per-op heap allocation, and a
/// stride that measures clean in the dispatch loop (see `Op::Pad`).
const _: () = assert!(std::mem::size_of::<Op>() == 24, "Op must stay 24 bytes");
