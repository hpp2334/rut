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
//!   legend() -> String                         token-type names, JSON array
//!   analyze(uri, src) -> String                store the doc + return
//!                                              {diags, tokens, symbols}
//!   forget(uri)                                drop a closed doc
//!   hover(uri, line, ch) -> String             LSP Hover JSON or null
//!   complete(uri, line, ch) -> String          LSP CompletionItem array
//!   complete_in(uri, src, line, ch) -> String  the same array over the
//!                                              CURRENT text (stored
//!                                              first — the doc-sync
//!                                              lane; completion fires
//!                                              on the keystroke, the
//!                                              analyze lane debounces)
//!   definition(uri, line, ch) -> String        LSP Location array or null
//!   type_definition(uri, line, ch) -> String   same shape
//!   inlay(uri, sl, sc, el, ec) -> String       LSP InlayHint array or null
//!   references(uri, line, ch, incl) -> String  LSP Location array or null
//!   signature_help(uri, line, ch) -> String    LSP SignatureHelp or null
//!   add_def(uri, src)                          index a workspace file
//!   parse_manifest(text) -> String             rut.jsonc bytes -> the
//!                                              dep-table JSON (or the
//!                                              error envelope)
//!   set_lock(text) -> String                   rut.lock bytes -> the
//!                                              url→pin ledger
//!                                              parse_manifest stamps
//!                                              onto url rows
//!   set_direct_deps(names) -> String           the root manifest's dep
//!                                              names (JSON array) — the
//!                                              direct-dep stamp the dep
//!                                              indexes carry (the
//!                                              auto-import tier's gate)
//!   add_def_named(uri, name, src)              index a dep source file
//!                                              under its module NAME
//!   add_def_mod(uri, name, path, src)          index a dep source at a
//!                                              MOD PATH; the envelope
//!                                              names the file's own mod
//!                                              decls (the host's mount
//!                                              walk reads its next hop
//!                                              from them)
//!   add_bundle(bytes) -> String                .rutbundle bytes -> index
//!                                              the bundle's own sources;
//!                                              envelope names module +
//!                                              entry paths (or the error)
//!   extract_bundle(bytes) -> String            .rutbundle bytes -> the
//!                                              editable surface envelope
//!                                              {module, namespace,
//!                                              consts, files:[{path,
//!                                              src, mode, mod_path}]}
//!                                              — NO indexing, JS owns
//!                                              the disk
//!   doc_len(uri) -> u32                        an open doc's stored
//!                                              length (the smoke's
//!                                              round-trip check)
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
//! These are the deps face: the manifest/bundle plumbing runs through
//! [`crate::deps`] (pure — bytes in, values out) into the SAME
//! `state.defs` store `add_def` writes, so a dep's surface resolves
//! in hover/completion/definition exactly like a workspace file — one
//! index, no drift. `extract_bundle` is the same plumbing's extract
//! lane — bytes in, the surface envelope out, NO index write (JS owns
//! the disk; re-indexing goes back through `add_def`/`add_def_mod`).
//! `parse_manifest`/`add_bundle`/`extract_bundle` answer with an error
//! envelope (`{"error": …}`) on bad bytes; the host surfaces it as a
//! hint, never silent unparsed state.
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

use crate::hover::DefIndex;
use wasm_bindgen::prelude::wasm_bindgen;

