//! rut-lsp-wasm — the language core AND its wasm ABI, one crate.
//! The queries (`crate::analysis` and the sibling modules below) and the
//! values live here beside the wasm-bindgen face; the VS Code extension
//! instantiates it and registers its providers directly — no server
//! process, no per-platform binaries, no second crate to keep in step.
//!
//! wasm-bindgen over the language core (the `rut-wasm` pattern): the
//! module exports ONE class, [`RutLsp`]; the host constructs it with
//! `new RutLsp()` and calls its methods. Every method is an ASYNC RPC
//! bridge (a `Promise` — the actor seam): the call posts one
//! [`event_loop::Msg`] on the loop's mpsc inbox and awaits a oneshot
//! reply, while ONE long-lived loop task owns the `Core` exclusively and
//! answers messages FIFO — sync handlers over `&mut Core` (see
//! [`event_loop`]). A dropped loop answers the error envelope; a
//! panicked handler isolates to its own reply channel. Every text
//! answer is the JSON envelope; the wasm-bindgen runtime owns the
//! string copies both ways (the raw-ABI era's
//! `rut_begin`/`rut_alloc` arena and its ptr/len plumbing are gone — no
//! `unsafe` anywhere in this crate):
//!
//!   new RutLsp()                               the language core, with
//!                                              the std surface pre-indexed
//!   set_env(env)                               hand the core the READ-ONLY
//!                                              host environment (`HostEnv`:
//!                                              read/readDir/isDir/notify,
//!                                              see `env`) — the dep walk's
//!                                              capability
//!   resolve_deps(roots_json) -> String         the READ-ONLY dep walk over
//!                                              the injected env: one JSON
//!                                              array of workspace roots in,
//!                                              the walk's answer (`WalkOut`:
//!                                              the url rows it refused —
//!                                              informational rows for
//!                                              hosts) out; every mounted dep
//!                                              lands in the SAME defs store
//!                                              the add_def lanes write (see
//!                                              `walk`)
//!   legend() -> String                         token-type names, JSON array
//!   analyze(uri, src) -> String                store the doc + return
//!                                              {diags, tokens, symbols}
//!   forget(uri)                                drop a closed doc
//!   hover(uri, line, ch) -> String             LSP Hover JSON or null
//!   complete(uri, src, line, ch) -> String     LSP CompletionItem array
//!                                              over the doc's CURRENT
//!                                              text (stored first — the
//!                                              doc-sync lane; completion
//!                                              fires on the keystroke,
//!                                              the analyze lane
//!                                              debounces)
//!   definition(uri, line, ch) -> String        LSP Location array or null
//!   type_definition(uri, line, ch) -> String   same shape
//!   inlay(uri, sl, sc, el, ec) -> String       LSP InlayHint array or null
//!   references(uri, line, ch, incl) -> String  LSP Location array or null
//!   signature_help(uri, line, ch) -> String    LSP SignatureHelp or null
//!   add_def(uri, src)                          index a workspace file
//!   parse_manifest(text) -> String             rut.jsonc bytes -> the
//!                                              dep-table JSON (or the
//!                                              error envelope)
//!   add_def_mod(uri, name, path, src)          index a source file at a
//!                                              MOD PATH (the workspace
//!                                              scan lane); the envelope
//!                                              names the file's own mod
//!                                              decls (the scan's mount
//!                                              walk reads its next hop
//!                                              from them)
//!
//! The package the hosts consume is `pkg/`: `wasm-pack build --target
//! web` writes the generated glue there, then `scripts/build-lsp-pkg.mjs`
//! installs this crate's JS face (`js/index.js` — the one place that
//! `JSON.parse`s the answers) and its declarations (`js/index.d.ts`)
//! beside it. Hosts import `pkg/index.js` and never declare these shapes
//! themselves; the field names live in `wire` (this module's serializers)
//! and the JS face's d.ts mirror, locked by `tsc` in every host and by
//! the extension's wire-shape e2e block.
//!
//! THE SURFACE LAW — the ABI is exactly three lanes: the provider
//! queries above; the workspace-scan push (`add_def`/`add_def_mod`/
//! `parse_manifest` — the host's `findFiles` sees editor ignore
//! semantics the env cannot); and the dep pull (`set_env` +
//! `resolve_deps`). NO entry mutates dep state except through the
//! injected env — the lock feed, the direct-dep stamp, and the named
//! index lanes are Core internals the walk drives, not messages. The
//! old push family (`set_lock`, `set_direct_deps`, `add_def_named`,
//! `add_bundle`, `extract_bundle`, `extract_srczip`, `doc_len`) lived
//! in the blind-wasm era and is gone; its behaviors survive through
//! the walk (see `walk` and its tests).
//!
//! One deviation from `rut-wasm`'s one-shot model carries over from the
//! raw ABI: an LSP serves a long-lived editing session, so the instance
//! OWNS its session state (open docs + the index) for its lifetime —
//! the host holds one `RutLsp` per boot and mutates it per request.
//! The state lives in `Core`, owned exclusively by the loop task
//! ([`event_loop`]); the host's `RutLsp` is the inbox handle. Calls
//! serialize on the loop's FIFO — single-threaded JS makes the order
//! total, and no caller ever borrows the core.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use crate::hover::DefIndex;
use wasm_bindgen::prelude::wasm_bindgen;

