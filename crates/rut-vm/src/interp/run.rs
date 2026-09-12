//! The `run_loop` dispatch fast path and `do_ret` (RFC 0034 §3).
use super::*;

impl Vm {
    pub(super) fn run_loop(&mut self) -> Result<Value, Trap> {
        // one cheap Rc bump per run: lets the loop borrow the code while
        // still taking `&mut self`, so `Op` need not be cloned per dispatch
        let prog = Rc::clone(&self.prog);
        loop {
            if !self.running {
                return Ok(Value::Unit);
            }
            let Some(op) = prog.funcs[self.cur_func as usize]
                .code
                .get(self.cur_pc as usize)
            else {
                // fell off the end — verifier rejects this shape; be safe
                let ret = self.do_ret(Slot::int(0));
                if let Some(v) = ret {
                    return Ok(v);
                }
                continue;
            };
            // fuel: one unit per op (RFC 0040 §2); back-edges (LoopHead) and
            // every interrupt_every ops are the checkpoints
            self.fuel_used += 1;
            self.since_check += 1;
            if let Some(fuel) = self.fuel {
                if fuel == 0 {
                    // park the frame: pc already points at the next op —
                    // resume() re-executes from here (RFC 0034 §4)
                    return Err(Trap::new(
                        TrapKind::OutOfFuel,
                        "fuel exhausted — the frame is parked; add fuel and resume()",
                    ));
                }
                self.fuel = Some(fuel - 1);
            } else if self.since_check >= self.interrupt_every {
                self.since_check = 0;
            }
            // `Ret` is the one op that can end the run — handle it here so
            // the root return value surfaces cleanly
            if let Op::Ret { val } = op {
                let v = val
                    .map(|r| self.cur_regs[r as usize])
                    .unwrap_or(Slot::int(0));
                if let Some(out) = self.do_ret(v) {
                    return Ok(out);
                }
                continue;
            }
            // hot scalar ops run inline: no `Op` clone, no `step` call
            match op {
                Op::Mov { dst, src } => {
                    self.cur_regs[*dst as usize] = self.cur_regs[*src as usize];
                    self.cur_pc += 1;
                }
                Op::ConstRaw { dst, bits } => {
                    self.cur_regs[*dst as usize] = Slot { i: *bits as i64 };
                    self.cur_pc += 1;
                }
                Op::Not { dst, a } => {
                    let v = !self.cur_regs[*a as usize].as_bool();
                    self.cur_regs[*dst as usize] = Slot::bool(v);
                    self.cur_pc += 1;
                }
                // float ops are their own opcodes (the `specialize` pass), so
                // each arm is one machine op with no `prim`/`op` switch
                Op::AddF { prim, dst, a, b } => {
                    self.cur_regs[*dst as usize] = self.arith_float::<{ FOP_ADD }>(
                        *prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize]);
                    self.cur_pc += 1;
                }
                Op::SubF { prim, dst, a, b } => {
                    self.cur_regs[*dst as usize] = self.arith_float::<{ FOP_SUB }>(
                        *prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize]);
                    self.cur_pc += 1;
                }
                Op::MulF { prim, dst, a, b } => {
                    self.cur_regs[*dst as usize] = self.arith_float::<{ FOP_MUL }>(
                        *prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize]);
                    self.cur_pc += 1;
                }
                Op::DivF { prim, dst, a, b } => {
                    self.cur_regs[*dst as usize] = self.arith_float::<{ FOP_DIV }>(
                        *prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize]);
                    self.cur_pc += 1;
                }
                Op::ModF { prim, dst, a, b } => {
                    self.cur_regs[*dst as usize] = self.arith_float::<{ FOP_MOD }>(
                        *prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize]);
                    self.cur_pc += 1;
                }
                Op::NegF { prim, dst, a } => {
                    self.cur_regs[*dst as usize] =
                        self.neg_float(*prim, self.cur_regs[*a as usize]);
                    self.cur_pc += 1;
                }
                Op::EqF { dst, a, b } => {
                    let v = self.cmp_float::<{ COP_EQ }>(
                        self.cur_regs[*a as usize], self.cur_regs[*b as usize]);
                    self.cur_regs[*dst as usize] = Slot::bool(v);
                    self.cur_pc += 1;
                }
                Op::NeF { dst, a, b } => {
                    let v = self.cmp_float::<{ COP_NE }>(
                        self.cur_regs[*a as usize], self.cur_regs[*b as usize]);
                    self.cur_regs[*dst as usize] = Slot::bool(v);
                    self.cur_pc += 1;
                }
                Op::LtF { dst, a, b } => {
                    let v = self.cmp_float::<{ COP_LT }>(
                        self.cur_regs[*a as usize], self.cur_regs[*b as usize]);
                    self.cur_regs[*dst as usize] = Slot::bool(v);
                    self.cur_pc += 1;
                }
                Op::GtF { dst, a, b } => {
                    let v = self.cmp_float::<{ COP_GT }>(
                        self.cur_regs[*a as usize], self.cur_regs[*b as usize]);
                    self.cur_regs[*dst as usize] = Slot::bool(v);
                    self.cur_pc += 1;
                }
                Op::LeF { dst, a, b } => {
                    let v = self.cmp_float::<{ COP_LE }>(
                        self.cur_regs[*a as usize], self.cur_regs[*b as usize]);
                    self.cur_regs[*dst as usize] = Slot::bool(v);
                    self.cur_pc += 1;
                }
                Op::GeF { dst, a, b } => {
                    let v = self.cmp_float::<{ COP_GE }>(
                        self.cur_regs[*a as usize], self.cur_regs[*b as usize]);
                    self.cur_regs[*dst as usize] = Slot::bool(v);
                    self.cur_pc += 1;
                }
                // integer ops: the opcode is the operation, so `arith_int`/
                // `bitop_int`/`cmp_int` fold their switch away; `prim` stays
                // for the per-prim width fitting
                Op::AddI { prim, dst, a, b } => {
                    let v = self.arith_int::<{ IOP_ADD }, false>(*prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize])?;
                    self.cur_regs[*dst as usize] = v;
                    self.cur_pc += 1;
                }
                Op::SubI { prim, dst, a, b } => {
                    let v = self.arith_int::<{ IOP_SUB }, false>(*prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize])?;
                    self.cur_regs[*dst as usize] = v;
                    self.cur_pc += 1;
                }
                Op::MulI { prim, dst, a, b } => {
                    let v = self.arith_int::<{ IOP_MUL }, false>(*prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize])?;
                    self.cur_regs[*dst as usize] = v;
                    self.cur_pc += 1;
                }
                Op::DivI { prim, dst, a, b } => {
                    let v = self.arith_int::<{ IOP_DIV }, false>(*prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize])?;
                    self.cur_regs[*dst as usize] = v;
                    self.cur_pc += 1;
                }
                Op::ModI { prim, dst, a, b } => {
                    let v = self.arith_int::<{ IOP_MOD }, false>(*prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize])?;
                    self.cur_regs[*dst as usize] = v;
                    self.cur_pc += 1;
                }
                Op::WAddI { prim, dst, a, b } => {
                    let v = self.arith_int::<{ IOP_ADD }, true>(*prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize])?;
                    self.cur_regs[*dst as usize] = v;
                    self.cur_pc += 1;
                }
                Op::WSubI { prim, dst, a, b } => {
                    let v = self.arith_int::<{ IOP_SUB }, true>(*prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize])?;
                    self.cur_regs[*dst as usize] = v;
                    self.cur_pc += 1;
                }
                Op::WMulI { prim, dst, a, b } => {
                    let v = self.arith_int::<{ IOP_MUL }, true>(*prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize])?;
                    self.cur_regs[*dst as usize] = v;
                    self.cur_pc += 1;
                }
                Op::WDivI { prim, dst, a, b } => {
                    let v = self.arith_int::<{ IOP_DIV }, true>(*prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize])?;
                    self.cur_regs[*dst as usize] = v;
                    self.cur_pc += 1;
                }
                Op::WModI { prim, dst, a, b } => {
                    let v = self.arith_int::<{ IOP_MOD }, true>(*prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize])?;
                    self.cur_regs[*dst as usize] = v;
                    self.cur_pc += 1;
                }
                Op::AndI { prim, dst, a, b } => {
                    let v = self.bitop_int::<{ BOP_AND }>(*prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize])?;
                    self.cur_regs[*dst as usize] = v;
                    self.cur_pc += 1;
                }
                Op::OrI { prim, dst, a, b } => {
                    let v = self.bitop_int::<{ BOP_OR }>(*prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize])?;
                    self.cur_regs[*dst as usize] = v;
                    self.cur_pc += 1;
                }
                Op::XorI { prim, dst, a, b } => {
                    let v = self.bitop_int::<{ BOP_XOR }>(*prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize])?;
                    self.cur_regs[*dst as usize] = v;
                    self.cur_pc += 1;
                }
                Op::ShlI { prim, dst, a, b } => {
                    let v = self.bitop_int::<{ BOP_SHL }>(*prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize])?;
                    self.cur_regs[*dst as usize] = v;
                    self.cur_pc += 1;
                }
                Op::ShrI { prim, dst, a, b } => {
                    let v = self.bitop_int::<{ BOP_SHR }>(*prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize])?;
                    self.cur_regs[*dst as usize] = v;
                    self.cur_pc += 1;
                }
                Op::WrapShlI { prim, dst, a, b } => {
                    let v = self.bitop_int::<{ BOP_WRAPSHL }>(*prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize])?;
                    self.cur_regs[*dst as usize] = v;
                    self.cur_pc += 1;
                }
                Op::EqI { prim, dst, a, b } => {
                    let v = self.cmp_int::<{ COP_EQ }>(*prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize]);
                    self.cur_regs[*dst as usize] = Slot::bool(v);
                    self.cur_pc += 1;
                }
                Op::NeI { prim, dst, a, b } => {
                    let v = self.cmp_int::<{ COP_NE }>(*prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize]);
                    self.cur_regs[*dst as usize] = Slot::bool(v);
                    self.cur_pc += 1;
                }
                Op::LtI { prim, dst, a, b } => {
                    let v = self.cmp_int::<{ COP_LT }>(*prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize]);
                    self.cur_regs[*dst as usize] = Slot::bool(v);
                    self.cur_pc += 1;
                }
                Op::GtI { prim, dst, a, b } => {
                    let v = self.cmp_int::<{ COP_GT }>(*prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize]);
                    self.cur_regs[*dst as usize] = Slot::bool(v);
                    self.cur_pc += 1;
                }
                Op::LeI { prim, dst, a, b } => {
                    let v = self.cmp_int::<{ COP_LE }>(*prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize]);
                    self.cur_regs[*dst as usize] = Slot::bool(v);
                    self.cur_pc += 1;
                }
                Op::GeI { prim, dst, a, b } => {
                    let v = self.cmp_int::<{ COP_GE }>(*prim, self.cur_regs[*a as usize], self.cur_regs[*b as usize]);
                    self.cur_regs[*dst as usize] = Slot::bool(v);
                    self.cur_pc += 1;
                }
                Op::NegI { prim, dst, a } => {
                    let v = self.neg_int(*prim, self.cur_regs[*a as usize])?;
                    self.cur_regs[*dst as usize] = v;
                    self.cur_pc += 1;
                }
                Op::Jmp { target } => self.cur_pc = *target,
                Op::Br { cond, then_t, else_t } => {
                    self.cur_pc =
                        if self.cur_regs[*cond as usize].as_bool() { *then_t } else { *else_t };
                }
                Op::LoopHead => self.cur_pc += 1,
                Op::ArrGet { dst, arr, idx, repr } => {
                    self.op_arr_get(*dst, *arr, *idx, *repr)?;
                    self.cur_pc += 1;
                }
                Op::ArrSet { arr, idx, val, repr } => {
                    self.op_arr_set(*arr, *idx, *val, *repr)?;
                    self.cur_pc += 1;
                }
                Op::GetF { dst, obj, field, repr } => {
                    self.op_getf(*dst, *obj, *field, *repr)?;
                    self.cur_pc += 1;
                }
                Op::SetF { obj, field, val, repr } => {
                    self.op_setf(*obj, *field, *val, *repr)?;
                    self.cur_pc += 1;
                }
                Op::MakeRecord { dst, ty, vals } => {
                    self.op_make_record(*dst, *ty, vals)?;
                    self.cur_pc += 1;
                }
                Op::Call { func, args, dst } => {
                    // pc advances first: `enter` saves it as the return
                    // address and resets the callee's pc to 0
                    self.cur_pc += 1;
                    self.op_call(*func, args, *dst);
                }
                Op::CallM { func, recv, args, dst } => {
                    self.cur_pc += 1;
                    self.op_call_m(*func, *recv, args, *dst);
                }
                Op::CallI { slot, recv, args, dst } => {
                    self.cur_pc += 1;
                    self.op_call_i(*slot, *recv, args, *dst)?;
                }
                Op::CallFn { fval, args, dst } => {
                    self.cur_pc += 1;
                    self.op_call_fn(*fval, args, *dst)?;
                }
                Op::CallNat { nat, recv, args, dst } => {
                    self.call_nat(*nat, *recv, args, *dst)?;
                    self.cur_pc += 1;
                }
                Op::ArrLit { dst, ty, elems } => {
                    self.op_arr_lit(*dst, *ty, elems)?;
                    self.cur_pc += 1;
                }
                Op::BrTable { idx, table, default } => {
                    let m = cell_of(self.cur_regs[*idx as usize])
                        .as_enum_member()
                        .unwrap_or(u32::MAX) as usize;
                    self.cur_pc = table.get(m).copied().unwrap_or(*default);
                }
                _ => {
                    self.cur_pc += 1;
                    self.step(op.clone())?;
                }
            }
        }
    }

    /// Execute `Ret`: returns Some(Value) if this was the root frame.
    pub(super) fn do_ret(&mut self, val: Slot) -> Option<Value> {
        let ret_ty = self.prog.funcs[self.cur_func as usize].ret;
        let func = self.cur_func;
        let dst = self.cur_ret_dst;
        let is_ref = self.is_ref(ret_ty);
        // retain ONCE for the pending transfer to the caller — the frame
        // release below drops the callee register's own reference
        if is_ref {
            self.heap.retain(val);
        }
        // release the active frame's registers (destructors, RFC 0016 §3)
        let regs = std::mem::take(&mut self.cur_regs);
        for &i in &self.ref_regs[func as usize] {
            self.heap.release(regs[i as usize]);
        }
        self.put_regs(regs);
        match self.frames.pop() {
            Some(f) => {
                self.cur_func = f.func;
                self.cur_pc = f.pc;
                self.cur_regs = f.regs;
                self.cur_ret_dst = f.ret_dst;
                if let Some(d) = dst {
                    // the dst register takes ownership of the pending ref
                    let old = self.cur_regs[d as usize];
                    self.cur_regs[d as usize] = val;
                    let dty = self.regs_ty(d);
                    if self.is_ref(dty) {
                        self.heap.release(old);
                    }
                } else if is_ref {
                    // value discarded — consume the pending ref
                    self.heap.release(val);
                }
                None
            }
            None => {
                self.running = false;
                let out = slot_to_value(val, ret_ty, &self.prog, &self.heap);
                if is_ref {
                    self.heap.release(val);
                }
                Some(out)
            }
        }
    }
}
