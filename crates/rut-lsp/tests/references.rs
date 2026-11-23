//! References' gate — the definition index read BACKWARDS through the
//! `references` envelope: the shadow-aware binding pass, the decl
//! layer, the receiver-typed members, the includeDeclaration toggle,
//! and the use graph's cross-file reverse edges.

mod common;

use common::*;
use serde_json::Value;

const WIDGET_SRC: &str = "class Widget {\n    id: i32;\n}\n";
const WIDGET_URI: &str = "file:///ws/gadgets/lib.rut";

/// byte starts of the reference Locations that point into `src`'s uri
fn ref_offsets(src: &str, uri: &str, hits: &[Value]) -> Vec<u32> {
    let mut out: Vec<u32> = hits
        .iter()
        .filter(|l| l["uri"] == uri)
        .map(|l| span_of(src, l).0 as u32)
        .collect();
    out.sort();
    out.dedup();
    out
}

/// references at `off` past the `occ`-th needle must be EXACTLY the
/// idents at `want` (byte starts over `src`, order-insensitive)
fn assert_refs(s: &mut S, uri: &str, src: &str, needle: &str, occ: usize, off: u32, want: &[u32]) {
    let hits = s.refs_in(uri, src, needle, occ, off, false);
    let got = ref_offsets(src, uri, &hits);
    let mut want_sorted = want.to_vec();
    want_sorted.sort();
    assert_eq!(
        got, want_sorted,
        "references at `{needle}` #{occ}: {hits:?}"
    );
}

#[test]
fn local_binding_refs_are_the_shadow_correct_set() {
    let mut s = S::boot();
    let src = "fn go() -> i32 {\n    let v = 1;\n    if (v > 0) {\n        let v = 2;\n        return v;\n    }\n    return v;\n}\n";
    let uri = "file:///t/main.rut";
    // the OUTER v: its use in the if-condition + the final return —
    // the inner shadow's decl and its return do NOT leak in
    assert_refs(
        &mut s,
        uri,
        src,
        "v = 1;",
        1,
        0,
        &[byte_of(src, "v > 0", 1), byte_of(src, "return v;", 2) + 7],
    );
    // the INNER v: exactly its own return — the outer's uses don't leak in
    assert_refs(
        &mut s,
        uri,
        src,
        "v = 2;",
        1,
        0,
        &[byte_of(src, "return v;", 1) + 7],
    );
    // a use site resolves to its own binding (the same set as its decl)
    assert_refs(
        &mut s,
        uri,
        src,
        "return v;", // occurrence 2 — the final return
        2,
        7,
        &[byte_of(src, "v > 0", 1), byte_of(src, "return v;", 2) + 7],
    );
}

#[test]
fn include_declaration_appends_the_declaring_ident() {
    let mut s = S::boot();
    let src = "fn go() -> i32 {\n    let c = 1;\n    return c;\n}\n";
    let uri = "file:///t/main.rut";
    let hits = s.refs_in(uri, src, "let c", 1, 4, false);
    assert_eq!(
        ref_offsets(src, uri, &hits),
        vec![byte_of(src, "return c;", 1) + 7],
        "use only without the toggle"
    );
    let hits = s.refs_in(uri, src, "let c", 1, 4, true);
    let mut offs = ref_offsets(src, uri, &hits);
    offs.sort();
    assert_eq!(
        offs,
        vec![
            byte_of(src, "let c", 1) + 4,
            byte_of(src, "return c;", 1) + 7
        ],
        "the decl ident joins with include_declaration"
    );
}

#[test]
fn param_refs_follow_the_landed_shadow_rule() {
    let mut s = S::boot();
    let src = "fn go(k: i32) -> i32 {\n    let k = k + 1;\n    return k;\n}\n";
    let uri = "file:///t/main.rut";
    // the PARAM k: nothing — even the initializer's `k` reads the
    // shadowing local (references MUST match what definition answers)
    assert_refs(&mut s, uri, src, "k: i32", 1, 0, &[]);
    // the LOCAL k: the initializer's k + the return
    assert_refs(
        &mut s,
        uri,
        src,
        "let k",
        1,
        4,
        &[byte_of(src, "k + 1", 1), byte_of(src, "return k;", 1) + 7],
    );
}

#[test]
fn scope_exit_never_leaks_a_use() {
    let mut s = S::boot();
    let src = "fn go() -> i32 {\n    if (true) {\n        let w = 1;\n        return w;\n    }\n    return 0;\n}\n";
    assert_refs(
        &mut s,
        "file:///t/main.rut",
        src,
        "w = 1;",
        1,
        0,
        &[byte_of(src, "return w;", 1) + 7],
    );
}

