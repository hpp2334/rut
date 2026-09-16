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
    let (mut code, mut tags, mut regs) = (st.code, st.tags, st.regs);
    let mut pc = pc0;
    let mut fuel = st.fuel;
    let mut used = st.fuel_used;
    loop {
        let tag = unsafe { *tags.add(pc as usize) };
        if tag == T_SLOW {
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
            T_ARRGET => m.op_arr_get(op, regs, pc)?,
            T_ARRSET => m.op_arr_set(op, regs, pc)?,
            T_ARRGETF => m.op_arr_get_f(op, regs, pc)?,
            T_ARRSETF => m.op_arr_set_f(op, regs, pc)?,
            T_GETF => m.op_getf(op, regs, pc)?,
            T_ARRGETREF => m.op_arr_get_ref(op, regs, pc)?,
            T_SETF => m.op_setf(op, regs, pc)?,
            T_CALL => m.op_call(op, regs, pc)?,
            T_CALLM => m.op_call_m(op, regs, pc)?,
            T_CALLI => m.op_call_i(op, regs, pc)?,
            T_CALLFN => m.op_call_fn(op, regs, pc)?,
            T_CALLNAT => m.op_call_nat(op, regs, pc)?,
            T_RET => m.op_ret(op, regs, pc)?,
            T_MULI => m.op_muli(op, regs, pc)?,
            T_DIVI => m.op_divi(op, regs, pc)?,
            T_MODI => m.op_modi(op, regs, pc)?,
            T_ANDI => m.op_andi(op, regs, pc)?,
            T_ORI => m.op_ori(op, regs, pc)?,
            T_XORI => m.op_xori(op, regs, pc)?,
            T_SHLI => m.op_shli(op, regs, pc)?,
            T_SHRI => m.op_shri(op, regs, pc)?,
            T_WRAPSHLI => m.op_wrapshli(op, regs, pc)?,
            T_EQI => m.op_eqi(op, regs, pc)?,
            T_NEI => m.op_nei(op, regs, pc)?,
            T_GTI => m.op_gti(op, regs, pc)?,
            T_LEI => m.op_lei(op, regs, pc)?,
            T_GEI => m.op_gei(op, regs, pc)?,
            T_NEGI => m.op_negi(op, regs, pc)?,
            T_DIVF => m.op_divf(op, regs, pc)?,
            T_MODF => m.op_modf(op, regs, pc)?,
            T_NEGF => m.op_negf(op, regs, pc)?,
            T_EQF => m.op_eqf(op, regs, pc)?,
            T_NEF => m.op_nef(op, regs, pc)?,
            T_LTF => m.op_ltf(op, regs, pc)?,
            T_GTF => m.op_gtf(op, regs, pc)?,
            T_LEF => m.op_lef(op, regs, pc)?,
            T_GEF => m.op_gef(op, regs, pc)?,
            T_MOVREF => m.op_movref(op, regs, pc)?,
            T_CONST => m.op_const(op, regs, pc)?,
            T_STRCMP => m.op_strcmp(op, regs, pc)?,
            T_ARRAYCMP => m.op_arraycmp(op, regs, pc)?,
            T_REFEQ => m.op_refeq(op, regs, pc)?,
            T_BRTABLE => m.op_brtable(op, regs, pc)?,
            T_NEWCELL => m.op_newcell(op, regs, pc)?,
            T_MAKERECORD => m.op_makerecord(op, regs, pc)?,
            T_OWN => m.op_own(op, regs, pc)?,
            T_ARRNEW => m.op_arrnew(op, regs, pc)?,
            T_ARRLIT => m.op_arrlit(op, regs, pc)?,
            T_ENUMNEW => m.op_enumnew(op, regs, pc)?,
            T_OPTSOME => m.op_optsome(op, regs, pc)?,
            T_OPTNONE => m.op_optnone(op, regs, pc)?,
            T_RESOK => m.op_resok(op, regs, pc)?,
            T_RESERR => m.op_reserr(op, regs, pc)?,
            T_SUMIS => m.op_sumis(op, regs, pc)?,
            T_UNWRAP => m.op_unwrap(op, regs, pc)?,
            T_UNWRAPOR => m.op_unwrapor(op, regs, pc)?,
            T_EXPECT => m.op_expect(op, regs, pc)?,
            T_TIDOF => m.op_tidof(op, regs, pc)?,
            T_ISTYPE => m.op_istype(op, regs, pc)?,
            T_ISTRAIT => m.op_istrait(op, regs, pc)?,
            T_UNBOX => m.op_unbox(op, regs, pc)?,
            T_BOX => m.op_box(op, regs, pc)?,
            T_MAKECLOSURE => m.op_makeclosure(op, regs, pc)?,
            T_PANIC => m.op_panic(op, regs, pc)?,
            T_ASSERT => m.op_assert(op, regs, pc)?,
            T_CONV => m.op_conv(op, regs, pc)?,
            T_STRCHARAT => m.op_strcharat(op, regs, pc)?,
            _ => m.op_loophead(op, regs, pc)?,
        };
        match flow {
            Flow::Next(n) => pc = n,
            Flow::Done(o) => return Ok(ThreadOut::Done(o)),
            Flow::Redispatch => {
                let (c, t, r, p) = m.frame_ptrs();
                code = c;
                tags = t;
                regs = r;
                pc = p;
            }
        }
    }
}
