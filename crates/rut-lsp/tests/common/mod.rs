//! The shared envelope harness — the generalization of the public-door
//! trio's shape (abi/sched/walk) that the whole tree drives. One
//! [`Session`] owns the `Lsp` + the native `LocalPool`; every door is
//! an envelope in, JSON out; the needle helpers spell LSP positions
//! the way a host would. The in-memory [`MemIo`] is the walk's world
//! (the manifest-read order IS the proof of which walk ran).
//!
//! Each integration test is its own crate and uses a different subset
//! of the helpers, so the module carries the one scoped dead-code
//! allow (the house style's one-line reason).

#![allow(dead_code)] // shared harness — every test binary compiles it and uses a subset

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::rc::Rc;

use futures::executor::LocalPool;
use rut_lsp::env::{Io, WakeFn};
use rut_lsp::Lsp;
use serde_json::Value;

// ---- the wire's number spellings (the LSP enumerations the
// envelopes carry; the names keep the asserts readable) ----

/// CompletionItemKind — the icons `lsp_item` maps `CompletionKind` to
/// (Parameter rides the VARIABLE icon: LSP spells no parameter kind).
pub mod k {
    pub const METHOD: i64 = 2;
    pub const FUNCTION: i64 = 3;
    pub const FIELD: i64 = 5;
    pub const VARIABLE: i64 = 6; // parameters too
    pub const CLASS: i64 = 7; // the type rows ride Class
    pub const INTERFACE: i64 = 8;
    pub const MODULE: i64 = 9;
    pub const ENUM_MEMBER: i64 = 20;
    pub const CONSTANT: i64 = 21;
    pub const KEYWORD: i64 = 14;
}

/// SymbolKind — the icons `symbol_kind` maps `SymKind` to.
pub mod sk {
    pub const MODULE: i64 = 2;
    pub const CLASS: i64 = 5;
    pub const METHOD: i64 = 6;
    pub const FIELD: i64 = 8;
    pub const ENUM: i64 = 10;
    pub const INTERFACE: i64 = 11;
    pub const FUNCTION: i64 = 12;
    pub const VARIABLE: i64 = 13;
    pub const ENUM_MEMBER: i64 = 22;
}

/// the semantic-token legend's indices (the order `legend()` spells;
/// the classify tests pin both the count and the names)
pub mod t {
    pub const KEYWORD: i64 = 0;
    pub const NUMBER: i64 = 1;
    pub const STRING: i64 = 2;
    pub const TYPE: i64 = 3;
    pub const FUNCTION: i64 = 4;
    pub const METHOD: i64 = 5;
    pub const VARIABLE: i64 = 6;
    pub const PARAMETER: i64 = 7;
    pub const PROPERTY: i64 = 8;
    pub const ENUM: i64 = 9;
    pub const ENUM_MEMBER: i64 = 10;
    pub const CLASS: i64 = 11;
    pub const INTERFACE: i64 = 12;
    pub const NAMESPACE: i64 = 13;
    pub const OPERATOR: i64 = 14;
}

// ---- the JSON helpers ----

/// parse one envelope's answer
pub fn take(json: String) -> Value {
    serde_json::from_str(&json).expect("the envelope is valid JSON")
}

/// an envelope's `contents.value` markdown (hover) — owned, so call
/// sites chain the door straight in
pub fn markdown(h: Value) -> String {
    h["contents"]["value"]
        .as_str()
        .expect("the hover carries markdown contents")
        .to_string()
}

/// does `h` answer null (no hover)?
pub fn no_hover(h: &Value) -> bool {
    h.is_null()
}

// ---- position helpers (LSP currency: 0-based lines, UTF-16 columns;
// the ASCII docs make columns plain byte counts) ----

