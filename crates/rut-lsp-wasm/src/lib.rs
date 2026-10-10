//! rut-lsp-wasm — the language core as an in-process wasm module
//!. The VS Code extension instantiates this and registers
//! its providers directly — no server process, no per-platform binaries.
//! The native `rut-lsp` stdio server remains the face for editors that
//! speak LSP; both run the *same* queries from `rut_lsp::analysis`, so
//! they can't drift.
//!
//! Raw ABI over wasm32 linear memory (the `rut-wasm` envelope pattern,
//! no wasm-bindgen):
//!
//!   rut_begin()                                start a request
//!   rut_alloc(len) -> ptr                      arena for the request's inputs
//!   rut_legend() -> ptr                        token-type names, JSON array
//!   rut_analyze(uri, src) -> ptr               store the doc + return
//!                                              {diags, tokens, symbols}
//!   rut_forget(uri)                            drop a closed doc
//!   rut_hover(uri, line, ch) -> ptr            LSP Hover JSON or null
//!   rut_complete(uri, line, ch) -> ptr         LSP CompletionItem array
//!   rut_definition(uri, line, ch) -> ptr       LSP Location array or null
//!   rut_type_definition(uri, line, ch) -> ptr  LSP Location array or null
//!   rut_inlay(uri, sl, sc, el, ec) -> ptr      LSP InlayHint array or null
//!   rut_references(uri, line, ch, incl) -> ptr LSP Location array or null
//!   rut_signature_help(uri, line, ch) -> ptr   LSP SignatureHelp or null
//!   rut_add_def(uri, src)                      index a workspace file
//!   rut_parse_manifest(ptr, len) -> ptr        rut.jsonc bytes -> the
//!                                              dep-table JSON (or the
//!                                              error envelope)
//!   rut_add_def_named(uri, name, src)          index a dep source file
//!                                              under its module NAME
//!   rut_add_def_mod(uri, name, path, src)
//!                                   -> ptr     index a dep source at a
//!                                              MOD PATH; the envelope
//!                                              names the file's own mod
//!                                              decls (the host's mount
//!                                              walk reads its next hop
//!                                              from them)
//!   rut_add_bundle(ptr, len) -> ptr            .rutbundle bytes -> index
//!                                              the bundle's own sources;
//!                                              envelope names module +
//!                                              entry paths (or the error)
//!
//! Every result is `[u32 little-endian length][bytes]` at the returned
//! ptr; every string argument is `(ptr, len)` written via `rut_alloc`.
//!
//! The last three are the deps face: the manifest/bundle plumbing runs
//! through `rut_lsp::deps` (pure — bytes in, values out) into the SAME
//! `state.defs` store `rut_add_def` writes, so a dep's surface resolves
//! in hover/completion/definition exactly like a workspace file — one
//! index, no drift. `rut_parse_manifest`/`rut_add_bundle` answer with an
//! error envelope (`{"error": …}`) on bad bytes; the host surfaces it as
//! a hint, never silent unparsed state.
//!
//! One deviation from `rut-wasm`'s one-shot model: an LSP serves a
//! long-lived editing session, so a monotonic arena would exhaust 16 MB
//! in a day of keystrokes. Each request starts with `rut_begin()`, which
//! resets the arena — inputs and the result envelope are only valid
//! until the next `rut_begin()`. Single-threaded JS reads the envelope
//! before the next request, so that's safe.

#![allow(static_mut_refs)]

use std::collections::HashMap;
use std::str::FromStr;

use ls_types::Uri;
use rut_lsp::hover::DefIndex;

// ---- the language core's state ----

struct State {
    /// open documents by URI (the LSP's full-text sync)
    docs: HashMap<String, String>,
    /// the std surface + workspace files, in lookup order
    defs: Vec<DefIndex>,
}

static mut STATE: Option<State> = None;

/// no threads on wasm32-unknown-unknown — one state, borrowed per export
fn state() -> &'static mut State {
    unsafe {
        if STATE.is_none() {
            STATE = Some(State {
                docs: HashMap::new(),
                defs: rut_lsp::std_surface::indexes(),
            });
        }
        STATE.as_mut().unwrap_unchecked()
    }
}

// ---- the arena (carved once per request, see rut_begin) ----

const HEAP_SIZE: usize = 16 * 1024 * 1024;
#[no_mangle]
static mut RUT_HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];
static mut HEAP_TOP: usize = 0;

