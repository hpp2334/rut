//! The interpreter loop — RFC 0034: one struct, one thread, one heap.
//! Register frames, typed ops, traps as `Err(Trap)` (never Rust panics),
//! fuel + heap budgets checked at loop back-edges and every allocation
//! (RFC 0040). Traps are catchable only at the host boundary (RFC 0001 P4).
//!
//! The ACTIVE frame lives directly on the `Vm` (`cur_*`); calls push it
//! onto `frames` and rets pop — keeps register access borrow-friendly.

use rut_core::binary::{ConstVal, Program};
use crate::heap::{cell_of, CellData, Heap, Packed, Slot, Trap, TrapKind, Value};
use rut_core::ops::*;
use rut_core::types::{PrimTy, TypeId, TyKind};
use std::cell::RefCell;
use std::rc::Rc;

/// A const-pool string materialized into a heap slot at load (immortal:
/// the module keeps the Slot alive for the program's lifetime).
fn const_to_slot(c: &ConstVal, heap: &Heap) -> Result<Slot, String> {
    Ok(match c {
        ConstVal::I64(v) => Slot::int(*v),
        ConstVal::F64(v) => Slot::float(*v),
        ConstVal::Bool(v) => Slot::bool(*v),
        ConstVal::Char(v) => Slot::ch(*v),
        ConstVal::Str(s) => heap
            .alloc_str(s.clone())
            .map_err(|t| format!("load: {}", t.msg))?,
    })
}

pub struct Limits {
    pub fuel: Option<u64>,
    pub heap_limit_bytes: Option<u64>,
    pub interrupt_every: u32,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits { fuel: None, heap_limit_bytes: None, interrupt_every: 1024 }
    }
}

#[derive(Default, Clone)]
pub struct HostHooks {
    /// the print sink — None is a silent no-op (RFC 0028: a script cannot
    /// spam an embedded host's stdout; the host opts in)
    pub print: Option<Rc<RefCell<dyn FnMut(&str)>>>,
}

struct SavedFrame {
    func: u32,
    pc: u32,
    regs: Vec<Slot>,
    ret_dst: Option<Reg>,
}

pub struct Vm {
    pub prog: Rc<Program>,
    pub heap: Heap,
    frames: Vec<SavedFrame>,
    // active frame
    cur_func: u32,
    cur_pc: u32,
    cur_regs: Vec<Slot>,
    cur_ret_dst: Option<Reg>,
    running: bool,
    fuel: Option<u64>,
    pub fuel_used: u64,
    interrupt_every: u32,
    since_check: u32,
    pub hooks: HostHooks,
    const_slots: Vec<Slot>,
    /// recycled per-frame register files (avoids a Vec alloc per call)
    reg_pool: Vec<Vec<Slot>>,
}

impl Vm {
    pub fn new(prog: Rc<Program>, limits: &Limits, hooks: HostHooks) -> Result<Vm, Trap> {
        let heap = Heap::new(limits.heap_limit_bytes);
        let mut const_slots = Vec::with_capacity(prog.consts.len());
        for c in &prog.consts {
            let s = const_to_slot(c, &heap).map_err(|m| Trap::new(TrapKind::Invalid, m))?;
            const_slots.push(s);
        }
        Ok(Vm {
            prog,
            heap,
            frames: Vec::new(),
            cur_func: 0,
            cur_pc: 0,
            cur_regs: Vec::new(),
            cur_ret_dst: None,
            running: false,
            fuel: limits.fuel,
            fuel_used: 0,
            interrupt_every: limits.interrupt_every.max(1),
            since_check: 0,
            hooks,
            const_slots,
            reg_pool: Vec::new(),
        })
    }

    pub fn heap_usage(&self) -> u64 {
        self.heap.used_bytes()
    }

    /// VM-heap high-water mark (RFC 0039 accounting) — the peak live
    /// heap over the run, for embedding/benching hosts.
    pub fn heap_peak(&self) -> u64 {
        self.heap.peak_bytes()
    }

    pub fn add_fuel(&mut self, n: u64) {
        self.fuel = self.fuel.map(|f| f.saturating_add(n));
    }
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Take a zeroed register file from the pool (or allocate one).
    fn take_regs(&mut self, n: usize) -> Vec<Slot> {
        match self.reg_pool.pop() {
            Some(mut v) => {
                v.resize(n, Slot::int(0));
                v
            }
            None => vec![Slot::int(0); n],
        }
    }

    /// Return a frame's register file to the pool (values are released by
    /// the caller first; slots are plain 8-byte words, so clearing is safe).
    fn put_regs(&mut self, mut v: Vec<Slot>) {
        const REG_POOL_MAX: usize = 64;
        if self.reg_pool.len() < REG_POOL_MAX {
            v.clear();
            self.reg_pool.push(v);
        }
    }

    fn enter(&mut self, func: u32, regs: Vec<Slot>, ret_dst: Option<Reg>) {        // save current frame if there is one
        if self.running {
            self.frames.push(SavedFrame {
                func: self.cur_func,
                pc: self.cur_pc,
                regs: std::mem::take(&mut self.cur_regs),
                ret_dst: self.cur_ret_dst,
            });
        }
        self.cur_func = func;
        self.cur_pc = 0;
        self.cur_regs = regs;
        self.cur_ret_dst = ret_dst;
        self.running = true;
    }

    /// Call an exported function with host values — the host boundary
    /// (RFC 0035 §3). Traps unwind here (RFC 0034 §2).
    pub fn call(&mut self, export: &str, args: &[Value]) -> Result<Value, Trap> {
        if self.running {
            return Err(Trap::new(TrapKind::Invalid, "vm busy — resume() first"));
        }
        let Some(func) = self.prog.export(export) else {
            return Err(Trap::new(TrapKind::Invalid, format!("no export `{export}`")));
        };
        let nparams = self.prog.funcs[func as usize].params.len();
        if args.len() != nparams {
            return Err(Trap::new(
                TrapKind::Invalid,
                format!("`{export}` takes {nparams} args, {} given", args.len()),
            ));
        }
        let nregs = self.prog.funcs[func as usize].regs.len();
        let mut regs = self.take_regs(nregs);
        for (i, a) in args.iter().enumerate() {
            let pty = self.param_ty(func, i);
            regs[i] = self.value_in(a, pty)
                .map_err(|m| Trap::new(TrapKind::Invalid, format!("`{export}` argument {i}: {m}")))?;
        }
        self.enter(func, regs, None);
        let v = self.run_loop()?;
        Ok(v)
    }

