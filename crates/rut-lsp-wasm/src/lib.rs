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
//!   rut_complete_in(uri, src, line, ch) -> ptr the same array over the
//!                                              CURRENT text (stored
//!                                              first — the doc-sync
//!                                              lane; completion fires
//!                                              on the keystroke, the
//!                                              analyze lane debounces)
//!   rut_definition(uri, line, ch) -> ptr       LSP Location array or null
//!   rut_type_definition(uri, line, ch) -> ptr  LSP Location array or null
//!   rut_inlay(uri, sl, sc, el, ec) -> ptr      LSP InlayHint array or null
//!   rut_references(uri, line, ch, incl) -> ptr LSP Location array or null
//!   rut_signature_help(uri, line, ch) -> ptr   LSP SignatureHelp or null
//!   rut_add_def(uri, src)                      index a workspace file
//!   rut_parse_manifest(ptr, len) -> ptr        rut.jsonc bytes -> the
//!                                              dep-table JSON (or the
//!                                              error envelope)
//!   rut_set_lock(ptr, len) -> ptr              rut.lock bytes -> the
//!                                              url→pin ledger
//!                                              rut_parse_manifest stamps
//!                                              onto url rows
//!   rut_set_direct_deps(ptr, len) -> ptr       the root manifest's dep
//!                                              names (JSON array) — the
//!                                              direct-dep stamp the dep
//!                                              indexes carry (the
//!                                              auto-import tier's gate)
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
//!   rut_extract_bundle(ptr, len) -> ptr        .rutbundle bytes -> the
//!                                              editable surface envelope
//!                                              {module, namespace,
//!                                              consts, files:[{path,
//!                                              src, mode, mod_path}]}
//!                                              — NO indexing, JS owns
//!                                              the disk
//!
//! Every result is `[u32 little-endian length][bytes]` at the returned
//! ptr; every string argument is `(ptr, len)` written via `rut_alloc`.
//!
//! These are the deps face: the manifest/bundle plumbing runs through
//! `rut_lsp::deps` (pure — bytes in, values out) into the SAME
//! `state.defs` store `rut_add_def` writes, so a dep's surface resolves
//! in hover/completion/definition exactly like a workspace file — one
//! index, no drift. `rut_extract_bundle` is the same plumbing's extract
//! lane — bytes in, the surface envelope out, NO index write (JS owns
//! the disk; re-indexing goes back through `rut_add_def`/
//! `rut_add_def_mod`). `rut_parse_manifest`/`rut_add_bundle`/
//! `rut_extract_bundle` answer with an error envelope (`{"error": …}`)
//! on bad bytes; the host surfaces it as a hint, never silent unparsed
//! state.
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
    /// url → pin, from the workspace root's `rut.lock` (the host reads
    /// the file and hands the bytes to `rut_set_lock`; the manifest
    /// grammar carries no pins). `rut_parse_manifest` stamps the rows
    /// onto its url deps.
    lock_pins: std::collections::BTreeMap<String, String>,
    /// the direct-dep module names — the open document's own package
    /// name plus every dep row its ROOT manifest declares (`deps`,
    /// `dev-deps` at the walk root). The host hands the
    /// set to `rut_set_direct_deps` when it parses a root manifest;
    /// every dep index named here stamps `direct_dep`, the
    /// auto-import tier's gate (a transitive-only dep's `use` is the
    /// compiler's D5 error). Union-accumulating: multi-folder walks
    /// and re-walks re-add the same names, never shrink the world.
    direct_deps: std::collections::HashSet<String>,
}

static mut STATE: Option<State> = None;

/// no threads on wasm32-unknown-unknown — one state, borrowed per export
fn state() -> &'static mut State {
    unsafe {
        if STATE.is_none() {
            STATE = Some(State {
                docs: HashMap::new(),
                defs: rut_lsp::std_surface::indexes(),
                lock_pins: std::collections::BTreeMap::new(),
                direct_deps: std::collections::HashSet::new(),
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
/// the wire). The diags are the MERGED set (B1+B2): the parse lane's
/// diags plus the check lane's — the doc's package (its stamped twin
/// names it) compiles with its dep closure and the checker's per-file
/// diags map back through the file stamps. No package context →
/// parse-only, exactly the pre-B1 answer.
/// # Safety
///
/// `uri_ptr`/`src_ptr` must point at `uri_len`/`src_len` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn rut_analyze(
    uri_ptr: *const u8,
    uri_len: usize,
    src_ptr: *const u8,
    src_len: usize,
) -> *mut u8 {
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
    let diags = {
        let defs = &state().defs;
        let dm = doc_mods_of(uri, defs);
        let groups = if dm.pkg.is_some() {
            rut_lsp::check::groups_from_defs(defs)
        } else {
            Vec::new()
        };
        rut_lsp::check::merged_doc_diags(uri, src, dm.pkg, dm.mod_path, groups, a.diags)
    };
    let value = serde_json::json!({
        "diags": diags,
        // SemanticTokens serializes `data` to the flat u32 wire format
        // (the same bytes the stdio server puts on the wire)
        "tokens": ls_types::SemanticTokens { result_id: None, data: a.tokens },
        "symbols": a.symbols,
    });
    json_envelope(value)
}

/// close: drop the document (the workspace/std index stays)
/// # Safety
///
/// `uri_ptr` must point at `uri_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn rut_forget(uri_ptr: *const u8, uri_len: usize) {
    let uri = unsafe { read_str(uri_ptr, uri_len) };
    state().docs.remove(uri);
}

/// hover at an LSP position — the doc index first, then std + workspace;
/// `null` when there's nothing to say
/// # Safety
///
/// `uri_ptr` must point at `uri_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn rut_hover(
    uri_ptr: *const u8,
    uri_len: usize,
    line: u32,
    ch: u32,
) -> *mut u8 {
    let uri = unsafe { read_str(uri_ptr, uri_len) };
    let Some(src) = state().docs.get(uri).cloned() else {
        return envelope(b"null");
    };
    let out = {
        let defs = &state().defs;
        let doc = doc_mods_of(uri, defs);
        rut_lsp::analysis::hover_at(uri, &src, defs, &doc, line, ch)
    };
    json_envelope(serde_json::json!(out))
}

/// completions at an LSP position (member after `.`, the use-path and
/// qualified-position tiers) — the doc's file-module context rides
/// from its stamped twin (see `doc_mods_of`)
/// # Safety
///
/// `uri_ptr` must point at `uri_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn rut_complete(
    uri_ptr: *const u8,
    uri_len: usize,
    line: u32,
    ch: u32,
) -> *mut u8 {
    let uri = unsafe { read_str(uri_ptr, uri_len) };
    complete_stored(uri, line, ch)
}

