//! rut-core — the VM (RFC 0041 §2): RutType table (0015), typed bytecode
//! (0032), module binary + verifier (0033), interpreter with budgets
//! (0034/0040), RC heap (0016).

pub mod binary;
pub mod heap;
pub mod interp;
pub mod ops;
pub mod types;
pub mod verify;

pub use heap::{Slot, Value};
pub use types::{PrimTy, RutType, TypeId, TypeTable, TyKind};
