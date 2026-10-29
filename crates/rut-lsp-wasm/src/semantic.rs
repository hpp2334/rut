//! The classifier lives in `rut-semantic` (pure frontend: lexer + ast +
//! parser tables, no LSP types); this crate re-exports it so the
//! queries and the ABI keep one path.

pub use rut_semantic::*;
