//! The interpreter loop: one struct, one thread, one heap.
//! Register frames, typed ops, traps as `Err(Trap)` (never Rust panics),
//! fuel ticking per op and the heap charged per allocation. Traps are
//! catchable only at the host boundary.
//!
//! The ACTIVE frame lives directly on the `Vm` (`cur_*`); calls push it
//! onto `frames` and rets pop — keeps register access borrow-friendly.

use crate::arena::OpaqueRef;
use crate::heap::{cell_of, CellData, CellVal, Heap, Slot, Trap, TrapKind, Value};
use rut_core::binary::{ConstVal, Program};
use rut_core::ops::*;
use rut_core::types::{
    PrimTy, Repr, TyKind, TypeId, TY_BOOL, TY_BYTES, TY_I16, TY_I32, TY_I64, TY_I8, TY_STR, TY_U16,
    TY_U32, TY_U64, TY_U8,
};
use std::rc::Rc;
use std::time::Duration;

mod boundary;
pub mod builder;
mod completer;
mod host;
mod native;
mod ops;
mod run;
mod scalar;
mod step;
mod threaded;
mod util;

pub use boundary::{rut_box_payload, CallArg, CallArgs, Ret};
pub use builder::{IntoVmParts, VmBuilder, VmError, VmParts};
pub use completer::{Completer, HostWakes, HostWakesHandle, FAILED, PENDING, READY};
use host::HostSlot;
pub use host::{ExpectedHostFns, HostPkg, HostPkgContext, HostRegistry};

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
        ConstVal::Str(s) => heap
            .alloc_str(s.clone())
            .map_err(|t| format!("load: {}", t.msg))?,
        ConstVal::TypeId(t) => Slot::int(*t as i64),
    })
}

#[derive(Clone, Debug)]
pub struct Limits {
    pub fuel: Option<u64>,
    pub heap_limit_bytes: Option<u64>,
    pub interrupt_every: u32,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            fuel: None,
            heap_limit_bytes: None,
            interrupt_every: 1024,
        }
    }
}

/// The payload of an `Opaque` box classified against the closed
/// native-key set (the nmap host experiment): the integer primitives and
/// `bool` cross as raw bits, `str`/`bytes` cross as owned copies (short;
/// once per call), and everything else — floats, chars, user records,
/// host payload boxes — comes back [`KeyPayload::Unsupported`] with the
/// type id naming it, so the caller's trap can point at the escape
/// hatches (encode the key canonically to `bytes`, or use `nmapset`)
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
/// (a host fn may call back into rut). The outer frame's
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
    /// the dispatch table, dense by func idx — the `Vm::new` join
    /// built it from the registry; the hot path copies one
    /// `HostSlot` (a code pointer, a state word, the declared return)
    /// and dispatches. No Rc, no name work, no allocation.
    host_slots: Vec<HostSlot>,
    /// the boxed host bodies — `host_slots[i].ctx` points into these;
    /// owning them keeps every ctx word valid for the machine's lifetime
    /// (single thread)
    /// (dead_code is the point: nothing reads it — owning the boxes
    /// keeps every `host_slots[i].ctx` word valid for the machine's
    /// lifetime)
    #[allow(dead_code)]
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
    // ---- the async driving loop ----
    /// launched/parked frames to (re)drive — plain cell handles, each
    /// queue entry owning one reference (retained on enqueue, released
    /// on pop)
    ready: std::collections::VecDeque<Slot>,
    /// sleep deadlines (the VM's virtual clock) → the frames they wake
    timers: std::collections::BTreeMap<u64, Vec<Slot>>,
    /// the virtual clock (deterministic: hosts advance it explicitly)
    now_ms: u64,
    /// every `Future<T>` instantiation's `yield` slot — `drive` tries
    /// them in order off the receiver's vtable row
    future_yield_slots: Vec<u32>,
    /// the host future lane: the minted `#hframe@` frame types (the
    /// boot scan finds them by their reserved prefix). A frame of one
    /// of these types completes through the poll set, not a compiled
    /// body.
    host_frames: Vec<u32>,
    /// host futures parked on their Completers — the driving loop's
    /// poll set. Each entry owns one reference (as `ready` does); the
    /// poll drives each once per spin, releases on completion, and
    /// keeps pending entries for the next spin.
    host_pending: Vec<Slot>,
    /// the machine's ONE wake cell — the registry's Arc, carried over
    /// by the boot join. Every host future's settle bumps it
    /// (`Completer::complete`/`fail` → `HostWakes::bump`), so a
    /// driving loop parked on it wakes exactly when a settle lands.
    /// One cell per machine, shared by every host future in it.
    host_wakes: HostWakesHandle,
    /// the wake-cell epoch the CURRENT driving turn's poll started at:
    /// `run_until_idle` snapshots it before each `run_ready`, and
    /// `wait_hosts` parks for a settle SINCE it. A settle that lands
    /// between the poll and the park moves the cell past this epoch,
    /// so the park answers at once and the loop re-polls (the
    /// race-free re-check — a park against the post-poll epoch could
    /// hang on work that already landed). Single-threaded machine:
    /// a `Cell`, no sync.
    host_seen: std::cell::Cell<u64>,
    /// the engine-minted cx record type (found by name; None when the
    /// program never mentions async)
    cx_ty: Option<TypeId>,
}

