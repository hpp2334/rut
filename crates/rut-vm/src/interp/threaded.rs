//! `impl rut_vm_threaded::Machine for Vm` — the threaded op bodies.
//!
//! These methods are the **single source of truth** for the threaded op
//! behavior, called by both the native `become` handlers and the portable
//! wasm loop. They read/write registers through the `regs` pointer passed
//! as an argument (not `self.cur_regs`), so the hot loop keeps state in
//! registers.
//!
//! Ops not implemented here map to `T_SLOW` and run through the match
//! interpreter in `run_loop`/`step_one`.

use super::*;
use rut_vm_threaded::{Flow, Machine, ThreadState};

macro_rules! int_arith {
    ($name:ident, $variant:ident, $OP:expr, $WRAP:expr) => {
        fn $name(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
            let Op::$variant { prim, dst, a, b } = op else {
                unreachable!(concat!(stringify!($name), ": unexpected op"))
            };
            let x = unsafe { *regs.add(*a as usize) };
            let y = unsafe { *regs.add(*b as usize) };
            let v = self.arith_int::<$OP, $WRAP>(*prim, x, y)?;
            unsafe { *regs.add(*dst as usize) = v };
            Ok(Flow::Next(pc + 1))
        }
    };
}

macro_rules! float_arith {
    ($name:ident, $variant:ident, $FOP:expr) => {
        fn $name(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
            let Op::$variant { prim, dst, a, b } = op else {
                unreachable!(concat!(stringify!($name), ": unexpected op"))
            };
            let x = unsafe { *regs.add(*a as usize) };
            let y = unsafe { *regs.add(*b as usize) };
            let v = self.arith_float::<$FOP>(*prim, x, y);
            unsafe { *regs.add(*dst as usize) = v };
            Ok(Flow::Next(pc + 1))
        }
    };
}

macro_rules! bitop {
    ($name:ident, $variant:ident, $OP:expr) => {
        fn $name(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
            let Op::$variant { prim, dst, a, b } = op else {
                unreachable!(concat!(stringify!($name), ": unexpected op"))
            };
            let x = unsafe { *regs.add(*a as usize) };
            let y = unsafe { *regs.add(*b as usize) };
            let v = self.bitop_int::<$OP>(*prim, x, y)?;
            unsafe { *regs.add(*dst as usize) = v };
            Ok(Flow::Next(pc + 1))
        }
    };
}

macro_rules! cmpi {
    ($name:ident, $variant:ident, $OP:expr) => {
        fn $name(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
            let Op::$variant { prim, dst, a, b } = op else {
                unreachable!(concat!(stringify!($name), ": unexpected op"))
            };
            let x = unsafe { *regs.add(*a as usize) };
            let y = unsafe { *regs.add(*b as usize) };
            let v = self.cmp_int::<$OP>(*prim, x, y);
            unsafe { *regs.add(*dst as usize) = Slot::bool(v) };
            Ok(Flow::Next(pc + 1))
        }
    };
}

macro_rules! cmpf {
    ($name:ident, $variant:ident, $OP:expr) => {
        fn $name(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
            let Op::$variant { dst, a, b } = op else {
                unreachable!(concat!(stringify!($name), ": unexpected op"))
            };
            let x = unsafe { *regs.add(*a as usize) };
            let y = unsafe { *regs.add(*b as usize) };
            let v = self.cmp_float::<$OP>(x, y);
            unsafe { *regs.add(*dst as usize) = Slot::bool(v) };
            Ok(Flow::Next(pc + 1))
        }
    };
}

impl Vm {
    /// One op of fuel for the match path (RFC 0040): parks at `pc` on
    /// exhaustion so `resume()` re-executes it.
    pub(super) fn tick(&mut self, pc: u32) -> Result<(), Trap> {
        self.fuel_used += 1;
        self.since_check += 1;
        if let Some(fuel) = self.fuel {
            if fuel == 0 {
                self.cur_pc = pc;
                return Err(Trap::new(
                    TrapKind::OutOfFuel,
                    "fuel exhausted — the frame is parked; add fuel and resume()",
                ));
            }
            self.fuel = Some(fuel - 1);
        } else if self.since_check >= self.interrupt_every {
            self.since_check = 0;
        }
        Ok(())
    }
}

impl Machine for Vm {
    type Out = Value;
    type Err = Trap;
    type Word = Slot;