/// start a request: drop the previous request's inputs and envelope
#[no_mangle]
pub extern "C" fn rut_begin() {
    unsafe { HEAP_TOP = 0 }
}

#[no_mangle]
pub extern "C" fn rut_alloc(len: usize) -> *mut u8 {
    unsafe {
        let top = HEAP_TOP;
        if top + len + 8 > HEAP_SIZE {
            return core::ptr::null_mut();
        }
        HEAP_TOP = top + len;
        RUT_HEAP.as_mut_ptr().add(top)
    }
}

/// stash the result envelope and return its pointer
fn envelope(bytes: &[u8]) -> *mut u8 {
    unsafe {
        let top = HEAP_TOP;
        if top + bytes.len() + 8 > HEAP_SIZE {
            return core::ptr::null_mut();
        }
        let len = (bytes.len() as u32).to_le_bytes();
        RUT_HEAP[top..top + 4].copy_from_slice(&len);
        RUT_HEAP[top + 4..top + 4 + bytes.len()].copy_from_slice(bytes);
        HEAP_TOP = top + 4 + bytes.len();
        RUT_HEAP.as_mut_ptr().add(top)
    }
}

fn json_envelope(value: serde_json::Value) -> *mut u8 {
    match serde_json::to_vec(&value) {
        Ok(bytes) => envelope(&bytes),
        Err(_) => envelope(b"null"), // unreachable for these types
    }
}

unsafe fn read_str<'a>(ptr: *const u8, len: usize) -> &'a str {
    let slice = core::slice::from_raw_parts(ptr, len);
    core::str::from_utf8(slice).unwrap_or("")
}

// ---- the ABI ----

/// the semantic-token legend (names in token-type index order)
#[no_mangle]
pub extern "C" fn rut_legend() -> *mut u8 {
    json_envelope(serde_json::json!(rut_lsp::semantic::TokenType::legend()))
}

/// open / FULL-sync change: store the document, return one analysis —
/// `{diags, tokens, symbols}` (the values the LSP face would push over
/// the wire)
#[no_mangle]
pub extern "C" fn rut_analyze(uri_ptr: *const u8, uri_len: usize, src_ptr: *const u8, src_len: usize) -> *mut u8 {
    let uri = unsafe { read_str(uri_ptr, uri_len) };
    let src = unsafe { read_str(src_ptr, src_len) };
    state().docs.insert(uri.to_string(), src.to_string());
    // analyze_at wires related-info URIs + hover provenance, exactly
    // like the stdio server; a degenerate URI still analyzes
    let a = match Uri::from_str(uri) {
        Ok(u) => rut_lsp::analysis::analyze_at(&u, src, rut_lsp::analysis::mode_of(uri)),
        Err(_) => {
            let mut a = rut_lsp::analysis::analyze(src, rut_lsp::analysis::mode_of(uri));
            a.index.origin = uri.to_string();
            a
        }
    };
    let value = serde_json::json!({
        "diags": a.diags,
        // SemanticTokens serializes `data` to the flat u32 wire format
        // (the same bytes the stdio server puts on the wire)
        "tokens": ls_types::SemanticTokens { result_id: None, data: a.tokens },
        "symbols": a.symbols,
    });
    json_envelope(value)
}

/// close: drop the document (the workspace/std index stays)
#[no_mangle]
pub extern "C" fn rut_forget(uri_ptr: *const u8, uri_len: usize) {
    let uri = unsafe { read_str(uri_ptr, uri_len) };
    state().docs.remove(uri);
}

/// hover at an LSP position — the doc index first, then std + workspace;
/// `null` when there's nothing to say
#[no_mangle]
pub extern "C" fn rut_hover(uri_ptr: *const u8, uri_len: usize, line: u32, ch: u32) -> *mut u8 {
    let uri = unsafe { read_str(uri_ptr, uri_len) };
    let Some(src) = state().docs.get(uri).cloned() else {
        return envelope(b"null");
    };
    let out = {
        let defs = &state().defs;
        rut_lsp::analysis::hover_at(uri, &src, defs, line, ch)
    };
    json_envelope(serde_json::json!(out))
}

/// completions at an LSP position (member after `.`, the use-path and
/// qualified-position tiers) — the doc's file-module context rides
/// from its stamped twin (see `doc_mods_of`)
#[no_mangle]
pub extern "C" fn rut_complete(uri_ptr: *const u8, uri_len: usize, line: u32, ch: u32) -> *mut u8 {
    let uri = unsafe { read_str(uri_ptr, uri_len) };
    let Some(src) = state().docs.get(uri).cloned() else {
        return envelope(b"[]");
    };
    let items = {
        let defs = &state().defs;
        let doc = doc_mods_of(uri, defs);
        rut_lsp::analysis::complete_at(uri, &src, defs, line, ch, &doc)
    };
    json_envelope(serde_json::json!(items))
}

