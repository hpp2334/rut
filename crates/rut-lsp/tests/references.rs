use rut_lexer::lexer::{lex, normalize};
use rut_lexer::token::Token;
use rut_lsp::definition;
use rut_lsp::definition::DefLocation;
use rut_lsp::hover::DefIndex;
use rut_lsp::line_index::LineIndex;
use rut_lsp::references::*;
use rut_parser::{parse, Mode};

const DOC: &str = "file:///t/main.rut";

struct Doc {
    src: String,
    toks: Vec<Token>,
    ast: rut_ast::ast::Ast,
    index: DefIndex,
}

fn doc_at(uri: &str, src: &str) -> Doc {
    let src = normalize(src);
    let (toks, _) = lex(&src);
    let (ast, _) = parse(&src, Mode::Impl);
    let mut index = rut_lsp::hover::index(&src, &ast, &toks);
    index.origin = uri.to_string();
    Doc {
        src,
        toks,
        ast,
        index,
    }
}

fn doc(src: &str) -> Doc {
    doc_at(DOC, src)
}

/// byte offset of the `n`th (1-based) occurrence of `needle`
fn at(src: &str, needle: &str, occurrence: usize) -> u32 {
    let mut from = 0;
    for _ in 0..occurrence {
        match src[from..].find(needle) {
            Some(i) => from += i + needle.len(),
            None => panic!("needle {needle:?} not found (occurrence {occurrence})"),
        }
    }
    (from - needle.len()) as u32
}

/// every reference's span start over `src` (locations whose uri is
/// not the doc are listed separately by the cross-file tests)
fn ref_offsets(src: &str, uri: &str, locs: &[DefLocation]) -> Vec<u32> {
    let ix = LineIndex::new(src);
    locs.iter()
        .filter(|l| l.uri == uri)
        .map(|l| ix.byte(src, l.range.start.line, l.range.start.character))
        .collect()
}

fn refs(d: &Doc, pos: u32, extra: &[DefIndex], include_decl: bool) -> Vec<DefLocation> {
    let idxs: Vec<&DefIndex> = std::iter::once(&d.index).chain(extra.iter()).collect();
    let dm = rut_lsp::mods::DocMods::default();
    let ctx = definition::Ctx {
        doc_uri: DOC,
        toks: &d.toks,
        ast: &d.ast,
        idxs: &idxs,
        doc_mods: &dm,
    };
    references(&ctx, pos, include_decl)
}

/// the query at byte `pos` references EXACTLY the idents at `want`
/// (byte offsets of the reference idents, order-insensitive)
fn assert_refs(d: &Doc, pos: u32, want: &[u32], extra: &[DefIndex]) {
    let hits = refs(d, pos, extra, false);
    let mut got = ref_offsets(&d.src, DOC, &hits);
    got.sort();
    got.dedup();
    let mut want_sorted = want.to_vec();
    want_sorted.sort();
    assert_eq!(got, want_sorted, "references at byte {pos}: {hits:?}");
}

// ---- the binding pass, reversed: shadow-aware ----

