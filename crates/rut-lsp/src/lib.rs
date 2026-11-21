//! rut-lsp — the rut language core for language servers: the queries
//! (`crate::query::analysis` and the sibling modules below), the
//! values, the actor event loop, and the session state, ONE crate
//! with no wasm coupling. Hosts embed it directly — the wasm-bindgen
//! face (the exported `RutLsp` class, the JS envelope glue) lives
//! beside it in `rut-lsp-wasm`, a thin delegation over this crate's
//! [`Lsp`].
//!
//! Every method on [`Lsp`] is an ASYNC RPC bridge (the actor seam):
//! the call posts one [`event_loop::Msg`] on the loop's mpsc inbox and
//! awaits a oneshot reply, while ONE long-lived loop task owns the
//! `SessionState` exclusively and answers messages FIFO — sync
//! handlers over `&mut SessionState` (see [`event_loop`]). A dropped
//! loop answers the error envelope; a panicked handler isolates to its
//! own reply channel. Every text answer is the JSON envelope:
//!
//!   set_env_io(io)                             hand the core the
//!                                              READ-ONLY host
//!                                              environment (`Io`:
//!                                              read/readDir/isDir/
//!                                              notify, plus the
//!                                              OPTIONAL clock
//!                                              `wakeAfter` — see
//!                                              `env`) — the dep
//!                                              walk's capability
//!   resolve_deps(roots_json) -> String         the READ-ONLY dep walk over
//!                                              the injected env: one JSON
//!                                              array of workspace roots in,
//!                                              the walk's answer (`WalkOut`:
//!                                              the url rows it refused —
//!                                              informational rows for
//!                                              hosts) out; every mounted dep
//!                                              lands in the SAME defs store
//!                                              the index lanes write (see
//!                                              `walk`) — the EAGER manual
//!                                              door
//!   reload_deps(roots_json) -> String          RELOAD the deps — the
//!                                              level-triggered door: the
//!                                              walk defers under the loop's
//!                                              own 200ms window (the env's
//!                                              optional `wakeAfter` clock)
//!                                              and coalesces; the promise
//!                                              carries the WalkOut once the
//!                                              walk runs (`null` when a
//!                                              later reload supersedes it)
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
//!   index_file(uri, src)                       index a workspace file
//!   parse_manifest(text) -> String             rut.jsonc bytes -> the
//!                                              dep-table JSON (or the
//!                                              error envelope)
//!   index_mod_at(uri, pkg, path, src)          index a source file at a
//!                                              MOD PATH (the workspace
//!                                              scan lane); the envelope
//!                                              names the file's own mod
//!                                              decls (the scan's mount
//!                                              walk reads its next hop
//!                                              from them)
//!
//! THE SURFACE LAW — the ABI is exactly three lanes: the provider
//! queries above; the workspace-scan push (`index_file`/`index_mod_at`/
//! `parse_manifest` — the host's `findFiles` sees editor ignore
//! semantics the env cannot); and the dep pull (`set_env` +
//! `resolve_deps`/`reload_deps`). NO entry mutates dep state except through
//! the injected env — the lock feed, the direct-dep stamp, and the named
//! index lanes are SessionState internals the walk drives, not messages. The
//! old push family (`set_lock`, `set_direct_deps`, `add_def_named`,
//! `add_bundle`, `extract_bundle`, `extract_srczip`, `doc_len`) lived
//! in the blind-wasm era and is gone; its behaviors survive through
//! the walk (see `walk` and its tests).
//!
//! An LSP serves a long-lived editing session, so the instance OWNS
//! its session state (open docs + the index) for its lifetime — the
//! host holds one [`Lsp`] per boot and mutates it per request. The
//! state lives in `SessionState` (see [`state`]), owned exclusively by
//! the loop task ([`event_loop`]); the host's [`Lsp`] is the inbox
//! handle. Calls serialize on the loop's FIFO — concurrent unresolved
//! calls resolve in send order, and no caller ever borrows the state.
//!
//! The executor is the HOST's choice: [`Lsp::boot`] takes one spawn
//! closure (the wasm face hands `wasm_bindgen_futures::spawn_local`),
//! and native tests take the whole pool from [`Lsp::spawn_native`].

