//! rut-ast — the tree and its grammar, one crate.
//!
//! Three modules, one law each:
//! - `ast` (SEALED `mod`) — PURE STRUCTURE: the typed-arena AST.
//!   The module is private — the same privacy wall as rut-lsp's
//!   `semantic`: consumers get the flat re-exports at the crate
//!   root and `parser::`, never an `ast::` module path. `Ast` is a
//!   CAPSULE whose mut doors are `pub(crate)` — only the in-crate
//!   pipeline may build or repair a tree; every other consumer reads.
//!   The artifact is the [`Doc`] (law v4): the tree holds no text,
//!   `Doc` owns the normalized text every span indexes, and
//!   `Doc::text(span)` is the join.
//! - [`parser`] — the grammar: the explicit-frame machine that
//!   turns text into that tree. Its public face is the entry pair
//!   (`parse` fail-fast / `parse_with_recover` always a tree), both
//!   wrappers over [`Doc::parse`]; the ONE engine entry
//!   (`parse_item_window`) is crate-internal — the `Doc` edit lane
//!   rides it with the dirty tail.
//! - [`Doc`] — THE document pipeline: text + tokens + tree + the one
//!   mutator (`Doc::edit`). Cold parse = window reparse of
//!   everything; an edit = token splice → dirty root item → window
//!   reparse into fresh slots. Edit ≡ cold, property-tested.
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
mod bindings;
mod document;
mod names;
pub mod parser;

pub use ast::*;
pub use bindings::{expr_children, resolve as resolve_binding, BindKind, Binding};
pub use document::Doc;
pub use names::name_positions;

// the parse lane's construction macros, crate-root reachable for the
// `use crate::slot_alloc;` house style — never public
pub(crate) use ast::{plain_alloc, slot_alloc, slot_any, slot_span};
// the edit lane's dispatch (the Doc pipeline's subtree walks)
pub(crate) use ast::{children_of, remove_any};
