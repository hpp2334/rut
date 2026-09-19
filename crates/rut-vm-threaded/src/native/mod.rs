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
                // the root `Ret`: the LAST writeback of the run — without
                // it a clean exit drops the register-resident fuel
                // counters and `fuel_used` reads 0 (RFC 0040)
                Flow::Done(o) => {
                    unsafe { (*m).sync_fuel(next_fuel, used) };
                    return Ok(ThreadOut::Done(o));
                }
                Flow::Redispatch => {
                    let (c, t, r, p) = unsafe { (*m).frame_ptrs() };
                    let tag = unsafe { *t.add(p as usize) } as usize;
                    let h = unsafe { *(table as *const Handler<M>).add(tag) };
                    become h(m, table, c, t, r, p, next_fuel, used)
                }
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
op_handler!(h_arrget, op_arr_get);
op_handler!(h_arrset, op_arr_set);
op_handler!(h_arrgetf, op_arr_get_f);
op_handler!(h_arrsetf, op_arr_set_f);
op_handler!(h_getf, op_getf);
op_handler!(h_setf, op_setf);
op_handler!(h_call, op_call);
op_handler!(h_callm, op_call_m);
op_handler!(h_calli, op_call_i);
op_handler!(h_callfn, op_call_fn);
op_handler!(h_callnat, op_call_nat);
op_handler!(h_ret, op_ret);
op_handler!(h_muli, op_muli);
op_handler!(h_divi, op_divi);
op_handler!(h_modi, op_modi);
op_handler!(h_andi, op_andi);
op_handler!(h_ori, op_ori);
op_handler!(h_xori, op_xori);
op_handler!(h_shli, op_shli);
op_handler!(h_shri, op_shri);
op_handler!(h_wrapshli, op_wrapshli);
op_handler!(h_eqi, op_eqi);
op_handler!(h_nei, op_nei);
op_handler!(h_gti, op_gti);
op_handler!(h_lei, op_lei);
op_handler!(h_gei, op_gei);
op_handler!(h_negi, op_negi);
op_handler!(h_divf, op_divf);
op_handler!(h_modf, op_modf);
op_handler!(h_negf, op_negf);
op_handler!(h_eqf, op_eqf);
op_handler!(h_nef, op_nef);
op_handler!(h_ltf, op_ltf);
op_handler!(h_gtf, op_gtf);
op_handler!(h_lef, op_lef);
op_handler!(h_gef, op_gef);
op_handler!(h_movref, op_movref);
op_handler!(h_const, op_const);
op_handler!(h_strcmp, op_strcmp);
op_handler!(h_arraycmp, op_arraycmp);
op_handler!(h_refeq, op_refeq);
op_handler!(h_brtable, op_brtable);
op_handler!(h_newcell, op_newcell);
op_handler!(h_makerecord, op_makerecord);
op_handler!(h_own, op_own);
op_handler!(h_arrnew, op_arrnew);
op_handler!(h_arrlit, op_arrlit);
op_handler!(h_enumnew, op_enumnew);
op_handler!(h_optsome, op_optsome);
op_handler!(h_optnone, op_optnone);
op_handler!(h_resok, op_resok);
op_handler!(h_reserr, op_reserr);
op_handler!(h_sumis, op_sumis);
op_handler!(h_unwrap, op_unwrap);
op_handler!(h_unwrapor, op_unwrapor);
op_handler!(h_expect, op_expect);
op_handler!(h_tidof, op_tidof);
op_handler!(h_istype, op_istype);
op_handler!(h_istrait, op_istrait);
op_handler!(h_unbox, op_unbox);
op_handler!(h_box, op_box);
op_handler!(h_makeclosure, op_makeclosure);
op_handler!(h_panic, op_panic);
op_handler!(h_assert, op_assert);
op_handler!(h_conv, op_conv);
op_handler!(h_strcharat, op_strcharat);

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
    t[T_ARRGET as usize] = h_arrget::<M>;
    t[T_ARRSET as usize] = h_arrset::<M>;
    t[T_ARRGETF as usize] = h_arrgetf::<M>;
    t[T_ARRSETF as usize] = h_arrsetf::<M>;
    t[T_GETF as usize] = h_getf::<M>;
    t[T_SETF as usize] = h_setf::<M>;
    t[T_CALL as usize] = h_call::<M>;
    t[T_CALLM as usize] = h_callm::<M>;
    t[T_CALLI as usize] = h_calli::<M>;
    t[T_CALLFN as usize] = h_callfn::<M>;
    t[T_CALLNAT as usize] = h_callnat::<M>;
    t[T_RET as usize] = h_ret::<M>;
    t[T_MULI as usize] = h_muli::<M>;
    t[T_DIVI as usize] = h_divi::<M>;
    t[T_MODI as usize] = h_modi::<M>;
    t[T_ANDI as usize] = h_andi::<M>;
    t[T_ORI as usize] = h_ori::<M>;
    t[T_XORI as usize] = h_xori::<M>;
    t[T_SHLI as usize] = h_shli::<M>;
    t[T_SHRI as usize] = h_shri::<M>;
    t[T_WRAPSHLI as usize] = h_wrapshli::<M>;
    t[T_EQI as usize] = h_eqi::<M>;
    t[T_NEI as usize] = h_nei::<M>;
    t[T_GTI as usize] = h_gti::<M>;
    t[T_LEI as usize] = h_lei::<M>;
    t[T_GEI as usize] = h_gei::<M>;
    t[T_NEGI as usize] = h_negi::<M>;
    t[T_DIVF as usize] = h_divf::<M>;
    t[T_MODF as usize] = h_modf::<M>;
    t[T_NEGF as usize] = h_negf::<M>;
    t[T_EQF as usize] = h_eqf::<M>;
    t[T_NEF as usize] = h_nef::<M>;
    t[T_LTF as usize] = h_ltf::<M>;
    t[T_GTF as usize] = h_gtf::<M>;
    t[T_LEF as usize] = h_lef::<M>;
    t[T_GEF as usize] = h_gef::<M>;
    t[T_MOVREF as usize] = h_movref::<M>;
    t[T_CONST as usize] = h_const::<M>;
    t[T_STRCMP as usize] = h_strcmp::<M>;
    t[T_ARRAYCMP as usize] = h_arraycmp::<M>;
    t[T_REFEQ as usize] = h_refeq::<M>;
    t[T_BRTABLE as usize] = h_brtable::<M>;
    t[T_NEWCELL as usize] = h_newcell::<M>;
    t[T_MAKERECORD as usize] = h_makerecord::<M>;
    t[T_OWN as usize] = h_own::<M>;
    t[T_ARRNEW as usize] = h_arrnew::<M>;
    t[T_ARRLIT as usize] = h_arrlit::<M>;
    t[T_ENUMNEW as usize] = h_enumnew::<M>;
    t[T_OPTSOME as usize] = h_optsome::<M>;
    t[T_OPTNONE as usize] = h_optnone::<M>;
    t[T_RESOK as usize] = h_resok::<M>;
    t[T_RESERR as usize] = h_reserr::<M>;
    t[T_SUMIS as usize] = h_sumis::<M>;
    t[T_UNWRAP as usize] = h_unwrap::<M>;
    t[T_UNWRAPOR as usize] = h_unwrapor::<M>;
    t[T_EXPECT as usize] = h_expect::<M>;
    t[T_TIDOF as usize] = h_tidof::<M>;
    t[T_ISTYPE as usize] = h_istype::<M>;
    t[T_ISTRAIT as usize] = h_istrait::<M>;
    t[T_UNBOX as usize] = h_unbox::<M>;
    t[T_BOX as usize] = h_box::<M>;
    t[T_MAKECLOSURE as usize] = h_makeclosure::<M>;
    t[T_PANIC as usize] = h_panic::<M>;
    t[T_ASSERT as usize] = h_assert::<M>;
    t[T_CONV as usize] = h_conv::<M>;
    t[T_STRCHARAT as usize] = h_strcharat::<M>;
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
