//! The LSP service — `Backend` over tower-lsp-server: full-document sync,
//! semantic tokens (full), document symbols, hover, inlay hints, pushed
//! diagnostics.
//! One server, every editor that speaks LSP (VS Code via
//! `integrations/vscode-extension`; Neovim / Helix / Zed / Emacs / Sublime
//! configs in `integrations/README.md`). Hover/completion resolve against
//! the open document first, then the embedded std surface
//! (`std_surface`), then the workspace's rut files, then the manifest
//! deps (see below). The wasm shim (`rut-lsp-wasm`) drives the same
//! queries without this process.
//!
//! # Deps — cache-only by law
//!
//! After the workspace scan (initialize's `root_uri` walk) the server
//! reads the workspace root's `rut.jsonc` via `rut_lsp::deps::dep_table`
//! — the SAME pure engine the wasm face and the extension ride, so the
//! rules can't drift: path deps resolve against the manifest dir
//! (outside-workspace dirs included — the server has fs) and recurse
//! cycle-safe under the depth/file caps, `deps` at any depth, the root's
//! `dev-deps` too, `peer-deps` only when locally resolvable. Every dep
//! source indexes through `rut_lsp::deps::index_dep` into the same defs
//! store the scan fills.
//!
//! Url deps are CACHE-ONLY here: the bytes are read from the dep table's
//! Rust-computed `.rut/cache/<sha256(url)>.rutbundle` path with the
//! sha256 pin verified at the mount door; a miss or an unpinned row is
//! ONE `window/showMessage` (info) naming the dep and the fetch
//! alternative. This server face NEVER touches the network — fetching is
//! the extension's job (`rut fetch` / the VS Code extension warm the
//! same shared cache). The asymmetry with the extension is deliberate.
//!
//! The server registers no file watchers today. When a client wires
//! `workspace/didChangeWatchedFiles` for `**/rut.jsonc` +
//! `.rut/cache/**`, its handler should re-run
//! [`walk_workspace_deps`] over the root — re-indexing is
//! replace-by-origin, so the re-walk is idempotent. No new server state
//! machine rides here.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, RwLock};

use ls_types::request::{GotoTypeDefinitionParams, GotoTypeDefinitionResponse};
use ls_types::*;
use rut_driver::bundle::files::MANIFEST_NAME;
use rut_parser::Mode;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::{Client, LanguageServer};

use crate::analysis::{self, Analysis};
use crate::deps::{self, DepSource};
use crate::hover::DefIndex;
use crate::mods::DocMods;
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
    /// per-file file-module context (the nearest manifest dir, the
    /// mod path relative to it) — the position-path completion's doc
    /// side, resolved on the fs once per file
    doc_mods: Arc<RwLock<HashMap<PathBuf, DocMods>>>,
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

// ---- the dep walk (the server face of `rut_lsp::deps`) ----

/// the dep walk's budget beside the 500-file workspace cap — the same
/// numbers the extension rides
const MAX_DEP_DEPTH: usize = 8;
const MAX_DEP_FILES: usize = 200;

/// one user-facing hint the walk produced — surfaced as
/// `window/showMessage`, never silent, never spammy (once per reason per
/// walk)
type DepHint = (MessageType, String);

/// walk the workspace root's manifest deps into `defs`: path deps
/// recurse against the manifest dir, url deps mount from `.rut/cache/`
/// (cache-only — see the module doc). Returns the hints to show.
fn walk_workspace_deps(root: &Path, defs: &mut Vec<DefIndex>) -> Vec<DepHint> {
    let mut w = Walk {
        root: root.to_path_buf(),
        defs,
        seen: HashSet::new(),
        files: MAX_DEP_FILES,
        hints: Vec::new(),
        hint_keys: HashSet::new(),
    };
    w.manifest(root, 1);
    w.hints
}

struct Walk<'a> {
    /// the workspace root — `.rut/cache/` resolves against it
    root: PathBuf,
    defs: &'a mut Vec<DefIndex>,
    /// resolved manifest dirs — the cycle break
    seen: HashSet<PathBuf>,
    /// remaining dep-source-file budget
    files: usize,
    hints: Vec<DepHint>,
    /// keys already hinted — once per reason per walk
    hint_keys: HashSet<String>,
}

impl<'a> Walk<'a> {
    fn hint_once(&mut self, key: String, ty: MessageType, msg: String) {
        if self.hint_keys.insert(key) {
            self.hints.push((ty, msg));
        }
    }

    /// replace-by-origin — the same store discipline every face rides
    fn insert(&mut self, idx: DefIndex) {
        match self.defs.iter().position(|d| d.origin == idx.origin) {
            Some(slot) => self.defs[slot] = idx,
            None => self.defs.push(idx),
        }
    }

