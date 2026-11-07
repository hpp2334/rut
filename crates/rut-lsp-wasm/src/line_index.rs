//! moved to `rut-semantic` (the document layer owns position math —
//! one home for lsp-wasm, fmt's inlined copy law, and ParsedDoc).
//! Re-export shim; the imports migrate in the layering phase 3.
pub use rut_semantic::LineIndex;
