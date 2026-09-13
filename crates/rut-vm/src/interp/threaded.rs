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
}