    /// Host `Value` → slot under the callee's declared param type. A kind
    /// mismatch is a trap naming both sides — the rut module's signatures
    /// were already checked against the crossing rule at compile time, so
    /// this only fires when the EMBEDDER passes the wrong shape.
    fn value_in(&mut self, v: &Value, ty: TypeId) -> Result<Slot, String> {
        use rut_core::types::{PrimTy, TyKind};
        let ty = if ty != u32::MAX { ty } else {
            return Err(format!("internal: untyped parameter"));
        };
        Ok(match (v, self.prog.types.kind(ty).clone()) {
            (Value::Unit, TyKind::Unit) | (Value::Unit, TyKind::Prim(_)) => Slot::int(0),
            (Value::I64(n), TyKind::Prim(p)) => {
                if !fits(*n, p) {
                    return Err(format!("`{n}` does not fit `{}`", self.prog.types.name(ty)));
                }
                Slot::int(*n)
            }
            (Value::F64(f), TyKind::Prim(PrimTy::F32)) => Slot::float(*f as f32 as f64),
            (Value::F64(f), TyKind::Prim(PrimTy::F64)) => Slot::float(*f),
            (Value::Bool(b), TyKind::Prim(PrimTy::Bool)) => Slot::bool(*b),
            (Value::Char(c), TyKind::Prim(PrimTy::Char)) => Slot::ch(*c),
            (Value::Str(s), TyKind::Str) => self.heap.alloc_str(s.clone()).map_err(|t| t.msg)?,
            (Value::Bytes(b), TyKind::Vec { elem }) if elem == self.ty_u8() => {
                let cell = self
                    .heap
                    .alloc_vec(elem, b.len(), &self.prog.types)
                    .map_err(|t| t.msg)?;
                if let CellData::Vec { items, .. } = &cell_of(cell).data {
                    *items.borrow_mut() =
                        Packed::from_slots(elem, &self.prog.types, b.iter().map(|&x| Slot::int(x as i64)).collect());
                }
                cell
            }
            (Value::Opt(None), TyKind::Option { .. }) => self.heap.alloc_sum(ty, 1, None).map_err(|t| t.msg)?,
            (Value::Opt(Some(inner)), TyKind::Option { elem }) => {
                let payload = self.value_in(inner, elem)?;
                self.heap.alloc_sum(ty, 0, Some(payload)).map_err(|t| t.msg)?
            }
            (Value::Res(Ok(inner)), TyKind::Result { ok, .. }) => {
                let payload = self.value_in(inner, ok)?;
                self.heap.alloc_sum(ty, 0, Some(payload)).map_err(|t| t.msg)?
            }
            (Value::Res(Err(inner)), TyKind::Result { err, .. }) => {
                let payload = self.value_in(inner, err)?;
                self.heap.alloc_sum(ty, 1, Some(payload)).map_err(|t| t.msg)?
            }
            (Value::Opaque(h), TyKind::Opaque) => {
                let s = Slot { r: Some(h.ptr()) };
                if !matches!(cell_of(s).data, CellData::OpaqueBox { .. }) {
                    return Err("not an Opaque box".to_string());
                }
                self.heap.retain(s); // the parameter register owns its reference
                s
            }
            (v, _) => {
                return Err(format!(
                    "is `{}`, `{}` expected",
                    value_kind_name(v),
                    self.prog.types.name(ty)
                ))
            }
        })
    }

    fn ty_u8(&self) -> TypeId {
        // boot-table id for u8 (stable: interned at TypeTable::boot)
        self.prog
            .types
            .types
            .iter()
            .position(|t| matches!(t.kind, TyKind::Prim(PrimTy::U8)))
            .map(|i| i as u32)
            .unwrap_or(u32::MAX)
    }

    /// Resume after a budget trap (RFC 0034 §4: the frame IS the loop state).
    pub fn resume(&mut self) -> Result<Value, Trap> {
        if !self.running {
            return Err(Trap::new(TrapKind::Invalid, "nothing to resume"));
        }
        self.run_loop()
    }

    fn regs_ty(&self, r: Reg) -> TypeId {
        self.prog.funcs[self.cur_func as usize]
            .regs
            .get(r as usize)
            .copied()
            .unwrap_or(TY_ANY)
    }

    fn param_ty(&self, func: u32, i: usize) -> TypeId {
        self.prog.funcs[func as usize]
            .params
            .get(i)
            .copied()
            .unwrap_or(TY_ANY)
    }

    fn is_ref(&self, ty: TypeId) -> bool {
        ty != TY_ANY && self.prog.types.is_ref(ty)
    }