    fn thread_state(&mut self) -> ThreadState<Slot> {
        let f = &self.prog.funcs[self.cur_func as usize];
        ThreadState {
            code: f.code.as_ptr(),
            tags: self.op_tags[self.cur_func as usize].as_ptr(),
            regs: self.cur_regs.as_mut_ptr(),
            pc: self.cur_pc,
            fuel: self.fuel.map(|f| f.min(i64::MAX as u64) as i64).unwrap_or(-1),
            fuel_used: self.fuel_used,
        }
    }

    fn sync(&mut self, pc: u32, fuel: i64, fuel_used: u64) {
        self.cur_pc = pc;
        self.fuel_used = fuel_used;
        self.fuel = if fuel < 0 { None } else { Some(fuel as u64) };
    }

    fn park(&mut self, pc: u32, fuel_used: u64) -> Trap {
        self.cur_pc = pc;
        self.fuel_used = fuel_used;
        self.fuel = Some(0);
        Trap::new(
            TrapKind::OutOfFuel,
            "fuel exhausted — the frame is parked; add fuel and resume()",
        )
    }

    fn sync_fuel(&mut self, fuel: i64, fuel_used: u64) {
        self.fuel_used = fuel_used;
        self.fuel = if fuel < 0 { None } else { Some(fuel as u64) };
    }

    fn op_mov(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::Mov { dst, src } = op else { unreachable!("op_mov: unexpected op") };
        unsafe { *regs.add(*dst as usize) = *regs.add(*src as usize) };
        Ok(Flow::Next(pc + 1))
    }

    fn op_constraw(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::ConstRaw { dst, bits } = op else {
            unreachable!("op_constraw: unexpected op")
        };
        unsafe { *regs.add(*dst as usize) = Slot { i: *bits as i64 } };
        Ok(Flow::Next(pc + 1))
    }

    fn op_not(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::Not { dst, a } = op else { unreachable!("op_not: unexpected op") };
        let v = unsafe { !(*regs.add(*a as usize)).as_bool() };
        unsafe { *regs.add(*dst as usize) = Slot::bool(v) };
        Ok(Flow::Next(pc + 1))
    }

    int_arith!(op_addi, AddI, { IOP_ADD }, false);
    int_arith!(op_subi, SubI, { IOP_SUB }, false);
    int_arith!(op_waddi, WAddI, { IOP_ADD }, true);
    int_arith!(op_wsubi, WSubI, { IOP_SUB }, true);
    int_arith!(op_wmuli, WMulI, { IOP_MUL }, true);

    fn op_lti(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::LtI { prim, dst, a, b } = op else { unreachable!("op_lti: unexpected op") };
        let x = unsafe { *regs.add(*a as usize) };
        let y = unsafe { *regs.add(*b as usize) };
        let v = self.cmp_int::<{ COP_LT }>(*prim, x, y);
        unsafe { *regs.add(*dst as usize) = Slot::bool(v) };
        Ok(Flow::Next(pc + 1))
    }

    float_arith!(op_addf, AddF, { FOP_ADD });
    float_arith!(op_subf, SubF, { FOP_SUB });
    float_arith!(op_mulf, MulF, { FOP_MUL });

    int_arith!(op_muli, MulI, { IOP_MUL }, false);
    int_arith!(op_divi, DivI, { IOP_DIV }, false);
    int_arith!(op_modi, ModI, { IOP_MOD }, false);
    int_arith!(op_wdivi, WDivI, { IOP_DIV }, true);
    int_arith!(op_wmodi, WModI, { IOP_MOD }, true);

    bitop!(op_andi, AndI, { BOP_AND });
    bitop!(op_ori, OrI, { BOP_OR });
    bitop!(op_xori, XorI, { BOP_XOR });
    bitop!(op_shli, ShlI, { BOP_SHL });
    bitop!(op_shri, ShrI, { BOP_SHR });
    bitop!(op_wrapshli, WrapShlI, { BOP_WRAPSHL });

    cmpi!(op_eqi, EqI, { COP_EQ });
    cmpi!(op_nei, NeI, { COP_NE });
    cmpi!(op_gti, GtI, { COP_GT });
    cmpi!(op_lei, LeI, { COP_LE });
    cmpi!(op_gei, GeI, { COP_GE });

