//! rut-lsp — the rut language server (the M6 LSP slice, landed early —
//! RFC 0041 §2): **semantic tokens** (grammar highlighting), pushed
//! **diagnostics** (lexer + parser), **document symbols**, **hover**,
//! and **completions**, built on rut-lexer/rut-ast/rut-parser. One
//! server, every editor that speaks LSP (VS Code via
//! `integrations/vscode-extension`; Neovim / Helix / Zed / Emacs /
//! Sublime configs in `integrations/README.md`).
//! Layout: `line_index` (byte spans ⇄ UTF-16 positions over the
//! normalized source), `semantic` (the pure classifier — `legend`,
//! `tokens`, `names`, `recover`, `symbols`),
//! `analysis` (one pure pass
//! over a document → LSP values, plus the document-level hover /
//! completion / definition / inlay / references / signature-help
//! queries shared by both faces),
//! `std_surface` (the embedded std + its index), `hover` (the
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
//! `server` (the
//! tower-lsp service — open doc + the embedded std surface + a
//! workspace scan; feature `server`, native only). The wasm shim
//! (`rut-lsp-wasm`) wraps the same pure core over a raw ABI — no server
//! process.

pub mod analysis;
pub mod completion;
pub mod definition;
pub mod hover;
pub mod inlay;
pub mod line_index;
pub mod references;
pub mod semantic;
pub mod signature_help;
pub mod std_surface;
// feature `server` AND native — the deps are target-gated to
// `cfg(not(target_arch = "wasm32"))` (tokio rejects `io-std` on wasm),
// so on wasm32 the feature resolves to an empty set
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub mod server;