use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

mod event_loop;
mod walk;
mod wire;

// ---- the session state — the editing session's accumulated facts,
// moved whole to `state`; the loop and the boot doors spell
// `state::SessionState` ----
mod state;

// ---- the language core — the queries and the values ----
//
// `query` and every sibling below are PRIVATE impl: the host's whole
// world is the [`Lsp`] handle and its JSON envelopes — no internal
// name crosses the crate boundary (the face crate implements
// [`env::Io`] and nothing else). The canonical paths inside the crate
// spell the module tree (`crate::query::analysis`, `crate::world`, …).
mod query;

mod deps;
pub mod env;
// the classifier/document layer — absorbed from `rut-semantic` and
// SEALED: the private `mod` decl makes even the module's own `pub`
// items invisible outside the crate, so the queries keep one spelling
// (`crate::semantic::…`) and no consumer can reach past the ABI
mod prelude;
mod semantic;

// ---- the mounted world — the session's accumulated language facts,
// the decl layer + the binding pass every query reads (see the
// module's own doc). One-way layering: `query` spells `world`, never
// the reverse. ----
mod world;

// the unit-test tree — the sealed modules' tests live INSIDE the
// crate (an integration test in `tests/` cannot spell private paths)
#[cfg(test)]
mod tests;

/// the language core as the host holds it — one per editing session.
/// Not the state itself: the INBOX HANDLE. The loop task owns `SessionState`
/// exclusively; every entry posts one `event_loop::Msg` and awaits the
/// oneshot reply (the actor seam — the handlers are sync bodies over
/// `&mut SessionState`, so no bridge can borrow across an await and no handler
/// can post).
pub struct Lsp {
    tx: futures::channel::mpsc::UnboundedSender<event_loop::Msg>,
}

impl Lsp {
    /// the session boots with the prelude pre-indexed — the loop's
    /// parking is the CALLER's one choice: `spawn` receives the loop
    /// body as a boxed future and hands it to its executor (the wasm
    /// face: `wasm_bindgen_futures::spawn_local`; the native tests:
    /// [`Lsp::spawn_native`'s pool). Dropping the future without
    /// running it is the honest dead-loop shape — every bridge then
    /// answers the loop-gone envelope.
    pub fn boot(spawn: impl FnOnce(Pin<Box<dyn Future<Output = ()>>>)) -> Lsp {
        let (tx, rx) = event_loop::pair();
        spawn(Box::pin(event_loop::run(
            state::SessionState::boot(),
            tx.clone(),
            rx,
        )));
        Lsp { tx }
    }