// ---- the decl layer, reversed ----

#[test]
fn fn_refs_cover_calls_and_the_decl_toggle() {
    let mut s = S::boot();
    let src = "fn make() -> i32 {\n    return 1;\n}\nentry fn main() -> i32 {\n    let a = make();\n    let b = make();\n    return a + b;\n}\n";
    let uri = "file:///t/main.rut";
    let call1 = byte_of(src, "= make();", 1) + 2;
    let call2 = byte_of(src, "= make();", 2) + 2;
    // from the decl: both calls (the decl itself stays out) — the
    // click lands on the declared ident
    let hits = s.refs_in(uri, src, "fn make()", 1, 3, false);
    assert_eq!(ref_offsets(src, uri, &hits), vec![call1, call2], "{hits:?}");
    // include_declaration puts the declaring ident in
    let hits = s.refs_in(uri, src, "fn make()", 1, 3, true);
    assert_eq!(hits.len(), 3, "{hits:?}");
    // from a call site: the same set (references work from any occurrence)
    let hits = s.refs_pos(uri, S::pos_at_byte(src, call2), false);
    assert_eq!(ref_offsets(src, uri, &hits), vec![call1, call2], "{hits:?}");
}

#[test]
fn type_refs_cover_type_positions_and_ctor_paths() {
    let mut s = S::boot();
    let src = "class Circle {\n    r: f64;\n}\nimpl Circle {\n    fn area(self) -> f64 { return 3.14; }\n}\nfn go(c: Circle) -> f64 {\n    let w = Circle::new(1.0);\n    return c.area() + w.r;\n}\n";
    // the decl of `Circle` → the param annotation, the impl head, the
    // ctor path (NOT the field read `w.r` / members)
    assert_refs(
        &mut s,
        "file:///t/main.rut",
        src,
        "class Circle", // the decl ident
        1,
        6,
        &[
            byte_of(src, "c: Circle", 1) + 3,
            byte_of(src, "impl Circle", 1) + 5,
            byte_of(src, "= Circle::new", 1) + 2,
        ],
    );
}

#[test]
fn method_refs_need_the_receiver_type() {
    let mut s = S::boot();
    let src = "class Circle {\n    r: f64;\n}\nstruct Box2 {\n    s: f64;\n}\nimpl Circle {\n    fn area(self) -> f64 { return 3.14; }\n}\nimpl Box2 {\n    fn area(self) -> f64 { return 1.0; }\n}\nfn go(c: Circle, b: Box2) -> f64 {\n    let a = c.area();\n    let z = b.area();\n    return a + z;\n}\n";
    let uri = "file:///t/main.rut";
    // references on Circle's `area` decl: the `c.area()` call, whose
    // receiver types to Circle — but NOT `b.area()`
    let hits = s.refs_in(uri, src, "fn area(self) -> f64 { return 3.14;", 1, 3, false);
    assert_eq!(
        ref_offsets(src, uri, &hits),
        vec![byte_of(src, "c.area();", 1) + 2],
        "{hits:?}"
    );
    // and Box2's own `area` answers with its own call
    let hits = s.refs_in(uri, src, "fn area(self) -> f64 { return 1.0;", 1, 3, false);
    assert_eq!(
        ref_offsets(src, uri, &hits),
        vec![byte_of(src, "b.area();", 1) + 2],
        "{hits:?}"
    );
}

#[test]
fn field_refs_ride_the_receiver_inference() {
    let mut s = S::boot();
    let src = "class Circle {\n    r: f64;\n}\nfn go(c: Circle, k: f64) -> f64 {\n    c.r = k;\n    return c.r;\n}\n";
    assert_refs(
        &mut s,
        "file:///t/main.rut",
        src,
        "r: f64;",
        1,
        0,
        &[
            byte_of(src, "c.r = k;", 1) + 2,
            byte_of(src, "return c.r;", 1) + 9,
        ],
    );
}

#[test]
fn module_let_refs_skip_local_shadows() {
    let mut s = S::boot();
    let src = "let TOTAL = 41;\nfn bump() -> i32 {\n    return TOTAL + 1;\n}\nfn shadow() -> i32 {\n    let TOTAL = 5;\n    return TOTAL;\n}\n";
    let uri = "file:///t/main.rut";
    // the module let: bump's return only — the shadow's decl and its
    // return belong to the local
    assert_refs(
        &mut s,
        uri,
        src,
        "let TOTAL = 41;", // the decl ident
        1,
        4,
        &[byte_of(src, "return TOTAL + 1;", 1) + 7],
    );
    // the LOCAL TOTAL: its own return only
    assert_refs(
        &mut s,
        uri,
        src,
        "let TOTAL = 5;",
        1,
        4,
        &[byte_of(src, "return TOTAL;", 1) + 7],
    );
}

