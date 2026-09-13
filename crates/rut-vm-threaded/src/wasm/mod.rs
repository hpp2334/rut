//! Portable backend — a loop over the same [`Machine`] op methods.
//!
//! Used on wasm and any target without verified `become` lowering. Dispatch
//! is a `match` on the tag instead of tail jumps; the op behavior is the
//! same `Machine::op_*` code the native backend runs. On a [`T_SLOW`] op it
//! leaves the pc and returns [`ThreadOut::Bail`], exactly like native.

use crate::api::*;

/// No-op on the loop backend (dispatch is direct).
pub fn build_table<M: Machine>() -> Table<M> {
    Table(core::marker::PhantomData)
}

pub fn run<M: Machine>(m: &mut M, pc: u32, _table: &Table<M>) -> Result<ThreadOut<M::Out>, M::Err> {
    let mut pc = pc;
    loop {
        m.set_pc(pc);
        let flow = match m.op_tag(pc) {
            T_MOV => {
                m.tick(pc)?;
                m.op_mov(pc)?
            }
            T_CONSTRAW => {
                m.tick(pc)?;
                m.op_constraw(pc)?
            }
            T_NOT => {
                m.tick(pc)?;
                m.op_not(pc)?
            }
            T_ADDI => {
                m.tick(pc)?;
                m.op_addi(pc)?
            }
            T_SUBI => {
                m.tick(pc)?;
                m.op_subi(pc)?
            }
            T_WADDI => {
                m.tick(pc)?;
                m.op_waddi(pc)?
            }
            T_WSUBI => {
                m.tick(pc)?;
                m.op_wsubi(pc)?
            }
            T_WMULI => {
                m.tick(pc)?;
                m.op_wmuli(pc)?
            }
            T_LTI => {
                m.tick(pc)?;
                m.op_lti(pc)?
            }
            T_ADDF => {
                m.tick(pc)?;
                m.op_addf(pc)?
            }
            T_SUBF => {
                m.tick(pc)?;
                m.op_subf(pc)?
            }
            T_MULF => {
                m.tick(pc)?;
                m.op_mulf(pc)?
            }
            T_BR => {
                m.tick(pc)?;
                m.op_br(pc)?
            }
            T_JMP => {
                m.tick(pc)?;
                m.op_jmp(pc)?
            }
            T_LOOPHEAD => {
                m.tick(pc)?;
                m.op_loophead(pc)?
            }
            _ => return Ok(ThreadOut::Bail),
        };
        match flow {
            Flow::Next(n) => pc = n,
            Flow::Done(o) => return Ok(ThreadOut::Done(o)),
        }
    }
}
