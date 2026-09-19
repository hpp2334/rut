//! rut-lsp — the rut language server (the M6 LSP slice, landed early —
//! RFC 0041 §2): **semantic tokens** (grammar highlighting), pushed
//! **diagnostics** (lexer + parser), **document symbols**, **hover**,
//! and **completions**, built on rut-lexer/rut-ast/rut-parser. One
//! server, every editor that speaks LSP (VS Code via
//! `integrations/vscode-extension`; Neovim / Helix / Zed / Emacs /
//! Sublime configs in `integrations/README.md`).
//!
//! Layout: `line_index` (byte spans ⇄ UTF-16 positions over the
//! normalized source), `semantic` (the pure classifier — `legend`,
//! `tokens`, `names`, `recover`, `symbols`), `analysis` (one pure pass
//! over a document → LSP values, plus the document-level hover /
//! completion queries shared by both faces), `std_surface` (the
//! embedded std + its index), `hover` (the definition index + lookup —
//! `types`, `build`, `lookup`, `infer`, `render`), `completion` (the
//! same index, member + bare completion), `server` (the tower-lsp
//! service — open doc + the embedded std surface + a workspace scan;
//! feature `server`, native only). The wasm shim (`rut-lsp-wasm`) wraps
//! the same pure core over a raw ABI — no server process.

pub mod analysis;
pub mod completion;
pub mod hover;
pub mod line_index;
pub mod semantic;
pub mod std_surface;
#[cfg(feature = "server")]
pub mod server;
