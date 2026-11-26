//! References' gate — the definition index read BACKWARDS through the
//! `references` envelope: the shadow-aware binding pass, the decl
//! layer, the receiver-typed members, the includeDeclaration toggle,
//! and the use graph's cross-file reverse edges.

mod common;

use common::*;
use serde_json::Value;

/// the gadget pkg's export — `pub`: a cross-package leaf binds only
/// `pub` rows (the compiler's leaf gate, mirrored by the use tier)
const WIDGET_SRC: &str = "pub class Widget {\n    id: i32;\n}\n";
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
    // the lib rides the workspace scan's lane too — one world: a file
    // only an open document carries answers its own query, never other
    // files' resolution (the old scan lent every candidate the query's
    // own chain; hover from the importer and the reverse edge disagreed)
    s.index_file(WIDGET_URI, WIDGET_SRC);
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

both_uri_shapes! {
#[test]
fn qualified_paths_reference_through_the_module_lane() {
    // the qualified spelling's reference set rides the module lane
    // (the matcher law: a reference iff definition lands on the
    // target). The honest scan limit holds — the asking file and the
    // declaring module are the candidates (a mod-edge-only world
    // pulls no reverse edges), include_declaration adds the decl.
    let mut s = S::boot();
    let layout_src = "pub fn span() -> i32 { return 1; }\n";
    s.index_mod_at(
        "file:///ws/pkg/src/layout/mod.rut",
        "pkg",
        "layout",
        layout_src,
    );
    let one = "mod layout;\nentry fn main() -> i32 { return layout::span(); }\n";
    let uri = "file:///ws/pkg/src/mod.rut";
    s.index_mod_at(uri, "pkg", "", one);
    let hits = s.refs_in(uri, one, "layout::span()", 1, 9, false);
    assert_eq!(
        ref_offsets(one, uri, &hits),
        vec![byte_of(one, "layout::span();", 1) + 8],
        "the qualified call site is a reference: {hits:?}"
    );
    // include_declaration joins the module's declaring ident
    let hits = s.refs_in(uri, one, "layout::span()", 1, 9, true);
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(
        hits.iter()
            .any(|l| l["uri"] == "file:///ws/pkg/src/layout/mod.rut"),
        "the module's decl ident joins: {hits:?}"
    );
}
}

both_uri_shapes! {
#[test]
fn qualified_leaves_never_leak_through_use_rows() {
    // HIGH 2 through the matcher: the qualified tail resolves
    // decls-only, so a private use import in the target yields NO
    // references (the old file-scope layers leaked helper's decl in
    // through the definition answer)
    let mut s = S::boot();
    s.index_mod_at(
        "file:///deps/helper/src/mod.rut",
        "helper",
        "",
        "pub fn two() -> i32 { return 20; }\n",
    );
    s.index_mod_at(
        "file:///ws/pkg/src/layout/mod.rut",
        "pkg",
        "layout",
        "use helper::{ two };\npub fn span() -> i32 { return 1; }\n",
    );
    let doc = "mod layout;\nentry fn main() -> i32 { return layout::two(); }\n";
    let uri = "file:///ws/pkg/src/mod.rut";
    s.index_mod_at(uri, "pkg", "", doc);
    let hits = s.refs_in(uri, doc, "layout::two()", 1, 9, true);
    assert!(
        hits.is_empty(),
        "a use-row leaf has no references: {hits:?}"
    );
}
}

both_uri_shapes! {
#[test]
fn cross_package_module_lets_have_no_references() {
    // the references face of the lets rule: a module let behind a
    // cross-package head resolves to nothing, so its reference set is
    // empty (the compiler refuses the spelling)
    let mut s = S::boot();
    s.index_mod_at(
        "file:///deps/calc/src/mod.rut",
        "calc",
        "",
        "pub mod math;\n",
    );
    s.index_mod_at(
        "file:///deps/calc/src/math.rut",
        "calc",
        "math",
        "pub let PI2: f64 = 6.28;\npub fn half() -> f64 { return 1.0; }\n",
    );
    let doc = "entry fn main() -> f64 {\n    return calc::math::PI2;\n}\n";
    let uri = "file:///t/main.rut";
    let hits = s.refs_in(uri, doc, "calc::math::PI2", 1, 13, true);
    assert!(
        hits.is_empty(),
        "a cross-package module let has no references: {hits:?}"
    );
}
}

