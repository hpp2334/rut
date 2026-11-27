//! The session — one instance per editing session (the language
//! core's accumulated facts): ONE world of files (the prelude, the
//! walked deps, the workspace pushes, the open documents), the walk
//! lane's ledgers. The loop task owns this value exclusively (see
//! [`crate::actor::event_loop`]); the host's `Lsp` is only the inbox handle,
//! and no caller ever borrows the state.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use crate::actor::event_loop;
use crate::actor::wire;
use crate::env;
use crate::feature::inlay::InlayRow;
use crate::world::{File, FileId, Lane, World};

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
    /// the inlay pass's per-version cache — file id → (the document
    /// version the rows were computed at, the rows). A query recomputes
    /// when the file's version differs ([`Session::inlay_hints`]).
    /// IDS ARE KEYS ONLY BETWEEN MUTATIONS: a store mutation that
    /// shifts ids (a scratch file leaving the store — the close law's
    /// removal, a mount replacing one) drops the cache, so a cached
    /// row set can never answer for a DIFFERENT file
    /// ([`Session::drop_shifted_inlays`]).
    pub(crate) inlays: HashMap<FileId, (u32, Arc<[InlayRow]>)>,
    /// the injected READ-ONLY host environment — `set_env`'s store
    /// (replace-on-reinject). TWO pure readers: the dep walk
    /// ([`Session::resolve_deps`]) and the completion mod-name slot's
    /// disk probe; the loop's window arming reads the same capability
    /// (`arm_wake`). `None` until the host hands one over.
    pub(crate) env: Option<Rc<dyn env::Io>>,
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
            world: World::new(crate::load::prelude::index()),
            lock_pins: BTreeMap::new(),
            direct_deps: HashSet::new(),
            hints_shown: HashSet::new(),
            walk_task: None,
            inlays: HashMap::new(),
            env: None,
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
    ///
    /// A DOCUMENT'S PARSE MODE IS ITS MOUNT'S WHEN A MOUNT OWNS THE
    /// URI, THE FILENAME'S OTHERWISE. A file the world already holds
    /// keeps the tree its mount parsed ([`File::from_src`] parsed with
    /// `stamp.mode`; the sync only edits it), so a host tree's
    /// `tool.rut` stays Decl however its name reads; a uri no mount
    /// owns is a scratch, and [`Stamp::scratch`](crate::world::Stamp::scratch)
    /// derives its mode from the filename. No re-parse here revises a
    /// mount's word.
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
    /// mounted is still removed). A scratch REMOVAL shifts every later
    /// file's id, so the inlay cache drops (a cached row set must
    /// never answer for a different file).
    pub(crate) fn close(&mut self, uri: &str) {
        self.drop_shifted_inlays(uri);
        self.world.close(uri);
    }

    /// the inlay cache keys FileIds — ids are stable only between
    /// store mutations, and the two mutation shapes that shift them
    /// both run through a SCRATCH slot (the close law removes it; a
    /// mount replacing one re-pushes at the end). When `uri` holds a
    /// scratch file, the cached rows drop before the mutation.
    fn drop_shifted_inlays(&mut self, uri: &str) {
        if let Some(id) = self.world.get(uri) {
            if matches!(self.world.file(id).stamp.lane, Lane::Scratch) {
                self.inlays.clear();
            }
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
        match crate::load::manifest::lock_pins(text) {
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
        self.drop_shifted_inlays(uri);
        let file = crate::load::manifest::mount_file(name, uri, None, src);
        self.world.mount(file);
    }

    /// [`event_loop::Cmd::IndexModAt`]'s body: mount one dep source
    /// file at a MOD PATH (the pkg root `""`, a mounted child its
    /// `<path>`), answering the parsed file's own `mod` declarations —
    /// the walk's next hop (structure lives in the parse, never in
    /// path munging). `mode` is THE MOUNT'S WORD: the grammar the
    /// caller parsed the tree with (a host tree's children are Decl
    /// whatever their filenames say; the scan lane passes the
    /// filename default). The reply describes the PUSHED text even when
    /// the file is open (the host walks what is on disk): the decls
    /// read off the built file BEFORE the mount — an open file keeps
    /// its buffer and drops the incoming text.
    pub(crate) fn index_mod_at(
        &mut self,
        uri: &str,
        pkg: &str,
        mod_path: &str,
        src: String,
        mode: crate::doc::Mode,
    ) -> Vec<wire::ModDecl> {
        self.drop_shifted_inlays(uri);
        let mut stamp = crate::world::Stamp::workspace(uri, Some(pkg), Some(mod_path));
        // the lane's word: a mount stamps the mode it parsed with; the
        // filename convention answers only where no mount owns the file
        stamp.mode = mode;
        let file = File::from_src(stamp, src);
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
    /// open-doc-capable workspace stamp. LAW 1: a workspace file's stamp
    /// names its package whenever the host knows it (`pkg` rides the
    /// push); the uri-shape derivation remains only for a file no
    /// manifest owns (`pkg` `None`). Re-indexing a uri replaces its
    /// entry; an OPEN file keeps its buffer (the incoming text drops).
    pub(crate) fn index_file(&mut self, uri: &str, src: String, pkg: Option<String>) {
        self.drop_shifted_inlays(uri);
        let file = File::from_src(
            crate::world::Stamp::workspace(uri, pkg.as_deref(), None),
            src,
        );
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

    // ---- the requests — one method per ABI ask (the loop's dispatch
    // calls in; every JSON write happens at the loop's reply boundary,
    // so these bodies speak typed values only — the Value lanes spell
    // where the wire keeps lsp-types' own field order) ----

    /// the token legend — the names in token-type index order
    pub(crate) fn legend(&self) -> Vec<&'static str> {
        crate::doc::TokenType::legend()
    }

    /// open / FULL-sync change: store the document, return one
    /// analysis — the merged set (B1+B2). The file syncs FIRST (the
    /// doc-sync lane's store — the queries below read the same
    /// version), then the rows ride the file's kept document. One
    /// parse per change — never per query.
    pub(crate) fn analyze(&mut self, uri: &str, src: &str) -> wire::Analysis {
        let f = self.sync(uri, src);
        let mut a = crate::feature::analysis::of(&self.world.file(f).doc);
        // the merged set (B1+B2): the file's own package (its stamp
        // names it) compiles with its dep closure and the checker's
        // per-file diags map back through the file stamps. No package
        // context → parse-only, exactly the pre-B1 answer.
        let diags = self.check(f, std::mem::take(&mut a.diags));
        wire::Analysis {
            diags,
            // SemanticTokens serializes `data` to the flat u32 wire
            // format (the envelope carries the array verbatim)
            tokens: ls_types::SemanticTokens {
                result_id: None,
                data: a.tokens,
            },
            symbols: a.symbols,
        }
    }

    /// the check lane's merged answer for one open document — the
    /// world's package groups feed the compiler run; the file's own
    /// package context (its stamp names it) decides parse-only vs
    /// merged.
    fn check(
        &self,
        file: FileId,
        parse_diags: Vec<ls_types::Diagnostic>,
    ) -> Vec<ls_types::Diagnostic> {
        crate::feature::analysis::merged_diags_for(&self.world, file, parse_diags)
    }

    /// hover at an LSP position — the doc-sync law: queries read the
    /// OPEN file — no doc, no answer (the null value, the wire's
    /// `null`). The answer serializes THROUGH the `Value` map (the
    /// lsp-types fields' wire order is the map's; a direct struct
    /// serialization would reorder the object's keys on the wire).
    pub(crate) fn hover(&self, uri: &str, line: u32, ch: u32) -> serde_json::Value {
        let Some(f) = self.open_file(uri) else {
            return serde_json::Value::Null;
        };
        let pos = self.world.file(f).doc.byte(line, ch);
        let out = crate::feature::analysis::hover_on(&self.world, f, pos);
        serde_json::to_value(out).unwrap_or_default()
    }

    /// completions over the document's CURRENT text — the doc-sync
    /// entry: the CURRENT text syncs FIRST (the analyze lane
    /// debounces, completion fires on the keystroke), then the tiers
    /// lex the text the cursor sits in, dot included. The stored env
    /// feeds the mod-name slot's disk probe (`None` and that slot
    /// simply answers empty). Value-shaped, like every lsp-types lane.
    pub(crate) fn complete(
        &mut self,
        uri: &str,
        src: &str,
        line: u32,
        ch: u32,
    ) -> serde_json::Value {
        self.sync(uri, src);
        let items: Vec<ls_types::CompletionItem> = match self.open_file(uri) {
            None => Vec::new(),
            Some(f) => {
                let byte = self.world.file(f).doc.byte(line, ch);
                crate::feature::completion::complete(
                    &self.world,
                    f,
                    byte,
                    &self.direct_deps,
                    self.env.as_deref(),
                )
                .into_iter()
                .map(crate::feature::completion::lsp_item)
                .collect()
            }
        };
        serde_json::to_value(items).unwrap_or_default()
    }

    /// definitions at an LSP position — `None` when the doc is unknown
    /// (the wire's `null`), the rows (maybe empty) when it is open.
    /// The resolver's `DefLocation` is serde-free, so the wire's
    /// `Location` row is spelled here.
    pub(crate) fn definition(&self, uri: &str, line: u32, ch: u32) -> Option<Vec<wire::Location>> {
        let f = self.open_file(uri)?;
        let pos = self.world.file(f).doc.byte(line, ch);
        Some(crate::locations(&crate::feature::analysis::definition_on(
            &self.world,
            f,
            pos,
        )))
    }

    /// type definitions at an LSP position (an expression -> its type's
    /// declaration) — [`Session::definition`]'s shape
    pub(crate) fn type_definition(
        &self,
        uri: &str,
        line: u32,
        ch: u32,
    ) -> Option<Vec<wire::Location>> {
        let f = self.open_file(uri)?;
        let pos = self.world.file(f).doc.byte(line, ch);
        Some(crate::locations(
            &crate::feature::analysis::type_definition_on(&self.world, f, pos),
        ))
    }

    /// references — the definition index read BACKWARDS. `None` when
    /// the doc is unknown; `include_decl` follows the LSP toggle.
    pub(crate) fn references(
        &self,
        uri: &str,
        line: u32,
        ch: u32,
        include_decl: u32,
    ) -> Option<Vec<wire::Location>> {
        let f = self.open_file(uri)?;
        Some(crate::locations(&crate::feature::analysis::references_on(
            &self.world,
            f,
            line,
            ch,
            include_decl != 0,
        )))
    }

    /// signature help at an LSP position over the document's CURRENT
    /// text — the doc-sync entry (complete's twin): the provider fires
    /// the instant `(` is typed while the analyze lane debounces, so
    /// the CURRENT text syncs FIRST — a stored-only read would miss the
    /// very paren that triggered the request. Value-shaped, like every
    /// lsp-types lane.
    pub(crate) fn signature_help(
        &mut self,
        uri: &str,
        src: &str,
        line: u32,
        ch: u32,
    ) -> serde_json::Value {
        self.sync(uri, src);
        let out = match self.open_file(uri) {
            None => None,
            Some(f) => crate::feature::analysis::signature_help_on(&self.world, f, line, ch),
        };
        serde_json::to_value(out).unwrap_or_default()
    }

    /// the inlay query's body — the open gate (a closed file answers no
    /// position query), the per-version cache read, then the feature's
    /// range filter: the rows recompute when the file's document
    /// version differs from the cached one, and the asked range is
    /// presentation over the cached rows. The cache keys the file's
    /// id; the shift-drop at the store mutations keeps a cached row
    /// set from ever answering for a different file. Value-shaped,
    /// like every lsp-types lane.
    pub(crate) fn inlay_hints(
        &mut self,
        uri: &str,
        start_line: u32,
        start_ch: u32,
        end_line: u32,
        end_ch: u32,
    ) -> serde_json::Value {
        let Some(file) = self.open_file(uri) else {
            return serde_json::Value::Null;
        };
        let (lo, hi, version) = {
            let f = self.world.file(file);
            (
                f.doc.byte(start_line, start_ch),
                f.doc.byte(end_line, end_ch),
                f.doc.version(),
            )
        };
        let rows = match self.inlays.get(&file) {
            Some((v, rows)) if *v == version => rows.clone(),
            _ => {
                let rows: Arc<[InlayRow]> =
                    crate::feature::inlay::compute(&self.world, file).into();
                self.inlays.insert(file, (version, rows.clone()));
                rows
            }
        };
        let hints = crate::feature::inlay::inlay_hints(&self.world, file, lo, hi, &rows);
        serde_json::to_value(hints).unwrap_or_default()
    }

    /// parse one `rut.jsonc` — the dep table on Ok, the error envelope
    /// value on Err (both serialize at the loop's reply boundary)
    pub(crate) fn parse_manifest(&self, text: &str) -> Result<wire::DepTable, wire::Error> {
        crate::load::manifest::dep_table_pinned(text, &self.lock_pins)
            .map(|t| wire::DepTable::from(&t))
            .map_err(wire::Error::new)
    }

    /// the eager dep-walk door's body — the SAME walk the deferred
    /// door runs (D9 — one implementation, two doors): one fast
    /// handler turn over the stored env, no walk task, no awaits.
    /// TYPED: the calling arm owns the envelope; an env-less session
    /// answers the loud error value, never a silent empty walk.
    pub(crate) fn resolve_deps(&mut self, roots: &[String]) -> Result<wire::WalkOut, wire::Error> {
        let Some(io) = self.env.clone() else {
            return Err(wire::Error::new(
                "no host environment — the dep walk reads through the injected env \
                 (load(bytes, env) / set_env)",
            ));
        };
        Ok(wire::WalkOut::from(&crate::load::dep_walk::resolve(
            self,
            io.as_ref(),
            roots,
        )))
    }

    /// the reload lane's coalescing state — remember the outstanding
    /// deferred walk's handle, answering the PREVIOUS one: the caller
    /// (`Cmd::ReloadDeps`) cancels it through the loop's neutral door —
    /// superseded, merged into the newer reload (the LAST reload's
    /// promise carries the world).
    pub(crate) fn reload_deps(
        &mut self,
        t: event_loop::DeferTask,
    ) -> Option<event_loop::DeferTask> {
        self.walk_task.replace(t)
    }

    /// the capability store — replace-on-reinject, exactly the ledger
    /// setters' shape. The loop's SetEnv arm installs the WAKE SOURCE
    /// first; every reader (the dep walk, the completion probe, the
    /// window arming) reads the same capability.
    pub(crate) fn set_env(&mut self, io: Rc<dyn env::Io>) {
        self.env = Some(io);
    }
}