/// definitions at an LSP position — the doc index first, then std +
/// workspace; `null` when the doc is unknown, `[]` when nothing
/// resolves. Targets carry the doc URI, a workspace origin, or the std
/// surface's true `rut/...` source path (relative — the extension
/// resolves it against the workspace).
#[no_mangle]
pub extern "C" fn rut_definition(uri_ptr: *const u8, uri_len: usize, line: u32, ch: u32) -> *mut u8 {
    let uri = unsafe { read_str(uri_ptr, uri_len) };
    let Some(src) = state().docs.get(uri).cloned() else {
        return envelope(b"null");
    };
    let locs = {
        let defs = &state().defs;
        rut_lsp::analysis::definition_at(uri, &src, defs, line, ch)
    };
    locations_envelope(&locs)
}

/// type definitions at an LSP position (an expression -> its type's
/// declaration) — same shape as `rut_definition`
#[no_mangle]
pub extern "C" fn rut_type_definition(uri_ptr: *const u8, uri_len: usize, line: u32, ch: u32) -> *mut u8 {
    let uri = unsafe { read_str(uri_ptr, uri_len) };
    let Some(src) = state().docs.get(uri).cloned() else {
        return envelope(b"null");
    };
    let locs = {
        let defs = &state().defs;
        rut_lsp::analysis::type_definition_at(uri, &src, defs, line, ch)
    };
    locations_envelope(&locs)
}

/// the definition-target envelope — `DefLocation` is serde-free in
/// `rut-lsp` (the pure core carries no serializer), so the shim spells
/// the LSP `Location` JSON: `{ uri, range }`
fn locations_envelope(locs: &[rut_lsp::definition::DefLocation]) -> *mut u8 {
    let value: Vec<serde_json::Value> = locs
        .iter()
        .map(|l| serde_json::json!({ "uri": l.uri, "range": l.range }))
        .collect();
    json_envelope(serde_json::Value::Array(value))
}

/// inlay hints for a document range — the inline inference display:
/// TYPE hints on unannotated bindings + PARAMETER hints at exact-arity
/// call sites. `null` when the doc is unknown, `[]` when nothing
/// resolves. The hints are full LSP values (position + label + kind +
/// tooltip); the client renders them.
#[no_mangle]
pub extern "C" fn rut_inlay(
    uri_ptr: *const u8,
    uri_len: usize,
    start_line: u32,
    start_ch: u32,
    end_line: u32,
    end_ch: u32,
) -> *mut u8 {
    let uri = unsafe { read_str(uri_ptr, uri_len) };
    let Some(src) = state().docs.get(uri).cloned() else {
        return envelope(b"null");
    };
    let hints = {
        let defs = &state().defs;
        rut_lsp::analysis::inlay_hints_at(
            uri,
            &src,
            defs,
            ls_types::Range {
                start: ls_types::Position { line: start_line, character: start_ch },
                end: ls_types::Position { line: end_line, character: end_ch },
            },
        )
    };
    json_envelope(serde_json::json!(hints))
}

/// references — the definition index read BACKWARDS: every position
/// whose go-to-definition lands on the declaration under `line`/`ch`.
/// Within-file through the binding pass (shadow-aware), cross-file
/// through the use graph's reverse edges. `include_decl` (0/1) follows
/// the LSP toggle. `null` when the doc is unknown, `[]` when nothing
/// resolves — the same `{uri, range}` Location JSON `rut_definition`
/// speaks.
#[no_mangle]
pub extern "C" fn rut_references(
    uri_ptr: *const u8,
    uri_len: usize,
    line: u32,
    ch: u32,
    include_decl: u32,
) -> *mut u8 {
    let uri = unsafe { read_str(uri_ptr, uri_len) };
    let Some(src) = state().docs.get(uri).cloned() else {
        return envelope(b"null");
    };
    let locs = {
        let defs = &state().defs;
        rut_lsp::analysis::references_at(uri, &src, defs, line, ch, include_decl != 0)
    };
    locations_envelope(&locs)
}