/// (line, character) where the `occurrence`-th (1-based) needle STARTS
pub fn pos_of(src: &str, needle: &str, occurrence: usize) -> (u32, u32) {
    let mut from = 0usize;
    for _ in 0..occurrence {
        match src[from..].find(needle) {
            Some(i) => from += i + needle.len(),
            None => panic!("needle {needle:?} not found (occurrence {occurrence})"),
        }
    }
    let at = from - needle.len();
    let line = src[..at].matches('\n').count() as u32;
    let ch = at - src[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
    (line, ch as u32)
}

/// (line, character) just PAST the `occurrence`-th needle — the cursor
/// after a typed prefix (completion's shape)
pub fn pos_after(src: &str, needle: &str, occurrence: usize) -> (u32, u32) {
    let (line, ch) = pos_of(src, needle, occurrence);
    (line, ch + needle.len() as u32)
}

/// byte offset of the `occurrence`-th needle
pub fn byte_of(src: &str, needle: &str, occurrence: usize) -> u32 {
    let mut from = 0usize;
    for _ in 0..occurrence {
        match src[from..].find(needle) {
            Some(i) => from += i + needle.len(),
            None => panic!("needle {needle:?} not found (occurrence {occurrence})"),
        }
    }
    (from - needle.len()) as u32
}

/// byte offset of an LSP position over `src` — the UTF-16 walk the
/// server's own line index runs (the decode half of every range assert)
pub fn byte_at(src: &str, line: u32, ch: u32) -> usize {
    let mut line_start = 0usize;
    let mut cur = 0u32;
    while cur < line {
        match src[line_start..].find('\n') {
            Some(i) => {
                line_start += i + 1;
                cur += 1;
            }
            None => return src.len(), // beyond EOF: clamp
        }
    }
    let mut units = 0u32;
    for (off, c) in src[line_start..].char_indices() {
        if units >= ch || c == '\n' {
            return line_start + off;
        }
        units += c.len_utf16() as u32;
    }
    src.len()
}

/// the byte span a Location's range points at, over `src`
pub fn span_of(src: &str, loc: &Value) -> (usize, usize) {
    let s = &loc["range"]["start"];
    let e = &loc["range"]["end"];
    (
        byte_at(
            src,
            s["line"].as_u64().unwrap() as u32,
            s["character"].as_u64().unwrap() as u32,
        ),
        byte_at(
            src,
            e["line"].as_u64().unwrap() as u32,
            e["character"].as_u64().unwrap() as u32,
        ),
    )
}

/// a JSON position as (line, character) — the tuple the range asserts
/// compare
pub fn pos_pair(p: &Value) -> (i64, i64) {
    (
        p["line"].as_i64().expect("position line"),
        p["character"].as_i64().expect("position character"),
    )
}

// ---- completion answer helpers ----

/// the labels of a completion answer, in answer order
pub fn labels(items: &[Value]) -> Vec<&str> {
    items
        .iter()
        .map(|i| i["label"].as_str().expect("every item spells a label"))
        .collect()
}

/// the item with `label` — panics when the tier never offered it
pub fn item<'v>(items: &'v [Value], label: &str) -> &'v Value {
    items
        .iter()
        .find(|i| i["label"] == label)
        .unwrap_or_else(|| panic!("no `{label}` item: {:?}", labels(items)))
}

/// does any item carry `label`?
pub fn offers(items: &[Value], label: &str) -> bool {
    items.iter().any(|i| i["label"] == label)
}

/// the auto-import offer for `label`, if the tier rode one (an item
/// with additional text edits)
pub fn import_offer<'v>(items: &'v [Value], label: &str) -> Option<&'v Value> {
    items
        .iter()
        .find(|i| i["label"] == label && i["additionalTextEdits"].is_array())
}

// ---- the semantic-token decode (the delta stream → (text, type)) ----

/// one decoded token: its source text, its legend index, its position
#[derive(Debug)]
pub struct Tok {
    pub text: String,
    pub ty: i64,
    pub line: u32,
    pub ch: u32,
    /// the byte span the token covers over the analyzed source
    pub lo: usize,
    pub hi: usize,
}

/// decode `analyze`'s semantic-token stream over `src`: the flat
/// 5-int deltas re-joined to absolute positions and text (the decode
/// half of the LSP contract — every classify assert rides it)
pub fn decode_tokens(src: &str, analysis: &Value) -> Vec<Tok> {
    let data = analysis["tokens"]["data"]
        .as_array()
        .expect("the token stream rides as the flat array");
    // the wire is ONE flat u32 run — 5 ints per token (the ls-types
    // SemanticToken serializes flattened, exactly what a client reads)
    assert!(
        data.len().is_multiple_of(5),
        "the stream length is a multiple of 5: {}",
        data.len()
    );
    let mut line = 0u32;
    let mut ch = 0u32;
    let mut out = Vec::with_capacity(data.len() / 5);
    for row in data.chunks(5) {
        let (dl, ds, len, ty) = (
            row[0].as_i64().expect("deltaLine"),
            row[1].as_i64().expect("deltaStart"),
            row[2].as_i64().expect("length"),
            row[3].as_i64().expect("tokenType"),
        );
        line += dl as u32;
        ch = if dl == 0 { ch + ds as u32 } else { ds as u32 };
        let lo = byte_at(src, line, ch);
        let hi = byte_at(src, line, ch + len as u32);
        out.push(Tok {
            text: src[lo..hi].to_string(),
            ty,
            line,
            ch,
            lo,
            hi,
        });
    }
    out
}

