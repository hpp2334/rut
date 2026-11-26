//! rut-lsp-wasm — the wasm-bindgen FACE over the rut language core.
//! The queries, the values, the actor event loop, and the session
//! state live in [`rut_lsp`] (the core crate — native-clean AND
//! wasm32-clean; the VS Code extension instantiates this face and
//! registers its providers directly — no server process, no
//! per-platform binaries). The face is the boundary only: ONE class,
//! [`RutLsp`], every method a one-line delegation onto the core's
//! [`rut_lsp::Lsp`] handle — the names, the signatures, and the JSON
//! envelopes are frozen (the JS glue in `js/` and the hosts depend on
//! them byte for byte).
//!
//! The module exports ONE class; the host constructs it with
//! `new RutLsp()` and calls its methods. Every method is an ASYNC RPC
//! bridge (a `Promise` — the actor seam lives in the core): the call
//! posts one message on the loop's mpsc inbox and awaits a oneshot
//! reply, while ONE long-lived loop task owns the session state
//! exclusively and answers FIFO. A dropped loop answers the error
//! envelope. Every text answer is the JSON envelope; the wasm-bindgen
//! runtime owns the string copies both ways (no `unsafe` anywhere in
//! this crate).
//!
//!   new RutLsp()                               the language core, with
//!                                              the core prelude pre-indexed;
//!                                              the loop parks on the JS
//!                                              microtask queue
//!   set_env(env)                               hand the core the READ-ONLY
//!                                              host environment (`HostEnv`:
//!                                              read/readDir/isDir/notify,
//!                                              plus the OPTIONAL clock
//!                                              `wakeAfter` — wrapped into
//!                                              the core's `env::Io` by
//!                                              `JsEnv`, `js_env`)
//!
//! plus the provider queries, the workspace-scan push, and the dep
//! pull — all delegated verbatim (their contracts are spelled on the
//! core's `Lsp` methods).
//!
//! The package the hosts consume is `pkg/`: `wasm-pack build --target
//! web` writes the generated glue there, then `scripts/build-lsp-pkg.mjs`
//! installs this crate's JS face (`js/index.js` — the one place that
//! `JSON.parse`s the answers) and its declarations (`js/index.d.ts`)
//! beside it. Hosts import `pkg/index.js` and never declare these
//! shapes themselves; the field names live in the core's `wire` (the
//! serializers) and the JS face's d.ts mirror, locked by `tsc` in
//! every host and by the extension's wire-shape e2e block.
//!
//! THE SURFACE LAW — the ABI is exactly three lanes: the provider
//! queries (`analyze`, `hover`, `complete`/`signature-help` with src,
//! definition/inlay/references, `legend`, `forget`);
//! the workspace-scan push (`index_file`/`index_mod_at`/`parse_manifest`
//! — the host's `findFiles` respects editor ignore semantics the env
//! cannot); and the dep pull (`set_env` + `resolve_deps`/
//! `reload_deps`, the deferred door the hosts ride; `resolve_deps`
//! stays as the eager manual door — tests, cold boot, env-less
//! hosts). No entry may bypass the env capability to mutate dep
//! state — the law lives in the core now; this face cannot spell a
//! violation (it only forwards the core's own messages).

use std::rc::Rc;

use wasm_bindgen::prelude::wasm_bindgen;

mod js_env;
use js_env::JsEnv;

/// the wasm face of the language core — one per editing session.
/// Not the state itself: the DELEGATION HANDLE over the core's
/// [`rut_lsp::Lsp`] inbox (the loop task owns the session state
/// exclusively; see the core's `Lsp` docs for the actor seam).
#[wasm_bindgen]
pub struct RutLsp {
    core: rut_lsp::Lsp,
}

#[wasm_bindgen]
impl RutLsp {
    /// the session boots with the prelude pre-indexed — the loop
    /// parks on the JS microtask queue (`spawn_local`), the wasm
    /// spelling of the core's boot seam
    #[wasm_bindgen(constructor)]
    pub fn new() -> RutLsp {
        RutLsp {
            core: rut_lsp::Lsp::boot(wasm_bindgen_futures::spawn_local),
        }
    }

    // ---- the bridges — one-line delegations, the JS API is frozen ----

