//! rut-core — the shared artifact vocabulary (RFC 0041 §2): the RutType
//! table (0015), typed bytecode ops (0032), and the module binary format
//! (0033). rut-lir / rut-driver emit these; rut-vm loads, verifies,
//! and runs them.

pub mod binary;
pub mod id;
pub mod link;
pub mod ops;
pub mod types;

pub use id::{pack, scope_of, local_of, ScopeId, BOOT_SCOPE};
pub use link::{link, LinkError};
pub use types::{PrimTy, RutType, TypeId, TypeTable, TyKind};