/// the legend indices of every decoded token whose text is `text`
pub fn types_of(toks: &[Tok], text: &str) -> Vec<i64> {
    toks.iter()
        .filter(|t| t.text == text)
        .map(|t| t.ty)
        .collect()
}

// ---- the session ----

/// one editing session — the host's whole world: the `Lsp` inbox
/// handle beside its native pool (every door is `pool.run_until(bridge)`)
pub struct S {
    pub lsp: Lsp,
    pub pool: LocalPool,
}

impl S {
    /// a fresh core, the prelude pre-mounted, no env
    pub fn boot() -> S {
        let (lsp, pool) = Lsp::spawn_native();
        S { lsp, pool }
    }

    /// a fresh core with the recording env injected through the same
    /// door the wasm face posts its `JsEnv`
    pub fn with_io(io: &Rc<MemIo>) -> S {
        let (lsp, mut pool) = Lsp::spawn_native();
        pool.run_until(lsp.set_env_io(io.clone()));
        S { lsp, pool }
    }

    /// drive one future to completion on the pool (the disjoint-field
    /// shape — a test holding its own future pins its own ordering)
    pub fn run<T>(&mut self, fut: impl Future<Output = T>) -> T {
        self.pool.run_until(fut)
    }

    // ---- the provider doors (JSON in, JSON out) ----

    /// open / FULL-sync change, then the analysis envelope
    pub fn analyze(&mut self, uri: &str, src: &str) -> Value {
        take(
            self.pool
                .run_until(self.lsp.analyze(uri.to_string(), src.to_string())),
        )
    }

    /// the legend (token-type names in index order)
    pub fn legend(&mut self) -> Vec<String> {
        take(self.pool.run_until(self.lsp.legend()))
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect()
    }

    /// `analyze` then hover — the stored doc the query reads. The
    /// answer is the LSP Hover JSON or null.
    pub fn hover_at(&mut self, uri: &str, src: &str, needle: &str, occ: usize) -> Value {
        self.analyze(uri, src);
        let (l, c) = pos_of(src, needle, occ);
        take(self.pool.run_until(self.lsp.hover(uri.to_string(), l, c)))
    }

    /// hover the stored doc at a raw position
    pub fn hover_pos(&mut self, uri: &str, line: u32, ch: u32) -> Value {
        take(
            self.pool
                .run_until(self.lsp.hover(uri.to_string(), line, ch)),
        )
    }

    /// hover at `off` bytes past the `occ`-th needle's start — the
    /// position lands mid-ident (the use site inside a longer statement)
    pub fn hover_in(&mut self, uri: &str, src: &str, needle: &str, occ: usize, off: u32) -> Value {
        self.analyze(uri, src);
        let (l, c) = pos_of(src, needle, occ);
        self.hover_pos(uri, l, c + off)
    }

    /// completions at the cursor past the `occ`-th needle (the
    /// doc-sync door — the CURRENT text rides every call)
    pub fn complete_at(&mut self, uri: &str, src: &str, needle: &str, occ: usize) -> Vec<Value> {
        let (l, c) = pos_after(src, needle, occ);
        self.complete_pos(uri, src, l, c)
    }

    /// completions at a raw position over `src`
    pub fn complete_pos(&mut self, uri: &str, src: &str, line: u32, ch: u32) -> Vec<Value> {
        take(
            self.pool.run_until(
                self.lsp
                    .complete(uri.to_string(), src.to_string(), line, ch),
            ),
        )
        .as_array()
        .expect("the completion answer is an array")
        .clone()
    }

    /// `analyze` then definition at the `occ`-th needle — the Location
    /// array (empty when nothing resolves)
    pub fn def_at(&mut self, uri: &str, src: &str, needle: &str, occ: usize) -> Vec<Value> {
        self.analyze(uri, src);
        let (l, c) = pos_of(src, needle, occ);
        self.def_pos(uri, l, c)
    }

    /// definition at a raw position over the stored doc
    pub fn def_pos(&mut self, uri: &str, line: u32, ch: u32) -> Vec<Value> {
        take(
            self.pool
                .run_until(self.lsp.definition(uri.to_string(), line, ch)),
        )
        .as_array()
        .expect("the definition answer is an array")
        .clone()
    }

    /// definition at `off` bytes past the `occ`-th needle's start —
    /// the click lands mid-ident (the ident after its introducer)
    pub fn def_in(
        &mut self,
        uri: &str,
        src: &str,
        needle: &str,
        occ: usize,
        off: u32,
    ) -> Vec<Value> {
        self.analyze(uri, src);
        let (l, c) = pos_of(src, needle, occ);
        self.def_pos(uri, l, c + off)
    }

