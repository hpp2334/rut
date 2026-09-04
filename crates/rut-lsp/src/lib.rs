//! rut-lsp — the rut language server (the M6 LSP slice, landed early —
//! RFC 0041 §2): **semantic tokens** (grammar highlighting), pushed
//! **diagnostics** (lexer + parser), and **document symbols**, built on
//! rut-lexer/rut-ast/rut-parser. One server, every editor that speaks LSP
//! (VS Code via `integrations/vscode-extension`; Neovim / Helix / Zed /
//! Emacs / Sublime configs in `integrations/README.md`).
//!
//! Layout: `line_index` (byte spans ⇄ UTF-16 positions over the
//! normalized source), `semantic` (the pure classifier — legend, keyword
//! set, name-token recovery, symbol builder), `analysis` (one pure pass
//! over a document → LSP values), `server` (the tower-lsp service).

pub mod analysis;
pub mod line_index;
pub mod semantic;
pub mod server;