    /// one manifest: its own entry sources index under the module name
    /// (the uri is the file's real path so F12 jumps land), then its
    /// mod tree mounts (see [`Walk::mount_mod_tree`]), then its dep
    /// rows walk — `deps` at any depth, `dev-deps` only at the root
    /// (the loader's law: a dep's dev table never enters a consumer's
    /// world), `peer-deps` only when locally resolvable (the consumer
    /// supplies them; an unresolvable peer is the normal state, not a
    /// failure).
    fn manifest(&mut self, dir: &Path, depth: usize) {
        if depth > MAX_DEP_DEPTH || self.files == 0 {
            return;
        }
        let dir = normalize_path(dir);
        if !self.seen.insert(dir.clone()) {
            return;
        }
        let Ok(text) = std::fs::read_to_string(dir.join(MANIFEST_NAME)) else {
            return; // no manifest here — nothing to parse, nothing to hint
        };
        let table = match deps::dep_table(&text) {
            Ok(t) => t,
            Err(e) => {
                self.hint_once(
                    format!("manifest:{}", dir.display()),
                    MessageType::ERROR,
                    format!("rut: {MANIFEST_NAME} — {e}"),
                );
                return;
            }
        };
        let module = table.name.clone().unwrap_or_else(|| {
            dir.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        for e in &table.entries {
            if self.files == 0 {
                return;
            }
            let src_path = dir.join(&e.path);
            let Ok(src) = std::fs::read_to_string(&src_path) else {
                continue; // a manifest naming a missing entry — index what exists
            };
            self.files -= 1;
            let uri = src_path.to_string_lossy().into_owned();
            // namespace/consts have no voice on a path-dep surface (the
            // same rule `rut_add_def_named` speaks) — a dep needing them
            // mounts through a bundle
            let idx = deps::index_dep(&module, &uri, &src, e.mode, None, &[]);
            self.insert(idx);
        }
        // the mod tree — the pkg's file modules index under the same
        // name with their mod paths stamped (the editor face of the
        // loader's mount)
        let impl_entries: Vec<String> = table
            .entries
            .iter()
            .filter(|e| e.mode == Mode::Impl)
            .map(|e| e.path.clone())
            .collect();
        self.mount_mod_tree(&dir, &module, &impl_entries);
        let at_root = depth == 1;
        let rows = table
            .deps
            .iter()
            .map(|r| (r, false))
            .chain(table.dev_deps.iter().filter(|_| at_root).map(|r| (r, false)))
            .chain(table.peer_deps.iter().map(|r| (r, true)));
        for (row, peer) in rows {
            if self.files == 0 {
                return;
            }
            match &row.source {
                DepSource::Url { url, sha256, cache_path } => {
                    self.mount_url(&row.name, url, sha256.as_deref(), cache_path);
                }
                DepSource::Path { dir: rel } => {
                    let dep_dir = normalize_path(&dir.join(rel));
                    let dep_manifest = dep_dir.join(MANIFEST_NAME);
                    if !dep_manifest.is_file() {
                        if !peer {
                            self.hint_once(
                                format!("path:{}:{}", dir.display(), row.name),
                                MessageType::ERROR,
                                format!(
                                    "rut: dep '{}' — {rel} has no {MANIFEST_NAME}; \
                                     a path dep is a module directory (one manifest per module)",
                                    row.name
                                ),
                            );
                        }
                        continue;
                    }
                    // the optional peer's integration group (`lib` — an
                    // impl-only source in the OWNER's directory) is the
                    // owner's surface gated on the peer's presence: index
                    // it under the owner's module name, only when the
                    // peer resolved
                    if peer && row.optional {
                        if let Some(lib) = &row.lib {
                            let lib_path = dir.join(lib);
                            if let Ok(src) = std::fs::read_to_string(&lib_path) {
                                if self.files > 0 {
                                    self.files -= 1;
                                    let uri = lib_path.to_string_lossy().into_owned();
                                    let idx = deps::index_dep(
                                        &module,
                                        &uri,
                                        &src,
                                        analysis::mode_of(&uri),
                                        None,
                                        &[],
                                    );
                                    self.insert(idx);
                                }
                            }
                        }
                    }
                    self.manifest(&dep_dir, depth + 1);
                }
            }
        }
    }

    /// one url row — the CACHE-ONLY mount door. The pin is law: only
    /// pinned rows are even considered, only pin-verified bytes mount.
    /// The server cannot fetch (the deliberate asymmetry — the
    /// extension's job), so a miss/poisoned cache is ONE info hint
    /// naming the CLI/extension alternative, never silent, never network.
    fn mount_url(&mut self, name: &str, url: &str, pin: Option<&str>, cache_path: &str) {
        let Some(pin) = pin else {
            self.hint_once(
                format!("unpinned:{url}"),
                MessageType::INFO,
                format!(
                    "rut: dep '{name}' has no sha256 pin — the editor never mounts unpinned \
                     url bytes; pin the bundle's hash beside its url in {MANIFEST_NAME}"
                ),
            );
            return;
        };
        let bytes = match std::fs::read(self.root.join(cache_path)) {
            Ok(bytes) => {
                if hex_sha256(&bytes) != pin {
                    // poisoned entry: the pin is checked at the mount door
                    // on every load — the cache-only face cannot refetch to
                    // heal, so the bytes stay out and the hint names the
                    // re-warm
                    self.hint_once(
                        format!("pin:{url}"),
                        MessageType::INFO,
                        format!(
                            "rut: dep '{name}' — sha256 pin mismatch in .rut/cache for {url}: \
                             delete the cached file and run `rut fetch` to re-warm the cache"
                        ),
                    );
                    return;
                }
                bytes
            }
            Err(_) => {
                self.hint_once(
                    format!("fetch:{url}"),
                    MessageType::INFO,
                    format!(
                        "rut: dep '{name}' ({url}) is not in the local .rut/cache — the \
                         language server never touches the network; run `rut fetch` in the \
                         project (or let the extension fetch it), then reload"
                    ),
                );
                return;
            }
        };
        match deps::bundle_sources(&bytes) {
            Ok(b) => {
                for f in &b.files {
                    let uri = format!("bundle:{}/{}", b.module, f.path);
                    // a mounted mod child rides as `<path>/mod.rut` (the
                    // rows lane's root as `mod.rut`) — its mod path
                    // stamps the index; flat files keep today's shape
                    let idx = match deps::bundle_file_mod_path(&f.path) {
                        Some(p) => deps::index_dep_mod(
                            &b.module,
                            &uri,
                            &p,
                            &f.src,
                            rut_parser::Mode::Impl,
                            b.namespace.as_deref(),
                            &b.consts,
                        ),
                        None => deps::index_dep(
                            &b.module,
                            &uri,
                            &f.src,
                            f.mode,
                            b.namespace.as_deref(),
                            &b.consts,
                        ),
                    };
                    self.insert(idx);
                }
            }
            Err(e) => {
                self.hint_once(
                    format!("bundle:{url}"),
                    MessageType::ERROR,
                    format!("rut: dep '{name}' — the bundle did not mount: {e}"),
                );
            }
        }
    }

    /// the pkg's mod tree (the editor face of the loader's mount): the
    /// root module is `mod.rut` beside the manifest while no
    /// `entry.lib` stands (the transitional dual-read), else the entry
    /// lib spliced with `entry.libs` exactly as the loader does — and
    /// either way the root text's `mod` declarations mount the child
    /// tree from the directory, recursively, cycle-guarded. The root
    /// (when it is the mod.rut lane — the entries loop had nothing to
    /// index) and every child index under the pkg name with their mod
    /// paths stamped. A mount failure is ONE loud error hint (the
    /// loader's diagnostics, mirrored), never silent bytes.
    fn mount_mod_tree(&mut self, dir: &Path, module: &str, impl_entries: &[String]) {
        let (root_text, root_path, lib_lane) = if impl_entries.is_empty() {
            let p = dir.join("mod.rut");
            match std::fs::read_to_string(&p) {
                Ok(text) => (text, p, false),
                // an entry-less manifest with no mod.rut — the loader's
                // surface-only dev state; nothing mounts
                Err(_) => return,
            }
        } else {
            // the entry-lib lane: lib first, then libs, '\n'-joined —
            // the loader's splice, so the mount reads the same text
            let mut text = String::new();
            let mut last: Option<PathBuf> = None;
            for rel in impl_entries {
                let p = dir.join(rel.trim_start_matches("./"));
                match std::fs::read_to_string(&p) {
                    Ok(t) => {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(&t);
                        last = Some(p);
                    }
                    Err(_) => continue,
                }
            }
            match last {
                Some(p) => (text, p, true),
                None => return,
            }
        };
        if !lib_lane && self.files > 0 {
            self.files -= 1;
            let uri = root_path.to_string_lossy().into_owned();
            let idx = deps::index_dep_mod(module, &uri, "", &root_text, Mode::Impl, None, &[]);
            self.insert(idx);
        }
        let mut read = |parent: &str, name: &str| -> std::result::Result<rut_driver::ChildLookup, String> {
            Ok(fs_child_lookup(dir, parent, name))
        };
        let canon = root_path.to_string_lossy().into_owned();
        match rut_driver::mount_mod_children(&root_text, &canon, &mut read) {
            Ok(mods) => {
                for (path, m) in mods {
                    if self.files == 0 {
                        break;
                    }
                    self.files -= 1;
                    let child_file = dir.join(format!("{path}/mod.rut"));
                    let uri = child_file.to_string_lossy().into_owned();
                    let idx = deps::index_dep_mod(module, &uri, &path, &m.text, Mode::Impl, None, &[]);
                    self.insert(idx);
                }
            }
            Err(e) => {
                self.hint_once(
                    format!("mods:{}", dir.display()),
                    MessageType::ERROR,
                    format!("rut: {e}"),
                );
            }
        }
    }
}

/// a bundle file's mod path: the rows lane's root rides as `mod.rut`
/// (`Some("")`), a mounted child as `<path>/mod.rut` (`Some(path)`);
/// anything else is today's flat shape (`None`)
fn mod_path_of_bundle_file(path: &str) -> Option<String> {
    match path.strip_suffix("/mod.rut") {
        Some(p) => Some(p.to_string()),
        None if path == "mod.rut" => Some(String::new()),
        None => None,
    }
}

/// lexical `..`/`.` resolution for dep paths — the seen-set's keys and
/// entry uris compare consistently without touching the fs
fn normalize_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            _ => out.push(c.as_os_str()),
        }
    }
    out
}