    /// references at `off` past the needle's start
    pub fn refs_in(
        &mut self,
        uri: &str,
        src: &str,
        needle: &str,
        occ: usize,
        off: u32,
        include_decl: bool,
    ) -> Vec<Value> {
        self.analyze(uri, src);
        let (l, c) = pos_of(src, needle, occ);
        self.refs_pos(uri, (l, c + off), include_decl)
    }

    /// a byte offset as an LSP position over `src` (the ASCII docs
    /// make this a plain walk)
    pub fn pos_at_byte(src: &str, byte: u32) -> (u32, u32) {
        let line = src[..byte as usize].matches('\n').count() as u32;
        let ch = byte - src[..byte as usize].rfind('\n').map(|i| i + 1).unwrap_or(0) as u32;
        (line, ch)
    }

    /// typeDefinition at the `occ`-th needle
    pub fn typedef_at(&mut self, uri: &str, src: &str, needle: &str, occ: usize) -> Vec<Value> {
        self.analyze(uri, src);
        let (l, c) = pos_of(src, needle, occ);
        take(
            self.pool
                .run_until(self.lsp.type_definition(uri.to_string(), l, c)),
        )
        .as_array()
        .expect("the typeDefinition answer is an array")
        .clone()
    }

    /// typeDefinition at `off` past the needle's start
    pub fn typedef_in(
        &mut self,
        uri: &str,
        src: &str,
        needle: &str,
        occ: usize,
        off: u32,
    ) -> Vec<Value> {
        self.analyze(uri, src);
        let (l, c) = pos_of(src, needle, occ);
        take(
            self.pool
                .run_until(self.lsp.type_definition(uri.to_string(), l, c + off)),
        )
        .as_array()
        .expect("the typeDefinition answer is an array")
        .clone()
    }

    /// references at the `occ`-th needle
    pub fn refs_at(
        &mut self,
        uri: &str,
        src: &str,
        needle: &str,
        occ: usize,
        include_decl: bool,
    ) -> Vec<Value> {
        self.analyze(uri, src);
        let (l, c) = pos_of(src, needle, occ);
        take(
            self.pool.run_until(self.lsp.references(
                uri.to_string(),
                l,
                c,
                u32::from(include_decl),
            )),
        )
        .as_array()
        .expect("the references answer is an array")
        .clone()
    }

    /// signature help at the `occ`-th needle — the LSP value or null
    pub fn sig_at(&mut self, uri: &str, src: &str, needle: &str, occ: usize) -> Value {
        self.analyze(uri, src);
        let (l, c) = pos_of(src, needle, occ);
        take(
            self.pool
                .run_until(self.lsp.signature_help(uri.to_string(), l, c)),
        )
    }

    /// signature help at a raw position over the stored doc
    pub fn sig_pos(&mut self, uri: &str, pos: (u32, u32)) -> Value {
        take(
            self.pool
                .run_until(self.lsp.signature_help(uri.to_string(), pos.0, pos.1)),
        )
    }

    /// references at a raw position over the stored doc
    pub fn refs_pos(&mut self, uri: &str, pos: (u32, u32), include_decl: bool) -> Vec<Value> {
        take(self.pool.run_until(self.lsp.references(
            uri.to_string(),
            pos.0,
            pos.1,
            u32::from(include_decl),
        )))
        .as_array()
        .expect("the references answer is an array")
        .clone()
    }

    /// signature help at `off` past the needle's start (mid-arg)
    pub fn sig_in(&mut self, uri: &str, src: &str, needle: &str, occ: usize, off: u32) -> Value {
        self.analyze(uri, src);
        let (l, c) = pos_of(src, needle, occ);
        take(
            self.pool
                .run_until(self.lsp.signature_help(uri.to_string(), l, c + off)),
        )
    }

    /// inlay hints over the whole stored document
    pub fn inlay_all(&mut self, uri: &str, src: &str) -> Vec<Value> {
        self.analyze(uri, src);
        let lines = src.matches('\n').count() as u32;
        take(
            self.pool
                .run_until(self.lsp.inlay(uri.to_string(), 0, 0, lines, 0)),
        )
        .as_array()
        .expect("the inlay answer is an array")
        .clone()
    }