// ---- round 4: the rut-new layout — references across the own-package
//      lanes, through the extension's own mounts ----

both_uri_shapes! {
    #[test]
    fn references_reach_a_qualified_site_through_the_own_pkg_head() {
        // DEFECT 1: `app::LIMIT` from a child — the site is a
        // reference of the root's let (queried from the site, and
        // from the declaration: the scan resolves the child's token
        // in the CHILD's own context)
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let root = rut_new::at("src/mod.rut");
        let src = "fn k() -> i32 { return app::LIMIT; }\n";
        let site = byte_of(src, "app::LIMIT", 1) + 5;
        assert_refs(&mut s, &uri, src, "app::LIMIT", 1, 6, &[site]);
        // include_declaration joins the root's own ident
        let hits = s.refs_in(&uri, src, "app::LIMIT", 1, 6, true);
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!(hits.iter().any(|l| l["uri"] == root), "{hits:?}");
        // from the declaration: the sibling file's qualified site
        s.index_mod_at(&uri, "app", "child", src);
        let hits = s.refs_in(&root, rut_new::ROOT, "pub let LIMIT", 1, 8, false);
        assert_eq!(hits.len(), 1, "the child's site: {hits:?}");
        assert_eq!(hits[0]["uri"], uri, "{hits:?}");
        assert_eq!(span_of(src, &hits[0]), (site as usize, site as usize + 5), "{hits:?}");
    }
}

both_uri_shapes! {
    #[test]
    fn references_of_a_root_declared_edge_leaf_span_every_file() {
        // `mod m;` declared by the ROOT: every spelling of its leaf is
        // a reference — a grandchild's bare use binding, a child's
        // qualified call, the root's own call
        let (mut s, _io) = rut_new::boot();
        let child = rut_new::at("src/child/mod.rut");
        let sub = rut_new::at("src/child/sub/mod.rut");
        let root = rut_new::at("src/mod.rut");
        let child_src = "fn k() -> i32 { return m::mk(41).bump(); }\n";
        s.index_mod_at(&child, "app", "child", child_src);
        let sub_src = "use m::{ mk };\nfn k3() -> i32 { return mk(1).bump(); }\n";
        s.index_mod_at(&sub, "app", "child/sub", sub_src);
        let hits = s.refs_in(
            &rut_new::at("src/m/mod.rut"),
            rut_new::M,
            "fn mk(",
            1,
            3,
            false,
        );
        assert_eq!(
            ref_offsets(child_src, &child, &hits),
            vec![byte_of(child_src, "m::mk", 1) + 3],
            "{hits:?}"
        );
        assert_eq!(
            ref_offsets(sub_src, &sub, &hits),
            vec![byte_of(sub_src, "{ mk }", 1) + 2, byte_of(sub_src, "mk(1)", 1)],
            "{hits:?}"
        );
        assert_eq!(
            ref_offsets(rut_new::ROOT, &root, &hits),
            vec![byte_of(rut_new::ROOT, "m::mk(41)", 1) + 3],
            "{hits:?}"
        );
        assert_eq!(hits.len(), 4, "no other site: {hits:?}");
        // queried at a site: itself (and the declaration on request)
        let hits = s.refs_in(&child, child_src, "m::mk", 1, 3, false);
        // — the whole set (every reference of the name, the queried
        // site among them)
        assert_eq!(hits.len(), 4, "{hits:?}");
    }
}

both_uri_shapes! {
    #[test]
    fn references_span_an_intra_package_use_edge() {
        // DEFECT 2: `use m::{ mk }` — the leaf and the call site are
        // references of the module's fn (the use tier answers the
        // bare name, so the scan's matcher lands on the declaration)
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let src = "use m::{ mk };\nfn k() -> i32 { return mk(41).bump(); }\n";
        let want = [byte_of(src, "{ mk }", 1) + 2, byte_of(src, "mk(41)", 1)];
        assert_refs(&mut s, &uri, src, "mk(41)", 1, 0, &want);
        assert_refs(&mut s, &uri, src, "{ mk }", 1, 2, &want);
        // and from the declaration: the sibling's two sites beside
        // the root's own qualified call (`m::mk(41)` in main)
        s.index_mod_at(&uri, "app", "child", src);
        let hits = s.refs_in(
            &rut_new::at("src/m/mod.rut"),
            rut_new::M,
            "fn mk(",
            1,
            3,
            false,
        );
        assert_eq!(
            ref_offsets(src, &uri, &hits),
            want,
            "{hits:?}"
        );
        assert_eq!(hits.len(), 3, "the root's own call too: {hits:?}");
    }
}

