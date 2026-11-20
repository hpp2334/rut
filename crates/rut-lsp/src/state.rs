//! the session state — one instance per editing session (the
//! language core's accumulated facts): the open documents (the
//! versioned cache), the mounted-file world, the walk lane's ledgers.
//! The loop task owns this value exclusively (see
//! [`crate::event_loop`]); the host's `Lsp` is only the inbox handle,
//! and no caller ever borrows the state.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::env;
use crate::event_loop;
use crate::hover::{DefIndexCollection, Mount, MountedFile};
use crate::wire;

pub(crate) struct SessionState {
    /// open documents by URI (the LSP's full-text sync) — the
    /// VERSIONED CACHE: each entry is the document's [`MountedFile`]
    /// (the [`crate::semantic::Document`] + its stamp + its decl map),
    /// synced once per change (the incremental `Doc::edit` pipeline)
    /// and rebuilt there — so the query burst riding one version
    /// re-derives nothing (per-uri parse happens once per change,
    /// never per query).
    pub(crate) docs: HashMap<String, MountedFile>,
    /// the core prelude + walked/workspace files, in lookup order
    pub(crate) defs: DefIndexCollection,
    /// url → pin, from the workspace root's `rut.lock` (the host reads
    /// the file and hands the bytes to `set_lock`; the manifest
    /// grammar carries no pins). `parse_manifest` stamps the rows
    /// onto its url deps.
    pub(crate) lock_pins: BTreeMap<String, String>,
    /// the direct-dep module names — the open document's own package
    /// name plus every dep row its ROOT manifest declares (`deps`,
    /// `dev-deps` at the walk root). The host hands the
    /// set to `set_direct_deps` when it parses a root manifest;
    /// the auto-import tier reads it live (a transitive-only dep's
    /// `use` is the compiler's D5 error). Union-accumulating:
    /// multi-folder walks and re-walks re-add the same names, never
    /// shrink the world.
    pub(crate) direct_deps: HashSet<String>,
    /// the once-per-reason hint gate — every user-facing hint the
    /// walk and the ledger feeds emit keys here, so a reason names
    /// itself exactly once per session (never spam, however often the
    /// world re-walks). The extension's `hintOnce` map, moved in.
    hints_shown: HashSet<String>,
    /// the walk lane's coalescing state — the OUTSTANDING deferred
    /// walk's [`event_loop::DeferTask`] handle, moved in from the JS
    /// dep engine's flags (D1: policy lives in the `Cmd::ReloadDeps` arm,
    /// the defer queue stays neutral). `None` = no walk queued; a
    /// STALE handle (its walk already fired) is harmless —
    /// `cancel_defer` answers `None` for it.
    pub(crate) walk_task: Option<event_loop::DeferTask>,
}

impl SessionState {
    /// the session state as it boots — the engine prelude pre-mounted
    /// (`core` alone, the ONE ambient entry; every other std pkg rides
    /// the dep lane like any third-party lib), no session facts yet. The loop
    /// task owns this value exclusively; both constructors (`new` the
    /// wasm face, `spawn_native` the native harness) hand a fresh one
    /// to [`event_loop::run`].
    pub(crate) fn boot() -> SessionState {
        let mut defs = DefIndexCollection::new();
        defs.mount(crate::prelude::index());
        SessionState {
            docs: HashMap::new(),
            defs,
            lock_pins: BTreeMap::new(),
            direct_deps: HashSet::new(),
            hints_shown: HashSet::new(),
            walk_task: None,
        }
    }

    // ---- the sync lane (the doc-sync law: text arrives → sync →
    // queries read) ----

    /// one document change in, a coherent session out: the stored
    /// entry syncs (the incremental `Doc::edit` pipeline + the UTF-16
    /// view rebuild + the lazy views drop) and its decl map rebuilds —
    /// THE one parse per change, never per query. The entry's stamped
    /// mount — the (module, mod_path, mode) identity a walk lane laid
    /// down on the same uri — survives untouched; a fresh uri mounts
    /// with the open-doc stamp (origin only).
    pub(crate) fn sync(&mut self, uri: &str, src: &str) {
        match self.docs.get_mut(uri) {
            Some(f) => f.doc.sync(src),
            None => {
                let mount = Mount::doc(uri);
                self.docs.insert(
                    uri.to_string(),
                    MountedFile::from_document(
                        crate::semantic::Document::parse(
                            crate::semantic::mode_of(uri),
                            src.to_string(),
                        ),
                        mount,
                    ),
                );
            }
        }
        let f = self.docs.get_mut(uri).expect("just synced");
        f.decls = crate::hover::collection::decls_of(&f.doc);
    }

    /// the document at `uri`, if open
    pub(crate) fn doc(&self, uri: &str) -> Option<&crate::semantic::Document> {
        self.docs.get(uri).map(|f| &f.doc)
    }

