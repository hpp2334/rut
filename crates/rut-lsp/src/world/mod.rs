//! The mounted world — the session's accumulated language facts, read
//! through two layers (the lsp-features survey §3.2 design): the
//! **decl layer** (the mounted files' decl maps — types, fns, fields,
//! enum members, module lets, use names, all keyed on the nodes' own
//! recorded name spans; docs ride the decl nodes' `DocData` chains)
//! and the **binding pass** (`bindings` — every fn-local binding as
//! `{ name, decl_ident_span, scope_span, kind, ty }`, shadow-correct
//! resolve). `infer` is the display-side type heuristic both consume.
//! Heuristic, like the classifier: a miss is an empty answer, never
//! wrong text.
//!
//! Layout: `types` (the shared vocabulary — Mount, TyForm, type text)
//! → `collection` (MountedFile + DefIndexCollection in its `mod.rs`;
//! the member-law resolver family in `member`; completion's tier
//! doors in `tiers`; the per-node views in `views/`) → `bindings` +
//! `infer` (what's under the cursor) → `mods` (the file-module
//! namespace walk) + `pkg` (package identity and the doc's import
//! table). The queries above spell this layer — never the reverse.

pub mod bindings;
pub(crate) mod collection;
pub(crate) mod infer;
pub(crate) mod mods;
pub(crate) mod pkg;
pub(crate) mod types;

pub use bindings::{BindKind, Binding};
pub(crate) use collection::{DefIndexCollection, MountedFile};
pub use types::Mount;
