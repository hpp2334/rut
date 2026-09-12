//! Scalar arithmetic/bitwise/compare/convert bodies (RFC 0004 §3) and the
//! const-generic op codes the dispatch passes as const args.
use super::*;

impl Vm {
    // ---- arithmetic (RFC 0004 §3) ----

    /// Integer arithmetic, monomorphic per (operation, wrapping) via const
    /// generics — each specialized `addi`/`wmuli`/... opcode instantiates its
    /// own body. `inline(always)` guarantees the fold; `#[inline]` alone left
    /// the wider `Div`/`Mod` bodies as calls with a runtime `op` switch.
    #[inline(always)]
    pub(super) fn arith_int<const OP: i32, const WRAP: bool>(&self, p: PrimTy, x: Slot, y: Slot) -> Result<Slot, Trap> {
        use PrimTy::*;
        let a = unsafe { x.i };
        let b = unsafe { y.i };
        let unsigned = matches!(p, U8 | U16 | U32 | U64);
        let (r, o) = match OP {
            IOP_ADD => a.overflowing_add(b),
            IOP_SUB => a.overflowing_sub(b),
            IOP_MUL => a.overflowing_mul(b),
            IOP_DIV => {
                if b == 0 {
                    return Err(Trap::new(TrapKind::DivByZero, "division by zero"));
                }
                (if unsigned { (a as u64 / b as u64) as i64 } else { a / b }, false)
            }
            IOP_MOD => {
                if b == 0 {
                    return Err(Trap::new(TrapKind::DivByZero, "mod by zero"));
                }
                (if unsigned { (a as u64 % b as u64) as i64 } else { a % b }, false)
            }
            _ => unreachable!("bad arith op code"),
        };
        if !WRAP && (o || !fits(r, p)) {
            return Err(Trap::new(
                TrapKind::Overflow,
                "arithmetic overflow — use the &+ &- &* wrapping forms (RFC 0004 §3)",
            ));
        }
        Ok(Slot::int(trunc_to(r, p)))
    }

    /// Float arithmetic, monomorphic per operation via const generics — the
    /// `addf`/`subf`/... opcodes each instantiate their own body. The width
    /// stays in `p` (the opcode is one per operation, not per width), so the
    /// `f32` round still happens here, but the operation `match` folds.
    #[inline(always)]
    pub(super) fn arith_float<const OP: i32>(&self, p: PrimTy, x: Slot, y: Slot) -> Slot {
        let a = unsafe { x.f };
        let b = unsafe { y.f };
        let r = match OP {
            FOP_ADD => a + b,
            FOP_SUB => a - b,
            FOP_MUL => a * b,
            FOP_DIV => a / b,
            _ => a % b, // FOP_MOD
        };
        Slot::float(if p == PrimTy::F32 { r as f32 as f64 } else { r })
    }

    /// Float negate (`negf`); rounds to `f32` when `p` is `F32`.
    #[inline(always)]
    pub(super) fn neg_float(&self, p: PrimTy, x: Slot) -> Slot {
        let a = unsafe { x.f };
        Slot::float(if p == PrimTy::F32 { (-(a as f32)) as f64 } else { -a })
    }

    /// Float compare, monomorphic per operation via const generics. Values
    /// are compared as `f64`; IEEE `NaN` semantics fall out unchanged.
    #[inline(always)]
    pub(super) fn cmp_float<const OP: i32>(&self, x: Slot, y: Slot) -> bool {
        let a = unsafe { x.f };
        let b = unsafe { y.f };
        match OP {
            COP_EQ => a == b,
            COP_NE => a != b,
            COP_LT => a < b,
            COP_GT => a > b,
            COP_LE => a <= b,
            _ => a >= b, // COP_GE
        }
    }

    /// Integer bit-operation body without the `is_int` guard — the
    /// specialized `andi`/`shli`/... opcodes call it with a constant `OP`, so
    /// the operation `match` folds and each opcode gets its own body.
    #[inline(always)]
    pub(super) fn bitop_int<const OP: i32>(&self, p: PrimTy, x: Slot, y: Slot) -> Result<Slot, Trap> {
        use PrimTy::*;
        let a = unsafe { x.i };
        let b = unsafe { y.i };
        let (r, o) = match OP {
            BOP_AND => (a & b, false),
            BOP_OR => (a | b, false),
            BOP_XOR => (a ^ b, false),
            BOP_SHL => a.overflowing_shl((b & 63) as u32),
            // `>>` follows signedness: logical on unsigned (a u64 with its
            // top bit set must not sign-extend — RFC 0004 §1), arithmetic
            // on signed
            BOP_SHR => (
                if matches!(p, U8 | U16 | U32 | U64) {
                    ((a as u64) >> (b & 63)) as i64
                } else {
                    a.overflowing_shr((b & 63) as u32).0
                },
                false,
            ),
            // wrapping `&<<` (RFC 0004 §3): the shifted-out bits are simply
            // gone — truncate to the operand width, never trap
            BOP_WRAPSHL => return Ok(Slot::int(trunc_to(a.wrapping_shl((b & 63) as u32), p))),
            _ => unreachable!("bad bitop code"),
        };
        if o {
            return Err(Trap::new(TrapKind::Overflow, "shift overflow"));
        }
        if !fits(r, p) {
            return Err(Trap::new(TrapKind::Overflow, "bit op out of range"));
        }
        Ok(Slot::int(r))
    }

