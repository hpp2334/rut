//! Float opcode specialization (RFC 0032 "the opcode selects the type").
//!
//! LIR knows the exact primitive type of every scalar operand. For float
//! arithmetic, comparison and negation this pass replaces the generic
//! `Arith`/`Wrap`/`Cmp`/`Neg` — which carry a runtime `op`/`prim` pair the
//! interpreter must switch on — with a distinct opcode per operation. The VM
//! then executes a single `addsd`/`mulsd`/`comisd`, with no `is_float` test,
//! no `op` switch and none of the integer width machinery.
//!
//! Integers keep the generic forms: their `fits`/`trunc_to` width handling is
//! per-prim and does not fold, so expanding them would only bloat dispatch
//! and slow the (already fast) integer path.

use rut_core::ops::*;
use rut_core::types::PrimTy;

fn arith(op: ArithOp, prim: PrimTy, dst: Reg, a: Reg, b: Reg) -> Op {
    match op {
        ArithOp::Add => Op::AddF { prim, dst, a, b },
        ArithOp::Sub => Op::SubF { prim, dst, a, b },
        ArithOp::Mul => Op::MulF { prim, dst, a, b },
        ArithOp::Div => Op::DivF { prim, dst, a, b },
        ArithOp::Mod => Op::ModF { prim, dst, a, b },
    }
}

fn cmp(op: CmpOp, dst: Reg, a: Reg, b: Reg) -> Op {
    match op {
        CmpOp::Eq => Op::EqF { dst, a, b },
        CmpOp::Ne => Op::NeF { dst, a, b },
        CmpOp::Lt => Op::LtF { dst, a, b },
        CmpOp::Gt => Op::GtF { dst, a, b },
        CmpOp::Le => Op::LeF { dst, a, b },
        CmpOp::Ge => Op::GeF { dst, a, b },
    }
}

pub(crate) fn run(code: &mut [Op]) {
    for op in code.iter_mut() {
        let new = match op {
            Op::Arith { op, prim, dst, a, b } if prim.is_float() => {
                Some(arith(*op, *prim, *dst, *a, *b))
            }
            // wrapping is a no-op for floats — the same machine operation
            Op::Wrap { op, prim, dst, a, b } if prim.is_float() => {
                Some(arith(*op, *prim, *dst, *a, *b))
            }
            Op::Cmp { op, prim, dst, a, b } if prim.is_float() => {
                Some(cmp(*op, *dst, *a, *b))
            }
            Op::Neg { prim, dst, a } if prim.is_float() => {
                Some(Op::NegF { prim: *prim, dst: *dst, a: *a })
            }
            _ => None,
        };
        if let Some(n) = new {
            *op = n;
        }
    }
}
