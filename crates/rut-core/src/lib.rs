//! rut-core — the shared artifact vocabulary: the RutType
//! table, typed bytecode ops, and the module binary format.
//! rut-lir / rut-driver emit these; rut-vm loads, verifies,
//! and runs them.

pub mod async_frame;
pub mod binary;
pub mod id;
pub mod link;
pub mod ops;
pub mod sym;
pub mod types;

pub use id::{pack, scope_of, local_of, ScopeId, BOOT_SCOPE};
pub use link::{link, LinkError};
pub use sym::{IdentId, Interner};
pub use types::{PrimTy, RutType, TypeId, TypeTable, TyKind};