    // ---- the bridges ----
    //
    // Every entry: own the arguments (an async future must not borrow —
    // the caller may drop anything between microtasks), post one
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
            // a void completion (a superseded reload answers Done — its
            // WalkOut merged into the later reload's promise)
            Ok(_) => "null".to_string(),
            Err(_) => event_loop::loop_gone(),
        }
    }

    /// the semantic-token legend (names in token-type index order)
    pub async fn legend(&self) -> String {
        self.text(|tx| event_loop::Msg::Cmd(event_loop::Cmd::Legend { tx }))
            .await
    }

    /// open / FULL-sync change: store the document, return one analysis —
    /// `{diags, tokens, symbols}` (the values the LSP face would push over
    /// the wire). The diags are the MERGED set (B1+B2): the parse lane's
    /// diags plus the check lane's — the doc's package (its stamped twin
    /// names it) compiles with its dep closure and the checker's per-file
    /// diags map back through the file stamps. No package context →
    /// parse-only, exactly the pre-B1 answer.
    pub async fn analyze(&self, uri: String, src: String) -> String {
        self.text(|tx| event_loop::Msg::Cmd(event_loop::Cmd::Analyze { uri, src, tx }))
            .await
    }

    /// close: drop the document (the prelude/workspace index stays). The
    /// await is ordering only — this lane answers no payload.
    pub async fn forget(&self, uri: String) {
        let _ = self
            .rpc(|tx| event_loop::Msg::Cmd(event_loop::Cmd::Forget { uri, tx }))
            .await;
    }

    /// hover at an LSP position — the doc index first, then the
    /// prelude + workspace; `null` when there's nothing to say
    pub async fn hover(&self, uri: String, line: u32, ch: u32) -> String {
        self.text(|tx| event_loop::Msg::Cmd(event_loop::Cmd::Hover { uri, line, ch, tx }))
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
        self.text(|tx| {
            event_loop::Msg::Cmd(event_loop::Cmd::Complete {
                uri,
                src,
                line,
                ch,
                tx,
            })
        })
        .await
    }

    /// definitions at an LSP position — the doc index first, then the
    /// prelude + workspace; `null` when the doc is unknown, `[]` when nothing
    /// resolves. Targets carry the doc URI, a workspace origin, or the prelude's
    /// true `rut/...` source path (relative — the extension
    /// resolves it against the workspace). The doc's file-module context
    /// rides from its stamped twin (see `doc_mods_of`) — the mod-edge
    /// clicks and the own-package use lanes key on it.
    pub async fn definition(&self, uri: String, line: u32, ch: u32) -> String {
        self.text(|tx| event_loop::Msg::Cmd(event_loop::Cmd::Definition { uri, line, ch, tx }))
            .await
    }

    /// type definitions at an LSP position (an expression -> its type's
    /// declaration) — same shape as `definition`
    pub async fn type_definition(&self, uri: String, line: u32, ch: u32) -> String {
        self.text(|tx| event_loop::Msg::Cmd(event_loop::Cmd::TypeDefinition { uri, line, ch, tx }))
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
        self.text(|tx| {
            event_loop::Msg::Cmd(event_loop::Cmd::Inlay {
                uri,
                start_line,
                start_ch,
                end_line,
                end_ch,
                tx,
            })
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
        self.text(|tx| {
            event_loop::Msg::Cmd(event_loop::Cmd::References {
                uri,
                line,
                ch,
                include_decl,
                tx,
            })
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
        self.text(|tx| event_loop::Msg::Cmd(event_loop::Cmd::SignatureHelp { uri, line, ch, tx }))
            .await
    }

    /// index one workspace file (the host's stand-in for an fs walk —
    /// server's fs walk — the 500-file cap is the caller's). Re-indexing
    /// a URI replaces its entry. The await is ordering only.
    pub async fn index_file(&self, uri: String, src: String) {
        let _ = self
            .rpc(|tx| event_loop::Msg::Cmd(event_loop::Cmd::IndexFile { uri, src, tx }))
            .await;
    }

    // ---- the deps face (crate::deps — one engine, no JS-side parsing) ----

    /// parse one `rut.jsonc` — the dep-table JSON out (the module's
    /// identity, its entry sources with the mode each indexes in, and
    /// its dep rows with the url rows' cache paths and lock-sourced
    /// pins). A malformed manifest is the error envelope — the host
    /// surfaces it as a hint.
    pub async fn parse_manifest(&self, text: String) -> String {
        self.text(|tx| event_loop::Msg::Cmd(event_loop::Cmd::ParseManifest { text, tx }))
            .await
    }

    /// hand the core the READ-ONLY host environment — one [`env::Io`]
    /// implementation carrying `read` / `readDir` / `isDir` / `notify`
    /// (the wasm face's `HostEnv`, `js/index.d.ts`). The capability
    /// stores as-is (the loop installs the wake source); the dep walk
    /// ([`resolve_deps`](Self::resolve_deps)) reads through it and
    /// nothing else does. A host that never calls it keeps the
    /// env-less core, and its `resolve_deps` answers the no-env error
    /// envelope — loud, never a silent empty walk. Replace-on-reinject:
    /// a later call swaps the capability. The await is ordering only.
    pub async fn set_env_io(&self, io: Rc<dyn env::Io>) {
        let _ = self.rpc(|tx| event_loop::Msg::SetEnv { env: io, tx }).await;
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
        self.text(move |tx| event_loop::Msg::Cmd(event_loop::Cmd::ResolveDeps { roots, tx }))
            .await
    }

    /// RELOAD the deps — the level-triggered door onto the same walk
    /// [`resolve_deps`](Self::resolve_deps) runs eagerly: the walk
    /// DEFERS under the loop's own debounce window (200ms, armed
    /// through the env's optional `wakeAfter` clock — see
    /// [`env::Io::arm_wake`]) and COALESCES, so a burst of reloads
    /// walks once over the freshest roots (a superseded reload's
    /// promise answers `null` — merged, not lost; the LAST reload's
    /// promise carries the world). The promise resolves with the walk's
    /// `WalkOut` envelope once the deferred walk runs — the answer
    /// rides this RPC's own reply channel, no push lane exists. No
    /// clock on the env ⇒ the walk runs eagerly in-turn (D3), exactly
    /// the eager door.
    pub async fn reload_deps(&self, roots_json: String) -> String {
        let roots = match serde_json::from_str::<Vec<String>>(&roots_json) {
            Ok(r) => r,
            Err(e) => return json_envelope(wire::Error::new(e.to_string())),
        };
        self.text(move |tx| event_loop::Msg::Cmd(event_loop::Cmd::ReloadDeps { roots, tx }))
            .await
    }

    /// index one dep source file under its module NAME at a MOD PATH —
    /// the mod-aware twin of `index_named_at` (the dep walk's per-file
    /// move for a mod-carrying package: the root at `""`, each child at
    /// its path). Returns the parsed file's own `mod` declarations —
    /// `{ "decls": [{ "name", "vis" }] }` — so the host's mount walk
    /// reads its next hop from the core's parse (JS moves bytes, the
    /// module parses; no JS-side rut parsing).
    pub async fn index_mod_at(
        &self,
        uri: String,
        pkg: String,
        mod_path: String,
        src: String,
    ) -> String {
        self.text(|tx| {
            event_loop::Msg::Cmd(event_loop::Cmd::IndexModAt {
                uri,
                pkg,
                mod_path,
                src,
                tx,
            })
        })
        .await
    }
}

/// the native ABI harness — the executor seam's native half (cfg-gated:
/// the pool never ships in the wasm). The loop + its bridges over a
/// deterministic single-threaded `LocalPool`; tests drive
/// `pool.run_until(bridge)` and own the ordering. (A wasm host parks
/// its loop on `spawn_local` instead — one boot seam, two executors.)
#[cfg(not(target_family = "wasm"))]
impl Lsp {
    pub fn spawn_native() -> (Lsp, futures::executor::LocalPool) {
        use futures::task::LocalSpawnExt;
        let (tx, rx) = event_loop::pair();
        let pool = futures::executor::LocalPool::new();
        // tx.clone() is the loop's SELF-ADDRESS — the wake source the
        // SetEnv arm installs captures it (the clock's tick re-enters
        // this loop), while `tx` moves to the host handle
        pool.spawner()
            .spawn_local(event_loop::run(state::SessionState::boot(), tx.clone(), rx))
            .expect("the loop parks on the fresh pool");
        (Lsp { tx }, pool)
    }
}

// ---- the envelope writers (the answers are JSON strings) ----
//
// The field names live in `wire` — this module only serializes. The
// error envelope spells `{"error": …}`; every other answer is the
// struct `wire` declares, so the JS face's `.d.ts` has one mirror to
// keep true.

pub(crate) fn json_envelope<T: serde::Serialize>(value: T) -> String {
    match serde_json::to_string(&value) {
        Ok(json) => json,
        Err(_) => "null".to_string(), // unreachable for these types
    }
}

/// the definition-target envelope — `DefLocation` is serde-free in the
/// queries (they carry no serializer), so this module spells
/// the LSP `Location` JSON: `{ uri, range }`
pub(crate) fn locations_envelope(locs: &[crate::query::definition::DefLocation]) -> String {
    let value: Vec<wire::Location> = locs
        .iter()
        .map(|l| wire::Location {
            uri: l.uri.clone(),
            range: l.range,
        })
        .collect();
    json_envelope(value)
}