    /// Explicit numeric conversion (RFC 0007 §1): int↔int traps on
    /// narrowing loss; float→int traps on fraction/range; int→float and
    /// float↔float always convert.
    pub(super) fn convert(&self, v: Slot, from: PrimTy, to: PrimTy) -> Result<Slot, Trap> {
        use rut_core::types::PrimTy::*;
        let (fp, tp) = (from, to);
        Ok(match (fp.is_float(), tp.is_float()) {
            (false, false) => {
                let x = unsafe { v.i };
                if !fits(x, tp) {
                    return Err(Trap::new(
                        TrapKind::Overflow,
                        format!("narrowing conversion {} -> {} loses the value (RFC 0007 §1)", fp.name(), tp.name()),
                    ));
                }
                Slot::int(x)
            }
            (false, true) => {
                let x = unsafe { v.i } as f64;
                if tp == F32 {
                    Slot::float(x as f32 as f64)
                } else {
                    Slot::float(x)
                }
            }
            (true, false) => {
                let x = unsafe { v.f };
                if x.fract() != 0.0 || !x.is_finite() || !float_fits(x, tp) {
                    return Err(Trap::new(
                        TrapKind::Overflow,
                        format!("float {} does not convert to {} (fractional or out of range — RFC 0007 §1)", x, tp.name()),
                    ));
                }
                Slot::int(x as i64)
            }
            (true, true) => {
                let x = unsafe { v.f };
                if tp == F32 {
                    Slot::float(x as f32 as f64)
                } else {
                    Slot::float(x)
                }
            }
        })
    }

    /// Integer compare without the float branch — the specialized `lti`/...
    /// opcodes call it with a constant `OP`, so the operation `match` folds.
    /// `p` selects the signedness, so unsigned widths order unsigned.
    #[inline(always)]
    pub(super) fn cmp_int<const OP: i32>(&self, p: PrimTy, x: Slot, y: Slot) -> bool {
        let a = unsafe { x.i };
        let b = unsafe { y.i };
        match OP {
            COP_EQ => a == b,
            COP_NE => a != b,
            _ if p.is_unsigned() => {
                let (a, b) = (a as u64, b as u64);
                match OP {
                    COP_LT => a < b,
                    COP_GT => a > b,
                    COP_LE => a <= b,
                    _ => a >= b,
                }
            }
            _ => match OP {
                COP_LT => a < b,
                COP_GT => a > b,
                COP_LE => a <= b,
                _ => a >= b,
            },
        }
    }

    /// Integer negate without the float branch.
    #[inline(always)]
    pub(super) fn neg_int(&self, p: PrimTy, x: Slot) -> Result<Slot, Trap> {
        let v = unsafe { x.i };
        let (r, o) = v.overflowing_neg();
        if o || !fits(r, p) {
            return Err(Trap::new(TrapKind::Overflow, "negate overflow"));
        }
        Ok(Slot::int(r))
    }
}

#[inline(always)]
pub(super) fn trunc_to(v: i64, p: PrimTy) -> i64 {
    use PrimTy::*;
    match p {
        U8 => (v as u8) as i64,
        U16 => (v as u16) as i64,
        U32 => (v as u32) as i64,
        U64 => v,
        I8 => (v as i8) as i64,
        I16 => (v as i16) as i64,
        I32 => (v as i32) as i64,
        _ => v,
    }
}

#[inline(always)]
pub(super) fn fits(v: i64, p: PrimTy) -> bool {
    trunc_to(v, p) == v
}

#[inline(always)]
pub(super) fn float_fits(x: f64, p: PrimTy) -> bool {
    match p {
        PrimTy::U8 => x >= 0.0 && x <= u8::MAX as f64,
        PrimTy::U16 => x >= 0.0 && x <= u16::MAX as f64,
        PrimTy::U32 => x >= 0.0 && x <= u32::MAX as f64,
        PrimTy::U64 => x >= 0.0 && x <= u64::MAX as f64,
        PrimTy::I8 => x >= i8::MIN as f64 && x <= i8::MAX as f64,
        PrimTy::I16 => x >= i16::MIN as f64 && x <= i16::MAX as f64,
        PrimTy::I32 => x >= i32::MIN as f64 && x <= i32::MAX as f64,
        PrimTy::I64 => x >= i64::MIN as f64 && x <= i64::MAX as f64,
        _ => true,
    }
}
