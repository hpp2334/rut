//! The query layer — what an editor asks of a document. One module per
//! LSP request family; `analysis` is the aggregator both faces call.
//!
//! Private to the crate on purpose: `lib.rs` re-exports these modules at
//! the root, so the canonical paths stay the flat `rut_lsp::analysis`,
//! `rut_lsp::completion`, … that the two shells already spell.

pub mod analysis;
pub mod check;
pub mod completion;
pub mod definition;
pub mod inlay;
pub mod references;
pub mod signature_help;