/// completions over the document's CURRENT text — the sync twin of
/// `rut_complete`, `rut_analyze`'s uri+len+text+len shape: the passed
/// bytes are stored FIRST, so the tiers lex the text the cursor sits
/// in, dot included. The extension debounces `rut_analyze` (~200 ms)
/// while completion fires on the keystroke — the stored-only read
/// raced that window and answered a stale tail (typing `log.` hit the
/// bare keyword dump when the stored doc still ended at `log`).
/// # Safety
///
/// `uri_ptr` must point at `uri_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn rut_complete_in(
    uri_ptr: *const u8,
    uri_len: usize,
    src_ptr: *const u8,
    src_len: usize,
    line: u32,
    ch: u32,
) -> *mut u8 {
    let uri = unsafe { read_str(uri_ptr, uri_len) };
    let src = unsafe { read_str(src_ptr, src_len) };
    state().docs.insert(uri.to_string(), src.to_string());
    complete_stored(uri, line, ch)
}

/// the completion query over the stored doc — both completion entries'
/// body (`rut_complete_in` has already re-stored the text)
fn complete_stored(uri: &str, line: u32, ch: u32) -> *mut u8 {
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
/// resolves it against the workspace). The doc's file-module context
/// rides from its stamped twin (see `doc_mods_of`) — the mod-edge
/// clicks and the own-package use lanes key on it.
/// # Safety
///
/// `uri_ptr` must point at `uri_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn rut_definition(
    uri_ptr: *const u8,
    uri_len: usize,
    line: u32,
    ch: u32,
) -> *mut u8 {
    let uri = unsafe { read_str(uri_ptr, uri_len) };
    let Some(src) = state().docs.get(uri).cloned() else {
        return envelope(b"null");
    };
    let locs = {
        let defs = &state().defs;
        let doc = doc_mods_of(uri, defs);
        rut_lsp::analysis::definition_at(uri, &src, defs, &doc, line, ch)
    };
    locations_envelope(&locs)
}

/// type definitions at an LSP position (an expression -> its type's
/// declaration) — same shape as `rut_definition`
/// # Safety
///
/// `uri_ptr` must point at `uri_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn rut_type_definition(
    uri_ptr: *const u8,
    uri_len: usize,
    line: u32,
    ch: u32,
) -> *mut u8 {
    let uri = unsafe { read_str(uri_ptr, uri_len) };
    let Some(src) = state().docs.get(uri).cloned() else {
        return envelope(b"null");
    };
    let locs = {
        let defs = &state().defs;
        let doc = doc_mods_of(uri, defs);
        rut_lsp::analysis::type_definition_at(uri, &src, defs, &doc, line, ch)
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
/// # Safety
///
/// `uri_ptr` must point at `uri_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn rut_inlay(
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
                start: ls_types::Position {
                    line: start_line,
                    character: start_ch,
                },
                end: ls_types::Position {
                    line: end_line,
                    character: end_ch,
                },
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
/// # Safety
///
/// `uri_ptr` must point at `uri_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn rut_references(
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
        let doc = doc_mods_of(uri, defs);
        rut_lsp::analysis::references_at(uri, &src, defs, &doc, line, ch, include_decl != 0)
    };
    locations_envelope(&locs)
}

/// signature help at an LSP position — the callee resolves through the
/// same machinery the parameter-name hints ride (free calls by unique
/// recorded params, methods through the receiver's type head), the
/// signature renders verbatim, the active parameter comes from the
/// comma/paren depth at the request position. Ambiguity or arity
/// mismatch → `null` (never wrong help).
/// # Safety
///
/// `uri_ptr` must point at `uri_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn rut_signature_help(
    uri_ptr: *const u8,
    uri_len: usize,
    line: u32,
    ch: u32,
) -> *mut u8 {
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
/// # Safety
///
/// `src_ptr` must point at `src_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn rut_add_def(
    uri_ptr: *const u8,
    uri_len: usize,
    src_ptr: *const u8,
    src_len: usize,
) {
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
        rut_lsp::deps::DepSource::Url {
            url,
            sha256,
            cache_path,
        } => serde_json::json!({
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
    })
}

/// parse one `rut.jsonc` — the dep-table JSON out (the module's identity,
/// its entry sources with the mode each indexes in, and its dep rows
/// with the url rows' cache paths and lock-sourced pins). A malformed
/// manifest is the error envelope — the host surfaces it as a hint.
/// # Safety
///
/// `ptr` must point at `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn rut_parse_manifest(ptr: *const u8, len: usize) -> *mut u8 {
    let text = unsafe { read_str(ptr, len) };
    match rut_lsp::deps::dep_table_pinned(text, &state().lock_pins) {
        Ok(t) => json_envelope(dep_table_json(&t)),
        Err(e) => json_envelope(serde_json::json!({ "error": e })),
    }
}

/// hand the dep-table face the workspace root's `rut.lock` bytes — the
/// url→pin ledger `rut_parse_manifest` stamps onto url rows (the lock is
/// the pin's only source; the manifest grammar retired `sha256`).
/// Answers `{"pins": N}`, or the `{"error": …}` envelope for a
/// malformed lock — the host surfaces it as a hint, and the (empty)
/// ledger stands so url rows answer `sha256: null` (the unpinned
/// refusal), never guessed pins.
/// # Safety
///
/// `ptr` must point at `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn rut_set_lock(ptr: *const u8, len: usize) -> *mut u8 {
    let text = unsafe { read_str(ptr, len) };
    match rut_lsp::deps::lock_pins(text) {
        Ok(pins) => {
            let n = pins.len();
            state().lock_pins = pins;
            json_envelope(serde_json::json!({ "pins": n }))
        }
        Err(e) => {
            state().lock_pins = std::collections::BTreeMap::new();
            json_envelope(serde_json::json!({ "error": e }))
        }
    }
}