/// Const-generic op codes for the scalar op bodies: the opcode is
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
    /// The construction door — `Vm::builder().program(..)/.compiled(..)..build()`.
    /// `Vm::new` is crate-internal; every embedder boots through the
    /// builder ([`builder`](super::builder)).
    pub fn builder() -> crate::interp::VmBuilder {
        crate::interp::VmBuilder::default()
    }

    /// Boot the machine. `registry` is the embedder's host-fn binding
    /// table — built BEFORE the Vm (an embedder depends
    /// on nothing else) and consumed here: every host thunk the program
    /// declares is resolved against it, so a declared-but-unbound fn is
    /// a construction error, never a mid-run trap. Call
    /// `HostRegistry::verify_against` first for the full
    /// contract (it also checks the bound-but-undeclared direction).
    ///
    /// Crate-internal — the only construction door is
    /// [`Vm::builder`](super::builder::VmBuilder).
    pub(crate) fn new(
        prog: Rc<Program>,
        limits: &Limits,
        mut registry: HostRegistry,
    ) -> Result<Vm, Trap> {
        // the machine's ONE wake cell — the registry mints it, the
        // builder carries it here (the identity law: the registry's
        // Arc and the Vm's are the same object)
        let host_wakes = registry.wakes();
        // ---- the host join ----
        // Resolve every host thunk's binding NOW; slots are dense by
        // func idx, one row each: the adapter code pointer, the boxed
        // body's ctx word, the declared return for the refcount law,
        // and the return's is-ref verdict frozen as a bit (phase 2 —
        // the write-back used to re-derive it per call).
        let type_repr: Vec<Repr> = prog
            .types
            .types
            .iter()
            .map(|t| match &t.kind {
                TyKind::Prim(p) => Repr::Prim(*p),
                TyKind::Nil | TyKind::Fn { .. } => Repr::Any,
                _ => Repr::Ref,
            })
            .collect();
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
                                 register the body in the HostRegistry before Vm::new"
                            ),
                        )
                    })?;
                    if let Some(k) = b.keep {
                        host_keep.push(k);
                    }
                    // the exact `Vm::is_ref` predicate, evaluated once
                    // here; a degenerate program (ret id missing from
                    // the table — the unbound-thunk check below is the
                    // error that matters) reads Any, never a panic
                    let ret_is_ref = fc.ret != TY_ANY
                        && type_repr
                            .get(fc.ret as usize)
                            .copied()
                            .unwrap_or(Repr::Any)
                            .is_ref();
                    host_slots.push(HostSlot {
                        code: b.code,
                        ctx: b.ctx,
                        ret_is_ref,
                    });
                }
                None => host_slots.push(HostSlot::NEVER),
            }
        }
        let heap = Heap::new(
            limits.heap_limit_bytes,
            crate::arena::ReleasePlan::build(&prog.types, &prog.funcs),
            prog.disposal_impls.clone(),
        );
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
                // `?T`: a one-slot nullable box retains its payload
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
        // precompute the threaded dispatch tags (one per op)
        let op_tags: Vec<Vec<u8>> = prog
            .funcs
            .iter()
            .map(|f| f.code.iter().map(rut_vm_threaded::tag_of).collect())
            .collect();
        let thread_table = rut_vm_threaded::build_table::<Vm>();
        // the async driving loop's boot scan: every trait
        // named `Future<..>` contributes its `yield` slot, and the cx
        // record is the `RunContext`-named Data type — both minted by
        // the compiler; programs without async scan to empty
        let mut future_yield_slots: Vec<u32> = Vec::new();
        let mut cx_ty: Option<TypeId> = None;
        let cx_name = prog
            .interner
            .lookup(rut_core::async_frame::RUN_CONTEXT_TYPE);
        for (tid, t) in prog.ifaces.iter().enumerate() {
            if prog.interner.name(t.name).starts_with("Future<") {
                if let Some(slot) = prog
                    .iface_slots
                    .iter()
                    .position(|&(tr, m)| tr == tid as u32 && m == 0)
                {
                    future_yield_slots.push(slot as u32);
                }
            }
        }
        if let Some(cid) = cx_name {
            cx_ty = prog
                .types
                .types
                .iter()
                .enumerate()
                .find(|(_, t)| t.name == cid && matches!(t.kind, TyKind::Data { .. }))
                .map(|(i, _)| i as TypeId);
        }
        // the host future lane (phase 4): the minted `#hframe@` frame
        // types — reserved prefix, so the name IS the marker (no
        // program-side wire field; VERSION untouched)
        let host_frames: Vec<u32> = prog
            .types
            .types
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                prog.interner
                    .name(t.name)
                    .starts_with(rut_core::async_frame::HOST_FRAME_PREFIX)
            })
            .map(|(i, _)| i as u32)
            .collect();
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
            ready: std::collections::VecDeque::new(),
            timers: std::collections::BTreeMap::new(),
            now_ms: 0,
            future_yield_slots,
            host_frames,
            host_pending: Vec::new(),
            host_wakes,
            host_seen: std::cell::Cell::new(0),
            cx_ty,
        })
    }

    /// The machine's ONE wake cell (a clone of the Arc the boot join
    /// carried over from the registry) — the sugars mint every host
    /// future's [`Completer`](crate::Completer) against it, and the
    /// driving loop's park lane (`wait_hosts`, ridden by
    /// `run_until_idle`) waits on it. Every settle bumps the
    /// generation; a waiter that parked before it wakes.
    pub fn host_wakes(&self) -> HostWakesHandle {
        self.host_wakes.clone()
    }

    pub fn heap_usage(&self) -> u64 {
        self.heap.used_bytes()
    }

    /// The host's mint of a RUT seal box over a payload slot —
    /// [`Heap::alloc_opaque`] through the crate wall (the `heap` field is
    /// crate-private): nmap's iteration lane mints one fresh key box per
    /// yield. `ty` is the payload's OWN type id — the box's `val_ty`, the
    /// exact check `opaque::downcast<T>` runs — and the payload slot's
    /// ownership moves into the box (the entry's release walks it).
    pub fn alloc_opaque_box(&mut self, payload: Slot, ty: TypeId) -> Result<Slot, Trap> {
        self.heap.alloc_opaque(payload, ty)
    }

    /// The in-crossing rc: a host body storing a rut
    /// value takes its own reference (`retain` — the "retain on store"
    /// law) and releases it when the value leaves the store (`release`
    /// — "release on replace/remove"; §0.8 i's release-context
    /// law: rc work happens with the `Vm` at hand, inside the crossing).
    pub fn retain(&self, s: Slot) {
        self.heap.retain(s);
    }
    /// The in-crossing release — see [`Vm::retain`].
    pub fn release(&self, s: Slot) {
        self.heap.release(s);
    }

    /// VM-heap high-water mark — the peak live
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

    fn enter(&mut self, func: u32, regs: Vec<Slot>, ret_dst: Option<Reg>) {
        // save current frame if there is one
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
    ///. Traps unwind here. Re-entrant: a host
    /// fn holding `&mut Vm` may call back into rut — the
    /// nested call runs on a fresh frame stack under the same budget, and
    /// the outer cursor is restored whether the callee returns or traps
    /// (a propagated nested trap keeps the outer frame resumable; resuming
    /// re-runs the host fn from its op, the standing host-fn-trap rule).
    pub(crate) fn call_raw(&mut self, export: &str, args: &[Value]) -> Result<Value, Trap> {
        let Some(func) = self.prog.export(export) else {
            return Err(Trap::new(
                TrapKind::Invalid,
                format!("no export `{export}`"),
            ));
        };
        let nparams = self.prog.funcs[func as usize].params.len();
        if args.len() != nparams {
            return Err(Trap::new(
                TrapKind::Invalid,
                format!("`{export}` takes {nparams} params, {} given", args.len()),
            ));
        }
        let nregs = self.prog.funcs[func as usize].regs.len();
        let mut regs = self.take_regs(nregs);
        for (i, a) in args.iter().enumerate() {
            let pty = self.param_ty(func, i);
            regs[i] = self.value_in(a, pty).map_err(|m| {
                Trap::new(TrapKind::Invalid, format!("`{export}` argument {i}: {m}"))
            })?;
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
            // queued `dispose` calls run before the cursor restore —
            // the machine is idle here
            let r = self.drain_disposal(r);
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
            self.drain_disposal(r)
        }
    }

    /// Run queued `dispose` calls after a successful root call: each
    /// call receives the pinned cell as `self` and a freshly minted
    /// `DisposalContext`; the pin's release afterwards is what finally
    /// frees the cell. A dispose body may itself drop pointers — the
    /// loop drains until the queue stays empty. A trap from the main
    /// call skips the drain (unwinding semantics are OQ).
    fn drain_disposal(&mut self, r: Result<Value, Trap>) -> Result<Value, Trap> {
        let v = r?;
        while let Some((obj, fid)) = self.heap.take_pending_dispose() {
            let done = match self.heap.alloc_disposal_context() {
                Ok(cx) => {
                    let d = self.run_dispose(fid, obj, cx);
                    self.heap.release(cx);
                    d
                }
                Err(e) => Err(e),
            };
            self.heap.release(obj);
            done?;
        }
        Ok(v)
    }

    /// Call one `dispose` with the dying cell as `self` and the minted
    /// cx as its second argument — the same frame machine, run to
    /// completion. The call ABI's rc law: each argument the signature
    /// takes by reference is retained here and released by the callee's
    /// root ret (a trap parks the frame holding its own refs).
    fn run_dispose(&mut self, fid: u32, obj: Slot, cx: Slot) -> Result<(), Trap> {
        let nregs = self.prog.funcs[fid as usize].regs.len();
        let mut regs = self.take_regs(nregs);
        regs[0] = obj;
        regs[1] = cx;
        for (i, &s) in [obj, cx].iter().enumerate() {
            if self.is_ref(self.param_ty(fid, i)) {
                self.heap.retain(s);
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
        let ty = if ty != u32::MAX {
            ty
        } else {
            return Err("internal: untyped parameter".to_string());
        };
        Ok(match (v, self.prog.types.kind(ty).clone()) {
            (Value::Nil, TyKind::Nil)
            | (Value::I64(0), TyKind::Nil) // `return;` crosses as a zero word
            | (Value::Nil, TyKind::Prim(_)) => Slot::int(0),
            // nil binds a `?T` parameter as the null slot (the flat
            // representation; the zero — there is no `Value::Opt`,
            // so nil is the one shape the flat arm needs).
            (Value::Nil, TyKind::Opt { .. }) => Slot::null(),
            // a non-nil payload re-enters BOXED under `?T` (the host-side
            // `T → ?T` funnel): `call` decodes the root ret
            // through `slot_to_value` — which nil-flattens — and then
            // re-marshals that `Value` under the declared type, so a
            // `?T` component arrives unboxed and `alloc_opt_value` makes
            // the slot honest again before `Ret` reads it.
            (v, TyKind::Opt { elem }) => {
                let inner = self.value_in(v, elem).map_err(|m| format!("payload: {m}"))?;
                self.heap.alloc_opt_value(ty, inner).map_err(|t| t.msg)?
            }
            (Value::I64(n), TyKind::Prim(p)) => {
                if !fits(*n, p) {
                    return Err(format!("`{n}` does not fit `{}`", self.prog.type_name(ty)));
                }
                Slot::int(*n)
            }
            (Value::F64(f), TyKind::Prim(PrimTy::F32)) => Slot::float(*f as f32 as f64),
            (Value::F64(f), TyKind::Prim(PrimTy::F64)) => Slot::float(*f),
            (Value::Bool(b), TyKind::Prim(PrimTy::Bool)) => Slot::bool(*b),
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
                // the handle's word IS a store slot (the tagged entry
                // pointer, nmap-hostvals P2) — the retain routes to the
                // entry's rc, the parameter register owns its reference
                let s = Slot { r: h.ptr() };
                debug_assert!(
                    crate::heap::store::is_entry(s),
                    "Value::Opaque carrying a non-store word"
                );
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
    pub(super) fn call_host(
        &mut self,
        func: u32,
        argv_off: u32,
        argc: u16,
        dst: Reg,
    ) -> Result<(), Trap> {
        // the join resolved the binding; the entry is a code pointer, a
        // word, a TypeId, and the ret-is-ref bit — copied out in one
        // move, no Rc traffic
        let slot = self.host_slots[func as usize];
        // the operand span, sliced from the program's argv table. The
        // old shape paid an `Rc::clone` (+ drop) per call purely to
        // split borrows: the slice had to outlive the `&mut self` the
        // snapshot below takes. The raw pointer carries it instead.
        // SAFETY: `self.prog` is assigned once, in `Vm::new`, and never
        // reassigned — the `Rc` holds its pointee for the Vm's whole
        // life, so the slice stays valid through the end of this call;
        // only immutable argv-table words are read through it (the
        // program is frozen after construction — the same trust the
        // threaded loop's raw pointers run on).
        let args: &[Reg] = unsafe {
            let prog = &*Rc::as_ptr(&self.prog);
            let f = self.cur_func as usize;
            &prog.funcs[f].argv[argv_off as usize..argv_off as usize + argc as usize]
        };
        // snapshot the args: the register file may swap inside the body
        // (nested calls). `Slot` is Copy (one machine word).
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
        // transfer, not borrow (crossing-ownership law).
        // The is-ref verdict is the join's bit (phase 2) — no
        // `type_repr` lookup on the hot path.
        if let Some(d) = reg_opt(dst) {
            if slot.ret_is_ref {
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

    /// Resume after a budget trap (the frame IS the loop state).
    pub(crate) fn resume_raw(&mut self) -> Result<Value, Trap> {
        if !self.running {
            return Err(Trap::new(TrapKind::Invalid, "nothing to resume"));
        }
        self.run_loop()
    }

    /// The typed host boundary: call an export with
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
    /// named logger). The handle owns one reference.
    pub fn alloc_opaque_str(&mut self, s: String) -> Result<OpaqueRef, Trap> {
        let slot = self.heap.alloc_str(s)?;
        let p = self.heap.alloc_opaque(slot, rut_core::types::TY_STR)?;
        Ok(self.heap.opaque_handle_take(unsafe { p.r }))
    }

    /// The growth-charge hook for host-owned state: [`Heap::charge_public`]
    /// flowed through the VM because the heap is crate-private. A host
    /// package that grows its own buffer (the strbuild builder) consults
    /// this BEFORE growing, so the wasm heap budget governs host growth
    /// exactly as it governs engine cells.
    pub fn charge_public(&mut self, bytes: u64) -> Result<(), Trap> {
        self.heap.charge_public(bytes)
    }

    /// The key payload inside an `Opaque` box, classified against the
    /// closed native-key set (the nmap host experiment). Host code cannot
    /// read a rut-side box itself — `Slot` is crate-private — so this is
    /// the one pub reader: a Rut entry's payload classifies by its
    /// `val_ty` (ints/bool as raw bits, `str`/`bytes` as owned copies), a
    /// Host payload entry has no rut value inside and is
    /// `Unsupported`, and any other `val_ty` — a user-defined key — is
    /// `Unsupported` naming the type.
    pub fn opaque_key_payload(&self, h: &OpaqueRef) -> Result<KeyPayload, Trap> {
        let entry = unsafe { &*h.entry_ptr() };
        match &entry.e {
            crate::heap::store::OpaqueEntry::Rut(r) => match r.val_ty {
                TY_I8 | TY_I16 | TY_I32 | TY_I64 | TY_U8 | TY_U16 | TY_U32 | TY_U64 | TY_BOOL => {
                    Ok(KeyPayload::Bits {
                        val: unsafe { r.slot.i } as u64,
                        ty: r.val_ty,
                    })
                }
                TY_STR | TY_BYTES => {
                    // defensive: a `str`/`bytes` payload is a cell handle;
                    // a null there would deref below
                    if unsafe { r.slot.r }.is_null() {
                        return Err(Trap::new(
                            TrapKind::NilDeref,
                            "opaque_key_payload: nil key payload",
                        ));
                    }
                    let inner = cell_of(r.slot);
                    Ok(match r.val_ty {
                        TY_STR => KeyPayload::Str(inner.as_str().to_string()),
                        _ => KeyPayload::Bytes(inner.bytes_copy()),
                    })
                }
                other => Ok(KeyPayload::Unsupported(other)),
            },
            // a host payload entry: the payload is the host's own Rust
            // data — never a native key
            crate::heap::store::OpaqueEntry::Host(_) => {
                Ok(KeyPayload::Unsupported(rut_core::types::TY_OPAQUE))
            }
        }
    }

    pub(crate) fn export_ret_ty(&self, export: &str) -> Result<TypeId, Trap> {
        let f = self
            .prog
            .export(export)
            .ok_or_else(|| Trap::new(TrapKind::Invalid, format!("no export `{export}`")))?;
        Ok(self.prog.funcs[f as usize].ret)
    }

    /// Direct dispatch of a REGISTERED host row by its registration
    /// name (`call_raw` is the export twin; the compiler-minted row
    /// thunks — the host future lane's `__start`/`__yield`/`__take`/
    /// `__cancel` — are bodyless and carry no export entry). The args
    /// convert through the row's declared params (`value_in`), the
    /// body runs on the VM thread, and the answer decodes positionally
    /// (`slot_to_value`). A test/tooling read, like `first_ready`.
    pub fn call_host_row(&mut self, row: &str, args: &[Value]) -> Result<Value, Trap> {
        let func = self
            .prog
            .funcs
            .iter()
            .position(|f| {
                if f.host_id.is_none() {
                    return false;
                }
                // minted thunks carry the registration name as their
                // FuncCode name; decl rows carry it as the host_id (the
                // FuncCode name stays the bare decl name)
                self.prog.interner.name(f.name) == row
                    || f.host_id.is_some_and(|h| self.prog.interner.name(h) == row)
            })
            .ok_or_else(|| Trap::new(TrapKind::Invalid, format!("no host row `{row}`")))?
            as u32;
        let (params, ret) = {
            let f = &self.prog.funcs[func as usize];
            (f.params.clone(), f.ret)
        };
        if args.len() != params.len() {
            return Err(Trap::new(
                TrapKind::Invalid,
                format!(
                    "`{row}` takes {} params, {} given",
                    params.len(),
                    args.len()
                ),
            ));
        }
        let mut snapshot: Vec<Slot> = Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            snapshot.push(
                self.value_in(a, params[i]).map_err(|m| {
                    Trap::new(TrapKind::Invalid, format!("`{row}` argument {i}: {m}"))
                })?,
            );
        }
        let slot = self.host_slots[func as usize];
        let out = (slot.code)(self, &snapshot, slot.ctx);
        if let Some(t) = self.host_trap.take() {
            return Err(t);
        }
        // the answer decodes positionally; the out slot's reference is
        // ours (the crossing-ownership law) — released after the decode
        let v = util::slot_to_value(out, ret, &self.prog, &self.heap);
        if slot.ret_is_ref {
            self.heap.release(out);
        }
        Ok(v)
    }

    fn regs_ty(&self, r: Reg) -> TypeId {
        self.prog.funcs[self.cur_func as usize]
            .regs
            .get(r as usize)
            .copied()
            .unwrap_or(TY_ANY)
    }

    /// A scalar-repr vtable receiver's static type, if it is one
    ///. A primitive trait-impl target widens as a raw
    /// word — the slot names no cell, so the static register type (the
    /// verifier enforces registers hold their declared types) is the dispatch key. `None` = a ref-repr register: the cell
    /// handle's own type reaches the vtable.
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

/// The outcome of one drive step: the driven frame either
/// ran to completion (its state field retired to null) or suspended at
/// a checkpoint (a non-null checkpoint singleton).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drive {
    Done,
    Parked,
}

/// The driving protocol's parking policy ([`Vm::run_until_idle`]):
/// whether the driving thread may park on the machine's wake cell
/// while it waits for host work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    /// Park on [`HostWakes`](crate::HostWakes) whenever nothing but
    /// settling host futures remains — the native embedder's policy
    /// (the workers settle on other threads; the parked wake is the
    /// protocol's whole point). A host that never settles hangs the
    /// run by law — no wall-clock budget, no hang guard; best-effort
    /// cancellation is the disclosed contract.
    Block,
    /// Never park: answer [`Idle::WouldBlock`] as soon as nothing but
    /// host-parked work remains — the wasm lane's policy (one thread;
    /// a JS host settles inside its own call, so a parked thread could
    /// never wake).
    NonBlock,
}

/// [`Vm::run_until_idle`'s] answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Idle {
    /// Every frame retired — the ready queue, the timers, and the
    /// host poll set are all empty.
    Drained,
    /// Host-parked work remains and the [`Wait::NonBlock`] policy
    /// forbids parking — the embedder yields and re-enters.
    WouldBlock,
}

impl Vm {
    // ---- the engine-half record API ----
    // The host set's bodies receive `&mut Vm` — launch,
    // abort, and sleep live behind that crossing, so the frame-cell
    // surgery they perform goes through these focused helpers.

    /// Allocate a zeroed record of an in-table type; the slot owns one
    /// reference. The sleep future's mint is the intended caller.
    pub fn alloc_zeroed_record(&self, ty: TypeId, n: usize) -> Result<Slot, Trap> {
        self.heap.alloc_record_zeroed(ty, n)
    }

    /// Box `raw` as the `?opt_ty` one-slot cell; the slot owns one
    /// reference (the payload's own is the caller's to mind — the
    /// select composite's `?T` answer fields are the intended caller,
    /// and they retain through `set_record_field`).
    pub fn alloc_opt_value(&self, opt_ty: TypeId, raw: Slot) -> Result<Slot, Trap> {
        self.heap.alloc_opt_value(opt_ty, raw)
    }

    /// The checkpoint enum's immortal member singleton (the engine
    /// half's state writes answer with the minted cell, per the survey).
    pub fn enum_member_slot(&self, ty: TypeId, member: u32) -> Result<Slot, Trap> {
        self.heap.enum_member(ty, member)
    }

    /// Read a record field; `None` on a non-record or out of range.
    pub fn record_field(&self, cell: Slot, i: usize) -> Option<Slot> {
        if unsafe { cell.r.is_null() } {
            return None;
        }
        match &cell_of(cell).data {
            CellData::Record { fields } => fields.borrow().get(i),
            _ => None,
        }
    }

    /// Write a record field with the full rc law (retain the new value,
    /// release the old when the field is ref-repr). No-op on a
    /// non-record or out of range.
    pub fn set_record_field(&self, cell: Slot, i: usize, v: Slot) {
        if unsafe { cell.r.is_null() } {
            return;
        }
        let cell = cell_of(cell);
        let CellData::Record { fields } = &cell.data else {
            return;
        };
        let old = match fields.borrow_mut().set(i, v) {
            Some(o) => o,
            None => return,
        };
        let is_ref = match self.prog.types.kind(cell.ty) {
            TyKind::Data { fields } => fields
                .get(i)
                .map(|f| self.prog.types.repr_of(f.ty).is_ref())
                .unwrap_or(false),
            _ => false,
        };
        if is_ref {
            self.heap.retain(v);
            self.heap.release(old);
        }
    }

    /// The rut-value box's payload — the erasure box's
    /// engine-half recovery for host bodies (the async lane's `opaque`
    /// crossings): the held slot and its runtime type. `None` when the
    /// slot is not a box holding a rut value — a host-payload box
    /// recovers through `Opaque<T>`, a plain cell is not a box. The
    /// slot is a BORROW of the entry's held value (the entry owns its
    /// reference; a body that keeps it takes its own retain — the
    /// store's law).
    pub fn opaque_rut_value(&self, box_slot: Slot) -> Option<(Slot, TypeId)> {
        let e = crate::heap::store::store_entry(box_slot)?;
        match &e.e {
            crate::heap::store::OpaqueEntry::Rut(r) => Some((r.slot, r.val_ty)),
            crate::heap::store::OpaqueEntry::Host(_) => None,
        }
    }

    /// Seal a freshly minted rut value into the erasure box
    /// (the async lane's `__sleep` mint): the entry OWNS `val`'s
    /// reference, and the returned owning handle carries the box's own
    /// reference straight to the host-fn return path
    /// (`Ret::into_slot` transfers it — the mint→crossing ownership
    /// law, `opaque_handle_take`). `val_ty` is what `downcast<T>`
    /// compares against on the rut side.
    pub fn seal_opaque(&self, val: Slot, val_ty: TypeId) -> Result<crate::heap::OpaqueRef, Trap> {
        let boxed = self.heap.alloc_opaque(val, val_ty)?;
        Ok(self.heap.opaque_handle_take(unsafe { boxed.r }))
    }

    // ---- the async driving loop (the host loop's engine
    // half — the run_until_idle / next_deadline columns;
    // the model: `docs/src/reference/async.md`) ----

    /// The virtual clock (deterministic tests: hosts advance it; the
    /// sleep future's yield arms deadlines against it).
    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }
    /// Advance/reset the virtual clock. `next_deadline` answers the
    /// earliest sleep deadline; the host sets the clock to it (or
    /// sleeps wall-time and sets the wall value).
    pub fn set_now(&mut self, t_ms: u64) {
        self.now_ms = t_ms;
    }

    /// `launch_future`'s engine half: the queue owns one reference from
    /// here on (retained; released when the entry is driven).
    pub fn launch(&mut self, fut: Slot) {
        self.heap.retain(fut);
        self.ready.push_back(fut);
    }

    /// `LaunchedFutureHandle::abort`'s engine half: flag the task's
    /// cancellation (the cx probe reads it), re-enqueue so the probe at
    /// its checkpoint fires the drop path. `false` when already retired
    /// (state null — completed or dropped).
    pub fn cancel(&mut self, task: Slot) -> bool {
        if unsafe { task.r.is_null() } {
            return false;
        }
        let cell = cell_of(task);
        let CellData::Record { fields } = &cell.data else {
            return false;
        };
        let finished = fields
            .borrow()
            .get(rut_core::async_frame::STATE_FIELD as usize)
            .map(|s| unsafe { s.r.is_null() })
            .unwrap_or(true);
        if finished {
            return false;
        }
        fields.borrow_mut().set(
            rut_core::async_frame::CANCELLED_FIELD as usize,
            Slot::bool(true),
        );
        self.launch(task);
        true
    }

    /// The sleep future's yield arms its deadline here (engine-assisted
    /// mint; the timer map holds the reference until it fires).
    pub fn arm_timer(&mut self, deadline_ms: u64, fut: Slot) {
        self.heap.retain(fut);
        self.timers.entry(deadline_ms).or_default().push(fut);
    }

    /// Expire due timers into the ready queue; answer the earliest
    /// remaining deadline (the host loop's next wake). `None` when no
    /// timer is armed.
    pub fn next_deadline(&mut self) -> Option<u64> {
        loop {
            let (&d, _) = self.timers.iter().next()?;
            if d <= self.now_ms {
                if let Some(waiting) = self.timers.remove(&d) {
                    // the timer entry's references transfer to the queue
                    for s in waiting {
                        self.ready.push_back(s);
                    }
                }
            } else {
                return Some(d);
            }
        }
    }

    /// Frames not yet retired: the ready queue, the armed timers, and
    /// the host futures parked on their Completers — the host loop's
    /// idle test. A worker that never completes keeps this above zero
    /// (the disclosed best-effort cancellation law: the thread runs to
    /// its blocking completion).
    pub fn pending_tasks(&self) -> usize {
        self.ready.len()
            + self.timers.values().map(|v| v.len()).sum::<usize>()
            + self.host_pending.len()
    }

    /// The ready queue's front slot, without driving it — a test/
    /// tooling read for receipts held engine-side.
    pub fn first_ready(&self) -> Slot {
        *self.ready.front().unwrap()
    }

    /// Drain the ready queue, then poll the host futures, then drain
    /// again — until a spin produces no completion (the poll wakes
    /// awaiters into the ready queue; one pass settles a completed
    /// host future end to end). Every entry's reference is released
    /// exactly once, here or in the poll set's own walk.
    pub fn run_ready(&mut self) -> Result<usize, Trap> {
        let mut n = self.drain_ready()?;
        while self.poll_hosts()? {
            n += self.drain_ready()?;
        }
        Ok(n)
    }

    /// Park until a host future settles (or `timeout` fires) — answers
    /// whether the wake fired. Only from the driving thread, and only
    /// while the machine is quiescent (between polls — `run_until_idle`
    /// is the only caller): the workers touch only the Completers'
    /// atomics and this cell, so a parked driver cannot race the heap.
    ///
    /// The park's epoch is the snapshot the driving turn took BEFORE
    /// its poll (`host_seen`), never the generation at park time: a
    /// settle that landed between the poll and this park already moved
    /// the cell past it, so the door answers `true` at once and the
    /// caller re-polls — the immediate-return shape (parking on the
    /// post-poll epoch could hang on work that already landed).
    /// Spurious wakeups are fine — the caller re-polls regardless.
    pub fn wait_hosts(&self, timeout: Option<Duration>) -> bool {
        self.host_wakes
            .wait_since_for(self.host_seen.get(), timeout)
    }

    /// The driving protocol, engine-owned — the one loop a host loop
    /// needs (`docs/src/reference/async.md`'s model): drain the ready
    /// queue and poll the host futures to quiescence
    /// ([`Vm::run_ready`]), then, per turn —
    ///
    /// - a pending deadline: under [`Wait::Block`], park on the wake
    ///   cell bounded by it when host work shares the wait
    ///   (`wait_hosts(Some(d - now))` — the settle or the deadline,
    ///   whichever fires first); either way, jump the virtual clock to
    ///   the deadline (`set_now` — the deterministic fast-forward;
    ///   `next_deadline` expires the due timers as a side effect).
    ///   `Wait::NonBlock` skips the park (it never parks — the wasm
    ///   lane: a JS host can only settle OUTSIDE this call, so a park
    ///   there is a pure stall) and lets the clock lane run;
    /// - no deadline, [`Wait::Block`]: park until a host future
    ///   settles — forever, by law (no wall-clock budget, no hang
    ///   guard; a worker that never completes hangs the run, the
    ///   disclosed best-effort-cancellation contract);
    /// - no deadline, [`Wait::NonBlock`]: one probe drain
    ///   ([`Vm::run_ready`]) — the expiry above may have JUST moved
    ///   fired timers into the ready queue, so a driven round loops
    ///   and a quiet round answers [`Idle::WouldBlock`] (only
    ///   host-parked work remains and the policy forbids parking —
    ///   the verdict's honest shape; the embedder yields and
    ///   re-enters).
    ///
    /// until nothing is pending ([`Idle::Drained`]). The first trap
    /// propagates (`Err`) — other frames may stay pending; the
    /// embedder exits.
    pub fn run_until_idle(&mut self, wait: Wait) -> Result<Idle, Trap> {
        loop {
            // the turn's epoch: every settle from here on — during the
            // poll or in the park lane — moves the cell past it, so
            // the park below can never miss one (the pre-poll
            // snapshot is the race-free re-check's whole trick)
            self.host_seen.set(self.host_wakes.generation());
            self.run_ready()?;
            if self.pending_tasks() == 0 {
                return Ok(Idle::Drained);
            }
            match self.next_deadline() {
                Some(d) => {
                    // either-first, Block only: the park is bounded by
                    // the deadline (the condvar wakes on a settle; the
                    // timeout path just runs the clock out). The
                    // host-parked read is internal — the poll set's
                    // size is the loop's business, not a public door.
                    if wait == Wait::Block && !self.host_pending.is_empty() {
                        self.wait_hosts(Some(Duration::from_millis(d.saturating_sub(self.now_ms))));
                    }
                    self.set_now(d);
                }
                None => match wait {
                    Wait::Block => {
                        self.wait_hosts(None);
                    }
                    Wait::NonBlock => {
                        // the expiry may have JUST moved fired timers
                        // into the ready queue (next_deadline's side
                        // effect) — one probe round answers which: a
                        // driven round loops (the clock lane
                        // continues), a quiet round means only host
                        // parks remain (the WouldBlock verdict's
                        // honest shape — the consumer's "one probe
                        // round", folded into the door)
                        if self.run_ready()? == 0 {
                            return Ok(Idle::WouldBlock);
                        }
                    }
                },
            }
        }
    }

    /// One drive step of a future: a re-entrant call of its `yield`
    /// (the InterpCursor stash/restore — the nested-entry
    /// machinery), with a freshly minted cx over the frame edge. The
    /// answer reads off the state field: null → [`Drive::Done`] (and a
    /// registered awaiter, if any, re-enqueues), else [`Drive::Parked`].
    pub fn drive(&mut self, fut: Slot) -> Result<Drive, Trap> {
        if unsafe { fut.r.is_null() } {
            return Err(Trap::new(TrapKind::NilDeref, "drive on nil"));
        }
        let ty = cell_of(fut).ty;
        let fid = self
            .future_yield_slots
            .iter()
            .find_map(|&slot| {
                self.prog
                    .vtables
                    .get(ty as usize)
                    .and_then(|v| v.get(slot as usize))
                    .copied()
                    .flatten()
            })
            .ok_or_else(|| {
                Trap::new(
                    TrapKind::Invalid,
                    format!(
                        "drive: {} is not a future — no `Future::yield` row in its vtable",
                        self.prog.type_name(ty)
                    ),
                )
            })?;
        let cx_ty = self.cx_ty.ok_or_else(|| {
            Trap::new(
                TrapKind::Invalid,
                "drive: the program has no `RunContext` cx record",
            )
        })?;
        // the per-drive cx: one field, the frame edge (retained)
        let cx = self.heap.alloc_record_zeroed(cx_ty, 1)?;
        if let CellData::Record { fields } = &cell_of(cx).data {
            self.heap.retain(fut);
            fields.borrow_mut().set(0, fut);
        }
        let r = if self.prog.funcs[fid as usize].host_id.is_some() {
            // the engine-backed thunk: a bodyless FuncCode
            // whose host body implements `Future::yield` — the sleep
            // future's timer dance. Run the crossing directly on the
            // (frame, cx) pair: the crossing borrows both (the
            // op_call rc law — retain per ref param, release after).
            let host = self.host_slots[fid as usize];
            let params = self.prog.funcs[fid as usize].params.clone();
            const INLINE_ARITY: usize = 8;
            let mut snapshot = [Slot::null(); INLINE_ARITY];
            for (i, &s) in [fut, cx].iter().enumerate() {
                let ty = params.get(i).copied().unwrap_or(0);
                snapshot[i] = s;
                if self.is_ref(ty) {
                    self.heap.retain(s);
                }
            }
            let _ = (host.code)(self, &snapshot[..2], host.ctx);
            for (i, &s) in [fut, cx].iter().enumerate() {
                if self.is_ref(params.get(i).copied().unwrap_or(0)) {
                    self.heap.release(s);
                }
            }
            if let Some(t) = self.host_trap.take() {
                return Err(t);
            }
            Ok(Value::Nil)
        } else {
            // the call ABI (op_call_i's law): retain each argument the
            // callee's signature takes by reference; the callee's root
            // ret releases them — drive adds no references of its own
            let mut regs: Vec<Slot> = vec![fut, cx];
            for (i, &s) in regs.clone().iter().enumerate() {
                if self.is_ref(self.param_ty(fid, i)) {
                    self.heap.retain(s);
                }
            }
            let nregs = self.prog.funcs[fid as usize].regs.len();
            regs.resize(nregs, Slot::int(0));
            if self.running {
                // nested entry (a drive inside a drive — the host-fn
                // lane): stash the outer cursor, run the callee to its
                // root ret on a clean frame stack, restore either way
                let outer = InterpCursor {
                    frames: std::mem::take(&mut self.frames),
                    func: self.cur_func,
                    pc: self.cur_pc,
                    regs: std::mem::take(&mut self.cur_regs),
                    ret_dst: self.cur_ret_dst,
                };
                self.running = false;
                self.enter(fid, regs, None);
                let r = self.run_loop();
                // queued `dispose` calls run before the cursor restore —
                // a cancellation checkpoint's released locals dispose here
                let r = self.drain_disposal(r);
                self.frames = outer.frames;
                self.cur_func = outer.func;
                self.cur_pc = outer.pc;
                self.cur_regs = outer.regs;
                self.cur_ret_dst = outer.ret_dst;
                self.running = true;
                r
            } else {
                self.enter(fid, regs, None);
                let r = self.run_loop();
                self.drain_disposal(r)
            }
        };
        let _ = r?; // a trap (including OutOfFuel) parks the frame at pc;
        let st = cell_of(fut);
        let _ = st;
        // the outcome is the state field
        let cell = cell_of(fut);
        let CellData::Record { fields } = &cell.data else {
            return Err(Trap::new(
                TrapKind::Invalid,
                "drive: future is not a record",
            ));
        };
        let state = fields
            .borrow()
            .get(rut_core::async_frame::STATE_FIELD as usize)
            .unwrap_or(Slot::null());
        if unsafe { state.r.is_null() } {
            // completion wakes the awaiter: re-enqueue (the queue takes
            // its own reference) and clear the edge — one future, one
            // awaiter, set only on the park path
            let awaiter = fields
                .borrow()
                .get(rut_core::async_frame::AWAITER_FIELD as usize)
                .unwrap_or(Slot::null());
            if unsafe { !awaiter.r.is_null() } {
                fields
                    .borrow_mut()
                    .set(rut_core::async_frame::AWAITER_FIELD as usize, Slot::null());
                self.launch(awaiter);
            }
            Ok(Drive::Done)
        } else {
            // the park: host-future bookkeeping. A parked HOST future
            // joins the poll set itself (a launch-driven host future is
            // only ever seen here); a woven frame parked on one joins
            // ITS pending edge (the await site parks in compiled code,
            // so the poll set would otherwise never learn of it). The
            // disclosed double-await misuse is absorbed by the dedup.
            self.note_host_park(ty, fut);
            if !self.host_frames.contains(&ty) {
                // the pending CHAIN walk: a woven awaiter's own park is
                // compiled ops (the VM only sees THIS frame's park), so
                // a chain of nested awaits hides the host future at its
                // bottom — job parks on outer, outer on inner, inner on
                // the host future. Each intermediate frame's pending
                // edge leads deeper; the walk follows them until it
                // finds the host frame to join (the poll set's whole
                // business). Bounded — a misuse cycle cannot spin it.
                let mut pend = fields
                    .borrow()
                    .get(rut_core::async_frame::PENDING_FIELD as usize)
                    .unwrap_or(Slot::null());
                let mut guard = 0u32;
                while !unsafe { pend.r.is_null() } && guard < 64 {
                    let pty = cell_of(pend).ty;
                    if self.host_frames.contains(&pty) {
                        self.note_host_park(pty, pend);
                        break;
                    }
                    pend = cell_of(pend)
                        .record_get(rut_core::async_frame::PENDING_FIELD)
                        .unwrap_or(Slot::null());
                    guard += 1;
                }
            }
            Ok(Drive::Parked)
        }
    }

    /// The poll-set insert: one reference, one entry (the dedup makes
    /// a double park on the same future a no-op — the disclosed v1
    /// misuse, not a leak — and keeps the poll spin's own re-walk from
    /// duplicating the entry it is still holding).
    fn note_host_park(&mut self, ty: u32, fut: Slot) {
        if !self.host_frames.contains(&ty) {
            return;
        }
        if self.host_pending.iter().any(|s| Slot::same_ref(*s, fut)) {
            return;
        }
        self.heap.retain(fut);
        self.host_pending.push(fut);
    }

    /// One poll spin over the host-pending set: drive each frame once
    /// (its `Future::yield` vtable fill — the minted wrapper — runs the
    /// cancel probe / the `__yield` resumption probe). Completed frames
    /// leave the set (the entry's reference released); pending frames
    /// stay for the next spin. The walk keeps each entry IN the set
    /// while it drives, so the drive's own re-park note dedups against
    /// it, and an index walk (never `mem::take`) means a propagated
    /// trap — the fail path through `__take` — leaves every reference
    /// exactly where it belongs. Answers whether any frame COMPLETED,
    /// so `run_ready` can drain the woken awaiters in the same pass.
    fn poll_hosts(&mut self) -> Result<bool, Trap> {
        let mut completed = false;
        let mut i = 0usize;
        while i < self.host_pending.len() {
            let fut = self.host_pending[i];
            match self.drive(fut) {
                Ok(Drive::Done) => {
                    self.host_pending.remove(i);
                    self.heap.release(fut);
                    completed = true;
                }
                Ok(Drive::Parked) => i += 1,
                Err(t) => return Err(t),
            }
        }
        Ok(completed)
    }

    /// The ready queue's drain (the `run_ready` body, split so the
    /// host-poll spin can re-drain after a completion wake).
    fn drain_ready(&mut self) -> Result<usize, Trap> {
        let mut n = 0;
        while let Some(fut) = self.ready.pop_front() {
            let r = self.drive(fut);
            self.heap.release(fut);
            r?;
            n += 1;
        }
        Ok(n)
    }

    /// The queued host-future slots, front first — the test/tooling
    /// read for a frame the embedder wants to `cancel` (the
    /// `first_ready` precedent, host side).
    pub fn first_host_pending(&self) -> Option<Slot> {
        self.host_pending.first().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rut_core::binary::Program;
    use rut_core::types::{TypeTable, TY_F64};

    /// An empty program — no funcs, no consts, just the boot type table.
    fn empty_vm() -> Vm {
        let prog = Program {
            name: "keypayload".into(),
            scope: 0,
            interner: Default::default(),
            surface: Default::default(),
            types: TypeTable::boot(),
            ifaces: Vec::new(),
            iface_slots: Vec::new(),
            vtables: Vec::new(),
            disposal_impls: Vec::new(),
            consts: Vec::new(),
            funcs: Vec::new(),
            exports: Vec::new(),
            inst_types: Vec::new(),
            inst_fns: Vec::new(),
            templates: Vec::new(),
            template_stubs: Vec::new(),
            template_first_fid: 0,
            template_end_fid: 0,
        };
        Vm::new(Rc::new(prog), &Limits::default(), HostRegistry::new()).unwrap()
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
        // floats and stack-trace cells are rut values but not native
        // keys — the user-defined-key trap's raw material
        let f = boxed(&vm, Slot::float(1.5), TY_F64);
        assert_eq!(
            vm.opaque_key_payload(&f).unwrap(),
            KeyPayload::Unsupported(TY_F64)
        );
        let t = boxed(&vm, Slot::null(), rut_core::types::TY_STACK_TRACE);
        assert_eq!(
            vm.opaque_key_payload(&t).unwrap(),
            KeyPayload::Unsupported(rut_core::types::TY_STACK_TRACE)
        );
    }

    #[test]
    fn a_host_payload_box_is_unsupported() {
        let mut vm = empty_vm();
        // the box's payload is Rust, not a rut value — there
        // is no key to read, and the entry names Opaque for the trap
        let b = crate::heap::Opaque::alloc(&mut vm, 42i64).unwrap();
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
            name: prog.interner.intern("main"),
            params: vec![],
            ret: TY_I32,
            is_method: false,
            n_captures: 0,
            regs: vec![TY_ANY, TY_BYTES, TY_ANY, TY_ANY],
            argv: vec![],
            labels: vec![],
            code: vec![],
            spans: vec![],
            pos: vec![],
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
        // alias (the one `bytes.clone()` law)
        assert_ne!(
            unsafe { b.r },
            unsafe { c.r },
            "the clone must be a fresh cell"
        );
        assert_eq!(cell_of(c).bytes_copy(), vec![7, 8, 9]);
        assert_eq!(
            cell_of(b).bytes_copy(),
            vec![7, 8, 9],
            "the original is untouched"
        );
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

        let CellData::Array { items, .. } = &cell_of(b).data else {
            panic!("not bytes")
        };
        items.borrow_mut().set(0, Slot::int(255));
        assert_eq!(
            cell_of(c).bytes_copy(),
            vec![1, 2, 3],
            "the clone must not observe the original's mutation"
        );

        let CellData::Array { items, .. } = &cell_of(c).data else {
            panic!("not bytes")
        };
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

    // ---- the driving protocol's door-level tests (the plan's
    // Phase-4 matrix is its own session; these pin the DOORS on a
    // hand-minted machine — the compiler's host-frame shape spelled by
    // hand, so a REAL parked host future exists without a compiled
    // program) ----

    mod idle_tests {
        use super::*;
        use rut_core::async_frame as af;
        use rut_core::binary::{FuncCode, IfaceDesc, IfaceMethod, Program};
        use rut_core::ops::{Op, NOREG};
        use rut_core::types::{
            FieldInfo, FieldVis, PrimTy, Repr, RutType, TyKind, TypeId, TY_BOOL, TY_I32, TY_I64,
            TY_NIL, TY_OPAQUE,
        };
        use std::rc::Rc;
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        /// One `probe: () -> i64` host async row + one minted frame —
        /// the machine the door tests drive. `cell` is the settle
        /// handle (the `__start` row's minted Completer); `frame` the
        /// host frame, the test's own reference.
        struct HostMachine {
            vm: Vm,
            cell: Completer<i64>,
            frame: Slot,
            /// the mini sleep fixture's type ids (the timer lane)
            ckpt_ty: TypeId,
            sleep_ty: TypeId,
        }

        impl HostMachine {
            /// Mint the machine and ONE cold host frame: the state
            /// cell armed (the `__start` row's answer in the state
            /// field), nothing launched yet.
            fn new() -> HostMachine {
                // ---- the program: the compiler's `ensure_host_async`
                // mint (rut-lir), spelled by hand. The `#hframe@probe`
                // record, the `Future<i64>` iface slot, the RunContext
                // cx, the four sugar rows, and the sealing wrapper —
                // the same ops the compiler emits (the cancel probe,
                // the resumption probe, the take marshal). ----
                let mut interner = rut_core::Interner::default();
                let s_future = interner.intern("Future<i64>");
                let s_yield = interner.intern("yield");
                let s_cx = interner.intern(af::RUN_CONTEXT_TYPE);
                let s_frame_field = interner.intern("frame");
                let s_state = interner.intern("state");
                let s_cancelled = interner.intern("cancelled");
                let s_awaiter = interner.intern("awaiter");
                let s_pending = interner.intern("pending");
                let s_answer = interner.intern("answer");
                let s_ms = interner.intern("ms");
                let s_frame = interner.intern(&format!("{}probe", af::HOST_FRAME_PREFIX));
                let s_sleep = interner.intern("#frame@mini");
                let s_ckpt = interner.intern("#ckpt@mini");
                let row = |interner: &mut rut_core::Interner, suffix: &str| {
                    interner.intern(&format!("probe{suffix}"))
                };
                let s_start = row(&mut interner, "__start");
                let s_yield_row = row(&mut interner, "__yield");
                let s_take = row(&mut interner, "__take");
                let s_cancel = row(&mut interner, "__cancel");
                let s_wrap = interner.intern("probe__yield.wrap");
                let s_mini_yield = interner.intern("mini__yield");
                let s_mini_wrap = interner.intern("mini__yield.wrap");

                let mut types = rut_core::types::TypeTable::boot();
                let cx_ty = types.intern(RutType {
                    name: s_cx,
                    kind: TyKind::Data {
                        fields: vec![FieldInfo {
                            name: s_frame_field,
                            ty: TY_OPAQUE,
                            vis: FieldVis::Pub,
                        }],
                    },
                });
                let frame_ty = types.intern(RutType {
                    name: s_frame,
                    kind: TyKind::Data {
                        fields: vec![
                            FieldInfo {
                                name: s_state,
                                ty: TY_OPAQUE,
                                vis: FieldVis::Pub,
                            },
                            FieldInfo {
                                name: s_cancelled,
                                ty: TY_BOOL,
                                vis: FieldVis::Pub,
                            },
                            FieldInfo {
                                name: s_awaiter,
                                ty: TY_OPAQUE,
                                vis: FieldVis::Pub,
                            },
                            FieldInfo {
                                name: s_pending,
                                ty: TY_OPAQUE,
                                vis: FieldVis::Pub,
                            },
                            FieldInfo {
                                name: s_answer,
                                ty: TY_I64,
                                vis: FieldVis::Pub,
                            },
                        ],
                    },
                });
                let ckpt_ty = types.intern(RutType {
                    name: s_ckpt,
                    kind: TyKind::Enum {
                        members: vec![(s_state, 0), (s_pending, 1)],
                    },
                });
                let sleep_ty = types.intern(RutType {
                    name: s_sleep,
                    kind: TyKind::Data {
                        fields: vec![
                            FieldInfo {
                                name: s_state,
                                ty: ckpt_ty,
                                vis: FieldVis::Pub,
                            },
                            FieldInfo {
                                name: s_cancelled,
                                ty: TY_BOOL,
                                vis: FieldVis::Pub,
                            },
                            FieldInfo {
                                name: s_awaiter,
                                ty: TY_OPAQUE,
                                vis: FieldVis::Pub,
                            },
                            FieldInfo {
                                name: s_pending,
                                ty: TY_OPAQUE,
                                vis: FieldVis::Pub,
                            },
                            FieldInfo {
                                name: s_answer,
                                ty: TY_NIL,
                                vis: FieldVis::Pub,
                            },
                            FieldInfo {
                                name: s_ms,
                                ty: TY_I64,
                                vis: FieldVis::Pub,
                            },
                        ],
                    },
                });
                let n_types = types.types.len();

                // the funcs: the four sugar rows (bodyless host thunks
                // — the boot join resolves `host_id` against the
                // registry), the sealing wrapper (compiled ops), and
                // the mini sleep's engine-backed pair (the
                // `__sleep_yield` precedent: a compiled wrapper seals
                // the raw pair into boxes and calls the bodyless
                // thunk). Fid order here IS the wiring below;
                // `__start` dispatches by name through `call_host_row`.
                const YIELD_FID: u32 = 1;
                const TAKE_FID: u32 = 2;
                const CANCEL_FID: u32 = 3;
                const WRAP_FID: u32 = 4;
                const MINI_YIELD_FID: u32 = 5;
                const MINI_WRAP_FID: u32 = 6;
                let thunk = |name, params: Vec<TypeId>, ret: TypeId| -> FuncCode {
                    FuncCode {
                        name,
                        regs: params.clone(),
                        params,
                        ret,
                        is_method: false,
                        n_captures: 0,
                        argv: vec![],
                        labels: vec![],
                        code: vec![],
                        spans: vec![],
                        pos: vec![],
                        host_id: Some(name),
                    }
                };
                let funcs = vec![
                    thunk(s_start, vec![], TY_OPAQUE),
                    thunk(s_yield_row, vec![TY_OPAQUE, TY_OPAQUE], TY_I32),
                    thunk(s_take, vec![TY_OPAQUE], TY_I64),
                    thunk(s_cancel, vec![TY_OPAQUE], TY_NIL),
                    // the sealing wrapper: r0 frame, r1 cx, r2 state,
                    // r3 sealed cx, r4 probe, r5 null, r6 answer, r7
                    // cancelled flag
                    FuncCode {
                        name: s_wrap,
                        regs: vec![
                            frame_ty, cx_ty, TY_OPAQUE, TY_OPAQUE, TY_I32, TY_OPAQUE, TY_I64,
                            TY_BOOL,
                        ],
                        params: vec![frame_ty, cx_ty],
                        ret: TY_NIL,
                        is_method: false,
                        n_captures: 0,
                        // the argv pool: [state] for the cancel row ·
                        // [state, sealed cx] for the yield probe ·
                        // [state] for the take marshal
                        argv: vec![2, 3, 2],
                        labels: vec![],
                        code: vec![
                            // a re-drive of a retired frame answers here
                            Op::GetF {
                                dst: 2,
                                obj: 0,
                                field: af::STATE_FIELD,
                                repr: Repr::Ref,
                            },
                            Op::Br {
                                cond: 2,
                                then_t: 3,
                                else_t: 2,
                            },
                            Op::Ret { val: None },
                            // the cancel probe (the cx `cancelled()` data path)
                            Op::GetF {
                                dst: 7,
                                obj: 0,
                                field: af::CANCELLED_FIELD,
                                repr: Repr::Prim(PrimTy::Bool),
                            },
                            Op::Br {
                                cond: 7,
                                then_t: 5,
                                else_t: 9,
                            },
                            // the cancel arm: the abort row, then retire
                            Op::Call {
                                func: CANCEL_FID,
                                argv_off: 2,
                                argc: 1,
                                dst: NOREG,
                            },
                            Op::ConstRaw { dst: 5, bits: 0 },
                            Op::SetF {
                                obj: 0,
                                field: af::STATE_FIELD,
                                val: 5,
                                repr: Repr::Ref,
                            },
                            Op::Ret { val: None },
                            // the resumption probe: seal the cx, call `__yield`
                            Op::Box {
                                dst: 3,
                                val: 1,
                                ty: cx_ty,
                            },
                            Op::Call {
                                func: YIELD_FID,
                                argv_off: 0,
                                argc: 2,
                                dst: 4,
                            },
                            Op::Br {
                                cond: 4,
                                then_t: 12,
                                else_t: 16,
                            },
                            // settle (ready/failed): marshal the answer
                            // FIRST, then retire — a failed completer
                            // traps HERE, through `__take` (the state
                            // stays armed behind the propagated trap)
                            Op::Call {
                                func: TAKE_FID,
                                argv_off: 2,
                                argc: 1,
                                dst: 6,
                            },
                            Op::SetF {
                                obj: 0,
                                field: af::HOST_ANSWER_FIELD,
                                val: 6,
                                repr: Repr::Prim(PrimTy::I64),
                            },
                            Op::ConstRaw { dst: 5, bits: 0 },
                            Op::SetF {
                                obj: 0,
                                field: af::STATE_FIELD,
                                val: 5,
                                repr: Repr::Ref,
                            },
                            Op::Ret { val: None },
                        ],
                        spans: vec![],
                        pos: vec![],
                        host_id: None,
                    },
                    thunk(
                        s_mini_yield,
                        vec![TY_OPAQUE, TY_OPAQUE], // the row: sealed boxes
                        TY_NIL,
                    ),
                    // the mini sleep's sealing wrapper (the
                    // `__sleep_yield.wrap` shape): the driving loop's
                    // raw (frame, cx) pair seals into boxes and the
                    // host thunk crosses them
                    FuncCode {
                        name: s_mini_wrap,
                        regs: vec![sleep_ty, cx_ty, TY_OPAQUE, TY_OPAQUE],
                        params: vec![sleep_ty, cx_ty],
                        ret: TY_NIL,
                        is_method: false,
                        n_captures: 0,
                        argv: vec![2, 3],
                        labels: vec![],
                        code: vec![
                            Op::Box {
                                dst: 2,
                                val: 0,
                                ty: sleep_ty,
                            },
                            Op::Box {
                                dst: 3,
                                val: 1,
                                ty: cx_ty,
                            },
                            Op::Call {
                                func: MINI_YIELD_FID,
                                argv_off: 0,
                                argc: 2,
                                dst: NOREG,
                            },
                            // well-formedness: the fn ends in Ret
                            Op::Ret { val: None },
                        ],
                        spans: vec![],
                        pos: vec![],
                        host_id: None,
                    },
                ];

                let prog = Program {
                    name: "idle_fixture".into(),
                    scope: 0,
                    interner,
                    surface: Default::default(),
                    types,
                    ifaces: vec![IfaceDesc {
                        name: s_future,
                        methods: vec![IfaceMethod {
                            name: s_yield,
                            params: vec![frame_ty, cx_ty],
                            ret: TY_NIL,
                        }],
                    }],
                    iface_slots: vec![(0, 0)],
                    vtables: {
                        let mut v: Vec<Vec<Option<u32>>> = vec![Vec::new(); n_types];
                        v[frame_ty as usize] = vec![Some(WRAP_FID)];
                        v[sleep_ty as usize] = vec![Some(MINI_WRAP_FID)];
                        v
                    },
                    disposal_impls: vec![None; n_types],
                    consts: vec![],
                    funcs,
                    exports: vec![],
                    inst_types: vec![],
                    inst_fns: vec![],
                    templates: vec![],
                    template_stubs: vec![],
                    template_first_fid: 0,
                    template_end_fid: 0,
                };

                // ---- the registry: the probe row family through the
                // ONE-closure sugar, and the mini sleep's yield row ----
                let mut hosts = HostRegistry::new();
                let (tx, rx) = mpsc::channel::<Completer<i64>>();
                crate::register_async!(hosts, "probe", () -> i64, move |c: Completer<i64>| {
                    // the fixture's one mint — the test takes the
                    // settle handle through the channel
                    let _ = tx.send(c);
                });
                crate::register!(
                    hosts,
                    "mini__yield",
                    (OpaqueRef, OpaqueRef) -> (),
                    move |vm: &mut Vm, f: OpaqueRef, _cx: OpaqueRef| {
                        // the `__sleep_yield` dance, mini: the fresh arm
                        // arms the timer and parks at s1; the resumed arm
                        // (or a cancelled/retired frame) retires
                        let frame = vm
                            .opaque_rut_value(Slot { r: f.ptr() })
                            .map(|(s, _)| s)
                            .ok_or_else(|| {
                                Trap::new(
                                    TrapKind::Invalid,
                                    "mini sleep: the box holds no rut value",
                                )
                            })?;
                        let cancelled = vm
                            .record_field(frame, af::CANCELLED_FIELD as usize)
                            .map(|s| s.as_bool())
                            .unwrap_or(false);
                        let state = vm
                            .record_field(frame, af::STATE_FIELD as usize)
                            .unwrap_or(Slot::null());
                        let fresh = !unsafe { state.r.is_null() }
                            && vm
                                .enum_member_slot(ckpt_ty, 0)
                                .ok()
                                .map(|s0| Slot::same_ref(state, s0))
                                .unwrap_or(false);
                        if fresh && !cancelled {
                            let ms = vm
                                .record_field(frame, af::LOCALS_BASE as usize)
                                .map(|s| unsafe { s.i } as u64)
                                .unwrap_or(0);
                            let s1 = vm.enum_member_slot(ckpt_ty, 1)?;
                            vm.arm_timer(vm.now_ms() + ms, frame);
                            vm.set_record_field(frame, af::STATE_FIELD as usize, s1);
                        } else {
                            vm.set_record_field(frame, af::STATE_FIELD as usize, Slot::null());
                        }
                        Ok(())
                    }
                );

                let mut vm = Vm::new(Rc::new(prog), &Limits::default(), hosts).expect("fixture vm");

                // ---- the frame mint (the call site's shape): `__start`
                // answers the state cell; the cold frame arms it ----
                let out = vm
                    .call_host_row("probe__start", &[])
                    .expect("the start row");
                let Value::Opaque(_) = &out else {
                    panic!("__start answers an opaque, got {out:?}");
                };
                let cell = rx.try_recv().expect("the fixture's completer");
                let state = vm.value_in(&out, TY_OPAQUE).expect("the state cell in");
                let nfields = match vm.prog.types.kind(frame_ty) {
                    TyKind::Data { fields } => fields.len(),
                    _ => unreachable!("the host frame type is a record"),
                };
                let frame = vm
                    .alloc_zeroed_record(frame_ty, nfields)
                    .expect("the frame record");
                vm.set_record_field(frame, af::STATE_FIELD as usize, state);
                vm.release(state); // the register's ref — the field owns the cell
                drop(out); // the handle's ref — only the field's remains
                HostMachine {
                    vm,
                    cell,
                    frame,
                    ckpt_ty,
                    sleep_ty,
                }
            }

            /// Launch the frame — the machine's only work. It parks on
            /// the protocol's first poll.
            fn launched() -> HostMachine {
                let mut m = HostMachine::new();
                m.vm.launch(m.frame);
                m
            }

            /// Mint + launch one mini sleep frame (`ms`): the timer
            /// lane's fixture. Returns the frame (the test's own
            /// reference) so the retirement is readable.
            fn launch_mini_sleep(&mut self, ms: i64) -> Slot {
                let nfields = match self.vm.prog.types.kind(self.sleep_ty) {
                    TyKind::Data { fields } => fields.len(),
                    _ => unreachable!("the sleep frame type is a record"),
                };
                let frame = self
                    .vm
                    .alloc_zeroed_record(self.sleep_ty, nfields)
                    .expect("the sleep frame");
                let s0 = self.vm.enum_member_slot(self.ckpt_ty, 0).expect("s0");
                self.vm
                    .set_record_field(frame, af::STATE_FIELD as usize, s0);
                self.vm
                    .set_record_field(frame, af::LOCALS_BASE as usize, Slot::int(ms));
                self.vm.launch(frame);
                frame
            }
        }

        // ---- wait_hosts: the Vm-level park door ----

        /// The immediate-return shape AND the parked wake: a settle
        /// that landed before the park answers true at once; a settle
        /// landing from another thread wakes the park and answers
        /// true.
        #[test]
        fn wait_hosts_answers_the_parked_wake_and_the_pre_park_settle() {
            let vm = empty_vm();
            let wakes = vm.host_wakes();
            wakes.bump();
            assert!(
                vm.wait_hosts(None),
                "a settle that landed before the park answers true"
            );
            let bumper = {
                let wakes = wakes.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(50));
                    wakes.bump();
                })
            };
            assert!(vm.wait_hosts(None), "the parked wake answers true");
            bumper.join().expect("the bumper thread");
        }

        /// No settle: the timeout answers false, honors the deadline,
        /// and does not hang.
        #[test]
        fn wait_hosts_timeout_answers_false_without_hanging() {
            let vm = empty_vm();
            let t0 = Instant::now();
            assert!(
                !vm.wait_hosts(Some(Duration::from_millis(50))),
                "no settle: the timeout answers false"
            );
            assert!(
                t0.elapsed() >= Duration::from_millis(45),
                "the deadline was honored, not skipped"
            );
        }

        // ---- run_until_idle: the driving protocol ----

        /// `Wait::Block` on a machine whose only work is a settling
        /// host future (the row registered through `register_async!`,
        /// completed from a thread): the protocol parks, the worker's
        /// `complete` wakes it, the take marshal lands the answer, the
        /// frame retires — Drained.
        #[test]
        fn run_until_idle_block_drains_a_settling_host_future() {
            let mut m = HostMachine::launched();
            let worker = {
                let c = m.cell.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(30));
                    c.complete(42);
                })
            };
            let idle = m.vm.run_until_idle(Wait::Block).expect("clean run");
            worker.join().expect("the worker thread");
            assert_eq!(idle, Idle::Drained);
            assert!(
                m.vm.first_host_pending().is_none(),
                "the settled frame left the poll set"
            );
            assert_eq!(m.vm.pending_tasks(), 0, "nothing left pending");
            let answer =
                m.vm.record_field(m.frame, af::HOST_ANSWER_FIELD as usize)
                    .expect("the answer lane");
            assert_eq!(unsafe { answer.i }, 42, "the take marshal landed");
        }

        /// `Wait::NonBlock` never parks: host-parked work answers
        /// WouldBlock; work that settles inside the call (the JS-host
        /// contract) and an empty machine answer Drained.
        #[test]
        fn run_until_idle_nonblock_answers_wouldblock_and_drains() {
            let mut m = HostMachine::launched();
            assert_eq!(
                m.vm.run_until_idle(Wait::NonBlock).expect("clean run"),
                Idle::WouldBlock,
                "parked host work and a no-park policy: WouldBlock"
            );
            assert!(
                m.vm.first_host_pending().is_some(),
                "the driven frame sits in the poll set, unanswered"
            );

            // the JS-host shape: the future settles inside the call —
            // Drained without a single park
            let mut m2 = HostMachine::new();
            m2.cell.complete(7);
            m2.vm.launch(m2.frame);
            assert_eq!(
                m2.vm.run_until_idle(Wait::NonBlock).expect("clean run"),
                Idle::Drained,
                "a settle inside the call drains without parking"
            );

            let mut vm = empty_vm();
            assert_eq!(
                vm.run_until_idle(Wait::NonBlock).expect("clean run"),
                Idle::Drained,
                "no work at all drains at once"
            );
        }

        /// The deadline arm under NonBlock never parks either: host
        /// work plus a timer fast-forwards the virtual clock and
        /// answers WouldBlock — no stall (a bounded park here would
        /// show up as wall time). The WouldBlock answer comes only
        /// after the probe round retires the expired timer's frame:
        /// the verdict names genuinely un-drivable work.
        #[test]
        fn run_until_idle_nonblock_never_parks_the_deadline_arm() {
            let mut m = HostMachine::launched();
            let sleep_frame = m.launch_mini_sleep(50);
            let t0 = Instant::now();
            let idle = m.vm.run_until_idle(Wait::NonBlock).expect("clean run");
            assert_eq!(idle, Idle::WouldBlock);
            assert!(
                t0.elapsed() < Duration::from_millis(40),
                "the deadline arm must not park under NonBlock"
            );
            assert_eq!(m.vm.now_ms(), 50, "the clock lane still ran");
            let st =
                m.vm.record_field(sleep_frame, af::STATE_FIELD as usize)
                    .expect("the sleep frame's state lane");
            assert!(
                unsafe { st.r.is_null() },
                "the timer frame retired before the WouldBlock verdict"
            );
            assert!(
                m.vm.first_host_pending().is_some(),
                "only the host park remains"
            );
        }

        /// The clock lane: host work shares the wait with a timer —
        /// the Some(d) arm parks bounded by the deadline, jumps the
        /// virtual clock (`set_now`), the sleep frame retires on its
        /// re-drive after the expiry — and the host future's late
        /// settle (after the deadline) finishes the drain.
        #[test]
        fn run_until_idle_jumps_the_clock_while_host_work_parks() {
            let mut m = HostMachine::launched();
            let sleep_frame = m.launch_mini_sleep(50);
            // the settle lands AFTER the deadline: the bounded park
            // runs the clock out first, the Block park catches it late
            let worker = {
                let c = m.cell.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(80));
                    c.complete(42);
                })
            };
            let idle = m.vm.run_until_idle(Wait::Block).expect("clean run");
            worker.join().expect("the worker thread");
            assert_eq!(idle, Idle::Drained);
            assert_eq!(
                m.vm.now_ms(),
                50,
                "the virtual clock sits at the deadline it jumped to"
            );
            assert_eq!(
                m.vm.pending_tasks(),
                0,
                "the host future and the timer both retired"
            );
            let st =
                m.vm.record_field(sleep_frame, af::STATE_FIELD as usize)
                    .expect("the sleep frame's state lane");
            assert!(
                unsafe { st.r.is_null() },
                "the sleep frame retired on the re-drive after the expiry"
            );
        }

        /// The fail path: a worker's `fail` settles AND bumps, the
        /// parked protocol wakes, the `__take` marshal traps, and the
        /// first trap propagates out of `run_until_idle`.
        #[test]
        fn run_until_idle_propagates_a_failed_settle() {
            let mut m = HostMachine::launched();
            let worker = {
                let c = m.cell.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(30));
                    c.fail("kaboom".to_string());
                })
            };
            let err =
                m.vm.run_until_idle(Wait::Block)
                    .expect_err("the failed settle traps at the take marshal");
            worker.join().expect("the worker thread");
            assert!(err.msg.contains("kaboom"), "{}", err.msg);
        }
    }
}
