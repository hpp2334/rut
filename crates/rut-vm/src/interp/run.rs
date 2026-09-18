//! The `run_loop` driver (RFC 0034 §3).
//!
//! `rut-vm-threaded` runs stretches of threaded (scalar) ops with a
//! tail-call dispatcher that carries the frame state in arguments. When it
//! reaches an unthreaded op it syncs the machine and returns
//! [`rut_vm_threaded::ThreadOut::Bail`]; this driver executes exactly that
//! one op with the match interpreter (`step_one`) and re-enters threading.
use super::*;

impl Vm {
    pub(super) fn run_loop(&mut self) -> Result<Value, Trap> {
        loop {
            let pc = self.cur_pc;
            let len = self.prog.funcs[self.cur_func as usize].code.len() as u32;
            if pc < len {
                let table = self.thread_table; // Copy — a 680-byte fn-ptr array
                match rut_vm_threaded::run(self, pc, &table)? {
                    rut_vm_threaded::ThreadOut::Done(v) => return Ok(v),
                    rut_vm_threaded::ThreadOut::Bail => {}
                }
            }
            // either the stretch bailed, or pc fell off the end (the
            // verifier rejects that shape): run one op through the match
            if let Some(v) = self.step_one()? {
                return Ok(v);
            }
        }
    }

    /// Execute exactly one op with the match interpreter. Returns
    /// `Some(value)` if this was the root `ret`.
    pub(super) fn step_one(&mut self) -> Result<Option<Value>, Trap> {
        if !self.running {
            return Ok(Some(Value::Nil));
        }
        let prog = Rc::clone(&self.prog);
        let Some(op) = prog.funcs[self.cur_func as usize]
            .code
            .get(self.cur_pc as usize)
        else {
            // fell off the end — verifier rejects this shape; be safe
            return Ok(self.do_ret(Slot::int(0)));
        };
        self.tick(self.cur_pc)?;
        // `Ret` is the one op that can end the run — handle it here so the
        // root return value surfaces cleanly
        if let Op::Ret { val } = op {
            let v = val
                .map(|r| self.cur_regs[r as usize])
                .unwrap_or(Slot::int(0));
            return Ok(self.do_ret(v));
        }
        match op {
            Op::ArrGet { dst, arr, idx, repr } => {
                self.op_arr_get(*dst, *arr, *idx, *repr)?;
                self.cur_pc += 1;
            }
            Op::ArrSet { arr, idx, val, repr } => {
                self.op_arr_set(*arr, *idx, *val, *repr)?;
                self.cur_pc += 1;
            }
            Op::ArrGetF { dst, obj, field, idx, repr } => {
                self.op_arr_get_f(*dst, *obj, *field, *idx, *repr)?;
                self.cur_pc += 1;
            }
            Op::ArrSetF { obj, field, idx, val, repr } => {
                self.op_arr_set_f(*obj, *field, *idx, *val, *repr)?;
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
            Op::MakeRecord { dst, ty, argv_off, argc } => {
                self.op_make_record(*dst, *ty, *argv_off, *argc)?;
                self.cur_pc += 1;
            }
            Op::Call { func, argv_off, argc, dst } => {
                if self.prog.funcs[*func as usize].host_id.is_some() {
                    // run with the cursor AT this op: a trap (including a
                    // nested `vm.call` that ran out of fuel) parks here, so
                    // `resume()` re-runs the host fn instead of skipping it
                    self.call_host(*func, *argv_off, *argc, *dst)?;
                    self.cur_pc += 1;
                } else {
                    self.cur_pc += 1;
                    self.op_call(*func, *argv_off, *argc, *dst);
                }
            }
            Op::CallM { func, argv_off, argc, dst } => {
                self.cur_pc += 1;
                self.op_call(*func, *argv_off, *argc, *dst);
            }
            Op::CallI { slot, argv_off, argc, dst } => {
                self.cur_pc += 1;
                self.op_call_i(*slot, *argv_off, *argc, *dst)?;
            }
            Op::CallFn { fval, argv_off, argc, dst } => {
                self.cur_pc += 1;
                self.op_call_fn(*fval, *argv_off, *argc, *dst)?;
            }
            Op::CallNat { nat, recv, argv_off, argc, dst } => {
                self.call_nat(*nat, *recv, *argv_off, *argc, *dst)?;
                self.cur_pc += 1;
            }
            Op::ArrLit { dst, ty, argv_off, argc } => {
                self.op_arr_lit(*dst, *ty, *argv_off, *argc)?;
                self.cur_pc += 1;
            }
            Op::BrTable { idx, table_off, count, default } => {
                let table = &self.prog.funcs[self.cur_func as usize].labels
                    [*table_off as usize..*table_off as usize + *count as usize];
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
        Ok(None)
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