    fn run_loop(&mut self) -> Result<Value, Trap> {
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
                Op::Arith { op, prim, dst, a, b } => {
                    let v = self.arith(
                        *op,
                        *prim,
                        self.cur_regs[*a as usize],
                        self.cur_regs[*b as usize],
                        false,
                    )?;
                    self.cur_regs[*dst as usize] = v;
                    self.cur_pc += 1;
                }
                Op::Wrap { op, prim, dst, a, b } => {
                    let v = self.arith(
                        *op,
                        *prim,
                        self.cur_regs[*a as usize],
                        self.cur_regs[*b as usize],
                        true,
                    )?;
                    self.cur_regs[*dst as usize] = v;
                    self.cur_pc += 1;
                }
                Op::Bit { op, prim, dst, a, b } => {
                    let v = self.bitop(
                        *op,
                        *prim,
                        self.cur_regs[*a as usize],
                        self.cur_regs[*b as usize],
                    )?;
                    self.cur_regs[*dst as usize] = v;
                    self.cur_pc += 1;
                }
                Op::Cmp { op, prim, dst, a, b } => {
                    let v = self.cmp(
                        *op,
                        *prim,
                        self.cur_regs[*a as usize],
                        self.cur_regs[*b as usize],
                    );
                    self.cur_regs[*dst as usize] = Slot::bool(v);
                    self.cur_pc += 1;
                }
                Op::Not { dst, a } => {
                    let v = !self.cur_regs[*a as usize].as_bool();
                    self.cur_regs[*dst as usize] = Slot::bool(v);
                    self.cur_pc += 1;
                }
                Op::Jmp { target } => self.cur_pc = *target,
                Op::Br { cond, then_t, else_t } => {
                    self.cur_pc =
                        if self.cur_regs[*cond as usize].as_bool() { *then_t } else { *else_t };
                }
                Op::LoopHead => self.cur_pc += 1,
                Op::ArrGet { dst, arr, idx } => {
                    self.op_arr_get(*dst, *arr, *idx)?;
                    self.cur_pc += 1;
                }
                Op::ArrSet { arr, idx, val } => {
                    self.op_arr_set(*arr, *idx, *val)?;
                    self.cur_pc += 1;
                }
                Op::GetF { dst, obj, field } => {
                    self.op_getf(*dst, *obj, *field)?;
                    self.cur_pc += 1;
                }
                Op::SetF { obj, field, val } => {
                    self.op_setf(*obj, *field, *val)?;
                    self.cur_pc += 1;
                }
                _ => {
                    self.cur_pc += 1;
                    self.step(op.clone())?;
                }
            }
        }
    }

    /// Execute `Ret`: returns Some(Value) if this was the root frame.
    fn do_ret(&mut self, val: Slot) -> Option<Value> {
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
        for i in 0..regs.len() {
            let ty = self
                .prog
                .funcs[func as usize]
                .regs
                .get(i)
                .copied()
                .unwrap_or(TY_ANY);
            self.heap.release_typed(regs[i], ty, &self.prog.types);
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

    fn step(&mut self, op: Op) -> Result<(), Trap> {
        macro_rules! r {
            ($i:expr) => {
                self.cur_regs[$i as usize]
            };
        }
        match op {
            Op::LoopHead => {}
            Op::Mov { dst, src } => self.cur_regs[dst as usize] = r!(src),
            Op::MovRef { dst, src } => {
                let v = r!(src);
                self.heap.retain(v);
                let old = self.cur_regs[dst as usize];
                self.cur_regs[dst as usize] = v;
                self.heap.release(old);
            }
            Op::Const { dst, k } => {
                let s = self.const_slots[k as usize];
                self.heap.retain(s);
                let old = self.cur_regs[dst as usize];
                self.cur_regs[dst as usize] = s;
                self.heap.release(old);
            }
            Op::ConstRaw { dst, bits } => self.cur_regs[dst as usize] = Slot { i: bits as i64 },


            Op::Arith { op, prim, dst, a, b } => {
                let v = self.arith(op, prim, r!(a), r!(b), false)?;
                self.cur_regs[dst as usize] = v;
            }
            Op::Wrap { op, prim, dst, a, b } => {
                let v = self.arith(op, prim, r!(a), r!(b), true)?;
                self.cur_regs[dst as usize] = v;
            }
            Op::Bit { op, prim, dst, a, b } => {
                let v = self.bitop(op, prim, r!(a), r!(b))?;
                self.cur_regs[dst as usize] = v;
            }
            Op::Cmp { op, prim, dst, a, b } => {
                let v = self.cmp(op, prim, r!(a), r!(b));
                self.cur_regs[dst as usize] = Slot::bool(v);
            }
            Op::Not { dst, a } => {
                let v = !r!(a).as_bool();
                self.cur_regs[dst as usize] = Slot::bool(v);
            }
            Op::Neg { prim, dst, a } => {
                let x = r!(a);
                let v = match prim {
                    PrimTy::F32 => Slot::float(-(unsafe { x.f } as f32) as f64),
                    PrimTy::F64 => Slot::float(-(unsafe { x.f })),
                    p => {
                        let v = unsafe { x.i };
                        let (r, o) = v.overflowing_neg();
                        if o || !fits(r, p) {
                            return Err(Trap::new(TrapKind::Overflow, "negate overflow"));
                        }
                        Slot::int(r)
                    }
                };
                self.cur_regs[dst as usize] = v;
            }
            Op::StrCmp { eq, dst, a, b } => {
                let sa = cell_of(r!(a)).as_str();
                let sb = cell_of(r!(b)).as_str();
                self.cur_regs[dst as usize] = Slot::bool((sa == sb) == eq);
            }
            Op::RefEq { eq, dst, a, b } => {
                let v = Slot::same_ref(r!(a), r!(b)) == eq;
                self.cur_regs[dst as usize] = Slot::bool(v);
            }

            Op::Jmp { target } => self.cur_pc = target,
            Op::Br { cond, then_t, else_t } => {
                self.cur_pc = if r!(cond).as_bool() { then_t } else { else_t };
            }
            Op::BrTable { idx, table, default } => {
                let cell = cell_of(r!(idx));
                let m = cell.as_enum_member().unwrap_or(u32::MAX) as usize;
                self.cur_pc = table.get(m).copied().unwrap_or(default);
            }

            Op::Call { func, args, dst } => {
                let nregs = self.prog.funcs[func as usize].regs.len();
                let mut regs = self.take_regs(nregs);
                for (i, a) in args.iter().enumerate() {
                    regs[i] = r!(*a);
                    if self.is_ref(self.param_ty(func, i)) {
                        self.heap.retain(regs[i]);
                    }
                }
                self.enter(func, regs, dst);
            }
            Op::CallM { func, recv, args, dst } => {
                let nregs = self.prog.funcs[func as usize].regs.len();
                let mut regs = self.take_regs(nregs);
                regs[0] = r!(recv);
                if self.is_ref(self.param_ty(func, 0)) {
                    self.heap.retain(regs[0]);
                }
                for (i, a) in args.iter().enumerate() {
                    regs[i + 1] = r!(*a);
                    if self.is_ref(self.param_ty(func, i + 1)) {
                        self.heap.retain(regs[i + 1]);
                    }
                }
                self.enter(func, regs, dst);
            }
            Op::CallI { slot, recv, args, dst } => {
                let ty = cell_of(r!(recv)).ty;
                let fid = self
                    .prog
                    .vtables
                    .get(ty as usize)
                    .and_then(|v| v.get(slot as usize))
                    .and_then(|f| *f)
                    .ok_or_else(|| {
                        Trap::new(
                            TrapKind::Invalid,
                            format!(
                                "no impl for trait slot {slot} on {} — `is` would have said false",
                                self.prog.types.name(ty)
                            ),
                        )
                    })?;
                let nregs = self.prog.funcs[fid as usize].regs.len();
                let mut regs = self.take_regs(nregs);
                regs[0] = r!(recv);
                if self.is_ref(self.param_ty(fid, 0)) {
                    self.heap.retain(regs[0]);
                }
                for (i, a) in args.iter().enumerate() {
                    regs[i + 1] = r!(*a);
                    if self.is_ref(self.param_ty(fid, i + 1)) {
                        self.heap.retain(regs[i + 1]);
                    }
                }
                self.enter(fid, regs, dst);
            }
            Op::CallFn { fval, args, dst } => {
                let Some((fid, captures)) = cell_of(r!(fval)).as_closure() else {
                    return Err(Trap::new(TrapKind::Invalid, "call on non-closure"));
                };
                let nparams = self.prog.funcs[fid as usize].params.len();
                let ncaptures = self.prog.funcs[fid as usize].n_captures as usize;
                let declared = nparams.saturating_sub(ncaptures);
                let nregs = self.prog.funcs[fid as usize].regs.len();
                let mut regs = self.take_regs(nregs);
                for (i, a) in args.iter().enumerate().take(declared) {
                    regs[i] = r!(*a);
                    if self.is_ref(self.param_ty(fid, i)) {
                        self.heap.retain(regs[i]);
                    }
                }
                for (i, c) in captures.iter().enumerate() {
                    if declared + i < regs.len() {
                        regs[declared + i] = *c;
                        let ty = self.param_ty(fid, declared + i);
                        if self.is_ref(ty) {
                            self.heap.retain(*c);
                        }
                    }
                }
                self.enter(fid, regs, dst);
            }
            Op::CallNat { nat, recv, args, dst } => self.call_nat(nat, recv, &args, dst)?,
            // handled in run_loop (root returns surface the run's value)
            Op::Ret { .. } => unreachable!("Op::Ret is handled by the run loop"),
            Op::NewCell { dst, ty } => {
                let n = match self.prog.types.kind(ty) {
                    TyKind::Data { fields } => fields.len(),
                    _ => 0,
                };
                let c = self.heap.alloc_record_zeroed(ty, n)?;
                let old = self.cur_regs[dst as usize];
                self.cur_regs[dst as usize] = c;
                self.heap.release(old);
            }
            Op::GetF { dst, obj, field } => self.op_getf(dst, obj, field)?,
            Op::SetF { obj, field, val } => self.op_setf(obj, field, val)?,
            Op::Own { dst, src, ty } => {
                let v = self.heap.own(r!(src), ty, &self.prog.types)?;
                let old = self.cur_regs[dst as usize];
                self.cur_regs[dst as usize] = v;
                if self.prog.types.is_ref(ty) {
                    self.heap.release(old);
                }
            }

            Op::ArrNew { dst, ty, len } => {
                let elem = match self.prog.types.kind(ty) {
                    TyKind::Vec { elem } => *elem,
                    _ => return Err(Trap::new(TrapKind::Invalid, "arrnew on non-vec")),
                };
                let n = unsafe { r!(len).i }.max(0) as usize;
                let c = self.heap.alloc_vec(elem, n, &self.prog.types)?;
                if let crate::heap::CellData::Vec { items, .. } = &cell_of(c).data {
                    let mut fb = items.borrow_mut();
                    for _ in 0..n {
                        fb.push(default_slot(elem, &self.prog.types));
                    }
                }
                let old = self.cur_regs[dst as usize];
                self.cur_regs[dst as usize] = c;
                self.heap.release(old);
            }
            Op::ArrLit { dst, ty, elems } => {
                let elem = match self.prog.types.kind(ty) {
                    TyKind::Array { elem, .. } | TyKind::Vec { elem } => *elem,
                    _ => TY_ANY,
                };
                let vals: Vec<Slot> = elems.iter().map(|&e| r!(e)).collect();
                let c = self.heap.alloc_array(elem, vals, &self.prog.types)?;
                let old = self.cur_regs[dst as usize];
                self.cur_regs[dst as usize] = c;
                self.heap.release(old);
            }
            Op::ArrGet { dst, arr, idx } => self.op_arr_get(dst, arr, idx)?,
            Op::ArrSet { arr, idx, val } => self.op_arr_set(arr, idx, val)?,

            Op::EnumNew { dst, ty, member } => {
                let c = self.heap.enum_member(ty, member)?;
                // the singleton slot is BORROWED (enum_member leaks the
                // base references) — the destination register takes an
                // OWNED reference, like every other ref-typed store, or
                // frame teardown over-releases and frees the "immortal"
                // cell while the singleton map still points at it
                self.heap.retain(c);
                let old = self.cur_regs[dst as usize];
                self.cur_regs[dst as usize] = c;
                self.heap.release(old);
            }
            Op::OptSome { dst, ty, val } => {
                self.alloc_sum(dst, ty, 0, Some(r!(val)))?;
            }
            Op::OptNone { dst, ty } => {
                self.alloc_sum(dst, ty, 1, None)?;
            }
            Op::ResOk { dst, ty, val } => {
                self.alloc_sum(dst, ty, 0, Some(r!(val)))?;
            }
            Op::ResErr { dst, ty, val } => {
                self.alloc_sum(dst, ty, 1, Some(r!(val)))?;
            }
            Op::SumIs { dst, v, want_err } => {
                let tag = sum_tag(cell_of(r!(v)))?;
                self.cur_regs[dst as usize] = Slot::bool((tag == 1) == want_err);
            }
            Op::Unwrap { dst, v, want_err } => {
                let cell = cell_of(r!(v));
                let (tag, payload) = sum_parts(cell)?;
                if (tag == 1) != want_err {
                    return Err(Trap::new(
                        TrapKind::UnwrapNone,
                        if want_err {
                            "`.error` on Ok"
                        } else {
                            "`.value` on None/Err — the value is absent (RFC 0005)"
                        },
                    ));
                }
                let val = payload.unwrap_or(Slot::int(0));
                self.move_sum_val(dst, val, self.sum_payload_ty(cell.ty, want_err));
            }
            Op::UnwrapOr { dst, v, default } => {
                let cell = cell_of(r!(v));
                let (tag, payload) = sum_parts(cell)?;
                let val = if tag == 0 {
                    payload.unwrap_or(Slot::int(0))
                } else {
                    r!(default)
                };
                self.move_sum_val(dst, val, self.sum_payload_ty(cell.ty, false));
            }
            Op::Expect { dst, v, msg } => {
                let cell = cell_of(r!(v));
                let (tag, payload) = sum_parts(cell)?;
                if tag == 1 {
                    let m = cell_of(r!(msg)).as_str().to_string();
                    return Err(Trap::new(TrapKind::UnwrapNone, format!("expect failed: {m}")));
                }
                let val = payload.unwrap_or(Slot::int(0));
                self.move_sum_val(dst, val, self.sum_payload_ty(cell.ty, false));
            }

            Op::TidOf { dst, obj } => {
                let cell = cell_of(r!(obj));
                let ty = self.effective_ty(cell);
                self.cur_regs[dst as usize] = Slot::int(ty as i64);
            }
            Op::IsType { dst, obj, want } => {
                let cell = cell_of(r!(obj));
                let ty = self.effective_ty(cell);
                self.cur_regs[dst as usize] = Slot::bool(ty == want);
            }
            Op::IsTrait { dst, obj, want } => {
                let cell = cell_of(r!(obj));
                let ty = self.effective_ty(cell);
                let has = self
                    .prog
                    .trait_slots
                    .iter()
                    .enumerate()
                    .any(|(slot, &(t, _))| {
                        t == want
                            && self
                                .prog
                                .vtables
                                .get(ty as usize)
                                .and_then(|v| v.get(slot))
                                .is_some_and(|f| f.is_some())
                    });
                self.cur_regs[dst as usize] = Slot::bool(has);
            }
            Op::Unbox { dst, box_, ty } => {
                let Some((val, val_ty)) = cell_of(r!(box_)).as_opaque() else {
                    return Err(Trap::new(TrapKind::BadUnbox, "unbox on non-opaque"));
                };
                if val_ty != ty {
                    return Err(Trap::new(
                        TrapKind::BadUnbox,
                        "unbox type mismatch (the compiler guards this; the trap is the safety net)",
                    ));
                }
                self.move_sum_val(dst, val, ty);
            }
            Op::Box { dst, val, ty } => {
                let v = r!(val);
                if self.prog.types.is_ref(ty) {
                    self.heap.retain(v);
                }
                let c = self.heap.alloc_opaque(v, ty)?;
                let old = self.cur_regs[dst as usize];
                self.cur_regs[dst as usize] = c;
                self.heap.release(old);
            }

            Op::MakeClosure { dst, func, captures } => {
                let params = self.prog.funcs[func as usize].params.clone();
                let ret = self.prog.funcs[func as usize].ret;
                let caps: Vec<Slot> = captures.iter().map(|&c| r!(c)).collect();
                let cap_tys: Vec<TypeId> = (params.len() - caps.len()..params.len())
                    .map(|i| params[i])
                    .collect();
                // captures retained into the cell
                for (&c, &t) in caps.iter().zip(cap_tys.iter()) {
                    if self.is_ref(t) {
                        self.heap.retain(c);
                    }
                }
                let c = self.heap.alloc_closure(func, params, ret, caps, cap_tys)?;
                let old = self.cur_regs[dst as usize];
                self.cur_regs[dst as usize] = c;
                self.heap.release(old);
            }

            Op::Panic { msg } => {
                let m = cell_of(r!(msg)).as_str().to_string();
                return Err(Trap::new(TrapKind::Panic, m));
            }
            Op::Assert { cond, msg } => {
                if !r!(cond).as_bool() {
                    let m = msg
                        .map(|m| cell_of(r!(m)).as_str().to_string())
                        .unwrap_or_else(|| "assertion failed".to_string());
                    return Err(Trap::new(TrapKind::Assert, m));
                }
            }
            Op::Conv { dst, src, from, to } => {
                let v = self.convert(r!(src), from, to)?;
                self.cur_regs[dst as usize] = v;
            }
            Op::StrCharAt { dst, s, idx } => {
                let str_cell = cell_of(r!(s));
                let i = unsafe { r!(idx).i };
                let c = str_cell
                    .as_str()
                    .chars()
                    .nth(i as usize)
                    .ok_or_else(|| {
                        Trap::new(
                            TrapKind::IndexOutOfBounds,
                            format!("string index {i} out of bounds ({} chars)", str_cell.as_str().chars().count()),
                        )
                    })?;
                self.cur_regs[dst as usize] = Slot::ch(c);
            }
        }
        Ok(())
    }

    fn alloc_sum(&mut self, dst: Reg, ty: TypeId, tag: u32, payload: Option<Slot>) -> Result<(), Trap> {
        let inner = match (self.prog.types.kind(ty).clone(), tag) {
            (TyKind::Option { elem }, 0) => Some(elem),
            (TyKind::Result { ok, .. }, 0) => Some(ok),
            (TyKind::Result { err, .. }, 1) => Some(err),
            _ => None,
        };
        if let (Some(v), Some(inner)) = (payload, inner) {
            if self.is_ref(inner) {
                self.heap.retain(v);
            }
        }
        let c = self.heap.alloc_sum(ty, tag, payload)?;
        let old = self.cur_regs[dst as usize];
        self.cur_regs[dst as usize] = c;
        self.heap.release(old);
        Ok(())
    }

    /// move a sum payload into dst with ref discipline
    fn move_sum_val(&mut self, dst: Reg, val: Slot, ty: TypeId) {
        let old = self.cur_regs[dst as usize];
        self.cur_regs[dst as usize] = val;
        if self.is_ref(ty) {
            self.heap.retain(val);
            self.heap.release(old);
        }
    }

    fn effective_ty(&self, cell: &crate::heap::CellVal) -> TypeId {
        match &cell.data {
            crate::heap::CellData::OpaqueBox { val_ty, .. } => *val_ty,
            _ => cell.ty,
        }
    }

    fn field_ty(&self, ty: TypeId, field: u32) -> TypeId {
        match self.prog.types.kind(ty) {
            TyKind::Data { fields } => fields.get(field as usize).map(|f| f.ty).unwrap_or(TY_ANY),
            _ => TY_ANY,
        }
    }

    fn elem_ty_of(&self, cell: &crate::heap::CellVal) -> TypeId {
        match &cell.data {
            crate::heap::CellData::Vec { elem, .. } => *elem,
            crate::heap::CellData::Array { elem, .. } => *elem,
            _ => TY_ANY,
        }
    }

    // ---- op bodies shared by `step` and the `run_loop` fast path ----

    /// `ArrGet` — the fast path calls this directly, so it must stay small
    /// and inlinable.
    #[inline]
    fn op_arr_get(&mut self, dst: Reg, arr: Reg, idx: Reg) -> Result<(), Trap> {
        let i = unsafe { self.cur_regs[idx as usize].i };
        let cell = cell_of(self.cur_regs[arr as usize]);
        let v = seq_get(cell, i)?;
        let elem = self.elem_ty_of(cell);
        let old = self.cur_regs[dst as usize];
        self.cur_regs[dst as usize] = v;
        if self.is_ref(elem) {
            self.heap.retain(v);
            self.heap.release(old);
        }
        Ok(())
    }

    /// `ArrSet` — shared by `step` and the `run_loop` fast path.
    #[inline]
    fn op_arr_set(&mut self, arr: Reg, idx: Reg, val: Reg) -> Result<(), Trap> {
        let i = unsafe { self.cur_regs[idx as usize].i };
        let cell = cell_of(self.cur_regs[arr as usize]);
        let elem = self.elem_ty_of(cell);
        let v = self.cur_regs[val as usize];
        let old = seq_set(cell, i, v)?;
        if self.is_ref(elem) {
            self.heap.retain(v);
            self.heap.release(old);
        }
        Ok(())
    }

    /// `GetF` — shared by `step` and the `run_loop` fast path.
    #[inline]
    fn op_getf(&mut self, dst: Reg, obj: Reg, field: u32) -> Result<(), Trap> {
        let cell = cell_of(self.cur_regs[obj as usize]);
        let v = match &cell.data {
            CellData::Record { fields } => fields
                .borrow()
                .get(field as usize)
                .ok_or_else(|| Trap::new(TrapKind::Invalid, "field index out of range"))?,
            _ => return Err(Trap::new(TrapKind::Invalid, "field on non-record")),
        };
        let fty = self.field_ty(cell.ty, field);
        let old = self.cur_regs[dst as usize];
        self.cur_regs[dst as usize] = v;
        if self.is_ref(fty) {
            self.heap.retain(v);
            self.heap.release(old);
        }
        Ok(())
    }

    /// `SetF` — shared by `step` and the `run_loop` fast path.
    #[inline]
    fn op_setf(&mut self, obj: Reg, field: u32, val: Reg) -> Result<(), Trap> {
        let cell = cell_of(self.cur_regs[obj as usize]);
        let fty = self.field_ty(cell.ty, field);
        let v = self.cur_regs[val as usize];
        let old = if let CellData::Record { fields } = &cell.data {
            fields
                .borrow_mut()
                .set(field as usize, v)
                .ok_or_else(|| Trap::new(TrapKind::Invalid, "field index out of range"))?
        } else {
            return Err(Trap::new(TrapKind::Invalid, "field-set on non-record"));
        };
        if self.is_ref(fty) {
            self.heap.retain(v);
            self.heap.release(old);
        }
        Ok(())
    }

    fn sum_payload_ty(&self, ty: TypeId, want_err: bool) -> TypeId {
        match self.prog.types.kind(ty) {
            TyKind::Option { elem } => *elem,
            TyKind::Result { ok, err } => {
                if want_err {
                    *err
                } else {
                    *ok
                }
            }
            _ => TY_ANY,
        }
    }

    // ---- natives (RFC 0032 §1.1 R2: named things are natives, not ops) ----

    fn call_nat(&mut self, nat: Nat, recv: Option<Reg>, args: &[Reg], dst: Option<Reg>) -> Result<(), Trap> {
        macro_rules! r {
            ($i:expr) => {
                self.cur_regs[$i as usize]
            };
        }
        match nat {
            Nat::Print => {
                let s = cell_of(r!(args[0])).as_str().to_string();
                if let Some(h) = &self.hooks.print {
                    (h.borrow_mut())(&s);
                }
                if let Some(d) = dst {
                    self.cur_regs[d as usize] = Slot::int(0);
                }
            }
            Nat::Str => {
                let s = self.render(r!(args[0]), args[0])?;
                let c = self.heap.alloc_str(s)?;
                self.store_result(dst, c)?;
            }
            Nat::Concat => {
                let mut out = String::new();
                for a in args {
                    out.push_str(cell_of(r!(*a)).as_str());
                }
                let c = self.heap.alloc_str(out)?;
                self.store_result(dst, c)?;
            }
            Nat::StrLen => {
                let n = cell_of(r!(recv.unwrap())).as_str().chars().count() as i64;
                if let Some(d) = dst {
                    self.cur_regs[d as usize] = Slot::int(n);
                }
            }
            Nat::VecNew => {
                let c = self.heap.alloc_vec(TY_ANY, 0, &self.prog.types)?;
                self.store_result(dst, c)?;
            }
            Nat::VecZeroed => {
                let n = unsafe { r!(args[0]).i }.max(0) as usize;
                let c = self.heap.alloc_vec(TY_ANY, n, &self.prog.types)?;
                if let crate::heap::CellData::Vec { items, .. } = &cell_of(c).data {
                    let mut fb = items.borrow_mut();
                    for _ in 0..n {
                        fb.push(Slot::null());
                    }
                }
                self.store_result(dst, c)?;
            }
            Nat::VecFrom => {
                let cell = cell_of(r!(args[0]));
                let Some(elem) = cell.elem_ty() else {
                    return Err(Trap::new(TrapKind::Invalid, "Vec.from expects an array"));
                };
                let items = cell.seq_items_copy().unwrap_or_default();
                let c = self.heap.alloc_vec(elem, items.len(), &self.prog.types)?;
                if let crate::heap::CellData::Vec { items: out, .. } = &cell_of(c).data {
                    let mut ob = out.borrow_mut();
                    for it in items {
                        if self.is_ref(elem) {
                            self.heap.retain(it);
                        }
                        ob.push(it);
                    }
                }
                self.store_result(dst, c)?;
            }
            Nat::VecLen => {
                let cell = cell_of(r!(recv.unwrap()));
                let n = match &cell.data {
                    crate::heap::CellData::Vec { items, .. } => items.borrow().len() as i64,
                    _ => return Err(Trap::new(TrapKind::Invalid, "len on non-vec")),
                };
                if let Some(d) = dst {
                    self.cur_regs[d as usize] = Slot::int(n);
                }
            }
            Nat::VecPush => {
                let cell = cell_of(r!(recv.unwrap()));
                let elem = self.elem_ty_of(cell);
                let v = r!(args[0]);
                if let crate::heap::CellData::Vec { items, .. } = &cell.data {
                    // growth accounting rides the heap budget (RFC 0040 §1)
                    self.heap.charge_public(items.borrow().elem_width())?;
                    if self.is_ref(elem) {
                        self.heap.retain(v);
                    }
                    items.borrow_mut().push(v);
                    if let Some(d) = dst {
                        self.cur_regs[d as usize] = Slot::int(items.borrow().len() as i64);
                    }
                }
            }
            Nat::VecPop => {
                let cell = cell_of(r!(recv.unwrap()));
                let elem = self.elem_ty_of(cell);
                if let crate::heap::CellData::Vec { items, .. } = &cell.data {
                    if items.borrow().is_empty() {
                        return Err(Trap::new(TrapKind::IndexOutOfBounds, "pop on an empty vec"));
                    }
                    let popped = items.borrow_mut().pop();
                    match popped {
                        Some(v) => {
                            if let Some(d) = dst {
                                let old = self.cur_regs[d as usize];
                                self.cur_regs[d as usize] = v;
                                if self.is_ref(elem) {
                                    self.heap.release(old);
                                }
                            } else if self.is_ref(elem) {
                                self.heap.release(v);
                            }
                        }
                        None => {
                            if let Some(d) = dst {
                                self.cur_regs[d as usize] = Slot::int(0);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn store_result(&mut self, dst: Option<Reg>, v: Slot) -> Result<(), Trap> {
        if let Some(d) = dst {
            let old = self.cur_regs[d as usize];
            self.cur_regs[d as usize] = v;
            // store_result is used for freshly minted cells: the register
            // now owns the mint's single reference; release the old ref if
            // the dst register is ref-typed
            let ty = self.regs_ty(d);
            if self.is_ref(ty) {
                self.heap.release(old);
            }
        } else {
            self.heap.release(v);
        }
        Ok(())
    }

    /// Per-type formatting — the RFC 0007 §2 table. The register's static
    /// type says how to read the slot.
    fn render(&self, v: Slot, reg: Reg) -> Result<String, Trap> {
        let ty = self.regs_ty(reg);
        match self.prog.types.kind(ty) {
            TyKind::Prim(PrimTy::F32) => Ok(format!("{}", unsafe { v.f } as f32)),
            TyKind::Prim(PrimTy::F64) => Ok(format!("{}", unsafe { v.f })),
            TyKind::Prim(PrimTy::Bool) => Ok(if v.as_bool() { "true".into() } else { "false".into() }),
            TyKind::Prim(PrimTy::Char) => Ok(v.as_char().to_string()),
            TyKind::Prim(_) => Ok(unsafe { v.i }.to_string()),
            TyKind::Str => Ok(cell_of(v).as_str().to_string()),
            TyKind::Enum { members } => {
                let Some(member) = cell_of(v).as_enum_member() else {
                    return Err(Trap::new(TrapKind::Invalid, "render on non-enum"));
                };
                Ok(members[member as usize].0.clone())
            }
            _ => Err(Trap::new(
                TrapKind::Invalid,
                "this type is not formattable inside f\"...\" (RFC 0007 §2)",
            )),
        }
    }

    // ---- arithmetic (RFC 0004 §3) ----

    fn arith(&self, op: ArithOp, p: PrimTy, x: Slot, y: Slot, wrapping: bool) -> Result<Slot, Trap> {
        use PrimTy::*;
        if p.is_float() {
            let a = unsafe { x.f };
            let b = unsafe { y.f };
            let r = match op {
                ArithOp::Add => a + b,
                ArithOp::Sub => a - b,
                ArithOp::Mul => a * b,
                ArithOp::Div => a / b,
                ArithOp::Mod => a % b,
            };
            let r = if p == F32 { r as f32 as f64 } else { r };
            return Ok(Slot::float(r));
        }
        let a = unsafe { x.i };
        let b = unsafe { y.i };
        let unsigned = matches!(p, U8 | U16 | U32 | U64);
        let (r, o) = match op {
            ArithOp::Add => a.overflowing_add(b),
            ArithOp::Sub => a.overflowing_sub(b),
            ArithOp::Mul => a.overflowing_mul(b),
            ArithOp::Div => {
                if b == 0 {
                    return Err(Trap::new(TrapKind::DivByZero, "division by zero"));
                }
                (if unsigned { (a as u64 / b as u64) as i64 } else { a / b }, false)
            }
            ArithOp::Mod => {
                if b == 0 {
                    return Err(Trap::new(TrapKind::DivByZero, "mod by zero"));
                }
                (if unsigned { (a as u64 % b as u64) as i64 } else { a % b }, false)
            }
        };
        if !wrapping && (o || !fits(r, p)) {
            return Err(Trap::new(
                TrapKind::Overflow,
                "arithmetic overflow — use the &+ &- &* wrapping forms (RFC 0004 §3)",
            ));
        }
        Ok(Slot::int(trunc_to(r, p)))
    }

    fn bitop(&self, op: BitOp, p: PrimTy, x: Slot, y: Slot) -> Result<Slot, Trap> {
        use PrimTy::*;
        if !p.is_int() {
            return Err(Trap::new(TrapKind::Invalid, "bit op on non-integer"));
        }
        let a = unsafe { x.i };
        let b = unsafe { y.i };
        let (r, o) = match op {
            BitOp::And => (a & b, false),
            BitOp::Or => (a | b, false),
            BitOp::Xor => (a ^ b, false),
            BitOp::Shl => a.overflowing_shl((b & 63) as u32),
            // `>>` follows signedness: logical on unsigned (a u64 with its
            // top bit set must not sign-extend — RFC 0004 §1), arithmetic
            // on signed
            BitOp::Shr => (
                if matches!(p, U8 | U16 | U32 | U64) {
                    ((a as u64) >> (b & 63)) as i64
                } else {
                    a.overflowing_shr((b & 63) as u32).0
                },
                false,
            ),
            // wrapping `&<<` (RFC 0004 §3): the shifted-out bits are simply
            // gone — truncate to the operand width, never trap
            BitOp::WrapShl => return Ok(Slot::int(trunc_to(a.wrapping_shl((b & 63) as u32), p))),
        };
        if o {
            return Err(Trap::new(TrapKind::Overflow, "shift overflow"));
        }
        if !fits(r, p) {
            return Err(Trap::new(TrapKind::Overflow, "bit op out of range"));
        }
        Ok(Slot::int(r))
    }

    /// Explicit numeric conversion (RFC 0007 §1): int↔int traps on
    /// narrowing loss; float→int traps on fraction/range; int→float and
    /// float↔float always convert.
    fn convert(&self, v: Slot, from: PrimTy, to: PrimTy) -> Result<Slot, Trap> {
        use rut_core::types::PrimTy::*;
        let (fp, tp) = (from, to);
        Ok(match (fp.is_float(), tp.is_float()) {
            (false, false) => {
                let x = unsafe { v.i };
                if !fits(x, tp) {
                    return Err(Trap::new(
                        TrapKind::Overflow,
                        format!("narrowing conversion {} -> {} loses the value (RFC 0007 §1)", fp.name(), tp.name()),
                    ));
                }
                Slot::int(x)
            }
            (false, true) => {
                let x = unsafe { v.i } as f64;
                if tp == F32 {
                    Slot::float(x as f32 as f64)
                } else {
                    Slot::float(x)
                }
            }
            (true, false) => {
                let x = unsafe { v.f };
                if x.fract() != 0.0 || !x.is_finite() || !float_fits(x, tp) {
                    return Err(Trap::new(
                        TrapKind::Overflow,
                        format!("float {} does not convert to {} (fractional or out of range — RFC 0007 §1)", x, tp.name()),
                    ));
                }
                Slot::int(x as i64)
            }
            (true, true) => {
                let x = unsafe { v.f };
                if tp == F32 {
                    Slot::float(x as f32 as f64)
                } else {
                    Slot::float(x)
                }
            }
        })
    }

    fn cmp(&self, op: CmpOp, p: PrimTy, x: Slot, y: Slot) -> bool {
        if p.is_float() {
            let a = unsafe { x.f };
            let b = unsafe { y.f };
            return match op {
                CmpOp::Eq => a == b,
                CmpOp::Ne => a != b,
                CmpOp::Lt => a < b,
                CmpOp::Gt => a > b,
                CmpOp::Le => a <= b,
                CmpOp::Ge => a >= b,
            };
        }
        let a = unsafe { x.i };
        let b = unsafe { y.i };
        match op {
            CmpOp::Eq => a == b,
            CmpOp::Ne => a != b,
            CmpOp::Lt => a < b,
            CmpOp::Gt => a > b,
            CmpOp::Le => a <= b,
            CmpOp::Ge => a >= b,
        }
    }
}

const TY_ANY: TypeId = u32::MAX;

fn slot_to_value(v: Slot, ty: TypeId, prog: &Program, heap: &Heap) -> Value {
    use rut_core::types::TyKind;
    match prog.types.kind(ty) {
        TyKind::Prim(PrimTy::F32) | TyKind::Prim(PrimTy::F64) => Value::F64(unsafe { v.f }),
        TyKind::Prim(PrimTy::Bool) => Value::Bool(v.as_bool()),
        TyKind::Prim(PrimTy::Char) => Value::Char(v.as_char()),
        TyKind::Prim(_) | TyKind::Unit => Value::I64(unsafe { v.i }),
        TyKind::Str => Value::Str(cell_of(v).as_str().to_string()),
        // Vec<u8> is the one sequence that crosses (RFC 0023 §2)
        TyKind::Vec { elem: _ } => {
            let cell = cell_of(v);
            if let CellData::Vec { items, .. } = &cell.data {
                Value::Bytes(items.borrow().to_slots().iter().map(|s| unsafe { s.i } as u8).collect())
            } else {
                Value::Bytes(Vec::new())
            }
        }
        TyKind::Option { elem } => {
            let cell = cell_of(v);
            match &cell.data {
                CellData::Sum { tag: 1, .. } => Value::Opt(None),
                CellData::Sum { tag: _, payload } => Value::Opt(Some(Box::new(
                    slot_to_value(payload.expect("Some without payload"), *elem, prog, heap),
                ))),
                _ => Value::Opt(None),
            }
        }
        TyKind::Result { ok, err } => {
            let cell = cell_of(v);
            match &cell.data {
                CellData::Sum { tag: 1, payload } => Value::Res(Err(Box::new(slot_to_value(
                    payload.expect("Err without payload"),
                    *err,
                    prog,
                    heap,
                )))),
                CellData::Sum { tag: _, payload } => Value::Res(Ok(Box::new(slot_to_value(
                    payload.expect("Ok without payload"),
                    *ok,
                    prog,
                    heap,
                )))),
                _ => Value::Res(Ok(Box::new(Value::Unit))),
            }
        }
        // an Opaque box crosses as its handle: the handle owns a fresh
        // arena reference; the pending slot reference is released by do_ret
        TyKind::Opaque => {
            let p = (unsafe { v.r }).expect("opaque slot without a cell");
            Value::Opaque(heap.opaque_handle(p))
        }
        _ => Value::I64(unsafe { v.i }),
    }
}

fn value_kind_name(v: &Value) -> &'static str {
    match v {
        Value::Unit => "unit",
        Value::I64(_) => "an integer",
        Value::F64(_) => "a float",
        Value::Bool(_) => "a bool",
        Value::Char(_) => "a char",
        Value::Str(_) => "a string",
        Value::Bytes(_) => "bytes",
        Value::Opt(_) => "an Option",
        Value::Res(_) => "a Result",
        Value::Opaque(_) => "an Opaque",
    }
}

fn seq_get(cell: &crate::heap::CellVal, i: i64) -> Result<Slot, Trap> {
    let (items, what) = match &cell.data {
        crate::heap::CellData::Vec { items, .. } => (items.borrow(), "vec"),
        crate::heap::CellData::Array { items, .. } => (items.borrow(), "array"),
        _ => return Err(Trap::new(TrapKind::Invalid, "index on non-sequence")),
    };
    items
        .get(i as usize)
        .ok_or_else(|| Trap::new(TrapKind::IndexOutOfBounds, format!("{what} index {i} out of bounds (len {})", items.len())))
}

fn seq_set(cell: &crate::heap::CellVal, i: i64, v: Slot) -> Result<Slot, Trap> {
    let mut items = match &cell.data {
        crate::heap::CellData::Vec { items, .. } => items.borrow_mut(),
        crate::heap::CellData::Array { items, .. } => items.borrow_mut(),
        _ => return Err(Trap::new(TrapKind::Invalid, "index-set on non-sequence")),
    };
    let len = items.len();
    items
        .set(i as usize, v)
        .ok_or_else(|| Trap::new(TrapKind::IndexOutOfBounds, format!("index {i} out of bounds (len {len})")))
}

fn sum_tag(cell: &crate::heap::CellVal) -> Result<u32, Trap> {
    match &cell.data {
        crate::heap::CellData::Sum { tag, .. } => Ok(*tag),
        _ => Err(Trap::new(TrapKind::Invalid, "sum op on non-sum")),
    }
}

fn sum_parts(cell: &crate::heap::CellVal) -> Result<(u32, Option<Slot>), Trap> {
    match &cell.data {
        crate::heap::CellData::Sum { tag, payload } => Ok((*tag, *payload)),
        _ => Err(Trap::new(TrapKind::Invalid, "sum op on non-sum")),
    }
}

fn default_slot(ty: TypeId, table: &rut_core::types::TypeTable) -> Slot {
    match table.kind(ty) {
        TyKind::Prim(_) => Slot::int(0),
        _ => Slot::null(),
    }
}

fn trunc_to(v: i64, p: PrimTy) -> i64 {
    use PrimTy::*;
    match p {
        U8 => (v as u8) as i64,
        U16 => (v as u16) as i64,
        U32 => (v as u32) as i64,
        U64 => v,
        I8 => (v as i8) as i64,
        I16 => (v as i16) as i64,
        I32 => (v as i32) as i64,
        _ => v,
    }
}

fn fits(v: i64, p: PrimTy) -> bool {
    trunc_to(v, p) == v
}

fn float_fits(x: f64, p: PrimTy) -> bool {
    match p {
        PrimTy::U8 => x >= 0.0 && x <= u8::MAX as f64,
        PrimTy::U16 => x >= 0.0 && x <= u16::MAX as f64,
        PrimTy::U32 => x >= 0.0 && x <= u32::MAX as f64,
        PrimTy::U64 => x >= 0.0 && x <= u64::MAX as f64,
        PrimTy::I8 => x >= i8::MIN as f64 && x <= i8::MAX as f64,
        PrimTy::I16 => x >= i16::MIN as f64 && x <= i16::MAX as f64,
        PrimTy::I32 => x >= i32::MIN as f64 && x <= i32::MAX as f64,
        PrimTy::I64 => x >= i64::MIN as f64 && x <= i64::MAX as f64,
        _ => true,
    }
}