    /// the semantic-token legend (names in token-type index order)
    pub async fn legend(&self) -> String {
        self.core.legend().await
    }

    /// open / FULL-sync change: store the document, return one
    /// analysis — `{diags, tokens, symbols}`
    pub async fn analyze(&self, uri: String, src: String) -> String {
        self.core.analyze(uri, src).await
    }

    /// close: drop the document (the prelude/workspace index stays)
    pub async fn forget(&self, uri: String) {
        self.core.forget(uri).await;
    }

    /// hover at an LSP position — `null` when there's nothing to say
    pub async fn hover(&self, uri: String, line: u32, ch: u32) -> String {
        self.core.hover(uri, line, ch).await
    }

    /// completions over the document's CURRENT text (`uri`+`src`,
    /// the doc-sync lane)
    pub async fn complete(&self, uri: String, src: String, line: u32, ch: u32) -> String {
        self.core.complete(uri, src, line, ch).await
    }

    /// definitions at an LSP position — LSP `Location` array or null
    pub async fn definition(&self, uri: String, line: u32, ch: u32) -> String {
        self.core.definition(uri, line, ch).await
    }

    /// type definitions at an LSP position — same shape
    pub async fn type_definition(&self, uri: String, line: u32, ch: u32) -> String {
        self.core.type_definition(uri, line, ch).await
    }

    /// inlay hints for a document range — LSP `InlayHint` array or null
    pub async fn inlay(
        &self,
        uri: String,
        start_line: u32,
        start_ch: u32,
        end_line: u32,
        end_ch: u32,
    ) -> String {
        self.core
            .inlay(uri, start_line, start_ch, end_line, end_ch)
            .await
    }

    /// references — LSP `Location` array or null
    pub async fn references(&self, uri: String, line: u32, ch: u32, include_decl: u32) -> String {
        self.core.references(uri, line, ch, include_decl).await
    }

    /// signature help at an LSP position over the document's CURRENT
    /// text (`uri`+`src`, the doc-sync lane) — LSP `SignatureHelp` or
    /// null
    pub async fn signature_help(&self, uri: String, src: String, line: u32, ch: u32) -> String {
        self.core.signature_help(uri, src, line, ch).await
    }

    /// index one workspace file (the host's stand-in for an fs walk)
    /// carrying the package name the host knows (the owning manifest's);
    /// `pkg` omitted = a file no manifest owns (the uri-shape fallback)
    pub async fn index_file(&self, uri: String, src: String, pkg: Option<String>) {
        self.core.index_file(uri, src, pkg).await;
    }

    /// parse one `rut.jsonc` — the dep-table JSON out (or the error
    /// envelope)
    pub async fn parse_manifest(&self, text: String) -> String {
        self.core.parse_manifest(text).await
    }

    /// hand the core the READ-ONLY host environment — one object
    /// carrying `read` / `readDir` / `isDir` / `notify` (the JS face's
    /// `HostEnv`, `js/index.d.ts`), wrapped into the core's
    /// [`rut_lsp::env::Io`] by `JsEnv`. Replace-on-reinject; the
    /// await is ordering only.
    pub async fn set_env(&self, env: js_sys::Object) {
        let io = Rc::new(JsEnv::new(env)) as Rc<dyn rut_lsp::env::Io>;
        self.core.set_env_io(io).await;
    }

    /// the READ-ONLY dep walk — one JSON array of workspace roots in,
    /// the walk's `WalkOut` envelope out (the EAGER manual door)
    pub async fn resolve_deps(&self, roots_json: String) -> String {
        self.core.resolve_deps(roots_json).await
    }

    /// RELOAD the deps — the level-triggered door: defers under the
    /// loop's own debounce window and coalesces
    pub async fn reload_deps(&self, roots_json: String) -> String {
        self.core.reload_deps(roots_json).await
    }

    /// index one dep source file under its module NAME at a MOD PATH —
    /// the envelope names the file's own `mod` declarations
    pub async fn index_mod_at(
        &self,
        uri: String,
        pkg: String,
        mod_path: String,
        src: String,
    ) -> String {
        self.core.index_mod_at(uri, pkg, mod_path, src).await
    }
}

impl Default for RutLsp {
    fn default() -> Self {
        Self::new()
    }
}