mod event_loop;
mod walk;
mod wire;

// ---- the language core — the queries and the values ----
//
// One crate now: the queries live beside the ABI that serves them, so
// a feature cannot land on one face and miss the other. `query` is a
// private directory with public leaves — the canonical paths stay the
// flat `crate::analysis`, `crate::completion`, … that the ABI bodies
// (and the tests) spell.
mod query;

pub use query::{analysis, check, completion, definition, inlay, references, signature_help};

pub mod deps;
pub mod env;
pub mod hover;
pub mod line_index;
pub mod mods;
pub mod semantic;
pub mod std_surface;

// ---- the language core's state (one instance per editing session) ----

struct Core {
    /// open documents by URI (the LSP's full-text sync)
    docs: HashMap<String, String>,
    /// the std surface + workspace files, in lookup order
    defs: Vec<DefIndex>,
    /// url → pin, from the workspace root's `rut.lock` (the host reads
    /// the file and hands the bytes to `set_lock`; the manifest
    /// grammar carries no pins). `parse_manifest` stamps the rows
    /// onto its url deps.
    lock_pins: BTreeMap<String, String>,
    /// the direct-dep module names — the open document's own package
    /// name plus every dep row its ROOT manifest declares (`deps`,
    /// `dev-deps` at the walk root). The host hands the
    /// set to `set_direct_deps` when it parses a root manifest;
    /// every dep index named here stamps `direct_dep`, the
    /// auto-import tier's gate (a transitive-only dep's `use` is the
    /// compiler's D5 error). Union-accumulating: multi-folder walks
    /// and re-walks re-add the same names, never shrink the world.
    direct_deps: HashSet<String>,
    /// the once-per-reason hint gate — every user-facing hint the
    /// walk and the ledger feeds emit keys here, so a reason names
    /// itself exactly once per session (never spam, however often the
    /// world re-walks). The extension's `hintOnce` map, moved in.
    hints_shown: HashSet<String>,
}