/// hand the dep face the direct-dep module names — the open
/// document's own package name plus every dep row its ROOT manifest
/// declares. Every dep index named here stamps `direct_dep` (the
/// auto-import tier's gate): `rut_add_def_named` / `rut_add_def_mod`
/// / `rut_add_bundle` consult the set at index time, so the host
/// calls this once per root manifest BEFORE mounting its rows. The
/// bytes are a JSON array of names (`["pouch", "calc"]`); names
/// ACCUMULATE (a set — multi-folder walks and re-walks re-add the
/// same names). Answers `{"direct": N}` (the set size), or the error
/// envelope for malformed JSON.
/// # Safety
///
/// `ptr` must point at `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn rut_set_direct_deps(ptr: *const u8, len: usize) -> *mut u8 {
    let text = unsafe { read_str(ptr, len) };
    match serde_json::from_str::<Vec<String>>(text) {
        Ok(names) => {
            let set = &mut state().direct_deps;
            for n in names {
                set.insert(n);
            }
            json_envelope(serde_json::json!({ "direct": set.len() }))
        }
        Err(e) => json_envelope(serde_json::json!({ "error": e.to_string() })),
    }
}

/// index one dep source file under its module NAME — `deps::index_dep`
/// into the same store `rut_add_def` writes (re-indexing an origin
/// replaces its entry), so `use <name>::` resolves it like any index.
/// The mode rides the URI convention (`mode_of`, identical to
/// `rut_add_def`); namespace/consts have no ABI voice here — a dep
/// needing them indexes through `rut_add_bundle`, whose manifest is in
/// the bytes. The direct-dep marker stamps from the host's set.
/// # Safety
///
/// `src_ptr` must point at `src_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn rut_add_def_named(
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
    let mut idx =
        rut_lsp::deps::index_dep(name, uri, src, rut_lsp::analysis::mode_of(uri), None, &[]);
    idx.direct_dep = state().direct_deps.contains(name);
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
/// # Safety
///
/// `src_ptr` must point at `src_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn rut_add_def_mod(
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
    let mut idx = rut_lsp::deps::index_dep_mod(
        name,
        uri,
        mod_path,
        src,
        rut_lsp::analysis::mode_of(uri),
        None,
        &[],
    );
    idx.direct_dep = state().direct_deps.contains(name);
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
/// # Safety
///
/// `ptr` must point at `len` readable bytes of a bundle.
#[no_mangle]
pub unsafe extern "C" fn rut_add_bundle(ptr: *const u8, len: usize) -> *mut u8 {
    let bytes = unsafe { read_bytes(ptr, len) };
    let b = match rut_lsp::deps::bundle_sources(bytes) {
        Ok(b) => b,
        Err(e) => return json_envelope(serde_json::json!({ "error": e })),
    };
    let mut paths = Vec::with_capacity(b.files.len());
    let direct = state().direct_deps.contains(&b.module);
    let defs = &mut state().defs;
    for f in &b.files {
        let uri = format!("bundle:{}/{}", b.module, f.path);
        // a mounted mod child rides as `<path>/mod.rut` (a host tree's
        // as `<path>/mod.d.rut`, the rows lane's root as `mod.rut`) —
        // its mod path stamps the index in the file's own mode; flat
        // files keep today's shape. A DECL file is the declaration
        // entry (wire v12): ONE parse, the inline nesting stamps one
        // index per module — the consolidated group entry indexes whole.
        let idxs: Vec<_> = match rut_lsp::deps::bundle_file_mod_path(&f.path) {
            Some(p) if f.mode == rut_parser::Mode::Decl => rut_lsp::deps::index_dep_decl_group(
                &b.module,
                &uri,
                &p,
                &f.src,
                b.namespace.as_deref(),
                &b.consts,
            )
            .into_iter()
            .map(|(_, idx)| idx)
            .collect(),
            Some(p) => vec![rut_lsp::deps::index_dep_mod(
                &b.module,
                &uri,
                &p,
                &f.src,
                f.mode,
                b.namespace.as_deref(),
                &b.consts,
            )],
            None => vec![rut_lsp::deps::index_dep(
                &b.module,
                &uri,
                &f.src,
                f.mode,
                b.namespace.as_deref(),
                &b.consts,
            )],
        };
        for mut idx in idxs {
            idx.direct_dep = direct;
            // one uri per module: the shared entry uri for the root
            // stamp, `#<mod path>` for the nested ones (the (module,
            // mod_path) pair is the real key; the uri only names the
            // origin in diagnostics)
            match defs.iter().position(|d| d.origin == uri) {
                Some(slot) => defs[slot] = idx,
                None => defs.push(idx),
            }
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

/// unpack one `.rutbundle` into the envelope WITHOUT indexing — the
/// pure-extract twin of `rut_add_bundle`: bytes in, the editable
/// surface out as `{ module, namespace, consts, files: [{ path, src,
/// mode, mod_path }] }`. JS owns the disk: the extension writes the
/// files and re-indexes them through `rut_add_def`/`rut_add_def_mod`
/// (real `file://` origins, the go-to-definition face), so this lane
/// has NO index side effect. `mod_path` rides per file
/// (`deps::bundle_file_mod_path` — the rows-lane root spells `""`, a
/// mounted child its `<path>`; a flat file stays `null`), so the host
/// never re-derives the mount rule in TypeScript. A corrupt/foreign
/// archive is the error envelope, never bytes.
/// # Safety
///
/// `ptr` must point at `len` readable bytes of a bundle.
#[no_mangle]
pub unsafe extern "C" fn rut_extract_bundle(ptr: *const u8, len: usize) -> *mut u8 {
    let bytes = unsafe { read_bytes(ptr, len) };
    let b = match rut_lsp::deps::bundle_sources(bytes) {
        Ok(b) => b,
        Err(e) => return json_envelope(serde_json::json!({ "error": e })),
    };
    let files: Vec<serde_json::Value> = b
        .files
        .iter()
        .map(|f| {
            serde_json::json!({
                "path": f.path,
                "src": f.src,
                "mode": mode_str(f.mode),
                "mod_path": rut_lsp::deps::bundle_file_mod_path(&f.path),
            })
        })
        .collect();
    json_envelope(serde_json::json!({
        "module": b.module,
        "namespace": b.namespace,
        "consts": b.consts,
        "files": files,
    }))
}

/// an open doc's stored text — the smoke test's round-trip check
/// # Safety
///
/// `uri_ptr` must point at `uri_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn rut_doc_len(uri_ptr: *const u8, uri_len: usize) -> usize {
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

    const POUCH_BUNDLE: &[u8] = include_bytes!("../../../dist/std/pouch.rutbundle");
    /// the CDN url's own sha256 — the cache filename rut-lsp computes
    /// (the URL STRING is hashed, never the bytes)
    const POUCH_CACHE: &str =
        ".rut/cache/02bcf87de17b5c095f2c1a768f20f2506a10b9a56a1290ffe4241536e672dbe0.rutbundle";

    /// read an export's result envelope — `[u32 le length][json]`. The
    /// heap is a byte array (align 1), so the length reads byte-wise —
    /// the same read the extension's DataView does
    fn take(ptr: *mut u8) -> serde_json::Value {
        assert!(!ptr.is_null(), "arena overflow");
        unsafe {
            let len = u32::from_le_bytes([*ptr, *ptr.add(1), *ptr.add(2), *ptr.add(3)]) as usize;
            let bytes = core::slice::from_raw_parts(ptr.add(4), len);
            serde_json::from_slice(bytes).expect("the envelope is valid JSON")
        }
    }

    /// The plugin manifest's dep shape, INLINE and unpinned: the real
    /// `examples/03-plugin/plugin/rut.jsonc` migrates off the retired
    /// manifest `sha256` key in the docs/examples phase — the grammar
    /// refuses the key today, so the host-side fixture is the shape
    /// every manifest must now spell (pins ride `rut_set_lock`).
    const PLUGIN_SHAPE: &str = r#"{
  "format": "rutbundle",
  "format_version": 11,
  "name": "plugin",
  "deps": {
    "server": { "path": "../server" },
    "pouch": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v12/dist/std/pouch.rutbundle" }
  }
}"#;

    const POUCH_URL: &str =
        "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v12/dist/std/pouch.rutbundle";
    const POUCH_PIN: &str = "63ddd765287243ef45f23105471ff3eae5c5a0ac1d521a3d22636745fd5e72d8";

    /// hand a lock spelling `rows` to `rut_set_lock`, byte-typed
    fn set_lock(rows: Vec<rut_driver::LockRow>) -> serde_json::Value {
        let text = rut_driver::render_lock(&rut_driver::LockFile::from_rows(rows));
        rut_begin();
        take(unsafe { rut_set_lock(text.as_ptr(), text.len()) })
    }

    #[test]
    fn manifest_envelope_spells_the_dep_table() {
        let _lock = TEST_SERIAL.lock().unwrap();
        // the lock WITHOUT a pouch row stands first: the url row answers
        // `sha256: null` — the unpinned refusal, never a guessed pin
        let out = set_lock(vec![]);
        assert_eq!(out["pins"], 0, "{out}");
        rut_begin();
        let out = take(unsafe { rut_parse_manifest(PLUGIN_SHAPE.as_ptr(), PLUGIN_SHAPE.len()) });
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
        assert_eq!(
            server["source"],
            serde_json::json!({ "kind": "path", "dir": "../server" })
        );
        assert_eq!(server["optional"], false);
        let pouch = rows.iter().find(|r| r["name"] == "pouch").unwrap();
        assert_eq!(pouch["source"]["kind"], "url");
        assert_eq!(pouch["source"]["cache_path"], POUCH_CACHE);
        assert!(
            pouch["source"]["sha256"].is_null(),
            "no lock row ⇒ unpinned: {}",
            pouch["source"]
        );

        // the lock's row stamps the pin — the JSON keeps the `sha256`
        // field, now LOCK-SOURCED (the manifest itself stays unpinned)
        let out = set_lock(vec![rut_driver::LockRow {
            pkg: "pouch".into(),
            url: POUCH_URL.into(),
            sha256: POUCH_PIN.into(),
        }]);
        assert_eq!(out["pins"], 1, "{out}");
        rut_begin();
        let out = take(unsafe { rut_parse_manifest(PLUGIN_SHAPE.as_ptr(), PLUGIN_SHAPE.len()) });
        let pouch = out["deps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == "pouch")
            .unwrap();
        assert_eq!(
            pouch["source"]["sha256"], POUCH_PIN,
            "the pin is lock-sourced"
        );
        assert_eq!(pouch["source"]["cache_path"], POUCH_CACHE);
    }

    #[test]
    fn a_malformed_lock_is_the_error_envelope_and_stands_empty() {
        let _lock = TEST_SERIAL.lock().unwrap();
        rut_begin();
        let bad = take(unsafe {
            rut_set_lock(b"{\"format\": \"rutlock\", \"format_version\": 1, \"deps\": [{\"pkg\": \"p\", \"url\": \"https://x/p.rutbundle\", \"sha256\": \"nothex\"}]}".as_ptr(), b"{\"format\": \"rutlock\", \"format_version\": 1, \"deps\": [{\"pkg\": \"p\", \"url\": \"https://x/p.rutbundle\", \"sha256\": \"nothex\"}]}".len())
        });
        assert!(
            bad["error"].as_str().unwrap().contains("64 hex digits"),
            "{bad}"
        );
        // the empty ledger stands: the url row answers null (the host
        // shows the unpinned refusal), never half-parsed pins
        rut_begin();
        let out = take(unsafe { rut_parse_manifest(PLUGIN_SHAPE.as_ptr(), PLUGIN_SHAPE.len()) });
        let pouch = out["deps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == "pouch")
            .unwrap();
        assert!(pouch["source"]["sha256"].is_null(), "{}", pouch["source"]);
    }

    #[test]
    fn a_decl_entry_and_the_error_envelope() {
        let _lock = TEST_SERIAL.lock().unwrap();
        rut_begin();
        let src = r#"{ "name": "host", "type": "host", "entry": { "type": "./surface.d.rut" } }"#;
        let out = take(unsafe { rut_parse_manifest(src.as_ptr(), src.len()) });
        assert_eq!(
            out["entries"],
            serde_json::json!([{ "path": "surface.d.rut", "mode": "Decl" }]),
            "entry.type indexes in Decl mode"
        );
        // a malformed manifest is the error envelope — the host's hint
        rut_begin();
        let bad = take(unsafe {
            rut_parse_manifest(b"{\"name\": six}".as_ptr(), b"{\"name\": six}".len())
        });
        assert!(
            bad["error"].as_str().unwrap().starts_with("line 1: "),
            "{bad}"
        );
    }

    #[test]
    fn complete_in_answers_locals_and_members_at_the_end_of_buffer() {
        let _lock = TEST_SERIAL.lock().unwrap();
        // plan3 phase 1 (A7): the doc ends EXACTLY at the incomplete
        // stmt — the parser ends the block's span at the EOF token, the
        // cursor sits ON `hi`, and before the scope extension the member
        // tier dropped the receiver (an empty list). The fn body's scope
        // extends through the source end, so the member tier answers
        // through the doc-sync entry — never the bare keyword dump.
        let doc = "class List {\n    items: i32;\n}\nimpl List {\n    fn count(self) -> i32 { return 0; }\n}\nfn go() -> nil {\n    let list = List.new();\n    list.";
        let uri = b"file:///ws/eof-member.rut";
        let last = doc.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line = doc[..last].matches('\n').count() as u32;
        let ch = (doc.len() - last) as u32;
        rut_begin();
        let items = take(unsafe {
            rut_complete_in(uri.as_ptr(), uri.len(), doc.as_ptr(), doc.len(), line, ch)
        });
        let labels: Vec<&str> = items
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["label"].as_str().unwrap())
            .collect();
        for want in ["items", "count"] {
            assert!(
                labels.contains(&want),
                "member completion at EOF lacks '{want}': {labels:?}"
            );
        }
        assert!(
            !labels.contains(&"fn"),
            "no keyword dump after a dot at EOF: {labels:?}"
        );

        // the locals tier (A1) at the same moment: `che⏐` after
        // `let mut checked …` — the binding rides the variable kind
        let doc = "fn go() -> nil {\n    let mut checked: str = \" \";\n    che";
        let uri = b"file:///ws/eof-locals.rut";
        let last = doc.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line = doc[..last].matches('\n').count() as u32;
        let ch = (doc.len() - last) as u32;
        rut_begin();
        let items = take(unsafe {
            rut_complete_in(uri.as_ptr(), uri.len(), doc.as_ptr(), doc.len(), line, ch)
        });
        let arr = items.as_array().unwrap();
        let labels: Vec<&str> = arr.iter().map(|i| i["label"].as_str().unwrap()).collect();
        assert!(
            labels.contains(&"checked"),
            "the local completes at EOF: {labels:?}"
        );
        let checked = arr.iter().find(|i| i["label"] == "checked").unwrap();
        assert_eq!(checked["kind"], 6, "LSP VARIABLE kind: {}", checked["kind"]);
        // ahead of the keyword table
        let checked_at = arr.iter().position(|i| i["label"] == "checked").unwrap();
        let let_at = arr.iter().position(|i| i["label"] == "let").unwrap();
        assert!(checked_at < let_at, "locals ride ahead of the keywords");
    }

    #[test]
    fn complete_in_answers_the_type_tier_at_an_annotation() {
        let _lock = TEST_SERIAL.lock().unwrap();
        // A3 through the raw ABI: `let v: u⏐` — the primitive table
        // rides the prefix, the keyword table / fns / locals stay off
        let doc = "fn go() -> nil {\n    let v: u\n}\n";
        let uri = b"file:///ws/type-tier.rut";
        rut_begin();
        let items = take(unsafe {
            rut_complete_in(uri.as_ptr(), uri.len(), doc.as_ptr(), doc.len(), 1, 12)
        });
        let labels: Vec<&str> = items
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["label"].as_str().unwrap())
            .collect();
        for want in ["u8", "u16", "u32", "u64"] {
            assert!(
                labels.contains(&want),
                "primitive `{want}` completes at the annotation: {labels:?}"
            );
        }
        for absent in ["fn", "let", "go"] {
            assert!(
                !labels.contains(&absent),
                "`{absent}` must stay off a type position: {labels:?}"
            );
        }
        // the bare colon boundary — the full tier, no keywords
        let doc = "fn go() -> nil {\n    let v: \n}\n";
        rut_begin();
        let items = take(unsafe {
            rut_complete_in(uri.as_ptr(), uri.len(), doc.as_ptr(), doc.len(), 1, 11)
        });
        let labels: Vec<&str> = items
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["label"].as_str().unwrap())
            .collect();
        assert!(
            labels.contains(&"i32") && labels.contains(&"str"),
            "the bare colon tier: {labels:?}"
        );
        assert!(
            !labels.contains(&"fn"),
            "no keyword table at the boundary: {labels:?}"
        );
    }

    #[test]
    fn the_bare_scope_gates_a_dep_until_used() {
        let _lock = TEST_SERIAL.lock().unwrap();
        // A2 through the raw ABI: a named dep's fn is NOT a plain bare
        // item until the doc `use`s it — the only bare-doc offer rides
        // the import edit, and the direct-dep set gates the tier
        let (u, n, s) = (
            b"file:///ws/kitdep/lib.rut".as_slice(),
            b"kitdep".as_slice(),
            &b"pub fn tag() -> i32 {\n    return 1;\n}\n"[..],
        );
        // a root manifest declaring ONLY the doc's own pkg — kitdep is
        // mounted but NOT direct (the transitive-only shape)
        rut_begin();
        take(unsafe { rut_set_direct_deps(br#"["myapp"]"#.as_ptr(), br#"["myapp"]"#.len()) });
        rut_begin();
        unsafe {
            rut_add_def_named(
                u.as_ptr(),
                u.len(),
                n.as_ptr(),
                n.len(),
                s.as_ptr(),
                s.len(),
            )
        };
        let doc = "fn main() -> nil {\n}\n";
        let uri = b"file:///ws/bare.rut";
        rut_begin();
        take(unsafe { rut_analyze(uri.as_ptr(), uri.len(), doc.as_ptr(), doc.len()) });
        rut_begin();
        let items = take(unsafe { rut_complete(uri.as_ptr(), uri.len(), 1, 0) });
        let arr = items.as_array().unwrap();
        assert!(
            arr.iter().all(|i| i["label"] != "tag"),
            "a transitive-only dep's fn must not offer at all: {:?}",
            arr.iter().map(|i| &i["label"]).collect::<Vec<_>>()
        );

        // the direct-dep set gains kitdep (the root manifest's row):
        // the bare doc offers `tag` ONLY through the import edit …
        rut_begin();
        take(unsafe {
            rut_set_direct_deps(
                br#"["myapp", "kitdep"]"#.as_ptr(),
                br#"["myapp", "kitdep"]"#.len(),
            )
        });
        rut_begin();
        unsafe {
            rut_add_def_named(
                u.as_ptr(),
                u.len(),
                n.as_ptr(),
                n.len(),
                s.as_ptr(),
                s.len(),
            )
        };
        rut_begin();
        let items = take(unsafe { rut_complete(uri.as_ptr(), uri.len(), 1, 0) });
        let arr = items.as_array().unwrap();
        let imp = arr
            .iter()
            .find(|i| i["label"] == "tag")
            .expect("the direct dep's fn offers");
        assert!(
            imp["additionalTextEdits"]
                .as_array()
                .is_some_and(|e| !e.is_empty()),
            "the offer carries the use-insert edit: {imp}"
        );

        // … and once the doc `use`s it, `tag` is a PLAIN bare item —
        // exactly one, with no edit riding
        let doc = "use kitdep::{ tag };\nfn main() -> nil {\n}\n";
        let uri = b"file:///ws/bare-used.rut";
        rut_begin();
        take(unsafe { rut_analyze(uri.as_ptr(), uri.len(), doc.as_ptr(), doc.len()) });
        rut_begin();
        let items = take(unsafe { rut_complete(uri.as_ptr(), uri.len(), 1, 0) });
        let arr = items.as_array().unwrap();
        let hits: Vec<&serde_json::Value> = arr.iter().filter(|i| i["label"] == "tag").collect();
        assert_eq!(hits.len(), 1, "one `tag` item: {hits:?}");
        assert!(
            hits[0]["additionalTextEdits"].is_null(),
            "the imported fn completes plain: {}",
            hits[0]
        );
    }

    #[test]
    fn definition_on_the_mod_edge_names_the_child_file_top() {
        let _lock = TEST_SERIAL.lock().unwrap();
        // the plan-C lane through the raw ABI: the root file is BOTH a
        // stamped index (its uri carries the (module, mod_path) pair
        // `doc_mods_of` reads) and the analyzed doc; the `helpers` ident
        // in `pub mod helpers;` jumps to the mounted child at 0:0
        rut_begin();
        let root = "pub mod helpers;\nentry fn main() -> nil {\n}\n";
        let child = "pub struct Slot {\n    n: i32;\n}\n";
        let (ru, cu) = (
            b"file:///ws/edgemod/mod.rut".as_slice(),
            b"file:///ws/edgemod/helpers/mod.rut".as_slice(),
        );
        let (n, mp) = (b"edgemod".as_slice(), b"".as_slice());
        unsafe {
            rut_add_def_mod(
                ru.as_ptr(),
                ru.len(),
                n.as_ptr(),
                n.len(),
                mp.as_ptr(),
                mp.len(),
                root.as_ptr(),
                root.len(),
            )
        };
        unsafe {
            rut_add_def_mod(
                cu.as_ptr(),
                cu.len(),
                n.as_ptr(),
                n.len(),
                b"helpers".as_ptr(),
                7,
                child.as_ptr(),
                child.len(),
            )
        };
        rut_begin();
        take(unsafe { rut_analyze(ru.as_ptr(), ru.len(), root.as_ptr(), root.len()) });
        // the edge ident starts at char 8 (`pub mod `)
        rut_begin();
        let locs = take(unsafe { rut_definition(ru.as_ptr(), ru.len(), 0, 8) });
        let arr = locs.as_array().unwrap();
        assert_eq!(arr.len(), 1, "exactly one target: {locs}");
        assert!(
            arr[0]["uri"]
                .as_str()
                .unwrap()
                .ends_with("/edgemod/helpers/mod.rut"),
            "the child uri: {locs}"
        );
        assert_eq!(
            arr[0]["range"],
            serde_json::json!({"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}),
            "the child file top: {locs}"
        );

        // an UNSTAMPED doc with the same text misses (the doc context
        // keys the lane — the scratch uri carries no stamp)
        let scratch = b"file:///ws/edgemod-scratch.rut".as_slice();
        rut_begin();
        take(unsafe { rut_analyze(scratch.as_ptr(), scratch.len(), root.as_ptr(), root.len()) });
        rut_begin();
        let none = take(unsafe { rut_definition(scratch.as_ptr(), scratch.len(), 0, 8) });
        assert!(
            none.as_array().unwrap().is_empty(),
            "unstamped miss: {none}"
        );
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
        unsafe {
            rut_add_def_named(
                u.as_ptr(),
                u.len(),
                n.as_ptr(),
                n.len(),
                s.as_ptr(),
                s.len(),
            )
        };
        let doc = "use toolkit::\nfn main() -> nil {\n}\n";
        let uri = "file:///ws/main.rut";
        rut_begin();
        take(unsafe { rut_analyze(uri.as_ptr(), uri.len(), doc.as_ptr(), doc.len()) });
        // `use toolkit::⏐` — the dep's public names
        rut_begin();
        let items = take(unsafe { rut_complete(uri.as_ptr(), uri.len(), 0, 13) });
        let labels: Vec<&str> = items
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["label"].as_str().unwrap())
            .collect();
        assert!(
            labels.contains(&"Widget"),
            "the dep type completes: {labels:?}"
        );
        assert!(labels.contains(&"tag"), "the dep fn completes: {labels:?}");
        // `use ⏐` — the module-name tier sees the named index
        rut_begin();
        let mods = take(unsafe { rut_complete(uri.as_ptr(), uri.len(), 0, 4) });
        let names: Vec<&str> = mods
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["label"].as_str().unwrap())
            .collect();
        assert!(
            names.contains(&"toolkit"),
            "the module name completes: {names:?}"
        );
    }

    #[test]
    fn complete_in_syncs_the_doc_and_the_dot_guard_holds() {
        let _lock = TEST_SERIAL.lock().unwrap();
        // ctor inference rides the embedded ink surface: `Logger("h")`
        // types the binding, `log.` lists the impl methods. The stored
        // doc is STALE on purpose (the tail still short the dot — the
        // analyze debounce's window): the `_in` entry stores the
        // CURRENT text first, so the member tier still answers.
        let stale = "fn go() -> nil {\n    let log = Logger(\"h\");\n    log\n}\n";
        let fresh = "fn go() -> nil {\n    let log = Logger(\"h\");\n    log.\n}\n";
        let uri = b"file:///ws/sync.rut";
        rut_begin();
        take(unsafe { rut_analyze(uri.as_ptr(), uri.len(), stale.as_ptr(), stale.len()) });
        rut_begin();
        let items = take(unsafe {
            rut_complete_in(uri.as_ptr(), uri.len(), fresh.as_ptr(), fresh.len(), 2, 8)
        });
        let labels: Vec<&str> = items
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["label"].as_str().unwrap())
            .collect();
        for want in ["new", "debug", "info", "warn", "error"] {
            assert!(
                labels.contains(&want),
                "`{want}` completes through the ctor binding: {labels:?}"
            );
        }
        // the bare tier's host fns never dump into a member position
        assert!(
            !labels.contains(&"abs"),
            "no host-fn dump after a dot: {labels:?}"
        );

        // the dot guard: a receiver that is not a name (`probe().` — the
        // token before the dot is a `)`) answers EMPTY, never the bare
        // keyword/host-fn dump
        let guarded = "fn go() -> nil {\n    let x = probe().\n}\n";
        rut_begin();
        let none = take(unsafe {
            rut_complete_in(
                uri.as_ptr(),
                uri.len(),
                guarded.as_ptr(),
                guarded.len(),
                1,
                20,
            )
        });
        assert!(none.as_array().unwrap().is_empty(), "{none}");
    }

    #[test]
    fn add_bundle_indexes_the_pouch_surface() {
        let _lock = TEST_SERIAL.lock().unwrap();
        rut_begin();
        let out = take(unsafe { rut_add_bundle(POUCH_BUNDLE.as_ptr(), POUCH_BUNDLE.len()) });
        assert_eq!(out["module"], "pouch");
        assert_eq!(out["files"], serde_json::json!(["mod.d.rut"]));
        // a corrupt archive is the error envelope, never silent state
        rut_begin();
        let junk = b"definitely not a zip";
        let err = take(unsafe { rut_add_bundle(junk.as_ptr(), junk.len()) });
        assert!(
            err["error"].as_str().unwrap().contains("not a zip"),
            "{err}"
        );

        // the indexed surface resolves: the use name jumps to BOTH pouch
        // indexes — the std surface's true source path AND the bundle's
        let doc = "use pouch::{ Vec };\nfn main() -> nil {\n    let v: Vec<i32> = Vec::new();\n    v.push(1);\n}\n";
        let uri = "file:///ws/uses-pouch.rut";
        rut_begin();
        take(unsafe { rut_analyze(uri.as_ptr(), uri.len(), doc.as_ptr(), doc.len()) });
        rut_begin();
        let locs = take(unsafe { rut_definition(uri.as_ptr(), uri.len(), 0, 13) }); // the use-line `Vec`
        let uris: Vec<&str> = locs
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["uri"].as_str().unwrap())
            .collect();
        assert!(
            uris.contains(&"bundle:pouch/mod.d.rut"),
            "the bundle index answers go-to-definition: {uris:?}"
        );
        // and the member surface hovers through the named module
        let push_ch = doc.lines().nth(3).unwrap().find("push").unwrap() as u32;
        rut_begin();
        let h = take(unsafe { rut_hover(uri.as_ptr(), uri.len(), 3, push_ch) });
        assert!(
            h["contents"]["value"].as_str().unwrap().contains("push"),
            "{h}"
        );
    }

    /// plan3 phase 4 (B1+B2): the CHECK LANE through the raw ABI — a
    /// doc whose package is STAMPED (`rut_add_def_mod` named it)
    /// compiles with its dep closure, and the checker's type errors
    /// surface with the right file and range: the bool→str assignment
    /// mismatch AND the missing record field, the diagnosis pair. An
    /// UNSTAMPED doc with the same text answers parse-only clean —
    /// today's behavior, the corpus lock's foundation.
    #[test]
    fn analyze_runs_the_checker_over_a_stamped_chain() {
        let _lock = TEST_SERIAL.lock().unwrap();
        // the dep: one mod-tree file mounted under its module NAME
        let dep_src =
            &b"pub struct Task {\n    id: u32;\n    title: str;\n    completed: bool;\n}\n"[..];
        let dep_uri = b"file:///ws/chk/dep/mod.rut";
        rut_begin();
        let stamped = take(unsafe {
            rut_add_def_mod(
                dep_uri.as_ptr(),
                dep_uri.len(),
                b"chkdep".as_ptr(),
                6,
                b"".as_ptr(),
                0,
                dep_src.as_ptr(),
                dep_src.len(),
            )
        });
        assert!(
            stamped["decls"].as_array().unwrap().is_empty(),
            "{}",
            stamped
        );
        // the doc's own pkg root, stamped the same way the extension's
        // workspace indexer stamps it
        let doc = "use chkdep::{ Task };\nfn main() -> nil {\n    let mut checked: str = \" \";\n    checked = true;\n    let t = Task { title: \"x\", completed: false };\n}\n";
        let app_uri = b"file:///ws/chk/app/mod.rut";
        rut_begin();
        let stamped = take(unsafe {
            rut_add_def_mod(
                app_uri.as_ptr(),
                app_uri.len(),
                b"chkapp".as_ptr(),
                6,
                b"".as_ptr(),
                0,
                doc.as_ptr(),
                doc.len(),
            )
        });
        assert!(
            stamped["decls"].as_array().unwrap().is_empty(),
            "{}",
            stamped
        );

        // the analyze lane returns the MERGED diags: both checker sites,
        // `source: "rut"`, each on its own line
        rut_begin();
        let a =
            take(unsafe { rut_analyze(app_uri.as_ptr(), app_uri.len(), doc.as_ptr(), doc.len()) });
        let diags = a["diags"].as_array().unwrap();
        let mismatch = diags
            .iter()
            .find(|d| {
                d["message"]
                    .as_str()
                    .unwrap_or("")
                    .contains("assignment type mismatch")
            })
            .unwrap_or_else(|| panic!("no assignment-mismatch diag: {a}"));
        let missing = diags
            .iter()
            .find(|d| {
                d["message"]
                    .as_str()
                    .unwrap_or("")
                    .contains("`id` is missing")
            })
            .unwrap_or_else(|| panic!("no missing-field diag: {a}"));
        for d in [mismatch, missing] {
            assert_eq!(d["source"], "rut", "{d}");
        }
        assert_eq!(mismatch["range"]["start"]["line"], 3, "{mismatch}");
        assert_eq!(missing["range"]["start"]["line"], 4, "{missing}");

        // the parse-only fallback: the SAME text at an UNSTAMPED uri is
        // clean (no package graph assembles for it)
        let plain_uri = b"file:///ws/chk/plain.rut";
        rut_begin();
        let plain = take(unsafe {
            rut_analyze(plain_uri.as_ptr(), plain_uri.len(), doc.as_ptr(), doc.len())
        });
        assert!(
            plain["diags"].as_array().unwrap().is_empty(),
            "an unstamped doc must stay parse-only: {plain}"
        );
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
        let out = take(unsafe {
            rut_add_def_mod(
                uri_root.as_ptr(),
                uri_root.len(),
                name.as_ptr(),
                name.len(),
                b"".as_ptr(),
                0,
                root_src.as_ptr(),
                root_src.len(),
            )
        });
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
        let out = take(unsafe {
            rut_add_def_mod(
                uri_layout.as_ptr(),
                uri_layout.len(),
                name.as_ptr(),
                name.len(),
                b"layout".as_ptr(),
                6,
                layout_src.as_ptr(),
                layout_src.len(),
            )
        });
        assert!(
            out["decls"].as_array().unwrap().is_empty(),
            "{}",
            out["decls"]
        );

        // `use gadgets::layout::⏐` — the walked module's exports (the
        // private `secret` edge walked would be empty)
        let doc = "use gadgets::layout::\nfn main() -> nil {\n}\n";
        let uri = "file:///ws/main.rut";
        rut_begin();
        take(unsafe { rut_analyze(uri.as_ptr(), uri.len(), doc.as_ptr(), doc.len()) });
        rut_begin();
        let items = take(unsafe { rut_complete(uri.as_ptr(), uri.len(), 0, 21) });
        let labels: Vec<&str> = items
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["label"].as_str().unwrap())
            .collect();
        assert!(labels.contains(&"Column"), "{labels:?}");
        assert!(labels.contains(&"mk"), "{labels:?}");
        // the root tier offers the pub mod child, never the private one
        rut_begin();
        let items = take(unsafe { rut_complete(uri.as_ptr(), uri.len(), 0, 13) });
        let labels: Vec<&str> = items
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["label"].as_str().unwrap())
            .collect();
        assert!(labels.contains(&"layout"), "{labels:?}");
        assert!(
            !labels.contains(&"secret"),
            "a bare `mod` edge never crosses: {labels:?}"
        );
        assert!(
            !labels.contains(&"Column"),
            "child members stay under their path: {labels:?}"
        );
    }

    /// a mod-carrying bundle in the pack writer's shape — the wasm
    /// face mounts the rows tree additively
    fn kit_bundle() -> Vec<u8> {
        use std::collections::BTreeMap;
        let mods = BTreeMap::from([(
            "layout".to_string(),
            rut_driver::ModSource {
                path: "layout".into(),
                vis: rut_ast::ast::Vis::Pub,
                text: "pub struct Column {\n    w: i32;\n}\n".into(),
                file: "mod.rut".into(),
            },
        )]);
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
        let out = take(unsafe { rut_add_bundle(bytes.as_ptr(), bytes.len()) });
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
        take(unsafe { rut_analyze(uri.as_ptr(), uri.len(), doc.as_ptr(), doc.len()) });
        rut_begin();
        let items = take(unsafe { rut_complete(uri.as_ptr(), uri.len(), 0, 16) });
        let labels: Vec<&str> = items
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["label"].as_str().unwrap())
            .collect();
        assert!(labels.contains(&"Column"), "{labels:?}");
        // and the segment hover resolves through the dep tree
        rut_begin();
        let h = take(unsafe { rut_hover(uri.as_ptr(), uri.len(), 0, 12) }); // the `layout` segment
        let md = h["contents"]["value"].as_str().unwrap();
        assert!(md.contains("file module"), "{md}");
        assert!(md.contains("layout/mod.rut"), "{md}");
    }

    /// the extract lane — `rut_add_bundle`'s unpack WITHOUT the index
    /// write: the file list mirrors `bundle_sources` byte-for-byte,
    /// every file spells its own mod path (the host never re-derives
    /// the mount rule), and the defs store ends the call untouched
    #[test]
    fn extract_bundle_names_the_surface_and_never_indexes() {
        let _lock = TEST_SERIAL.lock().unwrap();
        let origins_before: std::collections::BTreeSet<String> =
            state().defs.iter().map(|d| d.origin.clone()).collect();
        rut_begin();
        let out = take(unsafe { rut_extract_bundle(POUCH_BUNDLE.as_ptr(), POUCH_BUNDLE.len()) });
        assert_eq!(out["module"], "pouch");
        assert!(out["namespace"].is_null());
        // the file list mirrors `bundle_sources`' own unpack
        let b = rut_lsp::deps::bundle_sources(POUCH_BUNDLE).expect("the pouch bundle unpacks");
        let paths: Vec<&str> = out["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap())
            .collect();
        let want: Vec<&str> = b.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, want, "the envelope's files ride bundle_sources'");
        // the generated declaration entry rides, mode + mod path spelled
        // (wire v12 — the entry IS the surface the extension writes back)
        let f = &out["files"][0];
        assert_eq!(f["path"], "mod.d.rut");
        assert_eq!(f["mode"], "Decl");
        assert_eq!(f["mod_path"], "");
        assert!(
            f["src"].as_str().unwrap().contains("class Vec"),
            "the pouch entry rides"
        );
        // NO indexing side effect — the defs store ends where it started
        let origins_after: std::collections::BTreeSet<String> =
            state().defs.iter().map(|d| d.origin.clone()).collect();
        assert_eq!(
            origins_before, origins_after,
            "the extract lane never writes the index"
        );

        // a mod tree spells each file's mount path (the root `""`, each
        // child its `<path>`); a corrupt archive stays the error
        // envelope, never bytes
        let bytes = kit_bundle();
        rut_begin();
        let out = take(unsafe { rut_extract_bundle(bytes.as_ptr(), bytes.len()) });
        let spells: Vec<(&str, &str)> = out["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| (f["path"].as_str().unwrap(), f["mod_path"].as_str().unwrap()))
            .collect();
        assert_eq!(spells, vec![("mod.rut", ""), ("layout/mod.rut", "layout")]);
        rut_begin();
        let junk = b"definitely not a zip";
        let err = take(unsafe { rut_extract_bundle(junk.as_ptr(), junk.len()) });
        assert!(
            err["error"].as_str().unwrap().contains("not a zip"),
            "{err}"
        );
    }
}
