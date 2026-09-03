//! rut-ast — the flat arena AST (RFC 0030 §5): nodes are records in one
//! `Vec`, referenced by `NodeId`; built bottom-up by `rut-parser`,
//! consumed by resolve/typecheck (RFC 0031) and LIR (0032). Also home to
//! the astDump pretty-printer (`dump`) the demo page renders.

pub mod ast;
pub mod dump;
