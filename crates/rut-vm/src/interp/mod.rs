//! The interpreter loop — RFC 0034: one struct, one thread, one heap.
//! Register frames, typed ops, traps as `Err(Trap)` (never Rust panics),
//! fuel + heap budgets checked at loop back-edges and every allocation
//! (RFC 0040). Traps are catchable only at the host boundary (RFC 0001 P4).
//!
//! The ACTIVE frame lives directly on the `Vm` (`cur_*`); calls push it
//! onto `frames` and rets pop — keeps register access borrow-friendly.

use rut_core::binary::{ConstVal, Program};
use crate::heap::{cell_of, CellData, Heap, Slot, Trap, TrapKind, Value};
use rut_core::ops::*;
use rut_core::types::{PrimTy, Repr, TypeId, TyKind};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

mod native;
mod ops;
mod run;
mod scalar;
mod step;
mod threaded;
mod util;

// free helpers live in the submodules; pull them into `interp` so the
// sibling modules reach them through `use super::*`
use scalar::*;
use util::*;

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
        ConstVal::TypeId(t) => Slot::int(*t as i64),
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
pub struct HostHooks {}

/// An embedder implementation for a bodyless host function
/// (RFC 0022/0026): registered by name against `FuncCode.host`. It gets the
/// VM so it can allocate/inspect `Opaque` handles (RFC 0014).
pub type HostFn = Rc<RefCell<dyn FnMut(&mut Vm, &[Value]) -> Result<Value, Trap>>>;

struct SavedFrame {
    func: u32,
    pc: u32,
    regs: Vec<Slot>,
    ret_dst: Option<Reg>,
}

/// The full interpreter cursor, swappable for a re-entrant `vm.call`
/// (RFC 0022 §1: a host fn may call back into rut). The outer frame's
/// registers and frame stack are stashed while the nested call runs on a
/// fresh stack; both Ok and Trap restore the outer cursor exactly, so a
/// propagated nested trap leaves the outer frame resumable.
struct InterpCursor {
    frames: Vec<SavedFrame>,
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
    /// host-function impls by name (RFC 0022/0026)
    host_fns: HashMap<String, HostFn>,
    const_slots: Vec<Slot>,
    /// recycled per-frame register files (avoids a Vec alloc per call)
    reg_pool: Vec<Vec<Slot>>,
    /// per-function indices of reference-typed registers — `do_ret` only
    /// needs to release these, so it skips the type-table lookup per slot
    ref_regs: Vec<Vec<u16>>,
    /// per-type indices of reference-typed fields, for fused construction
    data_ref_fields: Vec<Vec<u16>>,
    /// `type_repr[t]` — the baked representation of type `t` (no runtime
    /// type-table match)
    type_repr: Vec<Repr>,
    /// `sum_payload_repr[t][tag]` — payload repr for Option/Result types
    sum_payload_repr: Vec<[Repr; 2]>,
    /// per-function dispatch tags for `rut-vm-threaded` (one per op)
    op_tags: Vec<Vec<u8>>,
    /// threaded handler table, built once and reused across bails
    thread_table: Rc<rut_vm_threaded::Table<Vm>>,
}

/// Const-generic op codes for the scalar op bodies — RFC 0032: the opcode is
/// the operation, so each specialized `addi`/`andi`/`lti`/... instantiates a
/// monomorphic body and the `match OP` folds without relying on inlining.
/// (Enums can't be const generic params on stable, hence the `i32` codes.)
const IOP_ADD: i32 = 0;
const IOP_SUB: i32 = 1;
const IOP_MUL: i32 = 2;
const IOP_DIV: i32 = 3;
const IOP_MOD: i32 = 4;

const BOP_AND: i32 = 0;
const BOP_OR: i32 = 1;
const BOP_XOR: i32 = 2;
const BOP_SHL: i32 = 3;
const BOP_SHR: i32 = 4;
const BOP_WRAPSHL: i32 = 5;

/// Compare codes are shared by the int and float bodies (the operation is the
/// same, only the operand interpretation differs).
const COP_EQ: i32 = 0;
const COP_NE: i32 = 1;
const COP_LT: i32 = 2;
const COP_GT: i32 = 3;
const COP_LE: i32 = 4;
const COP_GE: i32 = 5;

const FOP_ADD: i32 = 0;
const FOP_SUB: i32 = 1;
const FOP_MUL: i32 = 2;
const FOP_DIV: i32 = 3;
const FOP_MOD: i32 = 4;

const TY_ANY: TypeId = u32::MAX;

