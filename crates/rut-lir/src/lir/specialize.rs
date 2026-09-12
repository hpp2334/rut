//! Scalar opcode specialization (RFC 0032 "the opcode selects the type").
//!
//! LIR knows the exact primitive type of every scalar operand. This pass
//! replaces the generic `Arith`/`Wrap`/`Bit`/`Cmp`/`Neg` — which carry a
//! runtime `op`/`prim` pair the interpreter must switch on — with a distinct
//! opcode per operation:
//!
//!   * floats: the operation alone is the opcode (`addf`/`mulf`/`ltf`/...) and
//!     the interpreter folds to one `addsd`/`mulsd`/`comisd`, with no
//!     `is_float` test and none of the integer width machinery;
//!   * integers: the operation is the opcode (`addi`/`wmuli`/`lti`/...), which
//!     removes the `op` switch and the `is_float` test. The integer `prim`
//!     stays an operand because width fitting is per-prim and does not fold.
//!
//! `bool`/`char` (neither float nor int) keep the generic forms.

use rut_core::ops::*;
use rut_core::types::PrimTy;

fn float_arith(op: ArithOp, prim: PrimTy, dst: Reg, a: Reg, b: Reg) -> Op {
    match op {
        ArithOp::Add => Op::AddF { prim, dst, a, b },
        ArithOp::Sub => Op::SubF { prim, dst, a, b },
        ArithOp::Mul => Op::MulF { prim, dst, a, b },
        ArithOp::Div => Op::DivF { prim, dst, a, b },
        ArithOp::Mod => Op::ModF { prim, dst, a, b },
    }
}

fn float_cmp(op: CmpOp, dst: Reg, a: Reg, b: Reg) -> Op {
    match op {
        CmpOp::Eq => Op::EqF { dst, a, b },
        CmpOp::Ne => Op::NeF { dst, a, b },
        CmpOp::Lt => Op::LtF { dst, a, b },
        CmpOp::Gt => Op::GtF { dst, a, b },
        CmpOp::Le => Op::LeF { dst, a, b },
        CmpOp::Ge => Op::GeF { dst, a, b },
    }
}

fn int_arith(op: ArithOp, prim: PrimTy, dst: Reg, a: Reg, b: Reg) -> Op {
    match op {
        ArithOp::Add => Op::AddI { prim, dst, a, b },
        ArithOp::Sub => Op::SubI { prim, dst, a, b },
        ArithOp::Mul => Op::MulI { prim, dst, a, b },
        ArithOp::Div => Op::DivI { prim, dst, a, b },
        ArithOp::Mod => Op::ModI { prim, dst, a, b },
    }
}

fn int_wrap(op: ArithOp, prim: PrimTy, dst: Reg, a: Reg, b: Reg) -> Op {
    match op {
        ArithOp::Add => Op::WAddI { prim, dst, a, b },
        ArithOp::Sub => Op::WSubI { prim, dst, a, b },
        ArithOp::Mul => Op::WMulI { prim, dst, a, b },
        ArithOp::Div => Op::WDivI { prim, dst, a, b },
        ArithOp::Mod => Op::WModI { prim, dst, a, b },
    }
}

fn int_bit(op: BitOp, prim: PrimTy, dst: Reg, a: Reg, b: Reg) -> Op {
    match op {
        BitOp::And => Op::AndI { prim, dst, a, b },
        BitOp::Or => Op::OrI { prim, dst, a, b },
        BitOp::Xor => Op::XorI { prim, dst, a, b },
        BitOp::Shl => Op::ShlI { prim, dst, a, b },
        BitOp::Shr => Op::ShrI { prim, dst, a, b },
        BitOp::WrapShl => Op::WrapShlI { prim, dst, a, b },
    }
}

fn int_cmp(op: CmpOp, dst: Reg, a: Reg, b: Reg) -> Op {
    match op {
        CmpOp::Eq => Op::EqI { dst, a, b },
        CmpOp::Ne => Op::NeI { dst, a, b },
        CmpOp::Lt => Op::LtI { dst, a, b },
        CmpOp::Gt => Op::GtI { dst, a, b },
        CmpOp::Le => Op::LeI { dst, a, b },
        CmpOp::Ge => Op::GeI { dst, a, b },
    }
}

pub(crate) fn run(code: &mut [Op]) {
    for op in code.iter_mut() {
        let new = match op {
            Op::Arith { op, prim, dst, a, b } if prim.is_float() => {
                Some(float_arith(*op, *prim, *dst, *a, *b))
            }
            Op::Arith { op, prim, dst, a, b } if prim.is_int() => {
                Some(int_arith(*op, *prim, *dst, *a, *b))
            }
            // wrapping is a no-op for floats — the same machine operation
            Op::Wrap { op, prim, dst, a, b } if prim.is_float() => {
                Some(float_arith(*op, *prim, *dst, *a, *b))
            }
            Op::Wrap { op, prim, dst, a, b } if prim.is_int() => {
                Some(int_wrap(*op, *prim, *dst, *a, *b))
            }
            Op::Bit { op, prim, dst, a, b } if prim.is_int() => {
                Some(int_bit(*op, *prim, *dst, *a, *b))
            }
            Op::Cmp { op, prim, dst, a, b } if prim.is_float() => {
                Some(float_cmp(*op, *dst, *a, *b))
            }
            Op::Cmp { op, prim, dst, a, b } if prim.is_int() => {
                Some(int_cmp(*op, *dst, *a, *b))
            }
            Op::Neg { prim, dst, a } if prim.is_float() => {
                Some(Op::NegF { prim: *prim, dst: *dst, a: *a })
            }
            Op::Neg { prim, dst, a } if prim.is_int() => {
                Some(Op::NegI { prim: *prim, dst: *dst, a: *a })
            }
            _ => None,
        };
        if let Some(n) = new {
            *op = n;
        }
    }
}
