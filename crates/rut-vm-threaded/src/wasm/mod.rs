//! Portable backend — a loop over the same [`Machine`] op methods.
//!
//! Used on wasm and any target without verified `become` lowering. It reads
//! the same threaded state snapshot and calls the same `Machine::op_*`
//! methods, bailing on [`T_SLOW`] exactly like the native backend.

use crate::api::*;
use rut_core::ops::Op;

pub fn build_table<M: Machine>() -> Table<M> {
    Table(core::marker::PhantomData)
}

pub fn run<M: Machine>(m: &mut M, pc0: u32, _table: &Table<M>) -> Result<ThreadOut<M::Out>, M::Err> {
    let st = m.thread_state();
    let (code, tags, regs) = (st.code, st.tags, st.regs);
    let mut pc = pc0;
    let mut fuel = st.fuel;
    let mut used = st.fuel_used;
    loop {
        let tag = unsafe { *tags.add(pc as usize) };        if tag == T_SLOW {
            m.sync(pc, fuel, used);
            return Ok(ThreadOut::Bail);
        }
        used += 1;
        if fuel == 0 {
            return Err(m.park(pc, used));
        }
        if fuel > 0 {
            fuel -= 1;
        }
        let op = unsafe { &*code.add(pc as usize) };
        let flow = match tag {
            T_MOV => m.op_mov(op, regs, pc)?,
            T_CONSTRAW => m.op_constraw(op, regs, pc)?,
            T_NOT => m.op_not(op, regs, pc)?,
            T_ADDI => m.op_addi(op, regs, pc)?,
            T_SUBI => m.op_subi(op, regs, pc)?,
            T_WADDI => m.op_waddi(op, regs, pc)?,
            T_WSUBI => m.op_wsubi(op, regs, pc)?,
            T_WMULI => m.op_wmuli(op, regs, pc)?,
            T_LTI => m.op_lti(op, regs, pc)?,
            T_ADDF => m.op_addf(op, regs, pc)?,
            T_SUBF => m.op_subf(op, regs, pc)?,
            T_MULF => m.op_mulf(op, regs, pc)?,
            T_BR => m.op_br(op, regs, pc)?,
            T_JMP => m.op_jmp(op, regs, pc)?,
            _ => m.op_loophead(op, regs, pc)?,
        };
        match flow {
            Flow::Next(n) => pc = n,
            Flow::Done(o) => return Ok(ThreadOut::Done(o)),
        }
    }
}