    fn op_negi(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::NegI { prim, dst, a } = op else {
            unreachable!("op_negi: unexpected op")
        };
        let x = unsafe { *regs.add(*a as usize) };
        let v = self.neg_int(*prim, x)?;
        unsafe { *regs.add(*dst as usize) = v };
        Ok(Flow::Next(pc + 1))
    }

    float_arith!(op_divf, DivF, { FOP_DIV });
    float_arith!(op_modf, ModF, { FOP_MOD });

    fn op_negf(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::NegF { prim, dst, a } = op else {
            unreachable!("op_negf: unexpected op")
        };
        let x = unsafe { *regs.add(*a as usize) };
        let v = self.neg_float(*prim, x);
        unsafe { *regs.add(*dst as usize) = v };
        Ok(Flow::Next(pc + 1))
    }

    cmpf!(op_eqf, EqF, { COP_EQ });
    cmpf!(op_nef, NeF, { COP_NE });
    cmpf!(op_ltf, LtF, { COP_LT });
    cmpf!(op_gtf, GtF, { COP_GT });
    cmpf!(op_lef, LeF, { COP_LE });
    cmpf!(op_gef, GeF, { COP_GE });

    fn op_br(&mut self, op: &Op, regs: *mut Slot, _pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::Br { cond, then_t, else_t } = op else {
            unreachable!("op_br: unexpected op")
        };
        let c = unsafe { (*regs.add(*cond as usize)).as_bool() };
        let target = if c { *then_t } else { *else_t };
        Ok(Flow::Next(target))
    }

    fn op_jmp(&mut self, op: &Op, _regs: *mut Slot, _pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::Jmp { target } = op else { unreachable!("op_jmp: unexpected op") };
        Ok(Flow::Next(*target))
    }

    fn op_loophead(&mut self, _op: &Op, _regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        Ok(Flow::Next(pc + 1))
    }

    // ---- array / field ops (no frame change) ----

    fn op_arr_get(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::ArrGet { dst, arr, idx, repr } = op else {
            unreachable!("op_arr_get: unexpected op")
        };
        let i = unsafe { (*regs.add(*idx as usize)).i };
        let cell = cell_of(unsafe { *regs.add(*arr as usize) });
        let v = seq_get(cell, i)?;
        let old = unsafe { *regs.add(*dst as usize) };
        unsafe { *regs.add(*dst as usize) = v };
        if repr.is_ref() {
            self.heap.retain(v);
            self.heap.release(old);
        }
        Ok(Flow::Next(pc + 1))
    }

    fn op_arr_set(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::ArrSet { arr, idx, val, repr } = op else {
            unreachable!("op_arr_set: unexpected op")
        };
        let i = unsafe { (*regs.add(*idx as usize)).i };
        let cell = cell_of(unsafe { *regs.add(*arr as usize) });
        let v = unsafe { *regs.add(*val as usize) };
        let old = seq_set(cell, i, v)?;
        if repr.is_ref() {
            self.heap.retain(v);
            self.heap.release(old);
        }
        Ok(Flow::Next(pc + 1))
    }

    fn op_arr_get_f(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::ArrGetF { dst, obj, field, idx, repr } = op else {
            unreachable!("op_arr_get_f: unexpected op")
        };
        let i = unsafe { (*regs.add(*idx as usize)).i };
        let arr = {
            let obj_cell = cell_of(unsafe { *regs.add(*obj as usize) });
            match &obj_cell.data {
                CellData::Record { fields } => fields
                    .borrow()
                    .get(*field as usize)
                    .ok_or_else(|| Trap::new(TrapKind::Invalid, "field index out of range"))?,
                _ => return Err(Trap::new(TrapKind::Invalid, "field-array get on non-record")),
            }
        };
        let v = seq_get(cell_of(arr), i)?;
        let old = unsafe { *regs.add(*dst as usize) };
        unsafe { *regs.add(*dst as usize) = v };
        if repr.is_ref() {
            self.heap.retain(v);
            self.heap.release(old);
        }
        Ok(Flow::Next(pc + 1))
    }

