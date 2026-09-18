//! The embedder's host-fn binding table (RFC 0022/0025/0026) — built
//! BEFORE the `Vm` exists. An embedder is a host: its bindings depend on
//! nothing but itself (the closures capture their own state; the `&mut Vm`
//! in `HostFn`'s type is a call-time argument), so registration happens up
//! front and [`Vm::new`](super::Vm::new) joins the table against the
//! program's host thunks eagerly — a declared-but-unbound fn is a
//! construction error, never a mid-run trap.
//!
//! [`HostRegistry::verify_against`] is the RFC 0025 contract check, also
//! pre-VM: the session's declared `.d.rut` surface vs the registry,
//! panicking on the three mismatch classes before any rut code runs.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use rut_core::types::TypeId;

use super::{Trap, Vm};
use crate::heap::Value;

/// An embedder implementation for a bodyless host function
/// (RFC 0022/0026): registered by name against `FuncCode.host`. It gets
/// the VM so it can allocate/inspect `Opaque` handles (RFC 0014).
pub type HostFn = Rc<RefCell<dyn FnMut(&mut Vm, &[Value]) -> Result<Value, Trap>>>;

/// A host pkg's expected binding table (RFC 0025): `<scope>::<name>` →
/// `(params, ret)` — the `.d.rut` declarations a mounting session holds,
/// checked against a registry by [`HostRegistry::verify_against`].
pub type ExpectedHostFns = BTreeMap<String, (Vec<TypeId>, TypeId)>;

/// The resolved half of a host thunk — what the hot path dispatches
/// through, built once at the `Vm::new` join. Params are deliberately
/// NOT here: they are borrowed from the program's own `FuncCode` at
/// call time (the program is `Rc`-cloned into the call anyway).
pub(crate) struct HostEntry {
    pub(crate) f: HostFn,
    pub(crate) ret: TypeId,
}

/// The embedder's binding table, handed to `Vm::new` (fourth argument).
#[derive(Default)]
pub struct HostRegistry {
    fns: HashMap<String, HostFn>,
    sigs: HashMap<String, (Vec<TypeId>, TypeId)>,
}

impl HostRegistry {
    pub fn new() -> HostRegistry {
        HostRegistry::default()
    }

    /// Register an embedder implementation for a bodyless host function,
    /// WITH its declared signature — the pkg `.d.rut`'s (RFC 0025). The
    /// signature feeds the load-time contract check; the body feeds the
    /// `Vm::new` join.
    pub fn register<F>(&mut self, name: &str, params: Vec<TypeId>, ret: TypeId, f: F)
    where
        F: FnMut(&mut Vm, &[Value]) -> Result<Value, Trap> + 'static,
    {
        self.sigs.insert(name.to_string(), (params, ret));
        self.fns.insert(name.to_string(), Rc::new(RefCell::new(f)));
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
                TY_OPAQUE => "Opaque",
                _ => return format!("#{t:?}"),
            }
            .to_string()
        };
        let sig = |p: &Vec<TypeId>, r: TypeId| -> String {
            format!("({}) -> {}", p.iter().map(|&t| ty(t)).collect::<Vec<_>>().join(", "), ty(r))
        };
        for (name, (params, ret)) in expected {
            match self.sigs.get(name) {
                None => panic!(
                    "host fn `{name}` is declared by a mounted package but never bound — register the body before Vm::new (RFC 0025)"
                ),
                Some((bparams, bret)) => {
                    if bparams != params || bret != ret {
                        panic!(
                            "host fn `{name}` signature drift: the pkg declares {}, the binding is {} (RFC 0025)",
                            sig(params, *ret),
                            sig(bparams, *bret)
                        );
                    }
                }
            }
        }
        for name in self.sigs.keys() {
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
    pub(crate) fn take(&mut self, name: &str) -> Option<HostFn> {
        self.fns.remove(name)
    }
}
