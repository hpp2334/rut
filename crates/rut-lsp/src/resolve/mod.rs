//! The resolver layer — the one classification of a caret
//! ([`cursor`]), the one walk from a caret to declarations
//! ([`walk::resolve`]) answering the typed [`Def`], and the adapters
//! that carry the world's stamped files into rut-lir's pure mod laws
//! ([`mods`]). Every language law has ONE home; the features above
//! (hover, definition, references) classify, resolve, and render —
//! they never walk on their own.

pub(crate) mod cursor;
pub(crate) mod def;
pub(crate) mod mods;
pub(crate) mod walk;

pub(crate) use def::{Def, DefLocation, View};
