//! rut-ast — the tree and its grammar, one crate.
//!
//! Two modules, one law each:
//! - `ast` (SEALED `mod`) — PURE STRUCTURE: the typed-arena AST.
//!   The module is private — the same privacy wall as rut-lsp's
//!   `semantic`: consumers get the flat re-exports at the crate
//!   root and `parser::`, never an `ast::` module path. `Ast` is a
//!   CAPSULE whose mut doors are `pub(crate)` — only the in-crate
//!   parse lane may build a tree; every other consumer reads.
//!   The artifact is the PAIR `Parsed { ast, src }` (law v3): the
//!   tree holds no text, `src` is the normalized text every span
//!   indexes, and `Span::text(&src)` is the join.
//! - [`parser`] — the grammar: the explicit-frame machine that
//!   turns text into that pair (`parse` fail-fast /
//!   `parse_with_recover` always a tree).
//!
//! The tree vocabulary is re-exported FLAT at the crate root —
//! consumers never spell a module path; file layout stays free to
//! move. The grammar's entries live under `parser::` on purpose:
//! the parse lane is reachable by name, so hosts that only read
//! trees (the LSP wasm rides `rut-lsp`) can be checked to
//! never drive it directly.

// the `arenas!` inventory munches one expansion per node kind (~63)
// to stamp each `NodeKind` id — the default limit (128) is too tight
#![recursion_limit = "256"]

mod ast;
pub mod parser;

pub use ast::*;

// the parse lane's construction macros, crate-root reachable for the
// `use crate::slot_alloc;` house style — never public
pub(crate) use ast::{plain_alloc, slot_alloc, slot_any, slot_span};