    fn op_arr_set_f(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::ArrSetF { obj, field, idx, val, repr } = op else {
            unreachable!("op_arr_set_f: unexpected op")
        };
        let i = unsafe { (*regs.add(*idx as usize)).i };
        let v = unsafe { *regs.add(*val as usize) };
        let arr = {
            let obj_cell = cell_of(unsafe { *regs.add(*obj as usize) });
            match &obj_cell.data {
                CellData::Record { fields } => fields
                    .borrow()
                    .get(*field as usize)
                    .ok_or_else(|| Trap::new(TrapKind::Invalid, "field index out of range"))?,
                _ => return Err(Trap::new(TrapKind::Invalid, "field-array set on non-record")),
            }
        };
        let old = seq_set(cell_of(arr), i, v)?;
        if repr.is_ref() {
            self.heap.retain(v);
            self.heap.release(old);
        }
        Ok(Flow::Next(pc + 1))
    }

    fn op_getf(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::GetF { dst, obj, field, repr } = op else {
            unreachable!("op_getf: unexpected op")
        };
        let cell = cell_of(unsafe { *regs.add(*obj as usize) });
        let v = match &cell.data {
            CellData::Record { fields } => fields
                .borrow()
                .get(*field as usize)
                .ok_or_else(|| Trap::new(TrapKind::Invalid, "field index out of range"))?,
            _ => return Err(Trap::new(TrapKind::Invalid, "getf on non-record")),
        };
        let old = unsafe { *regs.add(*dst as usize) };
        unsafe { *regs.add(*dst as usize) = v };
        if repr.is_ref() {
            self.heap.retain(v);
            self.heap.release(old);
        }
        Ok(Flow::Next(pc + 1))
    }

    fn op_setf(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::SetF { obj, field, val, repr } = op else {
            unreachable!("op_setf: unexpected op")
        };
        let cell = cell_of(unsafe { *regs.add(*obj as usize) });
        let v = unsafe { *regs.add(*val as usize) };
        let old = match &cell.data {
            CellData::Record { fields } => fields
                .borrow_mut()
                .set(*field as usize, v)
                .ok_or_else(|| Trap::new(TrapKind::Invalid, "field index out of range"))?,
            _ => return Err(Trap::new(TrapKind::Invalid, "setf on non-record")),
        };
        if repr.is_ref() {
            self.heap.retain(v);
            self.heap.release(old);
        }
        Ok(Flow::Next(pc + 1))
    }

    // ---- calls / ret (frame change => Redispatch) ----

    fn op_call(&mut self, op: &Op, _regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::Call { func, args, dst } = op else {
            unreachable!("op_call: unexpected op")
        };
        if self.prog.funcs[*func as usize].host.is_some() {
            self.call_host(*func, args, *dst)?;
            Ok(Flow::Next(pc + 1))
        } else {
            self.cur_pc = pc + 1;
            self.op_call(*func, args, *dst);
            Ok(Flow::Redispatch)
        }
    }

    fn op_call_m(&mut self, op: &Op, _regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::CallM { func, recv, args, dst } = op else {
            unreachable!("op_call_m: unexpected op")
        };
        self.cur_pc = pc + 1;
        self.op_call_m(*func, *recv, args, *dst);
        Ok(Flow::Redispatch)
    }

    fn op_call_i(&mut self, op: &Op, _regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::CallI { slot, recv, args, dst } = op else {
            unreachable!("op_call_i: unexpected op")
        };
        self.cur_pc = pc + 1;
        self.op_call_i(*slot, *recv, args, *dst)?;
        Ok(Flow::Redispatch)
    }

    fn op_call_fn(&mut self, op: &Op, _regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::CallFn { fval, args, dst } = op else {
            unreachable!("op_call_fn: unexpected op")
        };
        self.cur_pc = pc + 1;
        self.op_call_fn(*fval, args, *dst)?;
        Ok(Flow::Redispatch)
    }

    fn op_call_nat(&mut self, op: &Op, _regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::CallNat { nat, recv, args, dst } = op else {
            unreachable!("op_call_nat: unexpected op")
        };
        self.call_nat(*nat, *recv, args, *dst)?;
        Ok(Flow::Next(pc + 1))
    }

    fn op_ret(&mut self, op: &Op, regs: *mut Slot, _pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::Ret { val } = op else { unreachable!("op_ret: unexpected op") };
        let v = val
            .map(|r| unsafe { *regs.add(r as usize) })
            .unwrap_or(Slot::int(0));
        if let Some(out) = self.do_ret(v) {
            Ok(Flow::Done(out))
        } else {
            Ok(Flow::Redispatch)
        }
    }
}