/// signature help at an LSP position — the callee resolves through the
/// same machinery the parameter-name hints ride (free calls by unique
/// recorded params, methods through the receiver's type head), the
/// signature renders verbatim, the active parameter comes from the
/// comma/paren depth at the request position. Ambiguity or arity
/// mismatch → `null` (never wrong help).
#[no_mangle]
pub extern "C" fn rut_signature_help(uri_ptr: *const u8, uri_len: usize, line: u32, ch: u32) -> *mut u8 {
    let uri = unsafe { read_str(uri_ptr, uri_len) };
    let Some(src) = state().docs.get(uri).cloned() else {
        return envelope(b"null");
    };
    let out = {
        let defs = &state().defs;
        rut_lsp::analysis::signature_help_at(uri, &src, defs, line, ch)
    };
    json_envelope(serde_json::json!(out))
}

/// index one workspace file (the host's stand-in for the native server's
/// fs walk — the 500-file cap is the caller's). Re-indexing a URI
/// replaces its entry.
#[no_mangle]
pub extern "C" fn rut_add_def(uri_ptr: *const u8, uri_len: usize, src_ptr: *const u8, src_len: usize) {
    let uri = unsafe { read_str(uri_ptr, uri_len) };
    let src = unsafe { read_str(src_ptr, src_len) };
    let idx = rut_lsp::analysis::index_at(uri, src);
    let defs = &mut state().defs;
    match defs.iter().position(|d| d.origin == uri) {
        Some(slot) => defs[slot] = idx,
        None => defs.push(idx),
    }
}

// ---- the deps face (rut_lsp::deps — one engine, two faces) ----

/// the entry-source mode as the envelope spells it (the variant name —
/// the manifest's entry key decided it, `entry.type` → Decl)
fn mode_str(m: rut_parser::Mode) -> &'static str {
    match m {
        rut_parser::Mode::Decl => "Decl",
        rut_parser::Mode::Impl => "Impl",
    }
}

/// one dep row's source: a `path` directory or a pinned `url` bundle
/// with the cache path rut-lsp already computed (`.rut/cache/
/// <sha256(url)>.rutbundle`) — the host never hashes URLs
fn dep_source_json(s: &rut_lsp::deps::DepSource) -> serde_json::Value {
    match s {
        rut_lsp::deps::DepSource::Path { dir } => serde_json::json!({ "kind": "path", "dir": dir }),
        rut_lsp::deps::DepSource::Url { url, sha256, cache_path } => serde_json::json!({
            "kind": "url",
            "url": url,
            "sha256": sha256,
            "cache_path": cache_path,
        }),
    }
}

fn dep_row_json(r: &rut_lsp::deps::DepRow) -> serde_json::Value {
    serde_json::json!({
        "name": r.name,
        "source": dep_source_json(&r.source),
        "optional": r.optional,
        "lib": r.lib,
    })
}

/// the DepTable as the envelope JSON — the struct's own field spellings
/// (`DefLocation`-style: the shim spells the wire, rut-lsp stays
/// serde-free here)
fn dep_table_json(t: &rut_lsp::deps::DepTable) -> serde_json::Value {
    serde_json::json!({
        "name": t.name,
        "entries": t
            .entries
            .iter()
            .map(|e| serde_json::json!({ "path": e.path, "mode": mode_str(e.mode) }))
            .collect::<Vec<_>>(),
        "namespace": t.namespace,
        "consts": t.consts,
        "deps": t.deps.iter().map(dep_row_json).collect::<Vec<_>>(),
        "dev_deps": t.dev_deps.iter().map(dep_row_json).collect::<Vec<_>>(),
        "peer_deps": t.peer_deps.iter().map(dep_row_json).collect::<Vec<_>>(),
    })
}

/// parse one `rut.jsonc` — the dep-table JSON out (the module's identity,
/// its entry sources with the mode each indexes in, and its dep rows
/// with the url rows' cache paths). A malformed manifest is the error
/// envelope — the host surfaces it as a hint.
#[no_mangle]
pub extern "C" fn rut_parse_manifest(ptr: *const u8, len: usize) -> *mut u8 {
    let text = unsafe { read_str(ptr, len) };
    match rut_lsp::deps::dep_table(text) {
        Ok(t) => json_envelope(dep_table_json(&t)),
        Err(e) => json_envelope(serde_json::json!({ "error": e })),
    }
}