    /// the query chain — the open document's entry at the head (the
    /// fresh version the sync lane keeps), the mounted world after
    /// (the prelude + workspace; a stamped twin of the SAME uri keeps
    /// its place — the definition layer's dedupe law handles the
    /// doubled answer). `None` = the doc is unknown.
    pub(crate) fn chain(&self, uri: &str) -> Option<crate::hover::collection::Chain<'_>> {
        let head = self.docs.get(uri)?;
        Some(self.defs.chain_with(head))
    }

    // ---- the lanes the ABI handlers and the dep walk SHARE (the
    // lifted handler bodies — one implementation, two doors: the
    // message arms and [`walk`]'s read-only walk index through the
    // same code, so a walked dep and a host-indexed dep can never
    // drift) ----

    /// the walk's per-root lock feed: parse one `rut.lock` into
    /// the url→pin ledger. Ok = the row count and the ledger standing;
    /// Err = the ledger CLEARED (the honest unpinned refusal, never
    /// half-parsed or stale pins) and the error for the caller to
    /// surface.
    pub(crate) fn set_lock_text(&mut self, text: &str) -> Result<usize, String> {
        match crate::deps::lock_pins(text) {
            Ok(pins) => {
                let n = pins.len();
                self.lock_pins = pins;
                Ok(n)
            }
            Err(e) => {
                self.lock_pins = BTreeMap::new();
                Err(e)
            }
        }
    }

    /// the walk's named-dep mount lane: mount one dep source
    /// file under its module NAME (replace-by-origin) — the FLAT
    /// mount (`mod_path` `None`).
    pub(crate) fn index_named_at(&mut self, uri: &str, name: &str, src: String) {
        let file = crate::deps::mount_file(name, uri, None, src);
        self.defs.mount(file);
    }

    /// [`event_loop::Cmd::IndexModAt`]'s body: mount one dep source
    /// file at a MOD PATH (the pkg root `""`, a mounted child its
    /// `<path>`), answering the parsed file's own `mod` declarations —
    /// the walk's next hop (structure lives in the parse, never in
    /// path munging).
    pub(crate) fn index_mod_at(
        &mut self,
        uri: &str,
        pkg: &str,
        mod_path: &str,
        src: String,
    ) -> Vec<wire::ModDecl> {
        let file = crate::deps::mount_file(pkg, uri, Some(mod_path), src);
        let decls: Vec<wire::ModDecl> = file
            .mods()
            .iter()
            .map(|m| wire::ModDecl {
                name: m.name().to_string(),
                vis: if matches!(m.vis(), rut_ast::Vis::Pub) {
                    "pub"
                } else {
                    "mod"
                },
            })
            .collect();
        self.defs.mount(file);
        decls
    }

    /// the once-per-reason hint gate: the first `key` notifies, every
    /// later one is silent. All walk hints are error-flagged (the
    /// extension's law for these lanes).
    pub(crate) fn hint_once(&mut self, io: &dyn env::Io, key: &str, msg: &str) {
        if self.hints_shown.insert(key.to_string()) {
            io.notify(msg, true);
        }
    }

    /// the open document's file-module context, read off the STAMPED
    /// twin in the mounted world: whichever face mounted the file
    /// under its real uri (the extension's dep walk / workspace
    /// indexer) laid down the (module, mod_path) pair. Unstamped files
    /// answer the default (position-path completion off; nothing else
    /// reads this).
    pub(crate) fn doc_mods_of(&self, uri: &str) -> crate::mods::DocMods {
        match self.defs.file_for(uri) {
            Some(f) => crate::mods::DocMods {
                mod_path: f.mount.mod_path.clone(),
                pkg: f.mount.module.clone(),
            },
            None => crate::mods::DocMods::default(),
        }
    }

    /// the completion query's body over the STORED document — TYPED:
    /// the state layer speaks values, and the calling arm owns the
    /// envelope (one law with every other query: JSON writes happen
    /// once, at the `Reply::Text` boundary). The doc-sync law: the
    /// caller synced FIRST, so the chain head's document IS the text
    /// the cursor sits in.
    pub(crate) fn complete_stored(
        &self,
        uri: &str,
        line: u32,
        ch: u32,
    ) -> Vec<ls_types::CompletionItem> {
        let Some(chain) = self.chain(uri) else {
            return Vec::new();
        };
        let head = chain.head();
        let byte = head.doc.lines().byte(head.doc.src(), line, ch);
        let doc = self.doc_mods_of(uri);
        crate::query::completion::complete(&chain, byte, head.doc.src(), &doc, &self.direct_deps)
            .into_iter()
            .map(crate::query::completion::lsp_item)
            .collect()
    }
}