impl Core {
    /// the core as it boots — the std surface pre-indexed, no session
    /// state yet. The loop task owns this value exclusively; both
    /// constructors (`new` the wasm face, `spawn_native` the native
    /// harness) hand a fresh one to [`event_loop::run`].
    fn boot() -> Core {
        Core {
            docs: HashMap::new(),
            defs: crate::std_surface::indexes(),
            lock_pins: BTreeMap::new(),
            direct_deps: HashSet::new(),
            hints_shown: HashSet::new(),
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
    /// file under its module NAME (replace-by-origin), the direct-dep
    /// marker stamped from the set the way it is at this call.
    pub(crate) fn add_def_named_at(&mut self, uri: &str, name: &str, src: &str) {
        let mut idx =
            crate::deps::index_dep(name, uri, src, crate::analysis::mode_of(uri), None, &[]);
        idx.direct_dep = self.direct_deps.contains(name);
        self.replace_by_origin(uri, idx);
    }

    /// [`event_loop::Msg::AddDefMod`]'s body: index one dep source
    /// file at a MOD PATH (the pkg root `""`, a mounted child its
    /// `<path>`), answering the parsed file's own `mod` declarations —
    /// the walk's next hop (structure lives in the parse, never in
    /// path munging).
    pub(crate) fn add_def_mod_at(
        &mut self,
        uri: &str,
        name: &str,
        mod_path: &str,
        src: &str,
    ) -> Vec<wire::ModDecl> {
        let mut idx = crate::deps::index_dep_mod(
            name,
            uri,
            mod_path,
            src,
            crate::analysis::mode_of(uri),
            None,
            &[],
        );
        idx.direct_dep = self.direct_deps.contains(name);
        let decls: Vec<wire::ModDecl> = idx
            .mods
            .iter()
            .map(|m| wire::ModDecl {
                name: m.name.clone(),
                vis: if matches!(m.vis, rut_ast::ast::Vis::Pub) {
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
    fn doc_mods_of(&self, uri: &str) -> crate::mods::DocMods {
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

    /// the completion query over the stored doc — both completion
    /// entries' body (`complete_in` has already re-stored the text)
    fn complete_stored(&self, uri: &str, line: u32, ch: u32) -> String {
        let Some(src) = self.docs.get(uri).cloned() else {
            return "[]".to_string();
        };
        let doc = self.doc_mods_of(uri);
        let items = crate::analysis::complete_at(uri, &src, &self.defs, line, ch, &doc);
        json_envelope(serde_json::json!(items))
    }
}

/// the language core as the host holds it — one per editing session.
/// Not the state itself: the INBOX HANDLE. The loop task owns `Core`
/// exclusively; every entry posts one `event_loop::Msg` and awaits the
/// oneshot reply (the actor seam — the handlers are sync bodies over
/// `&mut Core`, so no bridge can borrow across an await and no handler
/// can post).
#[wasm_bindgen]
pub struct RutLsp {
    tx: futures::channel::mpsc::UnboundedSender<event_loop::Msg>,
}

#[wasm_bindgen]
impl RutLsp {
    /// the core boots with the std surface pre-indexed — the same
    /// the crate indexes at construction
    #[wasm_bindgen(constructor)]
    pub fn new() -> RutLsp {
        RutLsp {
            tx: event_loop::spawn(Core::boot()),
        }
    }

    // ---- the bridges ----
    //
    // Every entry: own the arguments (an async future must not borrow —
    // the JS caller may drop anything between microtasks), post one
    // message, await the reply. A dropped loop answers the loop-gone
    // error envelope; a panicked handler answers through its canceled
    // oneshot the same way.

    /// post one message, await its reply — the shared bridge body. Both
    /// failure shapes (the inbox refused the post, the reply channel
    /// dropped) map onto `Canceled`: the loop cannot answer.
    async fn rpc(
        &self,
        build: impl FnOnce(event_loop::Tx) -> event_loop::Msg,
    ) -> Result<event_loop::Reply, futures::channel::oneshot::Canceled> {
        let (tx, rx) = futures::channel::oneshot::channel();
        self.tx
            .unbounded_send(build(tx))
            .map_err(|_| futures::channel::oneshot::Canceled)?;
        rx.await
    }

    /// the text-envelope bridge: the JSON answer, or the loop-gone
    /// error envelope
    async fn text(&self, build: impl FnOnce(event_loop::Tx) -> event_loop::Msg) -> String {
        match self.rpc(build).await {
            Ok(event_loop::Reply::Text(s)) => s,
            Ok(_) => "null".to_string(), // unreachable: these arms answer Text
            Err(_) => event_loop::loop_gone(),
        }
    }

    /// the semantic-token legend (names in token-type index order)
    pub async fn legend(&self) -> String {
        self.text(|tx| event_loop::Msg::Legend { tx }).await
    }

    /// open / FULL-sync change: store the document, return one analysis —
    /// `{diags, tokens, symbols}` (the values the LSP face would push over
    /// the wire). The diags are the MERGED set (B1+B2): the parse lane's
    /// diags plus the check lane's — the doc's package (its stamped twin
    /// names it) compiles with its dep closure and the checker's per-file
    /// diags map back through the file stamps. No package context →
    /// parse-only, exactly the pre-B1 answer.
    pub async fn analyze(&self, uri: String, src: String) -> String {
        self.text(|tx| event_loop::Msg::Analyze { uri, src, tx })
            .await
    }

    /// close: drop the document (the workspace/std index stays). The
    /// await is ordering only — this lane answers no payload.
    pub async fn forget(&self, uri: String) {
        let _ = self.rpc(|tx| event_loop::Msg::Forget { uri, tx }).await;
    }

    /// hover at an LSP position — the doc index first, then std +
    /// workspace; `null` when there's nothing to say
    pub async fn hover(&self, uri: String, line: u32, ch: u32) -> String {
        self.text(|tx| event_loop::Msg::Hover { uri, line, ch, tx })
            .await
    }

    /// completions over the document's CURRENT text — `analyze`'s
    /// uri+src shape: the text is stored FIRST, so the tiers lex the
    /// text the cursor sits in, dot included. The extension debounces
    /// `analyze` (~200 ms) while completion fires on the keystroke — a
    /// stored-only read would race that window and answer a stale tail
    /// (typing `log.` would hit the bare keyword dump when the stored
    /// doc still ended at `log`), so the source always rides the call.
    pub async fn complete(&self, uri: String, src: String, line: u32, ch: u32) -> String {
        self.text(|tx| event_loop::Msg::Complete {
            uri,
            src,
            line,
            ch,
            tx,
        })
        .await
    }

    /// definitions at an LSP position — the doc index first, then std +
    /// workspace; `null` when the doc is unknown, `[]` when nothing
    /// resolves. Targets carry the doc URI, a workspace origin, or the std
    /// surface's true `rut/...` source path (relative — the extension
    /// resolves it against the workspace). The doc's file-module context
    /// rides from its stamped twin (see `doc_mods_of`) — the mod-edge
    /// clicks and the own-package use lanes key on it.
    pub async fn definition(&self, uri: String, line: u32, ch: u32) -> String {
        self.text(|tx| event_loop::Msg::Definition { uri, line, ch, tx })
            .await
    }

    /// type definitions at an LSP position (an expression -> its type's
    /// declaration) — same shape as `definition`
    pub async fn type_definition(&self, uri: String, line: u32, ch: u32) -> String {
        self.text(|tx| event_loop::Msg::TypeDefinition { uri, line, ch, tx })
            .await
    }

    /// inlay hints for a document range — the inline inference display:
    /// TYPE hints on unannotated bindings + PARAMETER hints at exact-arity
    /// call sites. `null` when the doc is unknown, `[]` when nothing
    /// resolves. The hints are full LSP values (position + label + kind +
    /// tooltip); the client renders them.
    pub async fn inlay(
        &self,
        uri: String,
        start_line: u32,
        start_ch: u32,
        end_line: u32,
        end_ch: u32,
    ) -> String {
        self.text(|tx| event_loop::Msg::Inlay {
            uri,
            start_line,
            start_ch,
            end_line,
            end_ch,
            tx,
        })
        .await
    }

    /// references — the definition index read BACKWARDS: every position
    /// whose go-to-definition lands on the declaration under `line`/`ch`.
    /// Within-file through the binding pass (shadow-aware), cross-file
    /// through the use graph's reverse edges. `include_decl` (0/1) follows
    /// the LSP toggle. `null` when the doc is unknown, `[]` when nothing
    /// resolves — the same `{uri, range}` Location JSON `definition`
    /// speaks.
    pub async fn references(&self, uri: String, line: u32, ch: u32, include_decl: u32) -> String {
        self.text(|tx| event_loop::Msg::References {
            uri,
            line,
            ch,
            include_decl,
            tx,
        })
        .await
    }

    /// signature help at an LSP position — the callee resolves through the
    /// same machinery the parameter-name hints ride (free calls by unique
    /// recorded params, methods through the receiver's type head), the
    /// signature renders verbatim, the active parameter comes from the
    /// comma/paren depth at the request position. Ambiguity or arity
    /// mismatch → `null` (never wrong help).
    pub async fn signature_help(&self, uri: String, line: u32, ch: u32) -> String {
        self.text(|tx| event_loop::Msg::SignatureHelp { uri, line, ch, tx })
            .await
    }

    /// index one workspace file (the host's stand-in for an fs walk —
    /// server's fs walk — the 500-file cap is the caller's). Re-indexing
    /// a URI replaces its entry. The await is ordering only.
    pub async fn add_def(&self, uri: String, src: String) {
        let _ = self
            .rpc(|tx| event_loop::Msg::AddDef { uri, src, tx })
            .await;
    }

    // ---- the deps face (crate::deps — one engine, no JS-side parsing) ----

    /// parse one `rut.jsonc` — the dep-table JSON out (the module's
    /// identity, its entry sources with the mode each indexes in, and
    /// its dep rows with the url rows' cache paths and lock-sourced
    /// pins). A malformed manifest is the error envelope — the host
    /// surfaces it as a hint.
    pub async fn parse_manifest(&self, text: String) -> String {
        self.text(|tx| event_loop::Msg::ParseManifest { text, tx })
            .await
    }

    /// hand the core the READ-ONLY host environment — one object
    /// carrying `read` / `readDir` / `isDir` / `notify` (the JS face's
    /// `HostEnv`, `js/index.d.ts`). The four functions are cached off
    /// the object at this call ([`env::JsEnv`]) and the loop stores
    /// the capability; the dep walk ([`resolve_deps`](Self::resolve_deps))
    /// reads through it and nothing else does. A host that never
    /// calls it keeps the env-less core (the demo path), and its
    /// `resolve_deps` answers the no-env error envelope — loud, never
    /// a silent empty walk. Replace-on-reinject: a later call swaps
    /// the capability. The await is ordering only.
    pub async fn set_env(&self, env: js_sys::Object) {
        let _ = self
            .rpc(|tx| event_loop::Msg::SetEnv {
                env: Rc::new(crate::env::JsEnv::new(env)) as Rc<dyn crate::env::Io>,
                tx,
            })
            .await;
    }

    /// the READ-ONLY dep walk (`walk` — the extension's manifest walk
    /// moved into the crate): one JSON array of workspace-root paths
    /// in, the walk's answer out. Per root: its `rut.lock` feeds the
    /// pin ledger, its `rut.jsonc` walks — entries and mod trees index
    /// through the SAME index lanes the workspace scan uses, path
    /// rows recurse cycle-safe under the depth/file budgets, and url
    /// rows mount from the global dep store (`$RUT_HOME/deps`, the
    /// address rut-driver derives) when its `meta.json` vouches the
    /// lock pin — never fetched, never written; a refused row answers
    /// one `missing` row plus ONE hint per reason through the env's
    /// notify lane. The answer is `{"missing": [...]}` (see
    /// `wire::WalkOut`); the mounted world itself is the defs store's,
    /// exactly as a host-indexed dep leaves it.
    pub async fn resolve_deps(&self, roots_json: String) -> String {
        let roots = match serde_json::from_str::<Vec<String>>(&roots_json) {
            Ok(r) => r,
            Err(e) => return json_envelope(wire::Error::new(e.to_string())),
        };
        self.text(move |tx| event_loop::Msg::ResolveDeps { roots, tx })
            .await
    }

    /// index one dep source file under its module NAME at a MOD PATH —
    /// the mod-aware twin of `add_def_named` (the dep walk's per-file
    /// move for a mod-carrying package: the root at `""`, each child at
    /// its path). Returns the parsed file's own `mod` declarations —
    /// `{ "decls": [{ "name", "vis" }] }` — so the host's mount walk
    /// reads its next hop from the wasm's parse (JS moves bytes, the
    /// module parses; no JS-side rut parsing).
    pub async fn add_def_mod(
        &self,
        uri: String,
        name: String,
        mod_path: String,
        src: String,
    ) -> String {
        self.text(|tx| event_loop::Msg::AddDefMod {
            uri,
            name,
            mod_path,
            src,
            tx,
        })
        .await
    }
}

impl Default for RutLsp {
    fn default() -> Self {
        Self::new()
    }
}

/// the native ABI harness — the executor seam's native half (cfg-gated:
/// the pool never ships in the wasm). The loop + its bridges over a
/// deterministic single-threaded `LocalPool`; tests drive
/// `pool.run_until(bridge)` and own the ordering. (A wasm `RutLsp`
/// parks its loop on `spawn_local` instead — one cfg seam, two
/// executors.)
#[cfg(not(target_family = "wasm"))]
#[doc(hidden)]
impl RutLsp {
    pub fn spawn_native() -> (RutLsp, futures::executor::LocalPool) {
        use futures::task::LocalSpawnExt;
        let (tx, rx) = event_loop::pair();
        let pool = futures::executor::LocalPool::new();
        pool.spawner()
            .spawn_local(event_loop::run(Core::boot(), rx))
            .expect("the loop parks on the fresh pool");
        (RutLsp { tx }, pool)
    }

    /// the native tests' env seam: hand the loop ANY [`env::Io`] (the
    /// recording fs wrapper the walk tests drive) through the same
    /// door `set_env` posts its `JsEnv` — one capability lane, two
    /// constructors. cfg-gated like `spawn_native`: the wasm face
    /// builds its `JsEnv` at the bridge, native tests have no JS.
    pub async fn set_env_io(&self, io: Rc<dyn env::Io>) {
        let _ = self.rpc(|tx| event_loop::Msg::SetEnv { env: io, tx }).await;
    }
}

// ---- the envelope writers (the answers are JSON strings) ----
//
// The field names live in `wire` — this module only serializes. The
// error envelope spells `{"error": …}`; every other answer is the
// struct `wire` declares, so the JS face's `.d.ts` has one mirror to
// keep true.

fn json_envelope<T: serde::Serialize>(value: T) -> String {
    match serde_json::to_string(&value) {
        Ok(json) => json,
        Err(_) => "null".to_string(), // unreachable for these types
    }
}

/// the definition-target envelope — `DefLocation` is serde-free in the
/// queries (they carry no serializer), so this module spells
/// the LSP `Location` JSON: `{ uri, range }`
fn locations_envelope(locs: &[crate::definition::DefLocation]) -> String {
    let value: Vec<wire::Location> = locs
        .iter()
        .map(|l| wire::Location {
            uri: l.uri.clone(),
            range: l.range,
        })
        .collect();
    json_envelope(value)
}