/// index one dep source file under its module NAME — `deps::index_dep`
/// into the same store `rut_add_def` writes (re-indexing an origin
/// replaces its entry), so `use <name>::` resolves it like any index.
/// The mode rides the URI convention (`mode_of`, identical to
/// `rut_add_def`); namespace/consts have no ABI voice here — a dep
/// needing them indexes through `rut_add_bundle`, whose manifest is in
/// the bytes.
#[no_mangle]
pub extern "C" fn rut_add_def_named(
    uri_ptr: *const u8,
    uri_len: usize,
    name_ptr: *const u8,
    name_len: usize,
    src_ptr: *const u8,
    src_len: usize,
) {
    let uri = unsafe { read_str(uri_ptr, uri_len) };
    let name = unsafe { read_str(name_ptr, name_len) };
    let src = unsafe { read_str(src_ptr, src_len) };
    let idx = rut_lsp::deps::index_dep(name, uri, src, rut_lsp::analysis::mode_of(uri), None, &[]);
    let defs = &mut state().defs;
    match defs.iter().position(|d| d.origin == uri) {
        Some(slot) => defs[slot] = idx,
        None => defs.push(idx),
    }
}

/// index one dep source file under its module NAME at a MOD PATH —
/// the mod-aware twin of `rut_add_def_named` (the dep walk's per-file
/// move for a mod-carrying package: the root at `""`, each child at
/// its path). Returns the parsed file's own `mod` declarations —
/// `{ "decls": [{ "name", "vis" }] }` — so the host's mount walk
/// reads its next hop from the wasm's parse (JS moves bytes, the
/// module parses; no JS-side rut parsing).
#[no_mangle]
pub extern "C" fn rut_add_def_mod(
    uri_ptr: *const u8,
    uri_len: usize,
    name_ptr: *const u8,
    name_len: usize,
    mod_path_ptr: *const u8,
    mod_path_len: usize,
    src_ptr: *const u8,
    src_len: usize,
) -> *mut u8 {
    let uri = unsafe { read_str(uri_ptr, uri_len) };
    let name = unsafe { read_str(name_ptr, name_len) };
    let mod_path = unsafe { read_str(mod_path_ptr, mod_path_len) };
    let src = unsafe { read_str(src_ptr, src_len) };
    let idx = rut_lsp::deps::index_dep_mod(
        name,
        uri,
        mod_path,
        src,
        rut_lsp::analysis::mode_of(uri),
        None,
        &[],
    );
    let decls: Vec<serde_json::Value> = idx
        .mods
        .iter()
        .map(|m| {
            serde_json::json!({
                "name": m.name,
                "vis": if matches!(m.vis, rut_ast::ast::Vis::Pub) { "pub" } else { "mod" },
            })
        })
        .collect();
    let defs = &mut state().defs;
    match defs.iter().position(|d| d.origin == uri) {
        Some(slot) => defs[slot] = idx,
        None => defs.push(idx),
    }
    json_envelope(serde_json::json!({ "decls": decls }))
}

/// the open document's file-module context, read off the STAMPED
/// twin: whichever face indexed the file under its real uri (the
/// extension's dep walk / workspace indexer) laid down the (module,
/// mod_path) pair — the fresh per-query doc index borrows it.
/// Unstamped files answer the default (position-path completion off;
/// nothing else reads this).
fn doc_mods_of(uri: &str, defs: &[DefIndex]) -> rut_lsp::mods::DocMods {
    for d in defs {
        if d.origin == uri {
            return rut_lsp::mods::DocMods {
                mod_path: d.mod_path.clone(),
                pkg: d.module.clone(),
            };
        }
    }
    rut_lsp::mods::DocMods::default()
}

/// binary (ptr, len) reader — the bundle bytes are a zip, not text
unsafe fn read_bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    core::slice::from_raw_parts(ptr, len)
}

