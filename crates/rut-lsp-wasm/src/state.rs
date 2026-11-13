//! the session state — one instance per editing session (the
//! language core's accumulated facts): the open documents, the defs
//! store, the walk lane's ledgers. The loop task owns this value
//! exclusively (see [`crate::event_loop`]); the host's `RutLsp` is
//! only the inbox handle, and no caller ever borrows the state.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::env;
use crate::event_loop;
use crate::hover::DefIndex;
use crate::wire;

pub(crate) struct SessionState {
    /// open documents by URI (the LSP's full-text sync)
    pub(crate) docs: HashMap<String, String>,
    /// the core prelude + walked/workspace files, in lookup order
    pub(crate) defs: Vec<DefIndex>,
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
    /// the session state as it boots — the engine prelude pre-indexed
    /// (`core` alone, the ONE ambient index; every other std pkg rides
    /// the dep lane like any third-party lib), no session facts yet. The loop
    /// task owns this value exclusively; both constructors (`new` the
    /// wasm face, `spawn_native` the native harness) hand a fresh one
    /// to [`event_loop::run`].
    pub(crate) fn boot() -> SessionState {
        SessionState {
            docs: HashMap::new(),
            defs: vec![crate::prelude::index()],
            lock_pins: BTreeMap::new(),
            direct_deps: HashSet::new(),
            hints_shown: HashSet::new(),
            walk_task: None,
        }
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

    /// the walk's named-dep index lane: index one dep source
    /// file under its module NAME (replace-by-origin), the manifest's
    /// RETAINED `consts` riding the row (the compat-parsed store lane
    /// hands them; `&[]` for host-indexed files, which carry no
    /// manifest).
    pub(crate) fn index_named_at(
        &mut self,
        uri: &str,
        name: &str,
        src: &str,
        consts: &[(String, f64)],
    ) {
        let idx = crate::deps::index_dep(name, uri, src, consts);
        self.replace_by_origin(uri, idx);
    }

    /// [`event_loop::Cmd::IndexModAt`]'s body: index one dep source
    /// file at a MOD PATH (the pkg root `""`, a mounted child its
    /// `<path>`), answering the parsed file's own `mod` declarations —
    /// the walk's next hop (structure lives in the parse, never in
    /// path munging). The consts ride the pkg-ROOT stamp only.
    pub(crate) fn index_mod_at(
        &mut self,
        uri: &str,
        pkg: &str,
        mod_path: &str,
        src: &str,
        consts: &[(String, f64)],
    ) -> Vec<wire::ModDecl> {
        let idx = crate::deps::index_dep_mod(pkg, uri, mod_path, src, consts);
        let decls: Vec<wire::ModDecl> = idx
            .mods
            .iter()
            .map(|m| wire::ModDecl {
                name: m.name.clone(),
                vis: if matches!(m.vis, rut_ast::Vis::Pub) {
                    "pub"
                } else {
                    "mod"
                },
            })
            .collect();
        self.replace_by_origin(uri, idx);
        decls
    }

    /// the defs store's replace-by-origin law — one index per origin
    /// uri, re-indexing replaces (the world stays idempotent under
    /// re-walks).
    fn replace_by_origin(&mut self, uri: &str, idx: crate::hover::DefIndex) {
        match self.defs.iter().position(|d| d.origin == uri) {
            Some(slot) => self.defs[slot] = idx,
            None => self.defs.push(idx),
        }
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
    /// twin: whichever face indexed the file under its real uri (the
    /// extension's dep walk / workspace indexer) laid down the (module,
    /// mod_path) pair — the fresh per-query doc index borrows it.
    /// Unstamped files answer the default (position-path completion off;
    /// nothing else reads this).
    pub(crate) fn doc_mods_of(&self, uri: &str) -> crate::mods::DocMods {
        for d in &self.defs {
            if d.origin == uri {
                return crate::mods::DocMods {
                    mod_path: d.mod_path.clone(),
                    pkg: d.module.clone(),
                };
            }
        }
        crate::mods::DocMods::default()
    }

    /// the completion query's body over the stored doc — TYPED: the
    /// state layer speaks values, and the calling arm owns the
    /// envelope (one law with every other query: JSON writes happen
    /// once, at the `Reply::Text` boundary)
    pub(crate) fn complete_stored(
        &self,
        uri: &str,
        line: u32,
        ch: u32,
    ) -> Vec<ls_types::CompletionItem> {
        let Some(src) = self.docs.get(uri).cloned() else {
            return Vec::new();
        };
        let doc = self.doc_mods_of(uri);
        crate::analysis::complete_at(uri, &src, &self.defs, line, ch, &doc, &self.direct_deps)
    }
}
