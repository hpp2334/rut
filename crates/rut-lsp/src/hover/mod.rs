//! Hover — a definition index over the parsed document, plus lookup and
//! markdown rendering. One rule for member lookup (mirrors the
//! compiler's): **members on type `T` = T's own surface (class body,
//! `builtin`) ∪ inherent `impl T { .. }` methods; on an interface `I`
//! they are exactly `I`'s declared signatures — use-gated when `I` is
//! foreign (satisfaction is structural, so no impl joins the two).**
//! Heuristic, like the classifier: a
//! miss is an empty hover, never wrong text. Signatures render as
//! verbatim source slices — no pretty-printer, truthful to what was
//! written.
//!
//! Two layers (the lsp-features survey §3.2 design): the **decl layer**
//! (the mounted files' decl maps — types, fns, fields, enum members,
//! module lets, use names, all keyed on the nodes' own recorded name
//! spans; docs ride the decl nodes' `DocData` chains) and the
//! **binding pass** (`bindings` — every fn-local binding as
//! `{ name, decl_ident_span, scope_span, kind, ty }`, shadow-correct
//! resolve). `infer` is the display-side type heuristic both consume.

//!
//! Layout: `types` (the shared vocabulary — Mount, TyForm, type text)
//! → `collection` (MountedFile + DefIndexCollection, the views and the
//! doors) → `bindings` + `lookup` + `infer` (what's under the cursor)
//! → `render` (markdown).

pub mod bindings;
pub(crate) mod collection;
pub(crate) mod infer;
pub(crate) mod lookup;
mod render;
pub(crate) mod types;

pub use bindings::{BindKind, Binding};
pub(crate) use collection::{DefIndexCollection, MountedFile};
pub use lookup::hover;
pub(crate) use render::{binding_markdown, render_let};
pub use types::Mount;
