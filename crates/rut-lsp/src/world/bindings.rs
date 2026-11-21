//! The binding rows' lsp-side face. The pass itself lives in rut-ast —
//! `Doc::bindings()`, lazy per version, pure tree+text facts. No caller
//! runs a pass: the doors read `doc.bindings()` internally; this module
//! keeps the row vocabulary re-exported for the views and the one
//! display helper the resolver still needs.

pub use rut_ast::{BindKind, Binding};
