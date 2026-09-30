//! rut-vm — the execution crate: the RC heap with Slot /
//! Value / Trap, the load-time verifier (RFC
//! 0033 §2), and the interpreter with fuel+heap budgets.

pub mod heap;
pub mod interp;
pub mod verify;

pub(crate) mod arena;

pub use heap::{HostPayload, Opaque, OpaqueRef, Slot, Trap, TrapKind, ValSlot, Value};
pub use interp::{Completer, FAILED, PENDING, READY, HostPkg, HostPkgContext};