mod event_loop;
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

    /// completions at an LSP position (member after `.`, the use-path and
    /// qualified-position tiers) — the doc's file-module context rides
    /// from its stamped twin (see `doc_mods_of`)
    pub async fn complete(&self, uri: String, line: u32, ch: u32) -> String {
        self.text(|tx| event_loop::Msg::Complete { uri, line, ch, tx })
            .await
    }

    /// completions over the document's CURRENT text — the sync twin of
    /// `complete`, `analyze`'s uri+src shape: the text is stored FIRST,
    /// so the tiers lex the text the cursor sits in, dot included.
    /// The extension debounces `analyze` (~200 ms) while completion
    /// fires on the keystroke — the stored-only read raced that window
    /// and answered a stale tail (typing `log.` hit the bare keyword
    /// dump when the stored doc still ended at `log`).
    pub async fn complete_in(&self, uri: String, src: String, line: u32, ch: u32) -> String {
        self.text(|tx| event_loop::Msg::CompleteIn {
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

    /// hand the dep-table face the workspace root's `rut.lock` bytes — the
    /// url→pin ledger `parse_manifest` stamps onto url rows (the lock is
    /// the pin's only source; the manifest grammar retired `sha256`).
    /// Answers `{"pins": N}`, or the `{"error": …}` envelope for a
    /// malformed lock — the host surfaces it as a hint, and the (empty)
    /// ledger stands so url rows answer `sha256: null` (the unpinned
    /// refusal), never guessed pins.
    pub async fn set_lock(&self, text: String) -> String {
        self.text(|tx| event_loop::Msg::SetLock { text, tx }).await
    }

    /// hand the dep face the direct-dep module names — the open
    /// document's own package name plus every dep row its ROOT manifest
    /// declares. Every dep index named here stamps `direct_dep` (the
    /// auto-import tier's gate): `add_def_named` / `add_def_mod`
    /// / `add_bundle` consult the set at index time, so the host
    /// calls this once per root manifest BEFORE mounting its rows. The
    /// text is a JSON array of names (`["pouch", "calc"]`); names
    /// ACCUMULATE (a set — multi-folder walks and re-walks re-add the
    /// same names). Answers `{"direct": N}` (the set size), or the error
    /// envelope for malformed JSON.
    pub async fn set_direct_deps(&self, names_json: String) -> String {
        self.text(|tx| event_loop::Msg::SetDirectDeps { names_json, tx })
            .await
    }

    /// index one dep source file under its module NAME — `deps::index_dep`
    /// into the same store `add_def` writes (re-indexing an origin
    /// replaces its entry), so `use <name>::` resolves it like any index.
    /// The mode rides the URI convention (`mode_of`, identical to
    /// `add_def`); namespace/consts have no voice here — a dep
    /// needing them indexes through `add_bundle`, whose manifest is in
    /// the bytes. The direct-dep marker stamps from the host's set.
    /// The await is ordering only.
    pub async fn add_def_named(&self, uri: String, name: String, src: String) {
        let _ = self
            .rpc(|tx| event_loop::Msg::AddDefNamed { uri, name, src, tx })
            .await;
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

    /// unpack one `.rutbundle` — `deps::bundle_sources` reads the
    /// archive's own manifest, then each entry source indexes under the
    /// module NAME the bundle carries (origin `bundle:<module>/<path>`,
    /// replace-by-origin like every index). The envelope names the
    /// module + the entry paths indexed, or the error — a
    /// corrupt/foreign archive never lands bytes. The bytes param is
    /// owned (`Vec<u8>`): the archive is a zip, not text.
    pub async fn add_bundle(&self, bytes: Vec<u8>) -> String {
        self.text(|tx| event_loop::Msg::AddBundle { bytes, tx })
            .await
    }

    /// unpack one `.rutbundle` into the envelope WITHOUT indexing — the
    /// pure-extract twin of `add_bundle`: bytes in, the editable
    /// surface out as `{ module, namespace, consts, files: [{ path, src,
    /// mode, mod_path }] }`. JS owns the disk: the extension writes the
    /// files and re-indexes them through `add_def`/`add_def_mod`
    /// (real `file://` origins, the go-to-definition face), so this lane
    /// has NO index side effect. `mod_path` rides per file
    /// (`deps::bundle_file_mod_path` — the rows-lane root spells `""`, a
    /// mounted child its `<path>`; a flat file stays `null`), so the host
    /// never re-derives the mount rule in TypeScript. A corrupt/foreign
    /// archive is the error envelope, never bytes.
    pub async fn extract_bundle(&self, bytes: Vec<u8>) -> String {
        self.text(|tx| event_loop::Msg::ExtractBundle { bytes, tx })
            .await
    }

    /// A SOURCE-ZIP cache entry's extract — the source transport's editor
    /// face: the zip's own package files (`src/**`) as the editable tree,
    /// each with its mod path derived from the `src/` shell (`src/mod.rut`
    /// → `""` the root, `src/a/mod.rut` → `"a"`, `src/a.rut` → `"a"`).
    /// The embedded `deps/<identity>/` trees belong to other pkgs and stay
    /// out (the consumer's own dep rows cover them).
    pub async fn extract_srczip(&self, bytes: Vec<u8>) -> String {
        self.text(|tx| event_loop::Msg::ExtractSrczip { bytes, tx })
            .await
    }

    /// an open doc's stored text length — the smoke test's round-trip
    /// check. A dropped loop answers 0 — the doc's length is unknowable,
    /// which is already this entry's unknown-doc answer.
    pub async fn doc_len(&self, uri: String) -> u32 {
        match self.rpc(|tx| event_loop::Msg::DocLen { uri, tx }).await {
            Ok(event_loop::Reply::Len(n)) => n,
            _ => 0,
        }
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