fn hex_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// the fs walk behind `Backend::doc_mods`: the nearest ancestor
/// `rut.jsonc` names the package; the file's dir relative to it is
/// the mod path — the manifest dir's own files are the root module,
/// a `NAME/mod.rut` below it the `NAME` module (any other file shape
/// is not a module file — the default). Bounded like every walk here.
fn resolve_doc_mods(file: &Path) -> DocMods {
    let file_dir = match file.parent() {
        Some(d) => d.to_path_buf(),
        None => return DocMods::default(),
    };
    let is_mod_rut = file.file_name().map(|n| n == "mod.rut").unwrap_or(false);
    let mut dir: &Path = &file_dir;
    for _ in 0..MAX_DEP_DEPTH * 2 {
        if dir.join(MANIFEST_NAME).is_file() {
            if let Ok(text) = std::fs::read_to_string(dir.join(MANIFEST_NAME)) {
                if let Ok(t) = deps::dep_table(&text) {
                    if let Some(name) = t.name {
                        if !is_mod_rut && file_dir != dir {
                            return DocMods::default(); // a stray file in a child dir
                        }
                        let rel = file_dir.strip_prefix(dir).unwrap_or(&file_dir);
                        let mod_path = rel
                            .components()
                            .map(|c| c.as_os_str().to_string_lossy().into_owned())
                            .collect::<Vec<_>>()
                            .join("/");
                        return DocMods { mod_path: Some(mod_path), pkg: Some(name) };
                    }
                }
            }
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => break,
        }
    }
    DocMods::default()
}

