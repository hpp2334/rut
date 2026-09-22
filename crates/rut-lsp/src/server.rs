//! The LSP service — `Backend` over tower-lsp-server: full-document sync,
//! semantic tokens (full), document symbols, hover, pushed diagnostics.
//! One server, every editor that speaks LSP (VS Code via
//! `integrations/vscode-extension`; Neovim / Helix / Zed / Emacs / Sublime
//! configs in `integrations/README.md`). Hover/completion resolve against
//! the open document first, then the embedded std surface
//! (`std_surface`, RFC 0028/0029), then the workspace's rut files. The
//! wasm shim (`rut-lsp-wasm`) drives the same queries without this
//! process.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, RwLock};

use ls_types::request::{GotoTypeDefinitionParams, GotoTypeDefinitionResponse};
use ls_types::*;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::{Client, LanguageServer};

use crate::analysis::{self, Analysis};
use crate::hover::DefIndex;
use crate::semantic::TokenType;
use crate::std_surface;

#[derive(Debug)]
pub struct Backend {
    client: Client,
    documents: Arc<RwLock<HashMap<Uri, String>>>,
    /// std surface + workspace files, in lookup order
    defs: Arc<RwLock<Vec<DefIndex>>>,
    /// the workspace root (initialize's `root_uri`) — relative
    /// definition targets (the std surface's true `rut/...` paths)
    /// resolve against it
    root: Arc<RwLock<Option<PathBuf>>>,
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
    let uri = path.to_str()?;
    Some(analysis::index_at(uri, &src))
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Backend {
            client,
            documents: Arc::new(RwLock::new(HashMap::new())),
            defs: Arc::new(RwLock::new(std_surface::indexes())),
            root: Arc::new(RwLock::new(None)),
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
        let root_slot = self.root.clone();
        #[allow(deprecated)] // root_uri: VS Code still sends it first
        let root = params.root_uri.as_ref().map(|u| u.as_str().to_string());
        tokio::task::spawn_blocking(move || {
            let Some(root) = root else { return };
            let Ok(u) = Uri::from_str(&root) else { return };
            let Some(cow) = u.to_file_path() else { return };
            let path = cow.into_owned();
            *root_slot.write().unwrap() = Some(path.clone());
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
                definition_provider: Some(OneOf::Left(true)),
                type_definition_provider: Some(TypeDefinitionProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![".".to_string()]),
                    ..Default::default()
                }),
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
        let p = params.text_document_position_params.position;
        let out = {
            let defs = self.defs.read().unwrap();
            analysis::hover_at(uri.as_str(), &text, &defs, p.line, p.character)
        };
        Ok(out)
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let Some(text) = self.get(&uri) else { return Ok(None) };
        let p = params.text_document_position.position;
        let items = {
            let defs = self.defs.read().unwrap();
            analysis::complete_at(uri.as_str(), &text, &defs, p.line, p.character)
        };
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let Some(text) = self.get(&uri) else { return Ok(None) };
        let p = params.text_document_position_params.position;
        let locs = {
            let defs = self.defs.read().unwrap();
            analysis::definition_at(uri.as_str(), &text, &defs, p.line, p.character)
        };
        Ok(self.locations(locs))
    }

    async fn goto_type_definition(
        &self,
        params: GotoTypeDefinitionParams,
    ) -> Result<Option<GotoTypeDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let Some(text) = self.get(&uri) else { return Ok(None) };
        let p = params.text_document_position_params.position;
        let locs = {
            let defs = self.defs.read().unwrap();
            analysis::type_definition_at(uri.as_str(), &text, &defs, p.line, p.character)
        };
        Ok(self.locations(locs).map(|r| match r {
            GotoDefinitionResponse::Array(v) => GotoTypeDefinitionResponse::Array(v),
            other => GotoTypeDefinitionResponse::Scalar(match other {
                GotoDefinitionResponse::Scalar(l) => l,
                _ => unreachable!("locations() only builds Array/Scalar"),
            }),
        }))
    }
}

impl Backend {
    /// face-agnostic definition targets -> LSP locations: relative
    /// targets (the std surface's true `rut/...` paths) resolve against
    /// the workspace root; targets that resolve nowhere stay relative
    /// (the client may still know the file — never a wrong jump)
    fn locations(&self, locs: Vec<crate::definition::DefLocation>) -> Option<GotoDefinitionResponse> {
        if locs.is_empty() {
            return None;
        }
        let root = self.root.read().unwrap().clone();
        let out: Vec<Location> = locs
            .into_iter()
            .filter_map(|l| {
                Some(Location {
                    uri: resolve_target_uri(&l.uri, root.as_deref())?,
                    range: l.range,
                })
            })
            .collect();
        if out.is_empty() {
            return None;
        }
        Some(GotoDefinitionResponse::Array(out))
    }
}

/// a definition target string -> a URI: a real URI passes through, an
/// absolute path becomes one, a relative path needs the workspace root
fn resolve_target_uri(raw: &str, root: Option<&Path>) -> Option<Uri> {
    // scheme = a letter/letter-digit run followed by `:` (RFC 3986) —
    // checked on the raw string; ls-types' parser is strict about the rest
    let has_scheme = raw
        .split(':')
        .next()
        .is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
        && raw.contains(':');
    if has_scheme {
        if let Ok(u) = Uri::from_str(raw) {
            return Some(u);
        }
    }
    let p = Path::new(raw);
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.map(|r| r.join(p))?
    };
    Uri::from_str(&format!("file://{}", joined.to_str()?)).ok()
}
