//! rut-lsp — the rut language CORE (the M6 LSP slice, landed early):
//! **semantic tokens** (grammar highlighting), **diagnostics** (lexer +
//! parser + the checker), **document symbols**, **hover**, **completion**,
//! **definition**, built on rut-lexer/rut-ast/rut-parser. This crate is
//! the queries and the values; it owns no transport. Two shells serve it:
//! `rut-lsp-server` (stdio — the `rut-lsp` binary every editor that
//! speaks LSP spawns; Neovim / Helix / Zed / Emacs / Sublime configs in
//! `integrations/README.md`) and `rut-lsp-wasm` (the in-process module
//! the VS Code extension and the demo playground bundle). Both call the
//! same entry points here, so they cannot drift.
//! Layout: `line_index` (byte spans ⇄ UTF-16 positions over the
//! normalized source), `semantic` (the pure classifier — it lives in
//! `rut-semantic` and is re-exported here; `legend`, `tokens`, `names`,
//! `recover`, `symbols`),
//! `analysis` (one pure pass
//! over a document → LSP values, plus the document-level hover /
//! completion / definition / inlay / references / signature-help
//! queries shared by both faces),
//! `std_surface` (the embedded std + its index), `deps` (the
//! manifest/bundle bridge — `rut.jsonc` dep tables, `.rutbundle`
//! sources, dep indexes named by module), `hover` (the
//! definition index + lookup — `types`, `build`, `lookup`, `infer`,
//! `render`), `completion` (the same index, member + bare completion),
//! `definition` (go-to-def + go-to-type-def — the survey §3.3 layers),
//! `inlay` (the inline inference display — type hints on unannotated
//! bindings, param-name hints at exact-arity call sites),
//! `references` (the same index read BACKWARDS — a token is a reference
//! iff definition from it lands on the target; shadow-aware, use-graph
//! reverse edges, `includeDeclaration`),
//! `signature_help` (the phase-3 callee machinery at a call's argument
//! position — verbatim signature, depth-computed active parameter,
//! ambiguity/mismatch → no help),
//! `check` (the semantic diagnostics lane — "check package → per-file
//! diagnostics" over the driver's graph walk, the checker's type errors
//! mapped back through the chain's file stamps and merged with the
//! parse set; parse-only when no package graph assembles). No module here
//! touches a socket, a stdio handle, or a string of JSON on its way out —
//! shaping the values for a wire is the shell's job.

pub mod analysis;
pub mod check;
pub mod completion;
pub mod definition;
pub mod deps;
pub mod hover;
pub mod inlay;
pub mod line_index;
pub mod mods;
pub mod references;
pub mod semantic;
pub mod signature_help;
pub mod std_surface;
