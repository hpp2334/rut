//! The LSP service — `Backend` over tower-lsp-server: full-document sync,
//! semantic tokens (full), document symbols, hover, pushed diagnostics.
//! One server, every editor that speaks LSP (VS Code via
//! `integrations/vscode-extension`; Neovim / Helix / Zed / Emacs / Sublime
//! configs in `integrations/README.md`). Hover resolves against the open
//! document first, then the embedded std surface (`std/*.d.rut`,
//! RFC 0028/0029), then a scan of the workspace's rut files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, RwLock};

use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::*;
use tower_lsp_server::{Client, LanguageServer};

use crate::analysis::{self, Analysis};
use crate::hover::{self, DefIndex};
use crate::semantic::TokenType;

/// the toolchain's std surface — embedded, indexed before the workspace
/// (one directory per module: `rut/std-collection/rut.toml` names
/// `"std:collection"`)
pub mod std_surface {
    pub const CORE: &str = include_str!("../../../rut/std-core/core.d.rut");
    pub const MATH: &str = include_str!("../../../rut/std-math/math.d.rut");
    pub const COLLECTION: &str = include_str!("../../../rut/std-collection/collection.d.rut");
}

fn std_indexes() -> Vec<DefIndex> {
    [(CORE_LABEL, std_surface::CORE), ("std:math", std_surface::MATH), ("std:collection", std_surface::COLLECTION)]
        .into_iter()
        .map(|(origin, src)| {
            let src = rut_lexer::lexer::normalize(src);
            let (ast, _) = rut_parser::parse(&src, rut_parser::Mode::Decl);
            let mut idx = hover::index(&src, &ast);
            idx.origin = origin.to_string();
            idx
        })
        .collect()
}

const CORE_LABEL: &str = "std:core";

#[derive(Debug)]
pub struct Backend {
    client: Client,
    documents: Arc<RwLock<HashMap<Uri, String>>>,
    /// std surface + workspace files, in lookup order
    defs: Arc<RwLock<Vec<DefIndex>>>,
}

/// walk `root` for rut files; skip build/dependency trees and dot-dirs,
/// cap the count (an LSP is a guest, not an indexer daemon)
fn collect_rut_files(root: &Path) -> Vec<PathBuf> {
    const MAX_FILES: usize = 500;
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if out.len() >= MAX_FILES {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if p.is_dir() {
                if name.starts_with('.') || name == "target" || name == "node_modules" {
                    continue;
                }
                stack.push(p);
            } else if name.ends_with(".rut") {
                if out.len() >= MAX_FILES {
                    break;
                }
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn index_file(path: &Path) -> Option<DefIndex> {
    let src = std::fs::read_to_string(path).ok()?;
    let mode = analysis::mode_of(path.to_str().unwrap_or(""));
    let src = rut_lexer::lexer::normalize(&src);
    let (ast, _) = rut_parser::parse(&src, mode);
    let mut idx = hover::index(&src, &ast);
    idx.origin = path.display().to_string();
    Some(idx)
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Backend {
            client,
            documents: Arc::new(RwLock::new(HashMap::new())),
            defs: Arc::new(RwLock::new(std_indexes())),
        }
    }

    fn get(&self, uri: &Uri) -> Option<String> {
        self.documents.read().unwrap().get(uri).cloned()
    }

    fn set(&self, uri: Uri, text: String) {
        self.documents.write().unwrap().insert(uri, text);
    }

    fn analyze_uri(&self, uri: &Uri) -> Option<Analysis> {
        self.get(uri)
            .map(|text| analysis::analyze_at(uri, &text, analysis::mode_of(uri.as_str())))
    }

    async fn publish(&self, uri: Uri, version: Option<i32>) {
        // empty list on failure/fix — diagnostics must always publish, or
        // stale squiggles never clear
        let diags = self
            .analyze_uri(&uri)
            .map(|a| a.diags)
            .unwrap_or_default();
        self.client.publish_diagnostics(uri, diags, version).await;
    }
}

impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // workspace scan off the hot initialize path — hover may briefly
        // resolve against std + open docs only
        let defs = self.defs.clone();
        #[allow(deprecated)] // root_uri: VS Code still sends it first
        let root = params.root_uri.as_ref().map(|u| u.as_str().to_string());
        tokio::task::spawn_blocking(move || {
            let Some(root) = root else { return };
            let Ok(u) = Uri::from_str(&root) else { return };
            let Some(cow) = u.to_file_path() else { return };
            let path = cow.into_owned();
            let mut guard = defs.write().unwrap();
            for f in collect_rut_files(&path) {
                if let Some(idx) = index_file(&f) {
                    guard.push(idx);
                }
            }
        });
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::FULL),
                        ..Default::default()
                    },
                )),
                semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
                    SemanticTokensOptions {
                        legend: SemanticTokensLegend {
                            token_types: TokenType::legend()
                                .into_iter()
                                .map(SemanticTokenType::from)
                                .collect(),
                            token_modifiers: vec![],
                        },
                        full: Some(SemanticTokensFullOptions::Bool(true)),
                        range: Some(false),
                        ..Default::default()
                    },
                )),
                document_symbol_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "rut-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            offset_encoding: Some("utf-16".to_string()),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "rut-lsp ready")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let td = params.text_document;
        let uri = td.uri.clone();
        self.set(uri.clone(), td.text);
        self.publish(uri, Some(td.version)).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let version = params.text_document.version;
        // FULL sync: the last full-text change wins
        if let Some(change) = params.content_changes.into_iter().last() {
            self.set(uri.clone(), change.text);
        }
        self.publish(uri, Some(version)).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        self.documents.write().unwrap().remove(&uri);
        self.client.publish_diagnostics(uri, vec![], None).await;
    }

    async fn semantic_tokens_full(&self, params: SemanticTokensParams) -> Result<Option<SemanticTokensResult>> {
        match self.analyze_uri(&params.text_document.uri) {
            Some(a) => Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
                result_id: None,
                data: a.tokens,
            }))),
            None => Ok(None),
        }
    }

    async fn document_symbol(&self, params: DocumentSymbolParams) -> Result<Option<DocumentSymbolResponse>> {
        match self.analyze_uri(&params.text_document.uri) {
            Some(a) => Ok(Some(DocumentSymbolResponse::Nested(a.symbols))),
            None => Ok(None),
        }
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let Some(text) = self.get(&uri) else { return Ok(None) };
        let mode = analysis::mode_of(uri.as_str());
        let src = rut_lexer::lexer::normalize(&text);
        let (toks, _) = rut_lexer::lexer::lex(&src);
        let (ast, _) = rut_parser::parse(&src, mode);
        let mut doc = hover::index(&src, &ast);
        doc.origin = uri.as_str().to_string();
        let std_ws = self.defs.read().unwrap();
        let idxs: Vec<&DefIndex> = std::iter::once(&doc).chain(std_ws.iter()).collect();
        let pos = {
            let p = params.text_document_position_params.position;
            let index = crate::line_index::LineIndex::new(&src);
            index.byte(&src, p.line, p.character)
        };
        let out = hover::hover(&idxs, &src, &toks, &ast, pos);
        Ok(out.map(|h| Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: h.markdown,
            }),
            range: Some(analysis::range_of(&crate::line_index::LineIndex::new(&src), &src, h.span)),
        }))
    }
}
