//! Native backend — `become`-threaded dispatch with state in arguments.
//!
//! Each handler performs one op via the shared [`Machine`] method, then
//! **tail-calls** the next op's handler, carrying code/tags/regs/pc/fuel in
//! register arguments. `become` replaces the caller's frame with the
//! callee's, so a long scalar stretch never grows the stack and touches no
//! shared `self` memory.
//!
//! `h_slow` handles [`T_SLOW`]: it syncs the machine's pc/fuel and returns
//! [`ThreadOut::Bail`] so the VM's match interpreter runs that op.

use crate::api::*;
use rut_core::ops::Op;

extern "rust-preserve-none" fn h_slow<M: Machine>(
    m: *mut M,
    _table: *const (),
    _code: *const Op,
    _tags: *const u8,
    _regs: *mut M::Word,
    pc: u32,
    fuel: i64,
    fuel_used: u64,
) -> Result<ThreadOut<M::Out>, M::Err> {
    unsafe { (*m).sync(pc, fuel, fuel_used) };
    Ok(ThreadOut::Bail)
}

macro_rules! op_handler {
    ($name:ident, $method:ident) => {
        extern "rust-preserve-none" fn $name<M: Machine>(
            m: *mut M,
            table: *const (),
            code: *const Op,
            tags: *const u8,
            regs: *mut M::Word,
            pc: u32,
            fuel: i64,
            fuel_used: u64,
        ) -> Result<ThreadOut<M::Out>, M::Err> {
            let used = fuel_used + 1;
            if fuel == 0 {
                return Err(unsafe { (*m).park(pc, used) });
            }
            let next_fuel = if fuel < 0 { fuel } else { fuel - 1 };
            let op = unsafe { &*code.add(pc as usize) };
            match unsafe { (*m).$method(op, regs, pc)? } {
                Flow::Next(n) => {
                    let tag = unsafe { *tags.add(n as usize) } as usize;
                    let h = unsafe { *(table as *const Handler<M>).add(tag) };
                    become h(m, table, code, tags, regs, n, next_fuel, used)
                }
                Flow::Done(o) => return Ok(ThreadOut::Done(o)),
            }
        }
    };
}

op_handler!(h_mov, op_mov);
op_handler!(h_constraw, op_constraw);
op_handler!(h_not, op_not);
op_handler!(h_addi, op_addi);
op_handler!(h_subi, op_subi);
op_handler!(h_waddi, op_waddi);
op_handler!(h_wsubi, op_wsubi);
op_handler!(h_wmuli, op_wmuli);
op_handler!(h_lti, op_lti);
op_handler!(h_addf, op_addf);
op_handler!(h_subf, op_subf);
op_handler!(h_mulf, op_mulf);
op_handler!(h_br, op_br);
op_handler!(h_jmp, op_jmp);
op_handler!(h_loophead, op_loophead);

fn table<M: Machine>() -> Table<M> {
    let mut t: [Handler<M>; NTAGS] = [h_slow::<M>; NTAGS];
    t[T_MOV as usize] = h_mov::<M>;
    t[T_CONSTRAW as usize] = h_constraw::<M>;
    t[T_NOT as usize] = h_not::<M>;
    t[T_ADDI as usize] = h_addi::<M>;
    t[T_SUBI as usize] = h_subi::<M>;
    t[T_WADDI as usize] = h_waddi::<M>;
    t[T_WSUBI as usize] = h_wsubi::<M>;
    t[T_WMULI as usize] = h_wmuli::<M>;
    t[T_LTI as usize] = h_lti::<M>;
    t[T_ADDF as usize] = h_addf::<M>;
    t[T_SUBF as usize] = h_subf::<M>;
    t[T_MULF as usize] = h_mulf::<M>;
    t[T_BR as usize] = h_br::<M>;
    t[T_JMP as usize] = h_jmp::<M>;
    t[T_LOOPHEAD as usize] = h_loophead::<M>;
    Table { entries: t }
}

/// Build the handler table once (store it alongside the `Machine`).
pub fn build_table<M: Machine>() -> Table<M> {
    table::<M>()
}

/// Run a threaded stretch from `pc`.
pub fn run<M: Machine>(m: &mut M, pc: u32, table: &Table<M>) -> Result<ThreadOut<M::Out>, M::Err> {
    let st = m.thread_state();
    let mp = m as *mut M;
    let tp = table.entries.as_ptr() as *const ();
    let tag = unsafe { *st.tags.add(pc as usize) } as usize;
    let h = unsafe { *(tp as *const Handler<M>).add(tag) };
    h(mp, tp, st.code, st.tags, st.regs, pc, st.fuel, st.fuel_used)
}