#[test]
fn local_binding_refs_are_the_shadow_correct_set() {
    let src = [
        "fn go() -> i32 {",
        "    let v = 1;",
        "    if (v > 0) {",
        "        let v = 2;",
        "        return v;",
        "    }",
        "    return v;",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    // the OUTER v: its use in the if-condition + the final return —
    // the inner shadow's decl and its return do NOT leak in
    assert_refs(
        &d,
        at(&d.src, "v = 1;", 1),
        &[at(&d.src, "v > 0", 1), at(&d.src, "return v;", 2) + 7],
        &[],
    );
    // the INNER v: exactly its own return — the outer's uses don't leak in
    assert_refs(
        &d,
        at(&d.src, "v = 2;", 1),
        &[at(&d.src, "return v;", 1) + 7],
        &[],
    );
    // a use site resolves to its own binding (the same set as its decl)
    assert_refs(
        &d,
        at(&d.src, "return v;", 2) + 7,
        &[at(&d.src, "v > 0", 1), at(&d.src, "return v;", 2) + 7],
        &[],
    );
}

#[test]
fn include_declaration_appends_the_declaring_ident() {
    let src = "fn go() -> i32 {\n    let c = 1;\n    return c;\n}\n";
    let d = doc(src);
    let pos = at(&d.src, "let c", 1) + 4;
    let hits = refs(&d, pos, &[], false);
    assert_eq!(
        ref_offsets(&d.src, DOC, &hits),
        vec![at(&d.src, "return c;", 1) + 7],
        "use only without the toggle"
    );
    let hits = refs(&d, pos, &[], true);
    let mut offs = ref_offsets(&d.src, DOC, &hits);
    offs.sort();
    assert_eq!(
        offs,
        vec![at(&d.src, "let c", 1) + 4, at(&d.src, "return c;", 1) + 7],
        "the decl ident joins with include_declaration"
    );
}

#[test]
fn param_refs_follow_the_landed_shadow_rule() {
    let src = [
        "fn go(k: i32) -> i32 {",
        "    let k = k + 1;",
        "    return k;",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    // the PARAM k: nothing — even the initializer's `k` reads the
    // shadowing local under the landed latest-decl rule (references
    // MUST match what definition answers there — the law of this
    // module — not the compiler's initialize-before-bind reading)
    assert_refs(&d, at(&d.src, "k: i32", 1), &[], &[]);
    // the LOCAL k: the initializer's k + the return
    assert_refs(
        &d,
        at(&d.src, "let k", 1) + 4,
        &[at(&d.src, "k + 1", 1), at(&d.src, "return k;", 1) + 7],
        &[],
    );
}

#[test]
fn scope_exit_never_leaks_a_use() {
    let src = [
        "fn go() -> i32 {",
        "    if (true) {",
        "        let w = 1;",
        "        return w;",
        "    }",
        "    return 0;",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    assert_refs(
        &d,
        at(&d.src, "w = 1;", 1),
        &[at(&d.src, "return w;", 1) + 7],
        &[],
    );
}

// ---- the decl layer, reversed ----

#[test]
fn fn_refs_cover_calls_and_the_decl_toggle() {
    let src = [
        "fn make() -> i32 {",
        "    return 1;",
        "}",
        "entry fn main() -> i32 {",
        "    let a = make();",
        "    let b = make();",
        "    return a + b;",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    let call1 = at(&d.src, "= make();", 1) + 2;
    let call2 = at(&d.src, "= make();", 2) + 2;
    // from the decl: both calls (the decl itself stays out)
    assert_refs(&d, at(&d.src, "fn make()", 1) + 3, &[call1, call2], &[]);
    // include_declaration puts the declaring ident in
    let hits = refs(&d, at(&d.src, "fn make()", 1) + 3, &[], true);
    assert_eq!(hits.len(), 3, "{hits:?}");
    // from a call site: the same set (references work from any occurrence)
    assert_refs(&d, call2, &[call1, call2], &[]);
}

#[test]
fn type_refs_cover_type_positions_and_ctor_paths() {
    let src = [
        "class Circle {",
        "    r: f64;",
        "}",
        "impl Circle {",
        "    fn area(self) -> f64 { return 3.14; }",
        "}",
        "fn go(c: Circle) -> f64 {",
        "    let w = Circle::new(1.0);",
        "    return c.area() + w.r;",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    // the decl of `Circle` → the param annotation, the impl head,
    // the ctor path (NOT the field read `w.r` / members)
    assert_refs(
        &d,
        at(&d.src, "class Circle", 1) + 6,
        &[
            at(&d.src, "c: Circle", 1) + 3,
            at(&d.src, "impl Circle", 1) + 5,
            at(&d.src, "= Circle::new", 1) + 2,
        ],
        &[],
    );
}

#[test]
fn method_refs_need_the_receiver_type() {
    let src = [
        "class Circle {",
        "    r: f64;",
        "}",
        "struct Box2 {",
        "    s: f64;",
        "}",
        "impl Circle {",
        "    fn area(self) -> f64 { return 3.14; }",
        "}",
        "impl Box2 {",
        "    fn area(self) -> f64 { return 1.0; }",
        "}",
        "fn go(c: Circle, b: Box2) -> f64 {",
        "    let a = c.area();",
        "    let z = b.area();",
        "    return a + z;",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    // references on Circle's `area` decl: the `c.area()` call, whose
    // receiver types to Circle — but NOT `b.area()` (a same-named
    // method on a different receiver type)
    assert_refs(
        &d,
        at(&d.src, "fn area(self) -> f64 { return 3.14;", 1) + 3,
        &[at(&d.src, "c.area();", 1) + 2],
        &[],
    );
    // and Box2's own `area` answers with its own call
    assert_refs(
        &d,
        at(&d.src, "fn area(self) -> f64 { return 1.0;", 1) + 3,
        &[at(&d.src, "b.area();", 1) + 2],
        &[],
    );
}

#[test]
fn field_refs_ride_the_receiver_inference() {
    let src = [
        "class Circle {",
        "    r: f64;",
        "}",
        "fn go(c: Circle, k: f64) -> f64 {",
        "    c.r = k;",
        "    return c.r;",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    // the field decl's references: both dotted reads
    assert_refs(
        &d,
        at(&d.src, "r: f64;", 1),
        &[
            at(&d.src, "c.r = k;", 1) + 2,
            at(&d.src, "return c.r;", 1) + 9,
        ],
        &[],
    );
}

#[test]
fn module_let_refs_skip_local_shadows() {
    let src = [
        "let TOTAL = 41;",
        "fn bump() -> i32 {",
        "    return TOTAL + 1;",
        "}",
        "fn shadow() -> i32 {",
        "    let TOTAL = 5;",
        "    return TOTAL;",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    // the module let: bump's return only — the shadow's decl and its
    // return belong to the local (and under the landed latest-decl
    // rule, so does an initializer reading: `let TOTAL = TOTAL` is
    // the local's, definition-consistent)
    assert_refs(
        &d,
        at(&d.src, "let TOTAL = 41;", 1) + 4,
        &[at(&d.src, "return TOTAL + 1;", 1) + 7],
        &[],
    );
    // the LOCAL TOTAL: its own return only
    assert_refs(
        &d,
        at(&d.src, "let TOTAL = 5;", 1) + 8,
        &[at(&d.src, "return TOTAL;", 1) + 7],
        &[],
    );
}

// ---- cross-file: the use graph's reverse edges ----

const WIDGET_URI: &str = "file:///ws/gadgets/lib.rut";

fn widget_lib() -> Doc {
    doc_at(WIDGET_URI, "class Widget {\n    id: i32;\n}\n")
}

fn widget_user() -> (Doc, String) {
    let src = "use gadgets::{ Widget };\nfn main() -> nil {\n    let w = Widget::new();\n}\n";
    (doc(src), normalize(src))
}

#[test]
fn cross_file_refs_find_the_importing_files() {
    let lib = widget_lib();
    let (main, norm) = widget_user();
    let extra = [&lib.index];
    // references on the LIB's type — queried from the importing doc:
    // the use-statement name + the usage site, nothing else
    let idxs: Vec<&DefIndex> = std::iter::once(&main.index)
        .chain(extra.iter().copied())
        .collect();
    let dm = rut_lsp::mods::DocMods::default();
    let ctx = definition::Ctx {
        doc_uri: DOC,
        toks: &main.toks,
        ast: &main.ast,
        idxs: &idxs,
        doc_mods: &dm,
    };
    let hits = references(&ctx, at(&norm, "Widget::new", 1), false);
    let mut offs = ref_offsets(&norm, DOC, &hits);
    offs.sort();
    assert_eq!(
        offs,
        vec![at(&norm, "Widget }", 1), at(&norm, "Widget::new", 1)],
        "{hits:?}"
    );
}

#[test]
fn refs_from_the_decl_side_reach_the_importing_doc() {
    let lib = widget_lib();
    let (main, norm) = widget_user();
    let extra = [&main.index];
    // query from the LIB's decl ident, doc = the lib: the scan's
    // reverse edge finds the importing doc's two sites — the only
    // two entries, both carrying the DOC's uri
    let idxs: Vec<&DefIndex> = std::iter::once(&lib.index)
        .chain(extra.iter().copied())
        .collect();
    let dm = rut_lsp::mods::DocMods::default();
    let ctx = definition::Ctx {
        doc_uri: WIDGET_URI,
        toks: &lib.toks,
        ast: &lib.ast,
        idxs: &idxs,
        doc_mods: &dm,
    };
    let hits = references(&ctx, at(&lib.src, "class Widget", 1) + 6, false);
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(hits.iter().all(|l| l.uri == DOC), "{hits:?}");
    let mut offs = ref_offsets(&norm, DOC, &hits);
    offs.sort();
    assert_eq!(
        offs,
        vec![at(&norm, "Widget }", 1), at(&norm, "Widget::new", 1)]
    );
}

#[test]
fn an_unrelated_same_named_type_does_not_leak_in() {
    let lib = widget_lib();
    let foreign = doc_at(
        "file:///ws/other/lib.rut",
        "class Widget {\n    other: i32;\n}\n",
    );
    let (main, norm) = widget_user();
    let extra = [&lib.index, &foreign.index];
    // references on the gadgets Widget (via the importing doc's use
    // site): the other pkg's same-named type must contribute nothing
    let idxs: Vec<&DefIndex> = std::iter::once(&main.index)
        .chain(extra.iter().copied())
        .collect();
    let dm = rut_lsp::mods::DocMods::default();
    let ctx = definition::Ctx {
        doc_uri: DOC,
        toks: &main.toks,
        ast: &main.ast,
        idxs: &idxs,
        doc_mods: &dm,
    };
    let hits = references(&ctx, at(&norm, "Widget::new", 1), false);
    assert_eq!(hits.len(), 2, "use name + usage only: {hits:?}");
}

// ---- misses ----

#[test]
fn misses_stay_empty() {
    let d = doc("entry fn main() -> nil {\n}\n");
    assert!(
        refs(&d, 0, &[], true).is_empty(),
        "keywords have no references"
    );
    let d2 = doc("entry fn main() -> nil {\n    let x = mystery();\n}\n");
    assert!(
        refs(&d2, at(&d2.src, "mystery", 1), &[], true).is_empty(),
        "unknown names too"
    );
}
