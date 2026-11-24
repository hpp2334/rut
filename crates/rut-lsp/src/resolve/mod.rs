//! The shared mod laws — the LSP's adapters over rut-lir's pure walks.
//! Every language law here has ONE home (a `rut_lir::check` function);
//! this module only translates the session's world into the tree the
//! laws walk and calls them — no walk is spelled twice. Act 3 adds the
//! rest of the module (the caret's resolution).

pub(crate) mod mods;
