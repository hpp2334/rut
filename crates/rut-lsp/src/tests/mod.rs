// the unit-test tree — ONE module per file. The sealed `semantic`
// module's types only spell inside the crate, so the tests that drive
// Document/LineIndex live here; the ABI-level tests that see only the
// public surface stay in `tests/`.

mod analysis;
mod classify;
mod completion;
mod corpus;
mod definition;
mod deps;
mod doc_sync;
mod hover;
mod inlay;
mod line_index;
mod mods;
mod prelude;
mod references;
mod signature_help;