/// unpack one `.rutbundle` — `deps::bundle_sources` reads the archive's
/// own manifest, then each entry source indexes under the module NAME
/// the bundle carries (origin `bundle:<module>/<path>`, replace-by-origin
/// like every index). The envelope names the module + the entry paths
/// indexed, or the error — a corrupt/foreign archive never lands bytes.
#[no_mangle]
pub extern "C" fn rut_add_bundle(ptr: *const u8, len: usize) -> *mut u8 {
    let bytes = unsafe { read_bytes(ptr, len) };
    let b = match rut_lsp::deps::bundle_sources(bytes) {
        Ok(b) => b,
        Err(e) => return json_envelope(serde_json::json!({ "error": e })),
    };
    let mut paths = Vec::with_capacity(b.files.len());
    let defs = &mut state().defs;
    for f in &b.files {
        let uri = format!("bundle:{}/{}", b.module, f.path);
        // a mounted mod child rides as `<path>/mod.rut` (the rows
        // lane's root as `mod.rut`) — its mod path stamps the index;
        // flat files keep today's shape
        let idx = match rut_lsp::deps::bundle_file_mod_path(&f.path) {
            Some(p) => rut_lsp::deps::index_dep_mod(
                &b.module,
                &uri,
                &p,
                &f.src,
                rut_parser::Mode::Impl,
                b.namespace.as_deref(),
                &b.consts,
            ),
            None => rut_lsp::deps::index_dep(
                &b.module,
                &uri,
                &f.src,
                f.mode,
                b.namespace.as_deref(),
                &b.consts,
            ),
        };
        match defs.iter().position(|d| d.origin == uri) {
            Some(slot) => defs[slot] = idx,
            None => defs.push(idx),
        }
        paths.push(f.path.clone());
    }
    json_envelope(serde_json::json!({
        "module": b.module,
        "namespace": b.namespace,
        "consts": b.consts,
        "files": paths,
    }))
}

