//! rut-vm — the execution crate: the RC heap with Slot /
//! Value / Trap, the load-time verifier (RFC
//! 0033 §2), and the interpreter with fuel+heap budgets.

pub mod heap;
pub mod interp;
pub mod verify;

pub(crate) mod arena;

pub use heap::{HostPayload, Opaque, OpaqueRef, Slot, Trap, TrapKind, ValSlot, Value};
pub use interp::{
    builder::{IntoVmParts, VmBuilder, VmError, VmParts},
    Completer, FAILED, HostPkg, HostPkgContext, PENDING, READY, rut_box_payload,
};