/// the fs lane's child lookup — the loader's three-case law over the
/// directory: `NAME/mod.rut`, a same-named FILE sibling, a dir
/// without its `mod.rut`, or nothing. The canonical identity (the
/// cycle guard's key) is the resolved path.
fn fs_child_lookup(dir: &Path, parent: &str, name: &str) -> rut_driver::ChildLookup {
    let base = if parent.is_empty() { dir.to_path_buf() } else { dir.join(parent) };
    let mod_rut = base.join(name).join("mod.rut");
    if mod_rut.is_file() {
        let text = std::fs::read_to_string(&mod_rut).unwrap_or_default();
        let canon = mod_rut
            .canonicalize()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| mod_rut.to_string_lossy().into_owned());
        return rut_driver::ChildLookup::Found { text, canon };
    }
    if base.join(format!("{name}.rut")).is_file() {
        return rut_driver::ChildLookup::NotADir;
    }
    if base.join(name).is_dir() {
        return rut_driver::ChildLookup::NoModRut;
    }
    rut_driver::ChildLookup::Missing
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Backend {
            client,
            documents: Arc::new(RwLock::new(HashMap::new())),
            defs: Arc::new(RwLock::new(std_surface::indexes())),
            root: Arc::new(RwLock::new(None)),
            doc_mods: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// the open document's file-module context: the nearest manifest
    /// dir above the file (an fs walk, cached per file) names the
    /// package and roots the mod path — the manifest dir's own files
    /// are the root module (`""`), a `NAME/mod.rut` below it the
    /// `NAME` module. Files under no manifest (workspace strays)
    /// resolve to the default — position-path completion stays off
    /// for them; nothing else reads this.
    fn doc_mods(&self, uri: &Uri) -> DocMods {
        let path = match uri.to_file_path() {
            Some(cow) => cow.into_owned(),
            None => return DocMods::default(),
        };
        if let Some(d) = self.doc_mods.read().unwrap().get(&path) {
            return d.clone();
        }
        let d = resolve_doc_mods(&path);
        self.doc_mods.write().unwrap().insert(path, d.clone());
        d
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
        // workspace scan + dep walk off the hot initialize path — hover
        // may briefly resolve against std + open docs only. The walk's
        // hints surface as window/showMessage once it lands.
        let defs = self.defs.clone();
        let root_slot = self.root.clone();
        let client = self.client.clone();
        #[allow(deprecated)] // root_uri: VS Code still sends it first
        let root = params.root_uri.as_ref().map(|u| u.as_str().to_string());
        tokio::spawn(async move {
            let hints = tokio::task::spawn_blocking(move || {
                let Some(root) = root else { return Vec::new() };
                let Ok(u) = Uri::from_str(&root) else { return Vec::new() };
                let Some(cow) = u.to_file_path() else { return Vec::new() };
                let path = cow.into_owned();
                *root_slot.write().unwrap() = Some(path.clone());
                let mut guard = defs.write().unwrap();
                for f in collect_rut_files(&path) {
                    if let Some(idx) = index_file(&f) {
                        guard.push(idx);
                    }
                }
                // the dep walk — the manifest's deps ride the SAME defs
                // store the scan fills (cache-only; see the module doc)
                walk_workspace_deps(&path, &mut guard)
            })
            .await
            .unwrap_or_default();
            for (ty, msg) in hints {
                client.show_message(ty, msg).await;
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
                references_provider: Some(OneOf::Left(true)),
                inlay_hint_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![".".to_string()]),
                    ..Default::default()
                }),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
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
        let doc = self.doc_mods(&uri);
        let items = {
            let defs = self.defs.read().unwrap();
            analysis::complete_at(uri.as_str(), &text, &defs, p.line, p.character, &doc)
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
        // locations() only ever builds the Array response
        Ok(self.locations(locs).map(|r| match r {
            GotoDefinitionResponse::Array(v) => GotoTypeDefinitionResponse::Array(v),
            GotoDefinitionResponse::Scalar(l) => GotoTypeDefinitionResponse::Scalar(l),
            _ => unreachable!("locations() only builds Array"),
        }))
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let uri = params.text_document.uri;
        let Some(text) = self.get(&uri) else { return Ok(None) };
        let out = {
            let defs = self.defs.read().unwrap();
            analysis::inlay_hints_at(uri.as_str(), &text, &defs, params.range)
        };
        Ok(Some(out))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let Some(text) = self.get(&uri) else { return Ok(None) };
        let p = params.text_document_position.position;
        let locs = {
            let defs = self.defs.read().unwrap();
            analysis::references_at(
                uri.as_str(),
                &text,
                &defs,
                p.line,
                p.character,
                params.context.include_declaration,
            )
        };
        Ok(self.resolve_locations(locs))
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let uri = params.text_document_position_params.text_document.uri;
        let Some(text) = self.get(&uri) else { return Ok(None) };
        let p = params.text_document_position_params.position;
        let out = {
            let defs = self.defs.read().unwrap();
            analysis::signature_help_at(uri.as_str(), &text, &defs, p.line, p.character)
        };
        Ok(out)
    }
}

impl Backend {
    /// face-agnostic definition/references targets -> LSP locations:
    /// relative targets (the std surface's true `rut/...` paths) resolve
    /// against the workspace root; targets that resolve nowhere stay
    /// relative (the client may still know the file — never a wrong jump)
    fn resolve_locations(&self, locs: Vec<crate::definition::DefLocation>) -> Option<Vec<Location>> {
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
            None
        } else {
            Some(out)
        }
    }

    fn locations(&self, locs: Vec<crate::definition::DefLocation>) -> Option<GotoDefinitionResponse> {
        self.resolve_locations(locs).map(GotoDefinitionResponse::Array)
    }
}

/// a definition target string -> a URI: a real URI passes through, an
/// absolute path becomes one, a relative path needs the workspace root
fn resolve_target_uri(raw: &str, root: Option<&Path>) -> Option<Uri> {
    // scheme = a letter/letter-digit run followed by `:` —
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
