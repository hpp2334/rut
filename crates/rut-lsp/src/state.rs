//! The session — one instance per editing session (the language
//! core's accumulated facts): ONE world of files (the prelude, the
//! walked deps, the workspace pushes, the open documents), the walk
//! lane's ledgers. The loop task owns this value exclusively (see
//! [`crate::event_loop`]); the host's `Lsp` is only the inbox handle,
//! and no caller ever borrows the state.

use std::collections::{BTreeMap, HashSet};

use crate::env;
use crate::event_loop;
use crate::wire;
use crate::world::{File, FileId, World};

pub(crate) struct Session {
    /// ONE world — every file the session knows (the prelude boots
    /// first; the dep walk, the workspace scan, and the open documents
    /// mount under their origins; an open document adopts the file its
    /// loader lane laid down and its buffer is the truth), in store
    /// order. Rebuilt never — the world mutates in place per
    /// mount/sync/close.
    pub(crate) world: World,
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

impl Session {
    /// the session as it boots — the engine prelude pre-mounted
    /// (`core` alone, the ONE ambient entry; every other std pkg rides
    /// the dep lane like any third-party lib), no session facts yet. The loop
    /// task owns this value exclusively; both constructors (`new` the
    /// wasm face, `spawn_native` the native harness) hand a fresh one
    /// to [`event_loop::run`].
    pub(crate) fn boot() -> Session {
        Session {
            world: World::new(crate::prelude::index()),
            lock_pins: BTreeMap::new(),
            direct_deps: HashSet::new(),
            hints_shown: HashSet::new(),
            walk_task: None,
        }
    }

    // ---- the sync lane (the doc-sync law: text arrives → sync →
    // queries read) ----

    /// one document change in, a coherent session out, the file's id
    /// answered. THE ONE-WORLD LAW: a file under `uri` that is open
    /// syncs in place (the incremental `Doc::edit` pipeline + the
    /// UTF-16 view rebuild + the lazy views drop + the decl map
    /// rebuild — one parse per change, never per query); a file under
    /// `uri` that is closed REOPENS — `open = true` and the same sync
    /// over the text it kept (its stamp survives untouched); no file
    /// under the uri mounts
    /// [`File::open_scratch`](crate::world::File::open_scratch) — an
    /// open document no loader ever stamped.
    pub(crate) fn sync(&mut self, uri: &str, src: &str) -> FileId {
        let id = match self.world.open(uri).or_else(|| self.world.get(uri)) {
            Some(id) => id,
            None => return self.world.mount(File::open_scratch(uri, src.to_string())),
        };
        let f = self.world.file_mut(id);
        f.open = true;
        f.doc.sync(src);
        f.decls = crate::world::decls_of(&f.doc);
        id
    }

    /// the file at `uri`, only when open — the position queries' gate
    /// (a closed file answers no position query)
    pub(crate) fn open_file(&self, uri: &str) -> Option<FileId> {
        self.world.open(uri)
    }

    /// close: the file under `uri` closes — a loader-stamped file
    /// keeps its last buffer text (a later read answers from it); a
    /// never-stamped scratch file leaves the world (a file no loader
    /// mounted is still removed).
    pub(crate) fn close(&mut self, uri: &str) {
        self.world.close(uri);
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
        self.world.mount(file);
    }

    /// [`event_loop::Cmd::IndexModAt`]'s body: mount one dep source
    /// file at a MOD PATH (the pkg root `""`, a mounted child its
    /// `<path>`), answering the parsed file's own `mod` declarations —
    /// the walk's next hop (structure lives in the parse, never in
    /// path munging). The reply describes the PUSHED text even when
    /// the file is open (the host walks what is on disk): the decls
    /// read off the built file BEFORE the mount — an open file keeps
    /// its buffer and drops the incoming text.
    pub(crate) fn index_mod_at(
        &mut self,
        uri: &str,
        pkg: &str,
        mod_path: &str,
        src: String,
    ) -> Vec<wire::ModDecl> {
        let file = File::from_src(
            crate::world::Stamp::workspace(uri, Some(pkg), Some(mod_path)),
            src,
        );
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
        self.world.mount(file);
        decls
    }

    /// the host's workspace scan push: mount one file under its
    /// open-doc-capable workspace stamp (unstamped — the pkg name is
    /// the uri's derivation business). Re-indexing a uri replaces its
    /// entry; an OPEN file keeps its buffer (the incoming text
    /// drops).
    pub(crate) fn index_file(&mut self, uri: &str, src: String) {
        let file = File::from_src(crate::world::Stamp::workspace(uri, None, None), src);
        self.world.mount(file);
    }

    /// the once-per-reason hint gate: the first `key` notifies, every
    /// later one is silent. All walk hints are error-flagged (the
    /// extension's law for these lanes).
    pub(crate) fn hint_once(&mut self, io: &dyn env::Io, key: &str, msg: &str) {
        if self.hints_shown.insert(key.to_string()) {
            io.notify(msg, true);
        }
    }

    /// the completion query's body over the OPEN document — TYPED:
    /// the state layer speaks values, and the calling arm owns the
    /// envelope (one law with every other query: JSON writes happen
    /// once, at the `Reply::Text` boundary). The doc-sync law: the
    /// caller synced FIRST, so the file's document IS the text
    /// the cursor sits in. `io` is the stored host env (the loop's
    /// capability) — the mod-name slot's disk probe reads it; `None`
    /// and that slot simply answers empty.
    pub(crate) fn complete_stored(
        &self,
        uri: &str,
        line: u32,
        ch: u32,
        io: Option<&dyn env::Io>,
    ) -> Vec<ls_types::CompletionItem> {
        let Some(f) = self.open_file(uri) else {
            return Vec::new();
        };
        let file = self.world.file(f);
        let byte = file.doc.byte(line, ch);
        let dir = crate::query::completion::file_dir(uri);
        let facts = crate::query::completion::DocFile {
            dir: dir.as_deref(),
            // the flavor is the file's own stamp — the lane's word
            // (`.d.rut` Decl, everything else Impl), THE authority
            // (never a basename match)
            decl: file.stamp.mode == crate::semantic::Mode::Decl,
            io,
        };
        crate::query::completion::complete(
            &self.world,
            f,
            byte,
            file.doc.src(),
            &facts,
            &self.direct_deps,
        )
        .into_iter()
        .map(crate::query::completion::lsp_item)
        .collect()
    }
}