/// an open doc's stored text — the smoke test's round-trip check
#[no_mangle]
pub extern "C" fn rut_doc_len(uri_ptr: *const u8, uri_len: usize) -> usize {
    let uri = unsafe { read_str(uri_ptr, uri_len) };
    state().docs.get(uri).map(|s| s.len()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// STATE is one process-global — the exports assume the host's
    /// single-threaded JS; the test harness runs parallel threads, so
    /// every state-touching test holds this for its whole body
    static TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    const PLUGIN_MANIFEST: &str = include_str!("../../../examples/03-plugin/plugin/rut.jsonc");
    const POUCH_BUNDLE: &[u8] = include_bytes!("../../../dist/std/pouch.rutbundle");
    /// the CDN url's own sha256 — the cache filename rut-lsp computes
    /// (the URL STRING is hashed, never the bytes)
    const POUCH_CACHE: &str =
        ".rut/cache/683a9cb219067e36be3ebf79fc895ff0930c10a47c058e386a226e428971a74f.rutbundle";

    /// read an export's result envelope — `[u32 le length][json]`. The
    /// heap is a byte array (align 1), so the length reads byte-wise —
    /// the same read the extension's DataView does
    fn take(ptr: *mut u8) -> serde_json::Value {
        assert!(!ptr.is_null(), "arena overflow");
        unsafe {
            let len =
                u32::from_le_bytes([*ptr, *ptr.add(1), *ptr.add(2), *ptr.add(3)]) as usize;
            let bytes = core::slice::from_raw_parts(ptr.add(4), len);
            serde_json::from_slice(bytes).expect("the envelope is valid JSON")
        }
    }

    #[test]
    fn manifest_envelope_spells_the_dep_table() {
        let _lock = TEST_SERIAL.lock().unwrap();
        rut_begin();
        let out = take(rut_parse_manifest(PLUGIN_MANIFEST.as_ptr(), PLUGIN_MANIFEST.len()));
        assert_eq!(out["name"], "plugin");
        // the body rides the mod.rut convention (the entry keys are repealed)
        assert_eq!(
            out["entries"],
            serde_json::json!([{ "path": "mod.rut", "mode": "Impl" }]),
            "the entry carries its mode: {}",
            out["entries"]
        );
        let rows = out["deps"].as_array().unwrap();
        let server = rows.iter().find(|r| r["name"] == "server").unwrap();
        assert_eq!(server["source"], serde_json::json!({ "kind": "path", "dir": "../server" }));
        assert_eq!(server["optional"], false);
        let pouch = rows.iter().find(|r| r["name"] == "pouch").unwrap();
        assert_eq!(pouch["source"]["kind"], "url");
        assert_eq!(pouch["source"]["cache_path"], POUCH_CACHE);
        assert!(pouch["source"]["sha256"].as_str().unwrap().len() == 64);
    }

    #[test]
    fn a_decl_entry_and_the_error_envelope() {
        let _lock = TEST_SERIAL.lock().unwrap();
        rut_begin();
        let src = r#"{ "name": "host", "type": "host", "entry": { "type": "./surface.d.rut" } }"#;
        let out = take(rut_parse_manifest(src.as_ptr(), src.len()));
        assert_eq!(
            out["entries"],
            serde_json::json!([{ "path": "surface.d.rut", "mode": "Decl" }]),
            "entry.type indexes in Decl mode"
        );
        // a malformed manifest is the error envelope — the host's hint
        rut_begin();
        let bad = take(rut_parse_manifest(b"{\"name\": six}".as_ptr(), b"{\"name\": six}".len()));
        assert!(bad["error"].as_str().unwrap().starts_with("line 1: "), "{bad}");
    }

    #[test]
    fn add_def_named_feeds_the_use_path_tier() {
        let _lock = TEST_SERIAL.lock().unwrap();
        rut_begin();
        // a DIFFERENT module than the mod-tree fixture below — the two
        // tests' defs share the one process-global state, and one pkg
        // name must not answer from two def indexes at once
        let (u, n, s) = (
            b"file:///ws/toolkit/lib.rut".as_slice(),
            b"toolkit".as_slice(),
            &b"pub class Widget {\n    id: i32;\n}\npub fn tag() -> i32 {\n    return 1;\n}\n"[..],
        );
        rut_add_def_named(u.as_ptr(), u.len(), n.as_ptr(), n.len(), s.as_ptr(), s.len());
        let doc = "use toolkit::\nfn main() -> nil {\n}\n";
        let uri = "file:///ws/main.rut";
        rut_begin();
        take(rut_analyze(uri.as_ptr(), uri.len(), doc.as_ptr(), doc.len()));
        // `use toolkit::⏐` — the dep's public names
        rut_begin();
        let items = take(rut_complete(uri.as_ptr(), uri.len(), 0, 13));
        let labels: Vec<&str> = items.as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap()).collect();
        assert!(labels.contains(&"Widget"), "the dep type completes: {labels:?}");
        assert!(labels.contains(&"tag"), "the dep fn completes: {labels:?}");
        // `use ⏐` — the module-name tier sees the named index
        rut_begin();
        let mods = take(rut_complete(uri.as_ptr(), uri.len(), 0, 4));
        let names: Vec<&str> = mods.as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap()).collect();
        assert!(names.contains(&"toolkit"), "the module name completes: {names:?}");
    }

    #[test]
    fn add_bundle_indexes_the_pouch_surface() {
        let _lock = TEST_SERIAL.lock().unwrap();
        rut_begin();
        let out = take(rut_add_bundle(POUCH_BUNDLE.as_ptr(), POUCH_BUNDLE.len()));
        assert_eq!(out["module"], "pouch");
        assert_eq!(out["files"], serde_json::json!(["mod.rut"]));
        // a corrupt archive is the error envelope, never silent state
        rut_begin();
        let junk = b"definitely not a zip";
        let err = take(rut_add_bundle(junk.as_ptr(), junk.len()));
        assert!(err["error"].as_str().unwrap().contains("not a zip"), "{err}");

        // the indexed surface resolves: the use name jumps to BOTH pouch
        // indexes — the std surface's true source path AND the bundle's
        let doc = "use pouch::{ Vec };\nfn main() -> nil {\n    let v: Vec<i32> = Vec.new();\n    v.push(1);\n}\n";
        let uri = "file:///ws/uses-pouch.rut";
        rut_begin();
        take(rut_analyze(uri.as_ptr(), uri.len(), doc.as_ptr(), doc.len()));
        rut_begin();
        let locs = take(rut_definition(uri.as_ptr(), uri.len(), 0, 13)); // the use-line `Vec`
        let uris: Vec<&str> = locs.as_array().unwrap().iter().map(|l| l["uri"].as_str().unwrap()).collect();
        assert!(
            uris.contains(&"bundle:pouch/mod.rut"),
            "the bundle index answers go-to-definition: {uris:?}"
        );
        // and the member surface hovers through the named module
        let push_ch = doc.lines().nth(3).unwrap().find("push").unwrap() as u32;
        rut_begin();
        let h = take(rut_hover(uri.as_ptr(), uri.len(), 3, push_ch));
        assert!(h["contents"]["value"].as_str().unwrap().contains("push"), "{h}");
    }

    #[test]
    fn add_def_mod_stamps_the_tree_and_feeds_the_use_walk() {
        let _lock = TEST_SERIAL.lock().unwrap();
        rut_begin();
        // the root: `pub mod layout;` rides back as the mount walk's
        // next hop (the host reads its children from the envelope)
        let root_src = &b"pub mod layout;\nmod secret;\npub fn tag() -> i32 { return 1; }\n"[..];
        let uri_root = b"file:///ws/gadgets/mod.rut";
        let name = b"gadgets";
        let out = take(rut_add_def_mod(
            uri_root.as_ptr(),
            uri_root.len(),
            name.as_ptr(),
            name.len(),
            b"".as_ptr(),
            0,
            root_src.as_ptr(),
            root_src.len(),
        ));
        assert_eq!(
            out["decls"],
            serde_json::json!([{ "name": "layout", "vis": "pub" }, { "name": "secret", "vis": "mod" }]),
            "{}",
            out["decls"]
        );
        // the child, stamped at its mod path
        rut_begin();
        let layout_src = &b"pub struct Column {\n    w: i32;\n}\npub fn mk() -> Column { return Column { w: 1 }; }\n"[..];
        let uri_layout = b"file:///ws/gadgets/layout/mod.rut";
        let out = take(rut_add_def_mod(
            uri_layout.as_ptr(),
            uri_layout.len(),
            name.as_ptr(),
            name.len(),
            b"layout".as_ptr(),
            6,
            layout_src.as_ptr(),
            layout_src.len(),
        ));
        assert!(out["decls"].as_array().unwrap().is_empty(), "{}", out["decls"]);

        // `use gadgets::layout::⏐` — the walked module's exports (the
        // private `secret` edge walked would be empty)
        let doc = "use gadgets::layout::\nfn main() -> nil {\n}\n";
        let uri = "file:///ws/main.rut";
        rut_begin();
        take(rut_analyze(uri.as_ptr(), uri.len(), doc.as_ptr(), doc.len()));
        rut_begin();
        let items = take(rut_complete(uri.as_ptr(), uri.len(), 0, 21));
        let labels: Vec<&str> = items.as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap()).collect();
        assert!(labels.contains(&"Column"), "{labels:?}");
        assert!(labels.contains(&"mk"), "{labels:?}");
        // the root tier offers the pub mod child, never the private one
        rut_begin();
        let items = take(rut_complete(uri.as_ptr(), uri.len(), 0, 13));
        let labels: Vec<&str> = items.as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap()).collect();
        assert!(labels.contains(&"layout"), "{labels:?}");
        assert!(!labels.contains(&"secret"), "a bare `mod` edge never crosses: {labels:?}");
        assert!(!labels.contains(&"Column"), "child members stay under their path: {labels:?}");
    }

    /// a mod-carrying bundle in the pack writer's shape — the wasm
    /// face mounts the rows tree additively
    fn kit_bundle() -> Vec<u8> {
        use std::collections::BTreeMap;
        let mods = BTreeMap::from([
            (
                "layout".to_string(),
                rut_driver::ModSource {
                    path: "layout".into(),
                    vis: rut_ast::ast::Vis::Pub,
                    text: "pub struct Column {\n    w: i32;\n}\n".into(),
                },
            ),
        ]);
        let rows = rut_driver::mods::rows_json("pub mod layout;\n", &mods);
        rut_driver::bundle::write_bundle(&[
            (
                rut_driver::bundle::files::MANIFEST_NAME.into(),
                br#"{ "name": "kit" }"#.to_vec(),
            ),
            (rut_driver::mods::ROWS_NAME.into(), rows.into_bytes()),
        ])
        .unwrap()
    }

    #[test]
    fn add_bundle_mounts_a_mod_tree() {
        let _lock = TEST_SERIAL.lock().unwrap();
        rut_begin();
        let bytes = kit_bundle();
        let out = take(rut_add_bundle(bytes.as_ptr(), bytes.len()));
        assert_eq!(out["module"], "kit");
        assert_eq!(
            out["files"],
            serde_json::json!(["mod.rut", "layout/mod.rut"]),
            "the tree rides the file list: {}",
            out["files"]
        );
        // `use kit::layout::⏐` — the mounted child's exports
        let doc = "use kit::layout::{ Column };\nfn main() -> nil {\n    let c: Column = Column { w: 1 };\n}\n";
        let uri = "file:///ws/uses-kit.rut";
        rut_begin();
        take(rut_analyze(uri.as_ptr(), uri.len(), doc.as_ptr(), doc.len()));
        rut_begin();
        let items = take(rut_complete(uri.as_ptr(), uri.len(), 0, 16));
        let labels: Vec<&str> = items.as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap()).collect();
        assert!(labels.contains(&"Column"), "{labels:?}");
        // and the segment hover resolves through the dep tree
        rut_begin();
        let h = take(rut_hover(uri.as_ptr(), uri.len(), 0, 12)); // the `layout` segment
        let md = h["contents"]["value"].as_str().unwrap();
        assert!(md.contains("file module"), "{md}");
        assert!(md.contains("layout/mod.rut"), "{md}");
    }
}
