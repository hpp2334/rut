//! The interpreter loop — RFC 0034: one struct, one thread, one heap.
//! Register frames, typed ops, traps as `Err(Trap)` (never Rust panics),
//! fuel + heap budgets checked at loop back-edges and every allocation
//! (RFC 0040). Traps are catchable only at the host boundary (RFC 0001 P4).
//!
//! The ACTIVE frame lives directly on the `Vm` (`cur_*`); calls push it
//! onto `frames` and rets pop — keeps register access borrow-friendly.

use rut_core::binary::{ConstVal, Program};
use crate::heap::{cell_of, CellData, Heap, Slot, Trap, TrapKind, Value};
use crate::arena::OpaqueRef;
use rut_core::ops::*;
use rut_core::types::{
    PrimTy, Repr, TypeId, TyKind, TY_BOOL, TY_BYTES, TY_I16, TY_I32, TY_I64, TY_I8, TY_STR, TY_U16,
    TY_U32, TY_U64, TY_U8,
};
use std::rc::Rc;

mod boundary;
mod host;
mod native;
mod ops;
mod run;
mod scalar;
mod step;
mod threaded;
mod util;

pub use host::{ExpectedHostFns, HostRegistry};
use host::HostSlot;
pub use boundary::{CallArg, CallArgs, Ret};

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