// ---- cross-file: the use graph's reverse edges ----

fn widget_user_src() -> &'static str {
    "use gadgets::{ Widget };\nfn main() -> nil {\n    let w = Widget::new();\n}\n"
}

#[test]
fn cross_file_refs_find_the_importing_files() {
    let mut s = S::boot();
    s.index_file(WIDGET_URI, WIDGET_SRC);
    let norm = widget_user_src();
    let uri = "file:///t/main.rut";
    // references on the LIB's type — queried from the importing doc:
    // the use-statement name + the usage site, nothing else
    let hits = s.refs_in(uri, norm, "Widget::new", 1, 0, false);
    let mut offs = ref_offsets(norm, uri, &hits);
    offs.sort();
    assert_eq!(
        offs,
        vec![
            byte_of(norm, "Widget }", 1),
            byte_of(norm, "Widget::new", 1)
        ],
        "{hits:?}"
    );
}

#[test]
fn refs_from_the_decl_side_reach_the_importing_doc() {
    let mut s = S::boot();
    s.index_file("file:///t/main.rut", widget_user_src());
    let norm = widget_user_src();
    // query from the LIB's decl ident, doc = the lib: the scan's
    // reverse edge finds the importing doc's two sites
    let hits = s.refs_in(WIDGET_URI, WIDGET_SRC, "class Widget", 1, 6, false);
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(
        hits.iter().all(|l| l["uri"] == "file:///t/main.rut"),
        "{hits:?}"
    );
    let mut offs = ref_offsets(norm, "file:///t/main.rut", &hits);
    offs.sort();
    assert_eq!(
        offs,
        vec![
            byte_of(norm, "Widget }", 1),
            byte_of(norm, "Widget::new", 1)
        ]
    );
}

#[test]
fn an_unrelated_same_named_type_does_not_leak_in() {
    let mut s = S::boot();
    s.index_file(WIDGET_URI, WIDGET_SRC);
    s.index_file(
        "file:///ws/other/lib.rut",
        "class Widget {\n    other: i32;\n}\n",
    );
    let norm = widget_user_src();
    // references on the gadgets Widget (via the importing doc's use
    // site): the other pkg's same-named type must contribute nothing
    let hits = s.refs_in("file:///t/main.rut", norm, "Widget::new", 1, 0, false);
    assert_eq!(hits.len(), 2, "use name + usage only: {hits:?}");
}

#[test]
fn sibling_duplicates_scope_the_reference_set() {
    // Act 1 made sibling same-name decls legal — the reference set of
    // the queried file's own binding stays in the file's view: the
    // twins' same-named idents resolve to their OWN decls (the
    // matcher law rides the file scope now) and never join, and the
    // non-importing twin file is not even scanned
    let mut s = S::boot();
    s.index_file("file:///t/a.rut", "fn helper() -> i32 { return 1; }\n");
    let src =
        "fn helper() -> i32 { return 2; }\nentry fn main() -> nil {\n    let x = helper();\n}\n";
    let uri = "file:///t/main.rut";
    let hits = s.refs_in(uri, src, "fn helper()", 1, 3, true);
    let mut offs = ref_offsets(src, uri, &hits);
    offs.sort();
    assert_eq!(
        offs,
        vec![
            byte_of(src, "fn helper()", 1) + 3,
            byte_of(src, "= helper();", 1) + 2
        ],
        "the file's own set (the decl via include_declaration): {hits:?}"
    );
    assert!(
        hits.iter().all(|l| l["uri"] == uri),
        "the twin's file never joins: {hits:?}"
    );
}

// ---- misses ----

#[test]
fn misses_stay_empty() {
    let mut s = S::boot();
    let src = "entry fn main() -> nil {\n}\n";
    let hits = s.refs_at("file:///t/main.rut", src, "fn", 1, true);
    assert!(hits.is_empty(), "keywords have no references: {hits:?}");
    let src = "entry fn main() -> nil {\n    let x = mystery();\n}\n";
    let hits = s.refs_at("file:///t/main.rut", src, "mystery", 1, true);
    assert!(hits.is_empty(), "unknown names too: {hits:?}");
}
