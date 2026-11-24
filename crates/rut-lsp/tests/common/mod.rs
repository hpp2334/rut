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

use std::cell::{Cell, RefCell};
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

// ---- the URI shapes — the ONE flag every named-mount pin runs under ----

/// Where a package's files sit: under a project dir NAMED after the
/// package (`…/app/src/…` — `rut new`'s layout, the NORMATIVE shape:
/// the package name appears in every one of its documents' uris), or
/// under a dir the name never appears in (`…/p1/src/…`). A hand-built
/// world whose uris never name the package MASKS every matcher defect
/// that lands a package's root by a path coincidence — so the pins run
/// in both shapes ([`both_uri_shapes`]): one body, two tests.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    /// the project dir names the package (`file:///ws/app/src/…`)
    DirNamesThePackage,
    /// the project dir hides it (`file:///ws/p1/src/…`)
    DirHidesThePackage,
}

thread_local! {
    static SHAPE: Cell<Shape> = const { Cell::new(Shape::DirNamesThePackage) };
    /// the renamed package dirs of one run — `(the pin's prefix, the
    /// shaped prefix)`, in first-use order
    static RENAMED: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
}

/// run one pin body under `shape`: every uri the harness doors receive
/// is spelled the shape's way, every uri an answer carries comes back
/// in the pin's own spelling — the body reads the same under both
pub fn run_under(shape: Shape, body: impl FnOnce()) {
    SHAPE.with(|s| s.set(shape));
    RENAMED.with(|r| r.borrow_mut().clear());
    body();
    SHAPE.with(|s| s.set(Shape::DirNamesThePackage));
    RENAMED.with(|r| r.borrow_mut().clear());
}

/// the world roots a pin's package dirs sit under
const URI_ROOTS: [&str; 2] = ["file:///ws/", "file:///deps/"];

/// the uri as the running shape spells it: the package dir (the first
/// segment under a world root) renames to a neutral `pN` when the
/// shape hides the package — one dir always renames the same way
/// within a run, so a mount and its queries stay one file
pub fn spelled(uri: &str) -> String {
    if SHAPE.with(|s| s.get()) == Shape::DirNamesThePackage {
        return uri.to_string();
    }
    let Some(root) = URI_ROOTS.iter().find(|r| uri.starts_with(**r)) else {
        return uri.to_string();
    };
    let rest = &uri[root.len()..];
    let Some((dir, tail)) = rest.split_once('/') else {
        return uri.to_string();
    };
    if dir.is_empty() {
        return uri.to_string();
    }
    let pin_prefix = format!("{root}{dir}/");
    let shaped_prefix = RENAMED.with(|r| {
        let mut renamed = r.borrow_mut();
        if let Some((_, shaped)) = renamed.iter().find(|(pin, _)| *pin == pin_prefix) {
            return shaped.clone();
        }
        let fresh = format!("{root}p{}/", renamed.len() + 1);
        renamed.push((pin_prefix, fresh.clone()));
        fresh
    });
    format!("{shaped_prefix}{tail}")
}

/// an answer's uris back to the pin's spelling — every string an
/// envelope carries (location uris, a hover's provenance line)
fn unshaped(value: Value) -> Value {
    let renamed = RENAMED.with(|r| r.borrow().clone());
    match value {
        _ if renamed.is_empty() => value,
        Value::String(text) => Value::String(renamed.iter().fold(text, |text, (pin, shaped)| {
            // both spellings of the dir: a uri's and a rooted path's
            // (the dep walk's plain-path origins)
            let plain = |p: &str| p.trim_start_matches("file://").to_string();
            text.replace(shaped.as_str(), pin.as_str())
                .replace(plain(shaped).as_str(), plain(pin).as_str())
        })),
        Value::Array(items) => Value::Array(items.into_iter().map(unshaped).collect()),
        Value::Object(map) => {
            Value::Object(map.into_iter().map(|(k, v)| (k, unshaped(v))).collect())
        }
        other => other,
    }
}

/// Run one pin in BOTH URI shapes — `both_uri_shapes! { #[test] fn
/// name() { ..body.. } }` compiles the body twice: once with the
/// project dir naming the package (the normative layout), once with a
/// dir the name never appears in. The body reads the same under both:
/// the harness doors spell its uris the running shape's way and bring
/// answer uris back.
#[macro_export]
macro_rules! both_uri_shapes {
    (#[test] fn $name:ident() $body:block) => {
        mod $name {
            use super::*;

            #[test]
            fn the_project_dir_names_the_package() {
                $crate::common::run_under($crate::common::Shape::DirNamesThePackage, || $body);
            }

            #[test]
            fn the_project_dir_hides_the_package() {
                $crate::common::run_under($crate::common::Shape::DirHidesThePackage, || $body);
            }
        }
    };
}

