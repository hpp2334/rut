//! The embedder's host-fn binding table (RFC 0022/0025/0026) — built
//! BEFORE the `Vm` exists. An embedder is a host: its bindings depend on
//! nothing but itself (the closures capture their own state; the `&mut Vm`
//! in the body's type is a call-time argument), so registration happens up
//! front and [`Vm::new`](super::Vm::new) joins the table against the
//! program's host thunks eagerly — a declared-but-unbound fn is a
//! construction error, never a mid-run trap.
//!
//! [`HostRegistry::verify_against`] is the RFC 0025 contract check, also
//! pre-VM: the session's declared `.d.rut` surface vs the registry,
//! panicking on the three mismatch classes before any rut code runs.

use std::any::Any;
use std::collections::{BTreeMap, HashMap};

use rut_core::types::TypeId;

use super::boundary::HostHandler;
use super::Vm;
use crate::heap::Slot;

/// A host fn is one dispatch-table entry — the same unit every other
/// table in the VM trades in (threaded op handlers, vtables): a code
/// pointer, a state word, and the declared return for the refcount law.
pub type Ctx = *const ();
pub type HostCode = fn(&mut Vm, &[Slot], Ctx) -> Slot;

/// A binding's signature — a fixed array, no heap: max host arity 8 is
/// the boundary's own law (tuples cap at arity 8). DERIVED from the
/// callable's Rust shape (`HostHandler::SIG`), never written.
#[derive(Clone, Copy)]
pub(crate) struct HostSig {
    pub tys: [TypeId; 8],
    pub ret: TypeId,
    pub n: u8,
}

impl HostSig {
    pub const fn new(tys: &[TypeId], ret: TypeId) -> HostSig {
        let mut a = [0; 8];
        let mut i = 0;
        while i < tys.len() && i < 8 {
            a[i] = tys[i];
            i += 1;
        }
        HostSig { tys: a, ret, n: tys.len() as u8 }
    }

    pub fn slice(&self) -> &[TypeId] {
        &self.tys[..self.n as usize]
    }
}

/// What [`HostRegistry::register`] produced: the table entry plus the
/// owner of its state word. `keep` moves into the Vm at the join and
/// pins the boxed body for the machine's lifetime (single thread,
/// RFC 0034 — the same trust the threaded loop's raw pointers run on).
pub(crate) struct HostBinding {
    pub code: HostCode,
    pub ctx: Ctx,
    pub sigs: HostSig,
    pub keep: Option<Box<dyn Any>>,
}

/// The dispatch-table row, dense by func idx after the join. Copy: the
/// hot path copies one 16-byte entry out and dispatches.
#[derive(Clone, Copy)]
pub(crate) struct HostSlot {
    pub code: HostCode,
    pub ctx: Ctx,
    pub ret: TypeId,
}

impl HostSlot {
    /// filler for non-host funcs — never dispatched (the `host_id`
    /// discriminant in the `FuncCode` routes, as in `hotpath(2)`)
    fn never(_vm: &mut Vm, _slots: &[Slot], _ctx: Ctx) -> Slot {
        debug_assert!(false, "non-host funcs never dispatch through host_slots");
        Slot::int(0)
    }
    pub const NEVER: HostSlot = HostSlot { code: Self::never, ctx: std::ptr::null(), ret: 0 };
}

/// A host pkg's expected binding table (RFC 0025): `<scope>::<name>` →
/// `(params, ret)` — the `.d.rut` declarations a mounting session holds,
/// checked against a registry by [`HostRegistry::verify_against`].
pub type ExpectedHostFns = BTreeMap<String, (Vec<TypeId>, TypeId)>;

/// The embedder's binding table, handed to `Vm::new` (fourth argument).
pub struct HostRegistry {
    fns: HashMap<String, HostBinding>,
}

impl HostRegistry {
    pub fn new() -> HostRegistry {
        HostRegistry { fns: HashMap::new() }
    }

    /// The magic: the callable's Rust shape IS the `.d.rut` row —
    /// `hosts.register("calc::abs", |_vm, x: f64| x.abs())`. The
    /// signature is DERIVED (`F::SIG`, a fixed array, no heap); the
    /// adapter `F::entry` IS the table entry. Infallible `-> R` and
    /// fallible `-> Result<R, Trap>` bodies both fit — `K` is solved by
    /// whichever impl the body's return type matches, and never written.
    /// A param type with no `Arg` impl is a compile error here. For
    /// two-plus params the compiler needs the marker spelled — use the
    /// [`register!`](crate::register) sugar and the closure stays plain.
    pub fn register<F, A, R, K>(&mut self, name: &str, f: F)
    where
        F: HostHandler<A, R, K> + 'static,
    {
        let boxed = Box::new(f);
        // the heap pointee is stable for the box's life; the Box handle
        // itself moves into `keep`
        let ctx = &*boxed as *const F as Ctx;
        self.fns.insert(
            name.to_string(),
            HostBinding { code: F::entry, ctx, sigs: F::SIG, keep: Some(boxed) },
        );
    }

    /// The `.d.rut` ↔ host-impl contract check (RFC 0025), PANICKING
    /// before the Vm boots on three mismatch classes:
    ///
    /// - declared but unbound — a rut call would trap mid-run;
    /// - bound but undeclared — the pkg's surface lies about what exists;
    /// - signature drift — the crossing values would be misinterpreted.
    ///
    /// `expected` is the mounting session's table
    /// (`Session::expected_host_fns`). An embedder wiring bug is a
    /// panic, never a rut diagnostic.
    pub fn verify_against(&self, expected: &ExpectedHostFns) {
        let ty = |t: TypeId| -> String {
            // boot-table ids are stable — name them for the panic message
            use rut_core::types::*;
            match t {
                TY_NIL => "nil",
                TY_BOOL => "bool",
                TY_STR => "str",
                TY_BYTES => "bytes",
                TY_F32 => "f32",
                TY_F64 => "f64",
                TY_I8 => "i8",
                TY_I16 => "i16",
                TY_I32 => "i32",
                TY_I64 => "i64",
                TY_U8 => "u8",
                TY_U16 => "u16",
                TY_U32 => "u32",
                TY_U64 => "u64",
                TY_OPAQUE => "opaque",
                _ => return format!("#{t:?}"),
            }
            .to_string()
        };
        let sig = |p: &[TypeId], r: TypeId| -> String {
            format!("({}) -> {}", p.iter().map(|&t| ty(t)).collect::<Vec<_>>().join(", "), ty(r))
        };
        for (name, (params, ret)) in expected {
            match self.fns.get(name) {
                None => panic!(
                    "host fn `{name}` is declared by a mounted package but never bound — register the body before Vm::new (RFC 0025)"
                ),
                Some(b) => {
                    if b.sigs.slice() != params.as_slice() || b.sigs.ret != *ret {
                        panic!(
                            "host fn `{name}` signature drift: the pkg declares {}, the binding is {} (RFC 0025)",
                            sig(params, *ret),
                            sig(b.sigs.slice(), b.sigs.ret)
                        );
                    }
                }
            }
        }
        for name in self.fns.keys() {
            if !expected.contains_key(name) {
                panic!(
                    "host fn `{name}` is bound but declared by no mounted package — the surface is missing (RFC 0025)"
                );
            }
        }
    }

    /// Take one binding out (the `Vm::new` join consumes the table; the
    /// leftover entries are the embedder's business — `verify_against`
    /// is the check that they were all declared).
    pub(crate) fn take_binding(&mut self, name: &str) -> Option<HostBinding> {
        self.fns.remove(name)
    }
}