both_uri_shapes! {
    #[test]
    fn a_twice_bound_name_has_no_references() {
        // the one-name shape: two use leaves of one name — nothing
        // counts as a reference of either binding
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let math = format!(
            "{}pub fn mk(n: i32) -> Calc {{ return Calc {{ d: n }}; }}\n",
            rut_new::CALC_MATH
        );
        s.index_mod_at(&rut_new::calc("src/math/mod.rut"), "calc", "math", &math);
        let src = "use m::{ mk };\nuse calc::math::{ mk };\nfn k() -> i32 { return mk(41).bump(); }\n";
        for (needle, occ, off) in [("mk(41)", 1, 0), ("{ mk }", 1, 2), ("{ mk }", 2, 2)] {
            let hits = s.refs_in(&uri, src, needle, occ, off, true);
            assert!(hits.is_empty(), "[{needle} #{occ}]: {hits:?}");
        }
    }
}

both_uri_shapes! {
    #[test]
    fn a_member_call_site_is_a_reference_position() {
        // find-references ON a member use (a method call) answers the
        // call sites (and the declaration on request) — the member
        // token names its decl through the same door
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let src = "use m::{ mk };\nfn k() -> i32 { let c = mk(41); return c.bump(); }\n";
        let site = byte_of(src, "c.bump()", 1) + 2;
        assert_refs(&mut s, &uri, src, "c.bump()", 1, 2, &[site]);
        // ... and with the declaration: the module's `fn bump` — beside
        // the root's own `.bump()` call (`m::mk(41).bump()` in main)
        let hits = s.refs_in(&uri, src, "c.bump()", 1, 2, true);
        assert_eq!(hits.len(), 3, "{hits:?}");
        assert!(hits.iter().any(|l| l["uri"] == rut_new::at("src/m/mod.rut")), "{hits:?}");
        // a same-file method
        let src = "struct R1 { n: i32 }\nimpl R1 {\n    fn go(self) -> i32 { return self.n; }\n}\nfn k() -> i32 { let r = R1 { n: 1 }; return r.go(); }\n";
        assert_refs(&mut s, &uri, src, "r.go()", 1, 2, &[byte_of(src, "r.go()", 1) + 2]);
    }
}

both_uri_shapes! {
    #[test]
    fn references_on_a_use_head_answers_the_sibling_use_sites() {
        // DELTA (Act 4 — the scan matches the walk's DEFS): a use head
        // resolves to the package's own def (`Def::Module`), so a
        // sibling spelling of the same head is a reference of it. The
        // base tree's matcher keyed on decl records — a module has no
        // decl record, so the head answered `[]` (RED there).
        let mut s = S::boot();
        s.index_mod_at(WIDGET_URI, "gadgets", "", WIDGET_SRC);
        let uri = "file:///t/main.rut";
        let src = "use gadgets::{ Widget };\nentry fn main() -> nil {\n    let w = Widget { id: 1 };\n}\n";
        let hits = s.refs_in(uri, src, "use gadgets", 1, 4, true);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0]["uri"], uri, "{hits:?}");
        assert_eq!(span_of(src, &hits[0]).0 as u32, byte_of(src, "gadgets", 1), "{hits:?}");
        // the mechanism is caret-independent within the name: the MID
        // and END carets (`gadge|ts`, `gadgets|`) answer the same set
        let hits = s.refs_in(uri, src, "use gadgets", 1, 8, true);
        assert_eq!(hits.len(), 1, "mid: {hits:?}");
        assert_eq!(span_of(src, &hits[0]).0 as u32, byte_of(src, "gadgets", 1), "{hits:?}");
        let hits = s.refs_in(uri, src, "use gadgets", 1, 11, true);
        assert_eq!(hits.len(), 1, "end: {hits:?}");
        assert_eq!(span_of(src, &hits[0]).0 as u32, byte_of(src, "gadgets", 1), "{hits:?}");
        // include_decl adds nothing here: a module names no declaring
        // ident in the scan (its answer IS the file top)
        let hits = s.refs_in(uri, src, "use gadgets", 1, 4, false);
        assert_eq!(hits.len(), 1, "{hits:?}");
    }
}
