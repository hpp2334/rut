//! Hover — a definition index over the parsed document, plus lookup and
//! markdown rendering. One rule for method lookup (mirrors the
//! compiler's): **methods on type `T` = T's own surface (class body,
//! `builtin`) ∪ inherent `impl T { .. }` methods ∪ use-gated trait
//! methods from impls targeting `T`.** Heuristic, like the classifier: a
//! miss is an empty hover, never wrong text. Signatures render as
//! verbatim source slices — no pretty-printer, truthful to what was
//! written.
//!
//! Two layers (the lsp-features survey §3.2 design): the **decl layer**
//! (`build` — types, fns, fields, enum members, module lets, use names,
//! all with token-recovered name spans) and the **binding pass**
//! (`bindings` — every fn-local binding as
//! `{ name, decl_ident_span, scope_span, kind, ty }`, shadow-correct
//! resolve). `infer` is the display-side type heuristic both consume.

//!
//! Layout: `types` (the index data) → `build` (the index pass over a
//! parsed document) → `bindings` + `lookup` + `infer` (what's under the
//! cursor) → `render` (markdown).

mod build;
pub(crate) mod bindings;
pub(crate) mod infer;
pub(crate) mod lookup;
mod render;
pub(crate) mod types;

pub use build::index;
pub use bindings::{Binding, BindKind};
pub use lookup::{hover, HoverOut};
pub(crate) use render::{binding_markdown, render_let};
pub use types::{DefIndex, FnDef, ImplDef, LetDef, MemberSrc, TyDef, TyForm, UseDef};

#[cfg(test)]
mod tests;