    /// inlay hints over the range [needle .. end of document) — the
    /// viewport filter's shape
    pub fn inlay_from(&mut self, uri: &str, src: &str, needle: &str, occ: usize) -> Vec<Value> {
        self.analyze(uri, src);
        let (l, c) = pos_of(src, needle, occ);
        let lines = src.matches('\n').count() as u32;
        take(
            self.pool
                .run_until(self.lsp.inlay(uri.to_string(), l, c, lines, 0)),
        )
        .as_array()
        .expect("the inlay answer is an array")
        .clone()
    }

    // ---- the workspace-scan push + the deps face ----

    /// index one workspace file under its open-doc stamp
    pub fn index_file(&mut self, uri: &str, src: &str) {
        self.pool
            .run_until(self.lsp.index_file(uri.to_string(), src.to_string()));
    }

    /// index one dep source file at a MOD PATH — the envelope answers
    /// the file's own `mod` declarations (the mount walk's next hop)
    pub fn index_mod_at(&mut self, uri: &str, pkg: &str, mod_path: &str, src: &str) -> Value {
        take(self.pool.run_until(self.lsp.index_mod_at(
            uri.to_string(),
            pkg.to_string(),
            mod_path.to_string(),
            src.to_string(),
        )))
    }

    /// parse one manifest — the dep-table JSON (or the error envelope)
    pub fn parse_manifest(&mut self, text: &str) -> Value {
        take(
            self.pool
                .run_until(self.lsp.parse_manifest(text.to_string())),
        )
    }

    /// the eager walk over `roots` — the WalkOut JSON
    pub fn resolve(&mut self, roots: &[&str]) -> Value {
        let roots: Vec<String> = roots.iter().map(|s| s.to_string()).collect();
        take(
            self.pool.run_until(
                self.lsp
                    .resolve_deps(serde_json::to_string(&roots).unwrap()),
            ),
        )
    }

    /// close: drop the stored doc
    pub fn forget(&mut self, uri: &str) {
        self.pool.run_until(self.lsp.forget(uri.to_string()));
    }
}

// ---- the in-memory host env ----

/// the walk's whole world as a file map + dir set: a root is its
/// `rut.jsonc` + sources, so which manifest got read names which walk
/// ran. The clock rides `clock` (`false` = the capability-less env);
/// every read is recorded.
#[derive(Default)]
pub struct MemIo {
    pub files: HashMap<String, Vec<u8>>,
    pub dirs: HashSet<String>,
    pub clock: bool,
    pub reads: RefCell<Vec<String>>,
    pub arms: RefCell<Vec<u32>>,
    pub wake: RefCell<Option<WakeFn>>,
    pub notes: RefCell<Vec<String>>,
}

impl MemIo {
    /// lay one file (and every ancestor dir) into the world
    pub fn put(&mut self, path: &str, text: &str) {
        let mut dir = path;
        while let Some(i) = dir.rfind('/') {
            dir = &dir[..i];
            if dir.is_empty() {
                break;
            }
            self.dirs.insert(dir.to_string());
        }
        self.files
            .insert(path.to_string(), text.as_bytes().to_vec());
    }

    /// fire the stored wake source by hand — the test IS the timer
    pub fn fire(&self) {
        let wake = self
            .wake
            .borrow()
            .clone()
            .expect("the wake source installed at set_env");
        wake();
    }

    /// which manifests the walk read, in order
    pub fn manifests(&self) -> Vec<String> {
        self.reads
            .borrow()
            .iter()
            .filter(|p| p.ends_with("/rut.jsonc"))
            .cloned()
            .collect()
    }

    /// one root's world: `rut.jsonc` + `src/mod.rut`
    pub fn root(&mut self, path: &str, name: &str) {
        self.put(
            &format!("{path}/rut.jsonc"),
            &format!("{{\n  \"name\": \"{name}\"\n}}\n"),
        );
        self.put(
            &format!("{path}/src/mod.rut"),
            "pub fn tag() -> i32 { return 1; }\n",
        );
    }
}

impl Io for MemIo {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.reads.borrow_mut().push(path.to_string());
        self.files.get(path).cloned()
    }

    fn read_dir(&self, _path: &str) -> Option<Vec<String>> {
        None // the file-map world never asks this spelling
    }

    fn is_dir(&self, path: &str) -> bool {
        self.dirs.contains(path)
    }

    fn notify(&self, message: &str, _is_error: bool) {
        self.notes.borrow_mut().push(message.to_string());
    }

    fn set_wake(&self, wake: WakeFn) {
        *self.wake.borrow_mut() = Some(wake);
    }

    fn arm_wake(&self, ms: u32) -> bool {
        if !self.clock {
            return false;
        }
        self.arms.borrow_mut().push(ms);
        true
    }
}