impl Vm {
    pub fn new(prog: Rc<Program>, limits: &Limits, hooks: HostHooks) -> Result<Vm, Trap> {
        let heap =
            Heap::new(limits.heap_limit_bytes, crate::arena::ReleasePlan::build(&prog.types, &prog.funcs));
        let mut const_slots = Vec::with_capacity(prog.consts.len());
        for c in &prog.consts {
            let s = const_to_slot(c, &heap).map_err(|m| Trap::new(TrapKind::Invalid, m))?;
            const_slots.push(s);
        }
        let ref_regs: Vec<Vec<u16>> = prog
            .funcs
            .iter()
            .map(|f| {
                f.regs
                    .iter()
                    .enumerate()
                    .filter(|(_, &ty)| prog.types.is_ref(ty))
                    .map(|(i, _)| i as u16)
                    .collect()
            })
            .collect();
        let data_ref_fields: Vec<Vec<u16>> = prog
            .types
            .types
            .iter()
            .map(|t| match &t.kind {
                TyKind::Data { fields } => fields
                    .iter()
                    .enumerate()
                    .filter(|(_, fd)| prog.types.repr_of(fd.ty).is_ref())
                    .map(|(i, _)| i as u16)
                    .collect(),
                // `*T` (RFC 0005): a one-slot pointer box retains its payload
                TyKind::Ptr { elem } => {
                    if prog.types.repr_of(*elem).is_ref() {
                        vec![0]
                    } else {
                        Vec::new()
                    }
                }
                _ => Vec::new(),
            })
            .collect();
        let type_repr: Vec<Repr> = prog.types.types.iter().map(|t| match &t.kind {
            TyKind::Prim(p) => Repr::Prim(*p),
            TyKind::Unit | TyKind::Fn { .. } => Repr::Any,
            _ => Repr::Ref,
        }).collect();
        let sum_payload_repr: Vec<[Repr; 2]> = prog
            .types
            .types
            .iter()
            .map(|t| match &t.kind {
                TyKind::Option { elem } => [
                    type_repr.get(*elem as usize).copied().unwrap_or(Repr::Any),
                    Repr::Any,
                ],
                TyKind::Result { ok, err } => [
                    type_repr.get(*ok as usize).copied().unwrap_or(Repr::Any),
                    type_repr.get(*err as usize).copied().unwrap_or(Repr::Any),
                ],
                _ => [Repr::Any, Repr::Any],
            })
            .collect();
        // precompute the threaded dispatch tags (one per op)
        let op_tags: Vec<Vec<u8>> = prog
            .funcs
            .iter()
            .map(|f| f.code.iter().map(rut_vm_threaded::tag_of).collect())
            .collect();
        let thread_table = Rc::new(rut_vm_threaded::build_table::<Vm>());
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
            host_fns: HashMap::new(),
            const_slots,
            reg_pool: Vec::new(),
            ref_regs,
            data_ref_fields,
            type_repr,
            sum_payload_repr,
            op_tags,
            thread_table,
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
    /// (RFC 0035 §3). Traps unwind here (RFC 0034 §2). Re-entrant: a host
    /// fn holding `&mut Vm` may call back into rut (RFC 0022 §1) — the
    /// nested call runs on a fresh frame stack under the same budget, and
    /// the outer cursor is restored whether the callee returns or traps
    /// (a propagated nested trap keeps the outer frame resumable; resuming
    /// re-runs the host fn from its op, the standing host-fn-trap rule).
    pub fn call(&mut self, export: &str, args: &[Value]) -> Result<Value, Trap> {
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
        if self.running {
            // nested entry: stash the outer cursor, run the callee to its
            // root ret on a clean frame stack, restore either way
            let outer = InterpCursor {
                frames: std::mem::take(&mut self.frames),
                func: self.cur_func,
                pc: self.cur_pc,
                regs: std::mem::take(&mut self.cur_regs),
                ret_dst: self.cur_ret_dst,
            };
            self.running = false; // enter() must not push a phantom frame
            self.enter(func, regs, None);
            let r = self.run_loop();
            // queued `on_drop` callbacks run before the cursor restore —
            // the machine is idle here (RFC 0016 §3)
            let r = self.drain_after(r);
            self.frames = outer.frames;
            self.cur_func = outer.func;
            self.cur_pc = outer.pc;
            self.cur_regs = outer.regs;
            self.cur_ret_dst = outer.ret_dst;
            self.running = true;
            r
        } else {
            self.enter(func, regs, None);
            let r = self.run_loop();
            self.drain_after(r)
        }
    }

    /// Run queued `on_drop` callbacks after a successful root call
    /// (RFC 0016 §3): each callback receives the pinned cell; the pin's
    /// release afterwards is what finally frees it. A callback may itself
    /// drop pointers — the loop drains until the queue stays empty. A trap
    /// from the main call skips the drain (unwinding semantics are OQ).
    fn drain_after(&mut self, r: Result<Value, Trap>) -> Result<Value, Trap> {
        let v = r?;
        loop {
            let Some((obj, cleanup)) = self.heap.take_pending_drop() else {
                break;
            };
            let done = self.run_drop_callback(cleanup, obj);
            self.heap.release(obj);
            self.heap.release(cleanup);
            done?;
        }
        Ok(v)
    }

    /// Call one `on_drop` cleanup closure with the dying cell as its `*T`
    /// argument — the same frame machine, run to completion.
    fn run_drop_callback(&mut self, cleanup: Slot, arg: Slot) -> Result<(), Trap> {
        let (fid, captures): (u32, &[Slot]) = match &cell_of(cleanup).data {
            CellData::Closure { func, captures } => (*func, captures.as_slice()),
            _ => {
                return Err(Trap::new(
                    TrapKind::Invalid,
                    "on_drop cleanup is not a closure",
                ));
            }
        };
        let nparams = self.prog.funcs[fid as usize].params.len();
        let ncaptures = self.prog.funcs[fid as usize].n_captures as usize;
        let declared = nparams.saturating_sub(ncaptures);
        let nregs = self.prog.funcs[fid as usize].regs.len();
        let mut regs = self.take_regs(nregs);
        if declared >= 1 {
            regs[0] = arg;
            if self.is_ref(self.param_ty(fid, 0)) {
                self.heap.retain(arg);
            }
        }
        for (i, &c) in captures.iter().enumerate() {
            if declared + i < regs.len() {
                regs[declared + i] = c;
                let ty = self.param_ty(fid, declared + i);
                if self.is_ref(ty) {
                    self.heap.retain(regs[declared + i]);
                }
            }
        }
        let was_running = self.running;
        self.running = false;
        self.enter(fid, regs, None);
        let r = self.run_loop();
        self.running = was_running;
        r.map(|_| ())
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
            (Value::Tuple(parts), TyKind::Data { fields }) => {
                if parts.len() != fields.len() {
                    return Err(format!(
                        "tuple arity {}: {} fields expected",
                        parts.len(),
                        fields.len()
                    ));
                }
                let mut slots = Vec::with_capacity(parts.len());
                for (i, p) in parts.iter().enumerate() {
                    let fty = fields[i].ty;
                    slots.push(self.value_in(p, fty).map_err(|m| format!("field {i}: {m}"))?);
                }
                let c = self.heap.alloc_record_zeroed(ty, fields.len()).map_err(|t| t.msg)?;
                if let CellData::Record { fields: fs } = &cell_of(c).data {
                    for (i, sv) in slots.iter().enumerate() {
                        fs.borrow_mut().set(i, *sv);
                    }
                }
                c
            }
            (Value::Str(s), TyKind::Str) => self.heap.alloc_str(s.clone()).map_err(|t| t.msg)?,
            (Value::Bytes(b), TyKind::Bytes) => self.heap.alloc_bytes(b.clone()).map_err(|t| t.msg)?,
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
                let s = Slot { r: h.ptr() };
                if !matches!(cell_of(s).data, CellData::OpaqueBox { .. } | CellData::HostBoxed { .. }) {
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

    /// Register an embedder implementation for a bodyless host function
    /// (RFC 0022/0026), bound by the `FuncCode.host` name.
    pub fn register_host_fn<F>(&mut self, name: &str, f: F)
    where
        F: FnMut(&mut Vm, &[Value]) -> Result<Value, Trap> + 'static,
    {
        self.host_fns.insert(name.to_string(), Rc::new(RefCell::new(f)));
    }

    /// Dispatch `Op::Call` to a host function: convert the args to `Value`s,
    /// invoke the embedder's impl, convert the result. The name was bound at
    /// link; an unregistered name traps at the boundary.
    pub(super) fn call_host(&mut self, func: u32, args: &[Reg], dst: Option<Reg>) -> Result<(), Trap> {
        let fc = &self.prog.funcs[func as usize];
        let name = fc.host.clone().expect("call_host: not a host function");
        let params = fc.params.clone();
        let ret = fc.ret;
        let mut vals = Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            let ty = params.get(i).copied().unwrap_or(rut_core::types::TY_I32);
            vals.push(slot_to_value(self.cur_regs[*a as usize], ty, &self.prog, &self.heap));
        }
        let Some(f) = self.host_fns.get(&name).cloned() else {
            return Err(Trap::new(TrapKind::Invalid, format!("host function `{name}` is not registered")));
        };
        let out = {
            let mut f = f.borrow_mut();
            f(self, &vals)?
        };
        if let Some(d) = dst {
            let s = self
                .value_in(&out, ret)
                .map_err(|m| Trap::new(TrapKind::Invalid, format!("host `{name}` result: {m}")))?;
            if self.is_ref(ret) {
                let old = self.cur_regs[d as usize];
                self.heap.release(old);
            }
            self.cur_regs[d as usize] = s;
        }
        Ok(())
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
        ty != TY_ANY && self.type_repr[ty as usize].is_ref()
    }
}
