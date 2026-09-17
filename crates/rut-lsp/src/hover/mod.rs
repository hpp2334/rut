//! Hover — a definition index over the parsed document, plus lookup and
//! markdown rendering. One rule for method lookup (mirrors the
//! compiler's): **methods on type `T` = T's own surface (class body,
//! `builtin`) ∪ inherent `impl T { .. }` methods ∪ use-gated trait
//! methods from impls targeting `T`.** Heuristic, like the classifier: a
//! miss is an empty hover, never wrong text. Signatures render as
//! verbatim source slices — no pretty-printer, truthful to what was
//! written.

//!
//! Layout: `types` (the index data) → `build` (the index pass over a
//! parsed document) → `lookup` + `infer` (what's under the cursor) →
//! `render` (markdown).

mod build;
mod infer;
pub(crate) mod lookup;
mod render;
pub(crate) mod types;

pub use build::index;
pub use lookup::{hover, HoverOut};
pub use types::{DefIndex, FnDef, ImplDef, MemberSrc, TyDef, TyForm};

#[cfg(test)]
mod tests;
