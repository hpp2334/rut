//! rut-ast — the flat arena AST: nodes are records in one
//! `Vec`, referenced by `NodeId`; built bottom-up by `rut-parser`,
//! consumed by resolve/typecheck and LIR. Also home to
//! the astDump pretty-printer (`dump`) the demo page renders.

pub mod ast;
pub mod dump;
