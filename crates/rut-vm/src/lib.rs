//! rut-vm — the execution crate (RFC 0041 §2): the RC heap with Slot /
//! Value / Trap (RFC 0016/0039, 0015 §5), the load-time verifier (RFC
//! 0033 §2), and the interpreter with fuel+heap budgets (RFC 0034/0040).

pub mod heap;
pub mod interp;
pub mod verify;

pub(crate) mod arena;

pub use heap::{HostPayload, Opaque, OpaqueRef, Slot, Trap, TrapKind, ValSlot, Value};
pub use interp::HostVal;