/// The payload of an `Opaque` box classified against the closed
/// native-key set (the nmap host experiment): the integer primitives and
/// `bool` cross as raw bits, `str`/`bytes` cross as owned copies (short;
/// once per call), and everything else — floats, chars, user records,
/// host payload boxes — comes back [`KeyPayload::Unsupported`] with the
/// type id naming it, so the caller can trap pointing at `mapset`
/// instead of guessing at a key it cannot store.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum KeyPayload {
    /// an integer primitive or `bool`: the slot's raw 64 bits and the
    /// static type they belong to (`u64::MAX` is `-1i64` as bits)
    Bits { val: u64, ty: TypeId },
    /// a `str` key — the UTF-8 octets copied out
    Str(String),
    /// a `bytes` key — the octets copied out
    Bytes(Vec<u8>),
    /// not a natively supported key type; the id names it for the trap
    Unsupported(TypeId),
}

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
    pub(crate) heap: Heap,
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
    /// the dispatch table, dense by func idx — the `Vm::new` join
    /// (RFC 0025) built it from the registry; the hot path copies one
    /// `HostSlot` (a code pointer, a state word, the declared return)
    /// and dispatches. No Rc, no name work, no allocation.
    host_slots: Vec<HostSlot>,
    /// the boxed host bodies — `host_slots[i].ctx` points into these;
    /// owning them keeps every ctx word valid for the machine's lifetime
    /// (single thread, RFC 0034)
    host_keep: Vec<Box<dyn std::any::Any>>,
    /// the trap channel: a host body records its trap here instead of
    /// returning `Result`; `call_host` checks the flag the moment the
    /// body returns — the trap fires before the dst write
    host_trap: Option<Trap>,
    /// reused arg-snapshot spill for host arities past `INLINE_ARITY`
    /// (never taken in practice; the stack covers the common case)
    slot_scratch: Vec<Slot>,
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
    /// per-function dispatch tags for `rut-vm-threaded` (one per op)
    op_tags: Vec<Vec<u8>>,
    /// threaded handler table, built once and reused across bails
    thread_table: rut_vm_threaded::Table<Vm>,
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
    /// Boot the machine. `registry` is the embedder's host-fn binding
    /// table (RFC 0022/0025) — built BEFORE the Vm (an embedder depends
    /// on nothing else) and consumed here: every host thunk the program
    /// declares is resolved against it, so a declared-but-unbound fn is
    /// a construction error, never a mid-run trap. Call
    /// `HostRegistry::verify_against` first for the full RFC 0025
    /// contract (it also checks the bound-but-undeclared direction).
    pub fn new(
        prog: Rc<Program>,
        limits: &Limits,
        hooks: HostHooks,
        mut registry: HostRegistry,
    ) -> Result<Vm, Trap> {
        // ---- the host join (RFC 0025) ----
        // Resolve every host thunk's binding NOW; slots are dense by
        // func idx, one row each: the adapter code pointer, the boxed
        // body's ctx word, and the declared return for the refcount law.
        let mut host_slots: Vec<HostSlot> = Vec::with_capacity(prog.funcs.len());
        let mut host_keep: Vec<Box<dyn std::any::Any>> = Vec::new();
        for fc in prog.funcs.iter() {
            match fc.host_id {
                Some(hid) => {
                    let name = prog.interner.name(hid);
                    let b = registry.take_binding(name).ok_or_else(|| {
                        Trap::new(
                            TrapKind::Invalid,
                            format!(
                                "host fn `{name}` is declared by the program but never bound — \
                                 register the body in the HostRegistry before Vm::new (RFC 0025)"
                            ),
                        )
                    })?;
                    if let Some(k) = b.keep {
                        host_keep.push(k);
                    }
                    host_slots.push(HostSlot { code: b.code, ctx: b.ctx, ret: fc.ret });
                }
                None => host_slots.push(HostSlot::NEVER),
            }
        }
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
                // `?T` (RFC 0044): a one-slot nullable box retains its payload
                TyKind::Opt { elem } => {
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
            TyKind::Nil | TyKind::Fn { .. } => Repr::Any,
            _ => Repr::Ref,
        }).collect();
        // precompute the threaded dispatch tags (one per op)
        let op_tags: Vec<Vec<u8>> = prog
            .funcs
            .iter()
            .map(|f| f.code.iter().map(rut_vm_threaded::tag_of).collect())
            .collect();
        let thread_table = rut_vm_threaded::build_table::<Vm>();
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
            host_slots,
            host_keep,
            host_trap: None,
            slot_scratch: Vec::new(),
            const_slots,
            reg_pool: Vec::new(),
            ref_regs,
            data_ref_fields,
            type_repr,
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
    pub(crate) fn call_raw(&mut self, export: &str, args: &[Value]) -> Result<Value, Trap> {
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
    pub(crate) fn value_in(&mut self, v: &Value, ty: TypeId) -> Result<Slot, String> {
        use rut_core::types::{PrimTy, TyKind};
        let ty = if ty != u32::MAX { ty } else {
            return Err(format!("internal: untyped parameter"));
        };
        Ok(match (v, self.prog.types.kind(ty).clone()) {
            (Value::Nil, TyKind::Nil)
            | (Value::I64(0), TyKind::Nil) // `return;` crosses as a zero word
            | (Value::Nil, TyKind::Prim(_)) => Slot::int(0),
            (Value::I64(n), TyKind::Prim(p)) => {
                if !fits(*n, p) {
                    return Err(format!("`{n}` does not fit `{}`", self.prog.type_name(ty)));
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
            (Value::Opaque(h), TyKind::Opaque) => {
                let s = Slot { r: h.ptr() };
                if !matches!(cell_of(s).data, CellData::OpaqueBox { .. } | CellData::HostBoxed { .. }) {
                    return Err("not an opaque box".to_string());
                }
                self.heap.retain(s); // the parameter register owns its reference
                s
            }
            (v, _) => {
                return Err(format!(
                    "is `{}`, `{}` expected",
                    value_kind_name(v),
                    self.prog.type_name(ty)
                ))
            }
        })
    }

    /// Dispatch `Op::Call` to a host function. The binding was resolved
    /// at the `Vm::new` join: one `HostSlot` copy, one arg snapshot, one
    /// indirect call — no Rc, no name work, no `Value` enum, no envelope.
    /// The adapter (`HostHandler::entry`) reads the typed args itself and
    /// reports traps through the channel (§3.7 of the boundary plan).
    pub(super) fn call_host(&mut self, func: u32, argv_off: u32, argc: u16, dst: Reg) -> Result<(), Trap> {
        // the join resolved the binding; the entry is a code pointer, a
        // word, and a TypeId — copied out in one move, no Rc traffic
        let slot = self.host_slots[func as usize];
        let prog = Rc::clone(&self.prog);
        let args = self.cur_argv(&prog, argv_off, argc); // &[Reg], borrows the local Rc
        // snapshot the args: the register file may swap inside the body
        // (nested calls). `Slot` is Copy (one machine word, RFC 0015 §5).
        const INLINE_ARITY: usize = 8;
        let mut inline = [Slot::null(); INLINE_ARITY];
        let mut spill: Vec<Slot> = Vec::new();
        let snapshot: &[Slot] = if args.len() <= INLINE_ARITY {
            for (i, a) in args.iter().enumerate() {
                inline[i] = self.cur_regs[*a as usize];
            }
            &inline[..args.len()]
        } else {
            spill = std::mem::take(&mut self.slot_scratch); // reused, never in practice
            spill.extend(args.iter().map(|a| self.cur_regs[*a as usize]));
            &spill
        };
        // ONE indirect call — the same unit an op dispatch pays. The
        // adapter converts, invokes the body, and reports traps through
        // the channel.
        let out = (slot.code)(self, snapshot, slot.ctx);
        if args.len() > INLINE_ARITY {
            self.slot_scratch = spill;
        }
        if let Some(t) = self.host_trap.take() {
            return Err(t);
        }
        // the returned slot's reference count IS the register's —
        // transfer, not borrow (crossing-ownership law, RFC 0023 §2)
        if let Some(d) = reg_opt(dst) {
            if self.is_ref(slot.ret) {
                let old = std::mem::replace(&mut self.cur_regs[d as usize], out);
                self.heap.release(old);
            } else {
                self.cur_regs[d as usize] = out;
            }
        }
        Ok(())
    }

    /// The trap channel (host-fn ABI): record the trap and return a
    /// placeholder slot. `call_host` checks the flag the moment the body
    /// returns — the trap fires before the dst write. Return the
    /// placeholder (or anything); the pending flag wins.
    pub fn trap(&mut self, kind: TrapKind, msg: impl Into<String>) -> Slot {
        self.host_trap = Some(Trap::new(kind, msg.into()));
        Slot::int(0)
    }

    pub(crate) fn trap_taken(&mut self, t: Trap) -> Slot {
        self.host_trap = Some(t);
        Slot::int(0)
    }

    /// Resume after a budget trap (RFC 0034 §4: the frame IS the loop state).
    pub(crate) fn resume_raw(&mut self) -> Result<Value, Trap> {
        if !self.running {
            return Err(Trap::new(TrapKind::Invalid, "nothing to resume"));
        }
        self.run_loop()
    }

    /// The typed host boundary (RFC 0023, revised): call an export with
    /// Rust values, get a Rust value back — `Value`/`Slot` are internal
    /// marshaling formats, never seen by the embedder. Re-entrancy,
    /// budgets, and trap semantics are exactly `call`'s.
    pub fn call<A: CallArgs, R: Ret>(&mut self, export: &str, args: A) -> Result<R, Trap> {
        let values = args.into_values();
        let v = self.call_raw(export, &values)?;
        let ty = self.export_ret_ty(export)?;
        let slot = self
            .value_in(&v, ty)
            .map_err(|m| Trap::new(TrapKind::Invalid, format!("`{export}` result: {m}")))?;
        let out = R::from_slot(self, slot, ty)?;
        if self.is_ref(ty) {
            self.heap.release(slot); // from_slot cloned its own handle/copy
        }
        Ok(out)
    }

    /// Resume a budget-parked call, in Rust types.
    pub fn resume<R: Ret>(&mut self) -> Result<R, Trap> {
        let v = self.resume_raw()?;
        let ty = self.prog.funcs[self.cur_func as usize].ret;
        let slot = self
            .value_in(&v, ty)
            .map_err(|m| Trap::new(TrapKind::Invalid, format!("resume result: {m}")))?;
        let out = R::from_slot(self, slot, ty)?;
        if self.is_ref(ty) {
            self.heap.release(slot);
        }
        Ok(out)
    }

    /// Mint an `opaque` box owning a rut `str` — `opaque(str)` for
    /// hosts that hand rut a handle over host-built text (the logger's
    /// named logger; RFC 0014/0026). The handle owns one reference.
    pub fn alloc_opaque_str(&mut self, s: String) -> Result<OpaqueRef, Trap> {
        let slot = self.heap.alloc_str(s)?;
        let p = self.heap.alloc_opaque(slot, rut_core::types::TY_STR)?;
        Ok(self.heap.opaque_handle_take(unsafe { p.r }))
    }

    /// The key payload inside an `Opaque` box, classified against the
    /// closed native-key set (the nmap host experiment). Host code cannot
    /// read a rut-side box itself — `Slot` is crate-private — so this is
    /// the one pub reader: the box's `CellData::OpaqueBox { val, val_ty }`
    /// classifies by `val_ty` (ints/bool as raw bits, `str`/`bytes` as
    /// owned copies), a host payload box (`CellData::HostBoxed`, RFC 0023)
    /// has no rut value inside and is `Unsupported`, and any other
    /// `val_ty` — a user-defined key — is `Unsupported` naming the type.
    pub fn opaque_key_payload(&self, h: &OpaqueRef) -> Result<KeyPayload, Trap> {
        let cell = cell_of(Slot { r: h.ptr() });
        match &cell.data {
            CellData::OpaqueBox { val, val_ty } => match *val_ty {
                TY_I8 | TY_I16 | TY_I32 | TY_I64 | TY_U8 | TY_U16 | TY_U32 | TY_U64 | TY_BOOL => {
                    Ok(KeyPayload::Bits { val: unsafe { val.i } as u64, ty: *val_ty })
                }
                TY_STR | TY_BYTES => {
                    // defensive: a `str`/`bytes` payload is a cell handle;
                    // a null there would deref below
                    if unsafe { val.r }.is_null() {
                        return Err(Trap::new(
                            TrapKind::NilDeref,
                            "opaque_key_payload: nil key payload",
                        ));
                    }
                    let inner = cell_of(*val);
                    Ok(match *val_ty {
                        TY_STR => KeyPayload::Str(inner.as_str().to_string()),
                        _ => KeyPayload::Bytes(inner.bytes_copy()),
                    })
                }
                other => Ok(KeyPayload::Unsupported(other)),
            },
            // a host payload box: the payload is the host's own Rust
            // data (RFC 0023) — never a native key
            CellData::HostBoxed { .. } => Ok(KeyPayload::Unsupported(cell.ty)),
            // an `OpaqueRef` always names an opaque cell — defense, not
            // a reachable state
            _ => Err(Trap::new(
                TrapKind::Invalid,
                "opaque_key_payload: not an opaque box",
            )),
        }
    }

    pub(crate) fn export_ret_ty(&self, export: &str) -> Result<TypeId, Trap> {
        let f = self
            .prog
            .export(export)
            .ok_or_else(|| Trap::new(TrapKind::Invalid, format!("no export `{export}`")))?;
        Ok(self.prog.funcs[f as usize].ret)
    }

    fn regs_ty(&self, r: Reg) -> TypeId {
        self.prog.funcs[self.cur_func as usize]
            .regs
            .get(r as usize)
            .copied()
            .unwrap_or(TY_ANY)
    }

    /// A scalar-repr vtable receiver's static type, if it is one
    /// (RFC 0012 §2). A primitive trait-impl target widens as a raw
    /// word — the slot names no cell, so the static register type (the
    /// verifier enforces registers hold their declared types, RFC 0015
    /// §5) is the dispatch key. `None` = a ref-repr register: the cell
    /// handle's own type reaches the vtable (RFC 0015 §6).
    pub(crate) fn scalar_recv_ty(&self, recv: Reg) -> Option<TypeId> {
        let rty = self.regs_ty(recv);
        (rty != TY_ANY && !self.is_ref(rty)).then_some(rty)
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

#[cfg(test)]
mod tests {
    use super::*;
    use rut_core::binary::Program;
    use rut_core::types::{TypeTable, TY_CHAR, TY_F64};

    /// An empty program — no funcs, no consts, just the boot type table.
    fn empty_vm() -> Vm {
        let prog = Program {
            name: "keypayload".into(),
            scope: 0,
            interner: Default::default(),
            surface: Default::default(),
            types: TypeTable::boot(),
            traits: Vec::new(),
            trait_slots: Vec::new(),
            vtables: Vec::new(),
            consts: Vec::new(),
            funcs: Vec::new(),
            exports: Vec::new(),
        };
        Vm::new(
            Rc::new(prog),
            &Limits::default(),
            HostHooks::default(),
            HostRegistry::new(),
        )
        .unwrap()
    }

    /// Box `val` as an `Opaque` of static type `ty` — the `opaque(k)`
    /// shape. The handle takes over the mint reference (the same
    /// ownership `alloc_opaque_str` hands back).
    fn boxed(vm: &Vm, val: Slot, ty: TypeId) -> OpaqueRef {
        let b = vm.heap.alloc_opaque(val, ty).unwrap();
        vm.heap.opaque_handle_take(unsafe { b.r })
    }

    #[test]
    fn integer_and_bool_keys_cross_as_bits() {
        let vm = empty_vm();
        for (ty, v) in [
            (TY_I8, -5i64),
            (TY_I16, -300),
            (TY_I32, -70_000),
            (TY_I64, i64::MIN),
            (TY_U8, 255),
            (TY_U16, 65_535),
            (TY_U32, 4_000_000_000),
            (TY_U64, -1), // u64::MAX as raw bits
            (TY_BOOL, 1), // true — the slot's 0/1 word
        ] {
            let h = boxed(&vm, Slot::int(v), ty);
            match vm.opaque_key_payload(&h).unwrap() {
                KeyPayload::Bits { val, ty: t } => {
                    assert_eq!(t, ty, "type id must survive the read");
                    assert_eq!(val, v as u64, "raw bits for ty {ty}");
                }
                other => panic!("ty {ty}: expected Bits, got {other:?}"),
            }
        }
    }

    #[test]
    fn str_and_bytes_keys_copy_their_octets() {
        let vm = empty_vm();
        let s = vm.heap.alloc_str("hello".into()).unwrap();
        let h = boxed(&vm, s, TY_STR);
        assert_eq!(
            vm.opaque_key_payload(&h).unwrap(),
            KeyPayload::Str("hello".into())
        );

        let b = vm.heap.alloc_bytes(vec![9, 8, 7]).unwrap();
        let h = boxed(&vm, b, TY_BYTES);
        assert_eq!(
            vm.opaque_key_payload(&h).unwrap(),
            KeyPayload::Bytes(vec![9, 8, 7])
        );
    }

    #[test]
    fn unsupported_key_kinds_name_their_type() {
        let vm = empty_vm();
        // floats and chars are rut values but not native keys — the
        // user-defined-key trap's raw material
        let f = boxed(&vm, Slot::float(1.5), TY_F64);
        assert_eq!(vm.opaque_key_payload(&f).unwrap(), KeyPayload::Unsupported(TY_F64));
        let c = boxed(&vm, Slot::ch('x'), TY_CHAR);
        assert_eq!(vm.opaque_key_payload(&c).unwrap(), KeyPayload::Unsupported(TY_CHAR));
    }

    #[test]
    fn a_host_payload_box_is_unsupported() {
        let mut vm = empty_vm();
        // the box's payload is Rust, not a rut value (RFC 0023) — there
        // is no key to read, and the cell's own type (Opaque) names it
        let b = crate::heap::OpaqueBox::alloc(&mut vm, 42i64).unwrap();
        assert_eq!(
            vm.opaque_key_payload(b.handle()).unwrap(),
            KeyPayload::Unsupported(rut_core::types::TY_OPAQUE)
        );
    }

    #[test]
    fn a_nil_str_payload_is_a_trap_not_a_deref() {
        let vm = empty_vm();
        // defensive contract: a null payload under a ref key type traps
        let h = boxed(&vm, Slot::null(), TY_STR);
        let err = vm.opaque_key_payload(&h).unwrap_err();
        assert_eq!(err.kind, TrapKind::NilDeref, "{}", err.msg);
    }

    /// A VM whose program carries ONE dummy func declaring four
    /// registers — `store_result` answers the destination register's
    /// type from the current function's table, so the native path needs
    /// a func to answer from.
    fn vm_with_regs() -> Vm {
        let mut vm = empty_vm();
        let mut prog = (*vm.prog).clone();
        prog.funcs.push(rut_core::binary::FuncCode {
            name: rut_core::sym::MAIN,
            params: vec![],
            ret: TY_I32,
            is_method: false,
            n_captures: 0,
            regs: vec![TY_ANY, TY_BYTES, TY_ANY, TY_ANY],
            argv: vec![],
            labels: vec![],
            code: vec![],
            spans: vec![],
            host_id: None,
        });
        vm.prog = Rc::new(prog);
        vm
    }

    /// Set up `regs` slots and run the bytes-clone native over reg 0.
    /// (The native directly — `call_nat` slices the current function's
    /// argv pool, which this minimal program does not carry.)
    fn clone_reg0_of(vm: &mut Vm, src: Slot) -> Slot {
        vm.cur_regs = vec![Slot::null(); 4];
        vm.cur_regs[0] = src;
        vm.nat_bytes_clone(Some(0), Some(1)).expect("clone native");
        vm.cur_regs[1]
    }

    #[test]
    fn bytes_clone_mints_a_fresh_equal_buffer() {
        let mut vm = vm_with_regs();
        let b = vm.heap.alloc_bytes(vec![7, 8, 9]).unwrap();
        let c = clone_reg0_of(&mut vm, b);
        // equal content, DIFFERENT cell — the copy escape hatch, not an
        // alias (the one `bytes.clone()` law, RFC 0044)
        assert_ne!(unsafe { b.r }, unsafe { c.r }, "the clone must be a fresh cell");
        assert_eq!(cell_of(c).bytes_copy(), vec![7, 8, 9]);
        assert_eq!(cell_of(b).bytes_copy(), vec![7, 8, 9], "the original is untouched");
    }

    #[test]
    fn bytes_clone_isolates_mutations_in_both_directions() {
        // bytes are immutable in the language, so the isolation law is
        // proven engine-side: rewrite the original's storage (the way a
        // host boundary or future surface would) and check the clone
        // does not observe it — nor the reverse.
        let mut vm = vm_with_regs();
        let b = vm.heap.alloc_bytes(vec![1, 2, 3]).unwrap();
        let c = clone_reg0_of(&mut vm, b);

        let CellData::Array { items, .. } = &cell_of(b).data else { panic!("not bytes") };
        items.borrow_mut().set(0, Slot::int(255));
        assert_eq!(
            cell_of(c).bytes_copy(),
            vec![1, 2, 3],
            "the clone must not observe the original's mutation"
        );

        let CellData::Array { items, .. } = &cell_of(c).data else { panic!("not bytes") };
        items.borrow_mut().set(1, Slot::int(254));
        assert_eq!(
            cell_of(b).bytes_copy(),
            vec![255, 2, 3],
            "the original must not observe the clone's mutation"
        );
    }

    #[test]
    fn bytes_clone_traps_on_a_non_bytes_receiver() {
        // defensive: the lowering only emits the native on a `bytes`
        // receiver; a stray cell still traps, never silently copies
        let mut vm = empty_vm();
        let s = vm.heap.alloc_str("nope".into()).unwrap();
        vm.cur_regs = vec![Slot::null(); 4];
        vm.cur_regs[0] = s;
        let err = vm.nat_bytes_clone(Some(0), Some(1)).unwrap_err();
        assert_eq!(err.kind, TrapKind::Invalid, "{}", err.msg);
    }
}
