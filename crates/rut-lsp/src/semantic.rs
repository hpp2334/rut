//! The classifier lives in `rut-semantic` (pure frontend: lexer + ast +
//! parser tables, no LSP types); rut-lsp re-exports it so the server's
//! modules and downstream re-exports keep one path.

pub use rut_semantic::*;