// ---- the JSON helpers ----

/// parse one envelope's answer — its uris spelled back in the pin's
/// own way (no-op outside a shape-hiding run)
pub fn take(json: String) -> Value {
    unshaped(serde_json::from_str(&json).expect("the envelope is valid JSON"))
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
                .run_until(self.lsp.analyze(spelled(uri), src.to_string())),
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
        take(self.pool.run_until(self.lsp.hover(spelled(uri), l, c)))
    }

    /// hover the stored doc at a raw position
    pub fn hover_pos(&mut self, uri: &str, line: u32, ch: u32) -> Value {
        take(self.pool.run_until(self.lsp.hover(spelled(uri), line, ch)))
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
            self.pool
                .run_until(self.lsp.complete(spelled(uri), src.to_string(), line, ch)),
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
                .run_until(self.lsp.definition(spelled(uri), line, ch)),
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
                .run_until(self.lsp.type_definition(spelled(uri), l, c)),
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
                .run_until(self.lsp.type_definition(spelled(uri), l, c + off)),
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
            self.pool.run_until(
                self.lsp
                    .references(spelled(uri), l, c, u32::from(include_decl)),
            ),
        )
        .as_array()
        .expect("the references answer is an array")
        .clone()
    }

    /// signature help at the `occ`-th needle — the LSP value or null
    /// (the CURRENT text rides every call — the doc-sync door)
    pub fn sig_at(&mut self, uri: &str, src: &str, needle: &str, occ: usize) -> Value {
        self.analyze(uri, src);
        let (l, c) = pos_of(src, needle, occ);
        take(
            self.pool
                .run_until(self.lsp.signature_help(spelled(uri), src.to_string(), l, c)),
        )
    }

    /// signature help at a raw position over `src`
    pub fn sig_pos(&mut self, uri: &str, src: &str, pos: (u32, u32)) -> Value {
        take(
            self.pool.run_until(self.lsp.signature_help(
                spelled(uri),
                src.to_string(),
                pos.0,
                pos.1,
            )),
        )
    }

    /// references at a raw position over the stored doc
    pub fn refs_pos(&mut self, uri: &str, pos: (u32, u32), include_decl: bool) -> Vec<Value> {
        take(
            self.pool.run_until(self.lsp.references(
                spelled(uri),
                pos.0,
                pos.1,
                u32::from(include_decl),
            )),
        )
        .as_array()
        .expect("the references answer is an array")
        .clone()
    }

    /// signature help at `off` past the needle's start (mid-arg)
    pub fn sig_in(&mut self, uri: &str, src: &str, needle: &str, occ: usize, off: u32) -> Value {
        self.analyze(uri, src);
        let (l, c) = pos_of(src, needle, occ);
        take(
            self.pool.run_until(
                self.lsp
                    .signature_help(spelled(uri), src.to_string(), l, c + off),
            ),
        )
    }

    /// inlay hints over the whole stored document
    pub fn inlay_all(&mut self, uri: &str, src: &str) -> Vec<Value> {
        self.analyze(uri, src);
        let lines = src.matches('\n').count() as u32;
        take(
            self.pool
                .run_until(self.lsp.inlay(spelled(uri), 0, 0, lines, 0)),
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
                .run_until(self.lsp.inlay(spelled(uri), l, c, lines, 0)),
        )
        .as_array()
        .expect("the inlay answer is an array")
        .clone()
    }

    // ---- the workspace-scan push + the deps face ----

    /// index one workspace file under its open-doc stamp
    pub fn index_file(&mut self, uri: &str, src: &str) {
        self.pool
            .run_until(self.lsp.index_file(spelled(uri), src.to_string()));
    }

    /// index one dep source file at a MOD PATH — the envelope answers
    /// the file's own `mod` declarations (the mount walk's next hop)
    pub fn index_mod_at(&mut self, uri: &str, pkg: &str, mod_path: &str, src: &str) -> Value {
        take(self.pool.run_until(self.lsp.index_mod_at(
            spelled(uri),
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
        self.pool.run_until(self.lsp.forget(spelled(uri)));
    }
}

// ---- the rut-new world — the extension's own mounts, through the
//      public doors ----

/// `rut new`'s layout, as the extension mounts it. ONE fixture every
/// round-4 pin rides: the `app` package on disk as `rut new`
/// scaffolds it (a project dir named after the package; the root
/// `src/mod.rut` declaring `mod child;` and `mod m;`; `child`'s own
/// `mod sub;`; `m`'s nested `grid`), the path dep `calc` beside it
/// (its own `math` module with a nested `grid`), and BOTH lanes that
/// mount a workspace file in production — the workspace indexer's
/// `file://` stamps, then the crate's own dep walk over the same disk
/// (plain-path origins). The uris spell the NORMATIVE shape (the
/// package names its project dir); under [`both_uri_shapes`] the
/// harness renames the dirs for the hiding run.
pub mod rut_new {
    use super::*;

    /// the app package's file uris in the PIN's spelling —
    /// `at("src/mod.rut")` is its root. The harness doors spell a uri
    /// the running shape's way on the way in and bring answer uris
    /// back to this spelling, so a pin reads the same under both
    /// shapes.
    pub fn at(rel: &str) -> String {
        format!("file:///ws/app/{rel}")
    }

    /// the app dir ON DISK — the shape's spelling (the walk mounts
    /// from it; its plain-path origins key to the same files as the
    /// indexer's shaped stamps, so both lanes still land one entry
    /// per file)
    fn dir() -> String {
        spelled("file:///ws/app/rut.jsonc")
            .trim_start_matches("file://")
            .trim_end_matches("/rut.jsonc")
            .to_string()
    }

    /// the dep's files as the WALK stamps their origins — rooted
    /// paths, the path row's joined `..` carried (the walk's own law):
    /// a hover's provenance line reads `— /ws/app/../calc/…`. Spelled
    /// in the PIN's way (the harness brings answer uris back to it,
    /// whatever dir the hiding run put on disk)
    pub fn calc(rel: &str) -> String {
        format!("/ws/app/../calc/{rel}")
    }

    /// the same file as a jump target — the definition layer's uri
    /// for a rooted-path origin (`file://` + the path)
    pub fn calc_uri(rel: &str) -> String {
        format!("file://{}", calc(rel))
    }

    pub const ROOT: &str = "mod child;\nmod m;\n\npub let LIMIT: i32 = 7;\nlet HIDDEN: i32 = 3;\n\npub fn rootfn() -> i32 { return 1; }\nfn rootpriv() -> i32 { return 2; }\n\npub struct RootTy {\n    a: i32,\n}\n\nentry fn main() -> nil {\n    let x = m::mk(41).bump();\n}\n";

    /// `child`'s on-disk head (a pin's open document replaces it —
    /// the twin may lag an open edit, exactly as in the editor)
    pub const CHILD_HEAD: &str = "mod sub;\n\npub fn kid() -> i32 { return 3; }\n";

    /// the leaf module `m`: the leaf kinds the lanes gate — a fn, a
    /// type (+ its impl), a module let, a `pub(pkg)` fn, a private
    /// fn, a nested `grid`, and the use rows (a private import, a
    /// `pub use` re-export, a glob)
    pub const M: &str = "pub mod grid;\n\nuse grid::{ cell };\npub use grid::{ G };\nuse grid::*;\n\npub struct Counter {\n    n: i32,\n}\nimpl Counter {\n    pub fn bump(self) -> i32 { return self.n + 1; }\n}\npub fn mk(n: i32) -> Counter { return Counter { n: n }; }\nfn secret() -> i32 { return 5; }\npub(pkg) fn pkgfn() -> i32 { return 6; }\npub let STEP: i32 = 2;\npub struct Box2<T> {\n    v: T,\n}\nimpl<T> Box2<T> {\n    pub fn get(self) -> T { return self.v; }\n}\npub fn mk_box<T>(v: T) -> Box2<T> { return Box2 { v: v }; }\n";

    /// `m`'s nested module
    pub const GRID: &str =
        "pub fn cell() -> i32 { return 9; }\npub let G: i32 = 1;\npub let GK: i32 = 1;\n";

    /// `child`'s own child
    pub const SUB: &str = "pub fn deep() -> i32 { return 4; }\n";

    /// the dep's root: a module tree (`pub mod math;`) and a root fn
    /// of its own
    pub const CALC_ROOT: &str = "pub mod math;\npub fn ctop() -> i32 { return 10; }\n";

    /// the dep's `math` module — `m`'s shape under its own names
    pub const CALC_MATH: &str = "pub mod grid;\n\nuse grid::{ cell };\npub use grid::{ G };\nuse grid::*;\n\npub struct Calc {\n    d: i32,\n}\nimpl Calc {\n    pub fn deg(self) -> i32 { return self.d; }\n}\npub fn cmk(d: i32) -> Calc { return Calc { d: d }; }\nfn hidden() -> i32 { return 0; }\npub(pkg) fn cpkg() -> i32 { return 6; }\npub let CPI: f64 = 3.14;\n";

    pub const CALC_GRID: &str =
        "pub fn cell() -> i32 { return 9; }\npub let G: i32 = 1;\npub let GK: i32 = 1;\n";

    /// the app package's files: (the uri-relative path, its mod path,
    /// its text) — what both lanes mount
    pub const APP_FILES: [(&str, &str, &str); 5] = [
        ("src/mod.rut", "", ROOT),
        ("src/child/mod.rut", "child", CHILD_HEAD),
        ("src/child/sub/mod.rut", "child/sub", SUB),
        ("src/m/mod.rut", "m", M),
        ("src/m/grid/mod.rut", "m/grid", GRID),
    ];

    /// the dep's files on disk (the walk mounts them — plain-path
    /// origins, the `..` row carried)
    pub const CALC_FILES: [(&str, &str); 3] = [
        ("src/mod.rut", CALC_ROOT),
        ("src/math/mod.rut", CALC_MATH),
        ("src/math/grid/mod.rut", CALC_GRID),
    ];

    /// boot the extension's world: the in-memory disk holds the
    /// package and its dep; the workspace indexer's `file://` stamps
    /// land first (every `mod.rut` under the owner's `src/`, by its
    /// declared mod path), then the crate's dep walk runs over the
    /// same disk from the project dir (the roots the extension hands
    /// it). Both lanes, one entry per file.
    pub fn boot() -> (S, Rc<MemIo>) {
        let mut io = MemIo::default();
        let app_dir = dir();
        let calc_dir = lexical(&format!("{app_dir}/../calc"));
        io.put(
            &format!("{app_dir}/rut.jsonc"),
            "{\n  \"name\": \"app\",\n  \"deps\": {\n    \"calc\": { \"path\": \"../calc\" }\n  }\n}\n",
        );
        io.put(
            &format!("{calc_dir}/rut.jsonc"),
            "{\n  \"name\": \"calc\"\n}\n",
        );
        for (rel, _, text) in APP_FILES {
            io.put(&format!("{app_dir}/{rel}"), text);
        }
        for (rel, text) in CALC_FILES {
            io.put(&format!("{calc_dir}/{rel}"), text);
        }
        let io = Rc::new(io);
        let mut s = S::with_io(&io);
        // the workspace indexer's stamps
        for (rel, mod_path, text) in APP_FILES {
            s.index_mod_at(&at(rel), "app", mod_path, text);
        }
        // then the dep walk
        let root = dir();
        s.resolve(&[&root]);
        (s, io)
    }
}

// ---- the in-memory host env ----

/// the walk's whole world as a file map + dir set: a root is its
/// `rut.jsonc` + sources, so which manifest got read names which walk
/// ran — and the mod-name slot's disk probe lists through the same
/// world. The clock rides `clock` (`false` = the capability-less env);
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

/// a rooted path with `.` dropped and `..` collapsed segment by
/// segment — the host-normalizes law the walk's joined path rows
/// ride (`/ws/app/../calc` is `/ws/calc`)
pub fn lexical(path: &str) -> String {
    // only a rooted path normalizes — a `file://` uri (the mod-name
    // slot's probe spells the doc's directory that way) is a key as-is
    if !path.starts_with('/') {
        return path.to_string();
    }
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    format!("/{}", out.join("/"))
}

impl Io for MemIo {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.reads.borrow_mut().push(path.to_string());
        self.files.get(&lexical(path)).cloned()
    }

    fn read_dir(&self, path: &str) -> Option<Vec<String>> {
        // a real listing over the key map AND the recorded dirs: the
        // first segment under `path/`, sorted, deduped; `None` when
        // empty (the mod-name slot's probe lists through this too)
        let prefix = format!("{}/", lexical(path));
        let mut names: Vec<String> = Vec::new();
        for k in self.files.keys().chain(self.dirs.iter()) {
            let Some(rest) = k.strip_prefix(&prefix) else {
                continue;
            };
            if rest.is_empty() {
                continue;
            }
            let first = rest.split('/').next().unwrap_or(rest);
            if !names.iter().any(|n| n == first) {
                names.push(first.to_string());
            }
        }
        if names.is_empty() {
            return None;
        }
        names.sort();
        Some(names)
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
