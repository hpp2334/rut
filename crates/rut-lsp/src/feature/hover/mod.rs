//! Hover — a definition index over the parsed document, plus lookup
//! and markdown rendering. One rule for member lookup (mirrors the
//! compiler's): **members on type `T` = the members declared in T's
//! own body (class, `builtin`) ∪ inherent `impl T { .. }` methods; on
//! an interface `I` they are exactly `I`'s declared signatures —
//! use-gated when `I` is foreign (satisfaction is structural, so no
//! impl joins the two).**
//! Heuristic, like the classifier: a miss is an empty hover, never
//! wrong text. Signatures render as verbatim source slices — no
//! pretty-printer, truthful to what was written. The layers the
//! answers read — the decl layer and the binding pass — live in
//! [`crate::world`].

pub(crate) mod lookup;
mod render;

pub use lookup::hover;
pub(crate) use render::{binding_markdown, render_let};
