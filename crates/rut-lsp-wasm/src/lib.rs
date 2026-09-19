//! rut-lsp-wasm — the language core as an in-process wasm module
//! (RFC 0041 §2). The VS Code extension instantiates this and registers
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
//!   rut_add_def(uri, src)                      index a workspace file
//!
//! Every result is `[u32 little-endian length][bytes]` at the returned
//! ptr; every string argument is `(ptr, len)` written via `rut_alloc`.
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

/// completions at an LSP position (member after `.`, else bare)
#[no_mangle]
pub extern "C" fn rut_complete(uri_ptr: *const u8, uri_len: usize, line: u32, ch: u32) -> *mut u8 {
    let uri = unsafe { read_str(uri_ptr, uri_len) };
    let Some(src) = state().docs.get(uri).cloned() else {
        return envelope(b"[]");
    };
    let items = {
        let defs = &state().defs;
        rut_lsp::analysis::complete_at(uri, &src, defs, line, ch)
    };
    json_envelope(serde_json::json!(items))
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

/// an open doc's stored text — the smoke test's round-trip check
#[no_mangle]
pub extern "C" fn rut_doc_len(uri_ptr: *const u8, uri_len: usize) -> usize {
    let uri = unsafe { read_str(uri_ptr, uri_len) };
    state().docs.get(uri).map(|s| s.len()).unwrap_or(0)
}
