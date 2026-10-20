//! Portable backend — a loop over the same [`Machine`] op methods.
//!
//! Used on wasm and any target without verified `become` lowering. It reads
//! the same threaded state snapshot and calls the same `Machine::op_*`
//! methods, bailing on [`T_SLOW`] exactly like the native backend.

use crate::api::*;

pub fn build_table<M: Machine>() -> Table<M> {
    Table(core::marker::PhantomData)
}

pub fn run<M: Machine>(
    m: &mut M,
    pc0: u32,
    _table: &Table<M>,
) -> Result<ThreadOut<M::Out>, M::Err> {
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
            T_MOV => unsafe { m.op_mov(op, regs, pc)? },
            T_CONSTRAW => unsafe { m.op_constraw(op, regs, pc)? },
            T_NOT => unsafe { m.op_not(op, regs, pc)? },
            T_ADDI => unsafe { m.op_addi(op, regs, pc)? },
            T_SUBI => unsafe { m.op_subi(op, regs, pc)? },
            T_WADDI => unsafe { m.op_waddi(op, regs, pc)? },
            T_WSUBI => unsafe { m.op_wsubi(op, regs, pc)? },
            T_WMULI => unsafe { m.op_wmuli(op, regs, pc)? },
            T_LTI => unsafe { m.op_lti(op, regs, pc)? },
            T_ADDF => unsafe { m.op_addf(op, regs, pc)? },
            T_SUBF => unsafe { m.op_subf(op, regs, pc)? },
            T_MULF => unsafe { m.op_mulf(op, regs, pc)? },
            T_BR => unsafe { m.op_br(op, regs, pc)? },
            T_JMP => unsafe { m.op_jmp(op, regs, pc)? },
            T_ARRGET => unsafe { m.op_arr_get(op, regs, pc)? },
            T_ARRSET => unsafe { m.op_arr_set(op, regs, pc)? },
            T_ARRGETF => unsafe { m.op_arr_get_f(op, regs, pc)? },
            T_ARRSETF => unsafe { m.op_arr_set_f(op, regs, pc)? },
            T_GETF => unsafe { m.op_getf(op, regs, pc)? },
            T_SETF => unsafe { m.op_setf(op, regs, pc)? },
            T_CALL => unsafe { m.op_call(op, regs, pc)? },
            T_CALLM => unsafe { m.op_call_m(op, regs, pc)? },
            T_CALLI => unsafe { m.op_call_i(op, regs, pc)? },
            T_CALLFN => unsafe { m.op_call_fn(op, regs, pc)? },
            T_CALLNAT => unsafe { m.op_call_nat(op, regs, pc)? },
            T_RET => unsafe { m.op_ret(op, regs, pc)? },
            T_MULI => unsafe { m.op_muli(op, regs, pc)? },
            T_DIVI => unsafe { m.op_divi(op, regs, pc)? },
            T_MODI => unsafe { m.op_modi(op, regs, pc)? },
            T_ANDI => unsafe { m.op_andi(op, regs, pc)? },
            T_ORI => unsafe { m.op_ori(op, regs, pc)? },
            T_XORI => unsafe { m.op_xori(op, regs, pc)? },
            T_SHLI => unsafe { m.op_shli(op, regs, pc)? },
            T_SHRI => unsafe { m.op_shri(op, regs, pc)? },
            T_WRAPSHLI => unsafe { m.op_wrapshli(op, regs, pc)? },
            T_EQI => unsafe { m.op_eqi(op, regs, pc)? },
            T_NEI => unsafe { m.op_nei(op, regs, pc)? },
            T_GTI => unsafe { m.op_gti(op, regs, pc)? },
            T_LEI => unsafe { m.op_lei(op, regs, pc)? },
            T_GEI => unsafe { m.op_gei(op, regs, pc)? },
            T_NEGI => unsafe { m.op_negi(op, regs, pc)? },
            T_DIVF => unsafe { m.op_divf(op, regs, pc)? },
            T_MODF => unsafe { m.op_modf(op, regs, pc)? },
            T_NEGF => unsafe { m.op_negf(op, regs, pc)? },
            T_EQF => unsafe { m.op_eqf(op, regs, pc)? },
            T_NEF => unsafe { m.op_nef(op, regs, pc)? },
            T_LTF => unsafe { m.op_ltf(op, regs, pc)? },
            T_GTF => unsafe { m.op_gtf(op, regs, pc)? },
            T_LEF => unsafe { m.op_lef(op, regs, pc)? },
            T_GEF => unsafe { m.op_gef(op, regs, pc)? },
            T_MOVREF => unsafe { m.op_movref(op, regs, pc)? },
            T_CONST => unsafe { m.op_const(op, regs, pc)? },
            T_STRCMP => unsafe { m.op_strcmp(op, regs, pc)? },
            T_ARRAYCMP => unsafe { m.op_arraycmp(op, regs, pc)? },
            T_REFEQ => unsafe { m.op_refeq(op, regs, pc)? },
            T_BRTABLE => unsafe { m.op_brtable(op, regs, pc)? },
            T_NEWCELL => unsafe { m.op_newcell(op, regs, pc)? },
            T_MAKERECORD => unsafe { m.op_makerecord(op, regs, pc)? },
            T_OWN => unsafe { m.op_own(op, regs, pc)? },
            T_ARRNEW => unsafe { m.op_arrnew(op, regs, pc)? },
            T_ARRLIT => unsafe { m.op_arrlit(op, regs, pc)? },
            T_ENUMNEW => unsafe { m.op_enumnew(op, regs, pc)? },
            T_OPTSOME => unsafe { m.op_optsome(op, regs, pc)? },
            T_OPTNONE => unsafe { m.op_optnone(op, regs, pc)? },
            T_RESOK => unsafe { m.op_resok(op, regs, pc)? },
            T_RESERR => unsafe { m.op_reserr(op, regs, pc)? },
            T_SUMIS => unsafe { m.op_sumis(op, regs, pc)? },
            T_UNWRAP => unsafe { m.op_unwrap(op, regs, pc)? },
            T_UNWRAPOR => unsafe { m.op_unwrapor(op, regs, pc)? },
            T_EXPECT => unsafe { m.op_expect(op, regs, pc)? },
            T_TIDOF => unsafe { m.op_tidof(op, regs, pc)? },
            T_ISTYPE => unsafe { m.op_istype(op, regs, pc)? },
            T_ISTRAIT => unsafe { m.op_istrait(op, regs, pc)? },
            T_UNBOX => unsafe { m.op_unbox(op, regs, pc)? },
            T_BOX => unsafe { m.op_box(op, regs, pc)? },
            T_MAKECLOSURE => unsafe { m.op_makeclosure(op, regs, pc)? },
            T_PANIC => unsafe { m.op_panic(op, regs, pc)? },
            T_CONV => unsafe { m.op_conv(op, regs, pc)? },
            T_STRCODEAT => unsafe { m.op_strcodeat(op, regs, pc)? },
            _ => unsafe { m.op_loophead(op, regs, pc)? },
        };
        match flow {
            Flow::Next(n) => pc = n,
            // the root `Ret`: the LAST writeback of the run — without it
            // a clean exit drops the locals and `fuel_used` reads 0
            Flow::Done(o) => {
                m.sync_fuel(fuel, used);
                return Ok(ThreadOut::Done(o));
            }
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
