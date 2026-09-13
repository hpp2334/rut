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

/// Extract an op's fields. The tag→handler mapping makes a mismatch
/// impossible, but this keeps a **runtime check** so a bug is a clean panic
/// rather than UB. The diverging path is a `#[cold] #[inline(never)]` call,
/// so the compiler treats it as rarely taken and keeps it off the hot path
/// while the check itself stays.
macro_rules! unreachable_op {
    ($($t:tt)*) => {{
        #[cold]
        #[inline(never)]
        fn bad_op() -> ! {
            panic!($($t)*)
        }
        bad_op()
    }};
}

macro_rules! int_arith {
    ($name:ident, $variant:ident, $OP:expr, $WRAP:expr) => {
        fn $name(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
            let Op::$variant { prim, dst, a, b } = op else {
                unreachable_op!(concat!(stringify!($name), ": unexpected op"))
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
                unreachable_op!(concat!(stringify!($name), ": unexpected op"))
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
                unreachable_op!(concat!(stringify!($name), ": unexpected op"))
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
                unreachable_op!(concat!(stringify!($name), ": unexpected op"))
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
                unreachable_op!(concat!(stringify!($name), ": unexpected op"))
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

    fn frame_ptrs(&mut self) -> (*const Op, *const u8, *mut Slot, u32) {
        let f = &self.prog.funcs[self.cur_func as usize];
        (
            f.code.as_ptr(),
            self.op_tags[self.cur_func as usize].as_ptr(),
            self.cur_regs.as_mut_ptr(),
            self.cur_pc,
        )
    }

    fn op_mov(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::Mov { dst, src } = op else { unreachable_op!("op_mov: unexpected op") };
        unsafe { *regs.add(*dst as usize) = *regs.add(*src as usize) };
        Ok(Flow::Next(pc + 1))
    }

    fn op_constraw(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::ConstRaw { dst, bits } = op else {
            unreachable_op!("op_constraw: unexpected op")
        };
        unsafe { *regs.add(*dst as usize) = Slot { i: *bits as i64 } };
        Ok(Flow::Next(pc + 1))
    }

    fn op_not(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::Not { dst, a } = op else { unreachable_op!("op_not: unexpected op") };
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
        let Op::LtI { prim, dst, a, b } = op else { unreachable_op!("op_lti: unexpected op") };
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
            unreachable_op!("op_negi: unexpected op")
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
            unreachable_op!("op_negf: unexpected op")
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
            unreachable_op!("op_br: unexpected op")
        };
        let c = unsafe { (*regs.add(*cond as usize)).as_bool() };
        let target = if c { *then_t } else { *else_t };
        Ok(Flow::Next(target))
    }

    fn op_jmp(&mut self, op: &Op, _regs: *mut Slot, _pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::Jmp { target } = op else { unreachable_op!("op_jmp: unexpected op") };
        Ok(Flow::Next(*target))
    }

    fn op_loophead(&mut self, _op: &Op, _regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        Ok(Flow::Next(pc + 1))
    }

    // ---- array / field ops (no frame change) ----

    fn op_arr_get(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::ArrGet { dst, arr, idx, repr } = op else {
            unreachable_op!("op_arr_get: unexpected op")
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
            unreachable_op!("op_arr_set: unexpected op")
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
            unreachable_op!("op_arr_get_f: unexpected op")
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
            unreachable_op!("op_arr_set_f: unexpected op")
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
            unreachable_op!("op_getf: unexpected op")
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
            unreachable_op!("op_setf: unexpected op")
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
            unreachable_op!("op_call: unexpected op")
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
            unreachable_op!("op_call_m: unexpected op")
        };
        self.cur_pc = pc + 1;
        self.op_call_m(*func, *recv, args, *dst);
        Ok(Flow::Redispatch)
    }

    fn op_call_i(&mut self, op: &Op, _regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::CallI { slot, recv, args, dst } = op else {
            unreachable_op!("op_call_i: unexpected op")
        };
        self.cur_pc = pc + 1;
        self.op_call_i(*slot, *recv, args, *dst)?;
        Ok(Flow::Redispatch)
    }

    fn op_call_fn(&mut self, op: &Op, _regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::CallFn { fval, args, dst } = op else {
            unreachable_op!("op_call_fn: unexpected op")
        };
        self.cur_pc = pc + 1;
        self.op_call_fn(*fval, args, *dst)?;
        Ok(Flow::Redispatch)
    }

    fn op_call_nat(&mut self, op: &Op, _regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::CallNat { nat, recv, args, dst } = op else {
            unreachable_op!("op_call_nat: unexpected op")
        };
        self.call_nat(*nat, *recv, args, *dst)?;
        Ok(Flow::Next(pc + 1))
    }

    fn op_ret(&mut self, op: &Op, regs: *mut Slot, _pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::Ret { val } = op else { unreachable_op!("op_ret: unexpected op") };
        let v = val
            .map(|r| unsafe { *regs.add(r as usize) })
            .unwrap_or(Slot::int(0));
        if let Some(out) = self.do_ret(v) {
            Ok(Flow::Done(out))
        } else {
            Ok(Flow::Redispatch)
        }
    }

    // ---- remaining ops (no frame change): identical bodies to `step`, but
    // dispatched without the bail round-trip. They read `self.cur_regs`,
    // which aliases the `regs` pointer, so the shared heap helpers apply. ----

    fn op_movref(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::MovRef { dst, src } = op else { unreachable_op!("op_movref: unexpected op") };
        let v = unsafe { *regs.add(*src as usize) };
        self.heap.retain(v);
        let old = self.cur_regs[*dst as usize];
        self.cur_regs[*dst as usize] = v;
        self.heap.release(old);
        Ok(Flow::Next(pc + 1))
    }

    fn op_const(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::Const { dst, k } = op else { unreachable_op!("op_const: unexpected op") };
        let s = self.const_slots[*k as usize];
        let is_ref = matches!(self.prog.consts[*k as usize], ConstVal::Str(_));
        if is_ref {
            self.heap.retain(s);
        }
        let old = unsafe { *regs.add(*dst as usize) };
        unsafe { *regs.add(*dst as usize) = s };
        if is_ref {
            self.heap.release(old);
        }
        Ok(Flow::Next(pc + 1))
    }

    fn op_strcmp(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::StrCmp { eq, dst, a, b } = op else { unreachable_op!("op_strcmp: unexpected op") };
        let sa = cell_of(unsafe { *regs.add(*a as usize) }).as_str();
        let sb = cell_of(unsafe { *regs.add(*b as usize) }).as_str();
        unsafe { *regs.add(*dst as usize) = Slot::bool((sa == sb) == *eq) };
        Ok(Flow::Next(pc + 1))
    }

    fn op_arraycmp(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::ArrayCmp { eq, dst, a, b } = op else {
            unreachable_op!("op_arraycmp: unexpected op")
        };
        let same = cell_of(unsafe { *regs.add(*a as usize) })
            .array_eq(cell_of(unsafe { *regs.add(*b as usize) }));
        unsafe { *regs.add(*dst as usize) = Slot::bool(same == *eq) };
        Ok(Flow::Next(pc + 1))
    }

    fn op_refeq(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::RefEq { eq, dst, a, b } = op else { unreachable_op!("op_refeq: unexpected op") };
        let v = Slot::same_ref(
            unsafe { *regs.add(*a as usize) },
            unsafe { *regs.add(*b as usize) },
        ) == *eq;
        unsafe { *regs.add(*dst as usize) = Slot::bool(v) };
        Ok(Flow::Next(pc + 1))
    }

    fn op_brtable(&mut self, op: &Op, regs: *mut Slot, _pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::BrTable { idx, table, default } = op else {
            unreachable_op!("op_brtable: unexpected op")
        };
        let m = cell_of(unsafe { *regs.add(*idx as usize) })
            .as_enum_member()
            .unwrap_or(u32::MAX) as usize;
        Ok(Flow::Next(table.get(m).copied().unwrap_or(*default)))
    }

    fn op_newcell(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::NewCell { dst, ty } = op else { unreachable_op!("op_newcell: unexpected op") };
        let n = match self.prog.types.kind(*ty) {
            TyKind::Data { fields } => fields.len(),
            _ => 0,
        };
        let c = self.heap.alloc_record_zeroed(*ty, n)?;
        let old = unsafe { *regs.add(*dst as usize) };
        unsafe { *regs.add(*dst as usize) = c };
        self.heap.release(old);
        Ok(Flow::Next(pc + 1))
    }

    fn op_makerecord(&mut self, op: &Op, _regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::MakeRecord { dst, ty, vals } = op else {
            unreachable_op!("op_makerecord: unexpected op")
        };
        self.op_make_record(*dst, *ty, vals)?;
        Ok(Flow::Next(pc + 1))
    }

    fn op_own(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::Own { dst, src, ty } = op else { unreachable_op!("op_own: unexpected op") };
        let v = self
            .heap
            .own(unsafe { *regs.add(*src as usize) }, *ty, &self.prog.types)?;
        let old = unsafe { *regs.add(*dst as usize) };
        unsafe { *regs.add(*dst as usize) = v };
        if self.is_ref(*ty) {
            self.heap.release(old);
        }
        Ok(Flow::Next(pc + 1))
    }

    fn op_arrnew(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::ArrNew { dst, ty, len, repr } = op else {
            unreachable_op!("op_arrnew: unexpected op")
        };
        let elem = match self.prog.types.kind(*ty) {
            TyKind::Array { elem } => *elem,
            TyKind::Bytes => rut_core::types::TY_U8,
            _ => return Err(Trap::new(TrapKind::Invalid, "arrnew on non-array")),
        };
        let n = unsafe { (*regs.add(*len as usize)).i }.max(0) as usize;
        let c = self
            .heap
            .alloc_array_filled(elem, n, default_slot_repr(*repr), &self.prog.types)?;
        let old = unsafe { *regs.add(*dst as usize) };
        unsafe { *regs.add(*dst as usize) = c };
        self.heap.release(old);
        Ok(Flow::Next(pc + 1))
    }

    fn op_arrlit(&mut self, op: &Op, _regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::ArrLit { dst, ty, elems } = op else {
            unreachable_op!("op_arrlit: unexpected op")
        };
        self.op_arr_lit(*dst, *ty, elems)?;
        Ok(Flow::Next(pc + 1))
    }

    fn op_enumnew(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::EnumNew { dst, ty, member } = op else {
            unreachable_op!("op_enumnew: unexpected op")
        };
        let c = self.heap.enum_member(*ty, *member)?;
        self.heap.retain(c);
        let old = unsafe { *regs.add(*dst as usize) };
        unsafe { *regs.add(*dst as usize) = c };
        self.heap.release(old);
        Ok(Flow::Next(pc + 1))
    }

    fn op_optsome(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::OptSome { dst, ty, val } = op else {
            unreachable_op!("op_optsome: unexpected op")
        };
        self.alloc_sum(*dst, *ty, 0, Some(unsafe { *regs.add(*val as usize) }))?;
        Ok(Flow::Next(pc + 1))
    }

    fn op_optnone(&mut self, op: &Op, _regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::OptNone { dst, ty } = op else { unreachable_op!("op_optnone: unexpected op") };
        self.alloc_sum(*dst, *ty, 1, None)?;
        Ok(Flow::Next(pc + 1))
    }

    fn op_resok(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::ResOk { dst, ty, val } = op else { unreachable_op!("op_resok: unexpected op") };
        self.alloc_sum(*dst, *ty, 0, Some(unsafe { *regs.add(*val as usize) }))?;
        Ok(Flow::Next(pc + 1))
    }

    fn op_reserr(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::ResErr { dst, ty, val } = op else {
            unreachable_op!("op_reserr: unexpected op")
        };
        self.alloc_sum(*dst, *ty, 1, Some(unsafe { *regs.add(*val as usize) }))?;
        Ok(Flow::Next(pc + 1))
    }

    fn op_sumis(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::SumIs { dst, v, want_err } = op else {
            unreachable_op!("op_sumis: unexpected op")
        };
        let tag = sum_tag(cell_of(unsafe { *regs.add(*v as usize) }))?;
        unsafe { *regs.add(*dst as usize) = Slot::bool((tag == 1) == *want_err) };
        Ok(Flow::Next(pc + 1))
    }

    fn op_unwrap(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::Unwrap { dst, v, want_err } = op else {
            unreachable_op!("op_unwrap: unexpected op")
        };
        let cell = cell_of(unsafe { *regs.add(*v as usize) });
        let (tag, payload) = sum_parts(cell)?;
        if (tag == 1) != *want_err {
            return Err(Trap::new(
                TrapKind::UnwrapNone,
                if *want_err {
                    "`.error` on Ok"
                } else {
                    "`.value` on None/Err — the value is absent (RFC 0005)"
                },
            ));
        }
        let val = payload.unwrap_or(Slot::int(0));
        let pty = self.sum_payload_ty(cell.ty, *want_err);
        self.move_sum_val(*dst, val, pty);
        Ok(Flow::Next(pc + 1))
    }

    fn op_unwrapor(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::UnwrapOr { dst, v, default } = op else {
            unreachable_op!("op_unwrapor: unexpected op")
        };
        let cell = cell_of(unsafe { *regs.add(*v as usize) });
        let (tag, payload) = sum_parts(cell)?;
        let val = if tag == 0 {
            payload.unwrap_or(Slot::int(0))
        } else {
            unsafe { *regs.add(*default as usize) }
        };
        let pty = self.sum_payload_ty(cell.ty, false);
        self.move_sum_val(*dst, val, pty);
        Ok(Flow::Next(pc + 1))
    }

    fn op_expect(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::Expect { dst, v, msg } = op else { unreachable_op!("op_expect: unexpected op") };
        let cell = cell_of(unsafe { *regs.add(*v as usize) });
        let (tag, payload) = sum_parts(cell)?;
        if tag == 1 {
            let m = cell_of(unsafe { *regs.add(*msg as usize) }).as_str().to_string();
            return Err(Trap::new(TrapKind::UnwrapNone, format!("expect failed: {m}")));
        }
        let val = payload.unwrap_or(Slot::int(0));
        let pty = self.sum_payload_ty(cell.ty, false);
        self.move_sum_val(*dst, val, pty);
        Ok(Flow::Next(pc + 1))
    }

    fn op_tidof(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::TidOf { dst, obj } = op else { unreachable_op!("op_tidof: unexpected op") };
        let cell = cell_of(unsafe { *regs.add(*obj as usize) });
        let ty = self.effective_ty(cell);
        unsafe { *regs.add(*dst as usize) = Slot::int(ty as i64) };
        Ok(Flow::Next(pc + 1))
    }

    fn op_istype(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::IsType { dst, obj, want } = op else {
            unreachable_op!("op_istype: unexpected op")
        };
        let cell = cell_of(unsafe { *regs.add(*obj as usize) });
        let ty = self.effective_ty(cell);
        unsafe { *regs.add(*dst as usize) = Slot::bool(ty == *want) };
        Ok(Flow::Next(pc + 1))
    }

    fn op_istrait(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::IsTrait { dst, obj, want } = op else {
            unreachable_op!("op_istrait: unexpected op")
        };
        let cell = cell_of(unsafe { *regs.add(*obj as usize) });
        let ty = self.effective_ty(cell);
        let has = self
            .prog
            .trait_slots
            .iter()
            .enumerate()
            .any(|(slot, &(t, _))| {
                t == *want
                    && self
                        .prog
                        .vtables
                        .get(ty as usize)
                        .and_then(|v| v.get(slot))
                        .is_some_and(|f| f.is_some())
            });
        unsafe { *regs.add(*dst as usize) = Slot::bool(has) };
        Ok(Flow::Next(pc + 1))
    }

    fn op_unbox(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::Unbox { dst, box_, ty } = op else { unreachable_op!("op_unbox: unexpected op") };
        let Some((val, val_ty)) = cell_of(unsafe { *regs.add(*box_ as usize) }).as_opaque() else {
            return Err(Trap::new(TrapKind::BadUnbox, "unbox on non-opaque"));
        };
        if val_ty != *ty {
            return Err(Trap::new(
                TrapKind::BadUnbox,
                "unbox type mismatch (the compiler guards this; the trap is the safety net)",
            ));
        }
        self.move_sum_val(*dst, val, *ty);
        Ok(Flow::Next(pc + 1))
    }

    fn op_box(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::Box { dst, val, ty } = op else { unreachable_op!("op_box: unexpected op") };
        let v = unsafe { *regs.add(*val as usize) };
        if self.is_ref(*ty) {
            self.heap.retain(v);
        }
        let c = self.heap.alloc_opaque(v, *ty)?;
        let old = unsafe { *regs.add(*dst as usize) };
        unsafe { *regs.add(*dst as usize) = c };
        self.heap.release(old);
        Ok(Flow::Next(pc + 1))
    }

    fn op_makeclosure(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::MakeClosure { dst, func, captures } = op else {
            unreachable_op!("op_makeclosure: unexpected op")
        };
        let caps: Vec<Slot> = captures
            .iter()
            .map(|&c| unsafe { *regs.add(c as usize) })
            .collect();
        let nparams = self.prog.funcs[*func as usize].params.len();
        let cap_tys: Vec<TypeId> = (nparams - caps.len()..nparams)
            .map(|i| self.param_ty(*func, i))
            .collect();
        for (&c, &t) in caps.iter().zip(cap_tys.iter()) {
            if self.is_ref(t) {
                self.heap.retain(c);
            }
        }
        let c = self.heap.alloc_closure(*func, caps)?;
        let old = unsafe { *regs.add(*dst as usize) };
        unsafe { *regs.add(*dst as usize) = c };
        self.heap.release(old);
        Ok(Flow::Next(pc + 1))
    }

    fn op_panic(&mut self, op: &Op, regs: *mut Slot, _pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::Panic { msg } = op else { unreachable_op!("op_panic: unexpected op") };
        let m = cell_of(unsafe { *regs.add(*msg as usize) }).as_str().to_string();
        Err(Trap::new(TrapKind::Panic, m))
    }

    fn op_assert(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::Assert { cond, msg } = op else { unreachable_op!("op_assert: unexpected op") };
        if !unsafe { (*regs.add(*cond as usize)).as_bool() } {
            let m = msg
                .map(|m| cell_of(unsafe { *regs.add(m as usize) }).as_str().to_string())
                .unwrap_or_else(|| "assertion failed".to_string());
            return Err(Trap::new(TrapKind::Assert, m));
        }
        Ok(Flow::Next(pc + 1))
    }

    fn op_conv(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::Conv { dst, src, from, to } = op else {
            unreachable_op!("op_conv: unexpected op")
        };
        let v = self.convert(unsafe { *regs.add(*src as usize) }, *from, *to)?;
        unsafe { *regs.add(*dst as usize) = v };
        Ok(Flow::Next(pc + 1))
    }

    fn op_strcharat(&mut self, op: &Op, regs: *mut Slot, pc: u32) -> Result<Flow<Value>, Trap> {
        let Op::StrCharAt { dst, s, idx } = op else {
            unreachable_op!("op_strcharat: unexpected op")
        };
        let str_cell = cell_of(unsafe { *regs.add(*s as usize) });
        let i = unsafe { (*regs.add(*idx as usize)).i };
        let text = str_cell.as_str();
        // ASCII: char index == byte index (O(1)); else decode the prefix.
        let c = if str_cell.str_ascii() {
            match text.as_bytes().get(i as usize) {
                Some(b) => *b as char,
                None => {
                    return Err(Trap::new(
                        TrapKind::IndexOutOfBounds,
                        format!("string index {i} out of bounds ({} bytes)", text.len()),
                    ))
                }
            }
        } else {
            text.chars().nth(i as usize).ok_or_else(|| {
                Trap::new(
                    TrapKind::IndexOutOfBounds,
                    format!(
                        "string index {i} out of bounds ({} chars)",
                        text.chars().count()
                    ),
                )
            })?
        };
        unsafe { *regs.add(*dst as usize) = Slot::ch(c) };
        Ok(Flow::Next(pc + 1))
    }
}
