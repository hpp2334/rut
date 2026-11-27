//! Definition's gate — the four layers (binding pass → decl layer →
//! use graph → the mounted world) plus typeDefinition, driven through
//! the `definition`/`typeDefinition` envelopes and the workspace-scan
//! push lane. Pins: the shadow-aware targets, the body-over-decl
//! preference, the mounted-over-ambient law, the own-package lanes
//! keyed on the stamp the index lane laid down on the uri (opening
//! adopts it), the module matcher's derivation, and the
//! misses that stay misses.

mod common;

use common::*;
use serde_json::Value;

/// the gadget pkg's export — `pub`: a cross-package leaf binds only
/// `pub` rows (the compiler's leaf gate, mirrored by the use tier)
const WIDGET_SRC: &str = "pub class Widget {\n    id: i32;\n}\n";
const WIDGET_URI: &str = "file:///ws/gadgets/lib.rut";

/// the `occ`-th needle's byte span in the target file must be exactly
/// what the Location points at (the ASCII docs make this a plain
/// byte check)
fn assert_target(src: &str, loc: &Value, needle: &str, occ: usize, len: usize, what: &str) {
    let want = byte_of(src, needle, occ) as usize;
    assert_eq!(span_of(src, loc), (want, want + len), "{what}: {loc}");
}

#[test]
fn local_binding_jumps_to_declaring_ident() {
    let mut s = S::boot();
    let src = "fn go() -> i32 {\n    let c = 1;\n    return c;\n}\n";
    let uri = "file:///t/main.rut";
    let hits = s.def_in(uri, src, "return c", 1, 7);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], uri);
    assert_target(src, &hits[0], "c = 1", 1, 1, "the declaring ident");
}

#[test]
fn shadowing_picks_the_latest_decl() {
    let mut s = S::boot();
    let src = "fn go() -> i32 {\n    let v = 1;\n    if (v > 0) {\n        let v = 2;\n        return v;\n    }\n    return 0;\n}\n";
    let hits = s.def_in("file:///t/main.rut", src, "return v", 1, 7);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_target(src, &hits[0], "v = 2", 1, 1, "the INNER v");
}

#[test]
fn scope_exit_keeps_the_outer_binding() {
    let mut s = S::boot();
    let src = "fn go() -> i32 {\n    let v = 1;\n    if (v > 0) {\n        let v = 2;\n    }\n    return v;\n}\n";
    let hits = s.def_in("file:///t/main.rut", src, "return v", 1, 7);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_target(src, &hits[0], "v = 1", 1, 1, "the OUTER v");
}

#[test]
fn param_jumps_to_its_decl() {
    let mut s = S::boot();
    let src = "fn go(p: Point) -> f64 {\n    return p.x;\n}\nstruct Point {\n    x: f64;\n}\n";
    // the param decl site answers itself
    let hits = s.def_at("file:///t/main.rut", src, "p: Point", 1);
    assert_eq!(hits.len(), 1);
    // the use site
    let hits = s.def_in("file:///t/main.rut", src, "return p", 1, 7);
    assert_target(src, &hits[0], "p: Point", 1, 1, "the param's decl");
}

// ---- the decl layer ----

#[test]
fn field_read_jumps_to_the_field_decl() {
    let mut s = S::boot();
    let src = "class Circle {\n    r: f64;\n}\nstruct Wrap {\n    c: Circle;\n}\nfn go(wrap: Wrap) -> f64 {\n    let inner = wrap.c;\n    return inner.r;\n}\n";
    // `wrap.c` — the `c` after the dot → the field decl ident
    let hits = s.def_in("file:///t/main.rut", src, "wrap.c", 1, 5);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_target(src, &hits[0], "c: Circle", 1, 1, "the declaring c");
    // the field DECL site answers itself
    let hits = s.def_at("file:///t/main.rut", src, "c: Circle", 1);
    assert_target(src, &hits[0], "c: Circle", 1, 1, "the decl site");
}

#[test]
fn method_call_jumps_to_the_impl_fn_name() {
    let mut s = S::boot();
    let src = "class Circle {\n    r: f64;\n}\nimpl Circle {\n    fn area(self) -> f64 { return 3.14; }\n}\nfn go() -> f64 {\n    let c = Circle::new(1.0);\n    return c.area();\n}\n";
    let hits = s.def_in("file:///t/main.rut", src, "c.area", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_target(src, &hits[0], "area(self)", 1, 4, "the impl fn name");
}

#[test]
fn enum_member_jumps_to_its_decl_ident() {
    let mut s = S::boot();
    let src = "enum Color { Red, Blue }\nfn pick(c: Color) -> Color {\n    return c;\n}\nfn go() -> Color {\n    return Color::Red;\n}\n";
    let hits = s.def_in("file:///t/main.rut", src, "Color::Red", 1, 7);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_target(src, &hits[0], "Red,", 1, 3, "the member's decl ident");
}

#[test]
fn type_and_fn_names_jump_to_their_decls() {
    let mut s = S::boot();
    let src = "class Circle {\n    r: f64;\n}\nfn make() -> Circle {\n    return Circle::new(1.0);\n}\nentry fn main() -> nil {\n    let c = make();\n    let d = make();\n}\n";
    // the type token in `Circle::new` → the class decl ident
    let hits = s.def_at("file:///t/main.rut", src, "Circle::new(1.0)", 1);
    assert_eq!(hits.len(), 1, "{hits:?}");
    {
        let want = byte_of(src, "class Circle", 1) + 6;
        assert_eq!(
            span_of(src, &hits[0]),
            (want as usize, want as usize + 6),
            "the class decl ident"
        );
    }
    // a free-fn call site → the fn decl ident
    let hits = s.def_in("file:///t/main.rut", src, "= make();", 2, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    {
        let want = byte_of(src, "fn make()", 1) + 3;
        assert_eq!(
            span_of(src, &hits[0]),
            (want as usize, want as usize + 4),
            "the fn decl ident"
        );
    }
}

#[test]
fn module_let_use_site_jumps_to_the_decl() {
    let mut s = S::boot();
    let src = "let TOTAL = 41;\nfn bump() -> i32 {\n    return TOTAL + 1;\n}\n";
    let hits = s.def_in("file:///t/main.rut", src, "return TOTAL", 1, 7);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_target(src, &hits[0], "TOTAL", 1, 5, "the module let's decl");
}

// ---- the use graph ----

#[test]
fn use_name_jumps_to_the_exporting_module() {
    let mut s = S::boot();
    let uri = "file:///t/main.rut";
    s.index_file(WIDGET_URI, WIDGET_SRC);
    let src = "use gadgets::{ Widget };\nfn main() -> nil {\n    let w = Widget::new();\n}\n";
    // the use-statement name itself
    let hits = s.def_in(uri, src, "{ Widget }", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], WIDGET_URI);
    {
        let want = byte_of(WIDGET_SRC, "class Widget", 1) + 6;
        assert_eq!(
            span_of(WIDGET_SRC, &hits[0]),
            (want as usize, want as usize + 6),
            "the export's decl"
        );
    }
    // and a usage site resolves through the same edge
    let hits = s.def_in(uri, src, "= Widget::new();", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], WIDGET_URI);
}

#[test]
fn the_use_graph_restricts_the_search_to_the_imported_pkg() {
    let mut s = S::boot();
    s.index_file(WIDGET_URI, WIDGET_SRC);
    s.index_file(
        "file:///ws/other/lib.rut",
        "class Widget {\n    other: i32;\n}\n",
    );
    let src = "use gadgets::{ Widget };\nfn main() -> nil {\n    let w = Widget::new();\n}\n";
    let hits = s.def_in("file:///t/main.rut", src, "= Widget::new();", 1, 2);
    assert_eq!(
        hits.len(),
        1,
        "ambiguity from the OTHER pkg must not leak: {hits:?}"
    );
    assert_eq!(hits[0]["uri"], WIDGET_URI);
}

#[test]
fn un_imported_names_answer_nothing() {
    // the file-scope law (the one-name law's Act 2): the file never
    // binds the name — no decl, no use edge — so the world's
    // same-named decl is NOT the file's answer (the old flat chain
    // jumped to it; the compile refuses the bare spelling with a
    // `use` hint)
    let mut s = S::boot();
    s.index_file(WIDGET_URI, WIDGET_SRC);
    let src = "entry fn main() -> nil {\n    let w = Widget::new();\n}\n";
    let hits = s.def_in("file:///t/main.rut", src, "= Widget::new();", 1, 2);
    assert!(hits.is_empty(), "an un-imported name is no jump: {hits:?}");
}

#[test]
fn sibling_duplicates_answer_the_file_own_decl() {
    // Act 1 made sibling same-name decls legal — the queried file's
    // own binding IS the answer; the siblings contribute nothing (the
    // old flat chain answered a candidate list)
    let mut s = S::boot();
    s.index_file("file:///t/a.rut", "fn make() -> i32 { return 1; }\n");
    s.index_file("file:///t/b.rut", "fn make() -> i32 { return 2; }\n");
    let src = "fn make() -> i32 { return 3; }\nentry fn main() -> nil {\n    let x = make();\n}\n";
    let uri = "file:///t/main.rut";
    let hits = s.def_in(uri, src, "= make();", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], uri);
    assert_target(src, &hits[0], "make()", 1, 4, "the file's own decl");
}

// ---- the pkg's two files: the body over the decl surface ----

both_uri_shapes! {
#[test]
fn a_both_files_dep_answers_from_the_body() {
    // the dep indexes a pkg's `.d.rut` surface (Decl) AND its root
    // `mod.rut` body (Impl): the leaf resolves to the BODY's
    // declaring ident, and the pkg head lands at the body root's
    // file top (0:0)
    let mut s = S::boot();
    let surface = "pub host fn make() -> i32;\n";
    let body = "pub fn make() -> i32 { return 1; }\n";
    s.index_mod_at("file:///deps/pkg/pkg.d.rut", "pkg", "", surface);
    s.index_mod_at("file:///deps/pkg/mod.rut", "pkg", "", body);
    let src = "use pkg::{ make };\nentry fn main() -> nil {\n    let x = make();\n}\n";
    let uri = "file:///t/main.rut";
    // the use leaf: the body's `make`, not the surface's
    let hits = s.def_in(uri, src, "{ make }", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///deps/pkg/mod.rut");
    assert_target(body, &hits[0], "make()", 1, 4, "the body's ident");
    // a use site rides the same preference
    let hits = s.def_in(uri, src, "= make();", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///deps/pkg/mod.rut");
    // the pkg head: the body root, file top
    let hits = s.def_in(uri, src, "use pkg::", 1, 4);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///deps/pkg/mod.rut");
    assert_eq!(span_of(body, &hits[0]), (0, 0), "file top: {hits:?}");
}
}

both_uri_shapes! {
#[test]
fn a_decl_only_host_pkg_lands_in_the_surface() {
    // a host pkg indexes its `.d.rut` surface ALONE (calc's shape):
    // leaf and pkg head both land in it — the surface is all there is
    let mut s = S::boot();
    let surface = "pub host fn sqrt(x: f64) -> f64;\n";
    s.index_mod_at("file:///deps/calc/calc.d.rut", "calc", "", surface);
    let src = "use calc::{ sqrt };\nentry fn main() -> nil {\n    let x = sqrt(1.0);\n}\n";
    let uri = "file:///t/main.rut";
    let hits = s.def_in(uri, src, "{ sqrt }", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///deps/calc/calc.d.rut");
    assert_target(surface, &hits[0], "sqrt(x", 1, 4, "the surface's ident");
    let hits = s.def_in(uri, src, "use calc::", 1, 4);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///deps/calc/calc.d.rut");
    assert_eq!(span_of(surface, &hits[0]), (0, 0), "file top: {hits:?}");
}
}

both_uri_shapes! {
#[test]
fn a_mounted_root_beats_the_ambient_surface_twin() {
    // the transitive-mount world: a pkg the walk mounted (a real
    // file:// origin) AND the ambient dep-lane twin's index of the
    // same name (scheme-less `rut/…`) both carry a flat Decl root —
    // the mount wins for the pkg head and the leaf
    let mut s = S::boot();
    let surface = "pub host fn create_logger(name: str) -> opaque;\n";
    // the ambient twin registered first (the boot-time shape), the
    // mounted twin second
    s.index_mod_at("rut/ink_host/src/ink_host.d.rut", "ink_host", "", surface);
    s.index_mod_at(
        "file:///ws/.rut/deps/ink-eb2b568316ef/deps/ink_host@7e9a5d738fbd9797/src/ink_host.d.rut",
        "ink_host",
        "",
        surface,
    );
    let src = "use ink_host::{ create_logger };\nentry fn main() -> nil {\n    create_logger(\"a\");\n}\n";
    let uri = "file:///t/main.rut";
    // the pkg head: the MOUNTED surface's file top, never the ambient
    let hits = s.def_in(uri, src, "use ink_host::", 1, 4);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(
        hits[0]["uri"],
        "file:///ws/.rut/deps/ink-eb2b568316ef/deps/ink_host@7e9a5d738fbd9797/src/ink_host.d.rut"
    );
    assert_eq!(span_of(src, &hits[0]), (0, 0), "file top: {hits:?}");
    // the leaf: the MOUNTED one among the twins
    let hits = s.def_in(uri, src, "{ create_logger }", 1, 2);
    assert!(
        hits.iter().any(|h| h["uri"]
            .as_str()
            .unwrap()
            .starts_with("file:///ws/.rut/deps/")),
        "{hits:?}"
    );
}
}

#[test]
fn a_ctor_decl_click_answers_itself() {
    // `[constructor] fn new` — the one name every class reuses. The
    // decl click answers ITSELF, never the cross-pkg fan-out of every
    // other class's ctor the name layers would peek
    let mut s = S::boot();
    let src = "class Logger {\n    internal: opaque;\n}\n\nimpl Logger {\n    [constructor] pub fn new(name: str) -> Self {\n        return Self { internal: 1 };\n    }\n}\n";
    s.index_file(
        "file:///ws/flow/lib.rut",
        "class Ctx {\n    internal: opaque;\n}\n\nimpl Ctx {\n    [constructor] pub fn new(cap: i32) -> Self {\n        return Self { internal: 2 };\n    }\n}\n",
    );
    let uri = "file:///t/main.rut";
    let hits = s.def_in(uri, src, "fn new", 1, 3);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], uri);
    assert_target(src, &hits[0], "new(name", 1, 3, "itself");
}

#[test]
fn a_fn_decl_click_answers_itself_despite_a_foreign_twin() {
    let mut s = S::boot();
    let src = "fn make() -> i32 { return 1; }\nentry fn main() -> nil {\n    let x = make();\n}\n";
    s.index_file(
        "file:///ws/other/lib.rut",
        "fn make() -> i32 { return 2; }\n",
    );
    let uri = "file:///t/main.rut";
    let hits = s.def_in(uri, src, "fn make", 1, 3);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], uri);
    assert_target(src, &hits[0], "make()", 1, 4, "itself");
}

#[test]
fn an_indexed_file_opened_under_its_own_uri_answers_once() {
    // a file indexed by the scan lane at its own uri, then opened —
    // opening ADOPTS the stamp the index lane laid down: one entry,
    // and a name both lanes carry resolves once. The ambiguity peek
    // is for DISTINCT targets
    let mut s = S::boot();
    let src = "class Widget {\n    id: i32;\n}\n";
    s.index_file("file:///t/main.rut", src); // the index lane's stamp, which opening adopts
    let hits = s.def_at("file:///t/main.rut", src, "Widget", 1);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///t/main.rut");
    assert_target(src, &hits[0], "Widget", 1, 6, "the decl ident");
}

#[test]
fn a_workspace_member_beats_the_prelude_twin() {
    // the mounted-over-embed law through today's world shape: the
    // prelude's `str` surface is the ONE embedded index (its
    // `src_path` marks it), and a workspace file that carries the same
    // name wins the member lane — the definition answers in the
    // workspace file, never the embedded twin
    let mut s = S::boot();
    s.index_file(
        "file:///t/strx/lib.rut",
        "prelude builtin primitive str {\nfn len(self) -> i32;\n}\n",
    );
    let src = "fn go(s: str) -> i32 {\n    return s.len();\n}\n";
    let hits = s.def_in("file:///t/main.rut", src, "s.len", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///t/strx/lib.rut");
    assert_target(
        "prelude builtin primitive str {\nfn len(self) -> i32;\n}\n",
        &hits[0],
        "len(self)",
        1,
        3,
        "the workspace surface's ident",
    );
}

both_uri_shapes! {
#[test]
fn a_middle_use_segment_jumps_to_the_mod_edge() {
    // `use gadgets::layout::{ Column };` — the `layout` segment jumps
    // to the declaring `pub mod layout;` edge in the pkg root
    let mut s = S::boot();
    let root = "pub mod layout;\npub fn tag() -> i32 { return 1; }\n";
    let layout = "pub struct Column {\n    w: i32;\n}\n";
    s.index_mod_at("file:///deps/gadgets/mod.rut", "gadgets", "", root);
    s.index_mod_at(
        "file:///deps/gadgets/layout/mod.rut",
        "gadgets",
        "layout",
        layout,
    );
    let src = "use gadgets::layout::{ Column };\nentry fn main() -> nil {\n}\n";
    let hits = s.def_in("file:///t/main.rut", src, "gadgets::layout", 1, 9);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///deps/gadgets/mod.rut");
    assert_target(root, &hits[0], "layout;", 1, 6, "the declaring edge");
}
}

#[test]
fn a_pkg_head_with_no_indexed_pkg_is_an_honest_miss() {
    let mut s = S::boot();
    let src = "use ghost::{ Thing };\nentry fn main() -> nil {\n}\n";
    let hits = s.def_at("file:///t/main.rut", src, "use ghost::", 1);
    assert!(hits.is_empty(), "{hits:?}");
}

// ---- the own-package lanes (the uri's stamp keys them) ----

const APP_ROOT: &str = "mod helpers;\nentry fn main() -> nil {\n}\n";
const APP_URI: &str = "file:///ws/app/mod.rut";
const HELPERS_SRC: &str = "pub struct Slot {\n    n: i32;\n}\n";
const HELPERS_URI: &str = "file:///ws/app/helpers/mod.rut";

both_uri_shapes! {
#[test]
fn an_edge_click_opens_the_child_file_top() {
    let mut s = S::boot();
    s.index_mod_at(APP_URI, "app", "", APP_ROOT);
    s.index_mod_at(HELPERS_URI, "app", "helpers", HELPERS_SRC);
    let hits = s.def_in(APP_URI, APP_ROOT, "mod helpers;", 1, 4);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], HELPERS_URI);
    assert_eq!(pos_pair(&hits[0]["range"]["start"]), (0, 0), "the file top");
}
}

both_uri_shapes! {
#[test]
fn an_edge_click_on_an_unstamped_doc_is_a_miss() {
    // no stamp laid down on this uri — the own lanes stay off
    let mut s = S::boot();
    s.index_mod_at(HELPERS_URI, "app", "helpers", HELPERS_SRC);
    let hits = s.def_at("file:///ws/scratch.rut", APP_ROOT, "mod helpers;", 1);
    assert!(hits.is_empty(), "{hits:?}");
}
}

both_uri_shapes! {
#[test]
fn an_edge_click_on_an_unmounted_child_is_a_miss() {
    // the doc is stamped but the chain carries no index at the child's
    // mod path — an empty jump, never a wrong one
    let mut s = S::boot();
    s.index_mod_at(APP_URI, "app", "", APP_ROOT);
    let hits = s.def_at(APP_URI, APP_ROOT, "mod helpers;", 1);
    assert!(hits.is_empty(), "{hits:?}");
}
}

both_uri_shapes! {
#[test]
fn a_nested_edge_click_walks_from_the_doc_s_own_path() {
    // the doc IS `helpers` (mounted file-form): its `mod deep;` edge
    // resolves `helpers/deep`, mounted as `deep.rut`
    let mut s = S::boot();
    let doc = "mod deep;\npub struct Slot {\n    n: i32;\n}\n";
    let uri = "file:///ws/app/helpers.rut";
    s.index_mod_at(uri, "app", "helpers", doc);
    s.index_mod_at(
        "file:///ws/app/helpers/deep.rut",
        "app",
        "helpers/deep",
        "pub fn dig() -> i32 { return 1; }\n",
    );
    let hits = s.def_in(uri, doc, "mod deep;", 1, 4);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///ws/app/helpers/deep.rut");
    assert_eq!(pos_pair(&hits[0]["range"]["start"]), (0, 0));
}
}

both_uri_shapes! {
#[test]
fn an_edge_click_never_leaks_across_packages() {
    // another pkg's index stamped at the SAME mod path must not
    // answer the app root's edge click (the (pkg, path) pair keys)
    let mut s = S::boot();
    s.index_mod_at(APP_URI, "app", "", APP_ROOT);
    s.index_mod_at(
        "file:///ws/other/helpers/mod.rut",
        "other",
        "helpers",
        HELPERS_SRC,
    );
    let hits = s.def_at(APP_URI, APP_ROOT, "mod helpers;", 1);
    assert!(hits.is_empty(), "{hits:?}");
}
}

both_uri_shapes! {
#[test]
fn an_own_pkg_use_path_resolves_intra_package() {
    // `use app::helpers::{ Slot };` in the app root — the pkg-name
    // head opens the root module (the doc itself) at its top, the
    // segment rides the mounting edge, the leaf lands in the child
    let mut s = S::boot();
    let doc = "mod helpers;\nuse app::helpers::{ Slot };\nentry fn main() -> nil {\n}\n";
    let uri = APP_URI;
    s.index_mod_at(uri, "app", "", doc);
    // the stale indexed text (the doc's own rows must win)
    s.index_mod_at(
        "file:///ws/app/on-disk/mod.rut",
        "app",
        "",
        "mod helpers;\n",
    );
    s.index_mod_at(HELPERS_URI, "app", "helpers", HELPERS_SRC);
    // the pkg-name head: the doc's own file top (the open buffer's truth)
    let hits = s.def_in(uri, doc, "use app::", 1, 4);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], uri);
    assert_eq!(pos_pair(&hits[0]["range"]["start"]), (0, 0));
    // the `helpers` segment: the mounting edge in the root
    let hits = s.def_in(uri, doc, "app::helpers", 1, 6);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], uri);
    assert_target(doc, &hits[0], "helpers;", 1, 7, "the own edge");
    // the leaf: the child's declaring ident
    let hits = s.def_in(uri, doc, "{ Slot }", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], HELPERS_URI);
    assert_target(HELPERS_SRC, &hits[0], "Slot {", 1, 4, "the child's ident");
}
}

both_uri_shapes! {
#[test]
fn an_own_mod_edge_use_head_resolves_through_the_ancestor_walk() {
    // `use helpers::deep::{ Gem };` from the app root — the bare head
    // is the root's own mod edge: the head opens the child file at its
    // top, the `deep` segment its mounting edge, the leaf the walked
    // path's decl
    let mut s = S::boot();
    let doc = "use helpers::deep::{ Gem };\nmod helpers;\nentry fn main() -> nil {\n}\n";
    let uri = APP_URI;
    s.index_mod_at(uri, "app", "", doc);
    let helpers = "mod deep;\npub struct Slot {\n    n: i32;\n}\n";
    s.index_mod_at(HELPERS_URI, "app", "helpers", helpers);
    let deep = "pub struct Gem {\n    g: i32;\n}\n";
    let deep_uri = "file:///ws/app/helpers/deep.rut";
    s.index_mod_at(deep_uri, "app", "helpers/deep", deep);
    // the head: the child file's top (the mod-edge target law)
    let hits = s.def_in(uri, doc, "use helpers::", 1, 4);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], HELPERS_URI);
    assert_eq!(pos_pair(&hits[0]["range"]["start"]), (0, 0));
    // the `deep` segment: the `mod deep;` edge in the helpers module
    let hits = s.def_in(uri, doc, "helpers::deep", 1, 9);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], HELPERS_URI);
    assert_target(helpers, &hits[0], "deep;", 1, 4, "the nested edge");
    // the leaf: the walked path's decl
    let hits = s.def_in(uri, doc, "{ Gem }", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], deep_uri);
    assert_target(deep, &hits[0], "Gem {", 1, 3, "the leaf's ident");
}
}

// ---- the module matcher (the explicit name, then the derivation) ----

both_uri_shapes! {
#[test]
fn the_explicit_module_name_beats_the_path_derivation() {
    // a dep index named `pouch` whose paths spell something else: the
    // explicit module is the WHOLE match — the path stem must NOT
    // match once the module is named
    let mut s = S::boot();
    s.index_mod_at("file:///ws/gadgets/lib.rut", "pouch", "", WIDGET_SRC);
    let src = "use pouch::{ Widget };\nfn main() -> nil {\n    let w = Widget::new();\n}\n";
    let hits = s.def_in("file:///t/main.rut", src, "= Widget::new();", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///ws/gadgets/lib.rut");
    // the same file never speaks for its path's segment
    let src = "use gadgets::{ Widget };\nfn main() -> nil {\n    let w = Widget::new();\n}\n";
    let hits = s.def_at("file:///t/main2.rut", src, "= Widget::new();", 1);
    assert!(hits.is_empty(), "{hits:?}");
}
}

#[test]
fn the_unnamed_fallback_stays_the_path_derivation() {
    // an unnamed workspace index: the path segment (stem stripped)
    // resolves — `…/gadgets/lib.rut` speaks for `gadgets`,
    // `…/pouch.rut` for `pouch`
    let mut s = S::boot();
    s.index_file(WIDGET_URI, WIDGET_SRC);
    s.index_file("file:///ws/pouch.rut", "pub class Bag {\n    n: i32;\n}\n");
    let src = "use gadgets::{ Widget };\nfn main() -> nil {\n    let w = Widget::new();\n}\n";
    let hits = s.def_in("file:///t/main.rut", src, "= Widget::new();", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    let src = "use pouch::{ Bag };\nfn main() -> nil {\n    let b = Bag::new();\n}\n";
    let hits = s.def_in("file:///t/main2.rut", src, "= Bag::new();", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///ws/pouch.rut");
}

// ---- the stdlib dep lane ----

both_uri_shapes! {
#[test]
fn std_use_jumps_to_the_true_source_path() {
    // pouch rides the dep lane: the repo's own source mounted as a
    // fixture through the scan lane, and F12 lands on the origin the
    // fixture serves — the TRUE rut/... path, not a label
    const POUCH: &str = include_str!("../../../rut/pouch/src/mod.rut");
    let mut s = S::boot();
    s.index_mod_at("rut/pouch/src/mod.rut", "pouch", "", POUCH);
    let src = "use pouch::{ Vec };\nfn main() -> nil {\n    let v = Vec::new();\n}\n";
    let hits = s.def_in("file:///t/main.rut", src, "= Vec::new();", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "rut/pouch/src/mod.rut");
    // the reported range is the declaring `Vec` ident in the REAL
    // source file (normalized like the server normalizes)
    let real = POUCH.replace("\r\n", "\n");
    {
        let want = byte_of(&real, "class Vec", 1) + 6;
        assert_eq!(
            span_of(&real, &hits[0]),
            (want as usize, want as usize + 3),
            "the real source's ident"
        );
    }
    // the use-statement name resolves to the same target
    let hits = s.def_in("file:///t/main.rut", src, "{ Vec }", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "rut/pouch/src/mod.rut");
    {
        let want = byte_of(&real, "class Vec", 1) + 6;
        assert_eq!(
            span_of(&real, &hits[0]),
            (want as usize, want as usize + 3),
            "the real source's ident"
        );
    }
}
}

// ---- typeDefinition ----

#[test]
fn type_definition_from_a_binding() {
    let mut s = S::boot();
    let src = "class Circle {\n    r: f64;\n}\nfn go() -> f64 {\n    let c = Circle::new(1.0);\n    return c.r;\n}\n";
    let hits = s.typedef_in("file:///t/main.rut", src, "return c.r", 1, 7);
    assert_eq!(hits.len(), 1, "{hits:?}");
    {
        let want = byte_of(src, "class Circle", 1) + 6;
        assert_eq!(
            span_of(src, &hits[0]),
            (want as usize, want as usize + 6),
            "the class decl"
        );
    }
}

#[test]
fn type_definition_from_a_nullable_field_member() {
    let mut s = S::boot();
    let src = "class Circle {\n    r: f64;\n}\nstruct Slot {\n    item: ?Circle;\n}\nfn go(s: Slot) -> f64 {\n    return s.item.r;\n}\n";
    // `s.item` — the `?Circle` field's type head is `Circle`
    let hits = s.typedef_in("file:///t/main.rut", src, "s.item.r", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    {
        let want = byte_of(src, "class Circle", 1) + 6;
        assert_eq!(
            span_of(src, &hits[0]),
            (want as usize, want as usize + 6),
            "the class decl"
        );
    }
}

#[test]
fn type_definition_on_a_type_token_is_its_own_decl() {
    let mut s = S::boot();
    let src = "class Circle {\n    r: f64;\n}\nfn go() -> f64 {\n    let c = Circle::new(1.0);\n    return c.r;\n}\n";
    let hits = s.typedef_at("file:///t/main.rut", src, "Circle::new(1.0)", 1);
    assert_eq!(hits.len(), 1, "{hits:?}");
    {
        let want = byte_of(src, "class Circle", 1) + 6;
        assert_eq!(
            span_of(src, &hits[0]),
            (want as usize, want as usize + 6),
            "the class decl"
        );
    }
}

#[test]
fn misses_stay_misses() {
    let mut s = S::boot();
    // keywords, unknown names, and decl-less primitives are empty
    let src = "entry fn main() -> nil {\n}\n";
    let hits = s.def_at("file:///t/main.rut", src, "fn", 1);
    assert!(hits.is_empty(), "{hits:?}");
    let src = "entry fn main() -> nil {\n    let x = i32::max(1);\n}\n";
    let hits = s.def_at("file:///t/main.rut", src, "i32::max", 1);
    assert!(hits.is_empty(), "i32 has no decl to jump to: {hits:?}");
    let src = "entry fn main() -> nil {\n    let x = mystery();\n}\n";
    let hits = s.def_at("file:///t/main.rut", src, "mystery", 1);
    assert!(hits.is_empty(), "{hits:?}");
}

both_uri_shapes! {
#[test]
fn member_definition_gates_the_foreign_privates() {
    // the member-vis law on goto-def: a foreign class's private field
    // is no jump (the compile refuses it — Act 1), the pub impl
    // method still jumps, and the doc's own private field still does
    let mut s = S::boot();
    s.index_mod_at(
        "file:///deps/http/mod.rut",
        "http",
        "",
        "pub class RequestBuilder {\nheaders: str;\n}\n\
         impl RequestBuilder {\npub fn build(self) -> i32 { return 0; }\n}\n\
         pub fn builder() -> RequestBuilder { return nil; }\n",
    );
    let src = "use http::{ builder };\nfn go() -> nil {\n    let b = builder();\n    b.headers;\n    b.build();\n}\n";
    let hits = s.def_in("file:///t/main.rut", src, "b.headers", 1, 2);
    assert!(
        hits.is_empty(),
        "a private foreign field is no jump: {hits:?}"
    );
    let hits = s.def_in("file:///t/main.rut", src, "b.build", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///deps/http/mod.rut", "{hits:?}");

    let own =
        "class C {\nhidden: i32;\n}\nimpl C {\nfn read(self) -> i32 { return self.hidden; }\n}\n";
    let hits = s.def_in("file:///t/own.rut", own, "self.hidden", 1, 5);
    assert_eq!(hits.len(), 1, "own privates still jump: {hits:?}");
}
}

both_uri_shapes! {
#[test]
fn qualified_paths_resolve_through_the_module_lane() {
    // the qualified spelling is one of the two explicit forms of
    // file-scope access: `layout::span()` over a bare `mod layout;`
    // edge, `math::sqrt` over `use calc::{ math }`, and the nested
    // walk each land EXACTLY on the module's decl ident; an unbound
    // head is no jump (scope-true)
    let mut s = S::boot();
    // the own mod edge (the mixed-tree shape)
    s.index_mod_at(
        "file:///ws/pkg/src/layout/mod.rut",
        "pkg",
        "layout",
        "mod grid;\npub fn span() -> i32 { return 1; }\n",
    );
    s.index_mod_at(
        "file:///ws/pkg/src/layout/grid.rut",
        "pkg",
        "layout/grid",
        "pub fn cell() -> i32 { return 2; }\n",
    );
    let layout_src = "mod grid;\npub fn span() -> i32 { return 1; }\n";
    let grid_src = "pub fn cell() -> i32 { return 2; }\n";
    let doc = "mod layout;\nentry fn main() -> i32 { return layout::span(); }\n";
    let uri = "file:///ws/pkg/src/mod.rut";
    s.index_mod_at(uri, "pkg", "", doc);
    let hits = s.def_in(uri, doc, "layout::span()", 1, 9);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///ws/pkg/src/layout/mod.rut");
    assert_target(
        layout_src,
        &hits[0],
        "span()",
        1,
        4,
        "the module's decl ident",
    );
    // the nested walk
    let nested = "mod layout;\nentry fn main() -> i32 { return layout::grid::cell(); }\n";
    let hits = s.def_in(uri, nested, "layout::grid::cell", 1, 15);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///ws/pkg/src/layout/grid.rut");
    assert_target(
        grid_src,
        &hits[0],
        "cell()",
        1,
        4,
        "the deepest module's ident",
    );
    // the use-bound module
    s.index_mod_at(
        "file:///deps/calc/src/mod.rut",
        "calc",
        "",
        "pub mod math;\n",
    );
    let math_src = "pub fn sqrt(x: f64) -> f64 { return x; }\n";
    s.index_mod_at("file:///deps/calc/src/math.rut", "calc", "math", math_src);
    let calc_doc = "use calc::{ math };\nfn go(x: f64) -> f64 {\n    return math::sqrt(x);\n}\n";
    let hits = s.def_in("file:///t/main.rut", calc_doc, "math::sqrt", 1, 9);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///deps/calc/src/math.rut");
    assert_target(
        math_src,
        &hits[0],
        "sqrt(x",
        1,
        4,
        "the use-bound module's ident",
    );
    // the own pkg head
    let pkgdoc = "mod layout;\nentry fn main() -> i32 { return pkg::layout::span(); }\n";
    let hits = s.def_in(uri, pkgdoc, "pkg::layout::span", 1, 15);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///ws/pkg/src/layout/mod.rut");
    // the negative: no binding → empty, never the flat sweep
    let ghost = "entry fn main() -> i32 { return layout::span(); }\n";
    let hits = s.def_in("file:///t/ghost.rut", ghost, "layout::span()", 1, 9);
    assert!(
        hits.is_empty(),
        "an unbound qualified head is no jump: {hits:?}"
    );
}
}

both_uri_shapes! {
#[test]
fn pkg_rooted_dep_names_resolve() {
    // HIGH 1's definition face: the dep-rooted path lands exactly on
    // the module's decl ident; the type position stays silent (the
    // type ladder walks the own family only — the compiler refuses
    // `calc::math::Angle` with "unknown type"); the non-pub leaf and
    // the flat dep stay empty
    let mut s = S::boot();
    let math_src = "pub fn sqrt(x: f64) -> f64 { return x; }\npub let PI2: f64 = 6.28;\nfn hidden() -> i32 { return 0; }\n";
    s.index_mod_at(
        "file:///deps/calc/src/mod.rut",
        "calc",
        "",
        "pub mod math;\n",
    );
    s.index_mod_at("file:///deps/calc/src/math.rut", "calc", "math", math_src);
    let uri = "file:///t/main.rut";
    let doc = "entry fn main() -> f64 {\n    return calc::math::sqrt(4.0);\n}\n";
    let hits = s.def_in(uri, doc, "calc::math::sqrt", 1, 13);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///deps/calc/src/math.rut");
    assert_target(math_src, &hits[0], "sqrt(x", 1, 4, "the dep module's ident");
    // the type position: the type ladder takes no dep head
    let doc2 = "entry fn main() -> i32 {\n    let a: calc::math::Angle = nil;\n    return 0;\n}\n";
    let hits = s.def_in(uri, doc2, "calc::math::Angle", 1, 14);
    assert!(hits.is_empty(), "no type jump through a dep head: {hits:?}");
    // the non-pub leaf: no jump
    let doc3 = "entry fn main() -> i32 {\n    return calc::math::hidden();\n}\n";
    let hits = s.def_in(uri, doc3, "calc::math::hidden", 1, 13);
    assert!(hits.is_empty(), "a non-pub leaf is no jump: {hits:?}");
    // the flat dep: no root, no jump
    s.index_mod_at(
        "file:///deps/flat/src/lib.rut",
        "flatdep",
        "",
        "pub fn top() -> i32 { return 10; }\n",
    );
    let flatdoc = "entry fn main() -> i32 {\n    return flatdep::top();\n}\n";
    let hits = s.def_in("file:///t/flat.rut", flatdoc, "flatdep::top", 1, 10);
    assert!(hits.is_empty(), "a flat dep roots nothing: {hits:?}");
}
}

both_uri_shapes! {
#[test]
fn qualified_annotation_receiver_resolves_members() {
    // the member face of the receiver-inference law: the annotated
    // qualified shape's `.bump` jumps to exactly the module's impl fn
    let mut s = S::boot();
    let m_src = "pub struct Counter {\n    n: i32;\n}\nimpl Counter {\n    pub fn bump(self) -> i32 { return self.n + 1; }\n}\npub fn mk(n: i32) -> Counter { return Counter { n: n }; }\n";
    s.index_mod_at("file:///ws/pkg/src/m.rut", "pkg", "m", m_src);
    let doc = "mod m;\nentry fn main() -> i32 {\n    let c: m::Counter = m::mk(41);\n    return c.bump();\n}\n";
    let uri = "file:///ws/pkg/src/mod.rut";
    s.index_mod_at(uri, "pkg", "", doc);
    let hits = s.def_in(uri, doc, "c.bump();", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///ws/pkg/src/m.rut");
    assert_target(m_src, &hits[0], "bump(self)", 1, 4, "the module's impl fn");
    // the let cell behind a dep head: no jump (module lets cross no
    // package boundary qualified)
    let math_src = "pub fn sqrt(x: f64) -> f64 { return x; }\npub let PI2: f64 = 6.28;\n";
    s.index_mod_at(
        "file:///deps/calc/src/mod.rut",
        "calc",
        "",
        "pub mod math;\n",
    );
    s.index_mod_at("file:///deps/calc/src/math.rut", "calc", "math", math_src);
    let doc3 = "entry fn main() -> f64 {\n    return calc::math::PI2;\n}\n";
    let hits = s.def_in(uri, doc3, "calc::math::PI2", 1, 13);
    assert!(
        hits.is_empty(),
        "a module let is no jump behind a dep head: {hits:?}"
    );
}
}

// ---- round 4: the rut-new layout — the root landing and the
//      intra-package use edge, through the extension's own mounts ----

/// one jump target: `uri`, and exactly the DECLARED NAME's span (the
/// last non-keyword word of `needle`) over the TARGET file's own text
fn assert_jump(hits: &[Value], uri: &str, target: &str, needle: &str, occ: usize, what: &str) {
    assert_eq!(hits.len(), 1, "{what}: one target, exactly: {hits:?}");
    assert_eq!(hits[0]["uri"], uri, "{what}: {}", hits[0]["uri"]);
    const KW: [&str; 8] = ["pub", "fn", "let", "struct", "mod", "class", "impl", "pkg"];
    let (off, len) = needle
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .filter(|w| !KW.contains(w))
        .map(|w| (needle.rfind(w).expect("the word is in the needle"), w.len()))
        .next_back()
        .expect("the needle names a declaration");
    let want = byte_of(target, needle, occ) as usize + off;
    assert_eq!(
        span_of(target, &hits[0]),
        (want, want + len),
        "{what}: the declared name: {hits:?}"
    );
}

both_uri_shapes! {
    #[test]
    fn the_own_pkg_head_from_a_child_jumps_into_the_root_module() {
        // DEFECT 1: `app::LIMIT` from a child — the root's own decl,
        // never the open document (whose uri names the package)
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let root = rut_new::at("src/mod.rut");
        let hits = s.def_in(&uri, "fn k() -> i32 { return app::LIMIT; }\n", "app::LIMIT", 1, 6);
        assert_jump(&hits, &root, rut_new::ROOT, "pub let LIMIT", 1, "the root's let");
        let hits = s.def_in(
            &uri,
            "fn k() -> i32 { return app::m::mk(41).bump(); }\n",
            "app::m::mk",
            1,
            8,
        );
        assert_jump(
            &hits,
            &rut_new::at("src/m/mod.rut"),
            rut_new::M,
            "pub fn mk(",
            1,
            "the module behind the head",
        );
        // the head segment is the package's root module — its file top
        let hits = s.def_in(
            &uri,
            "fn k() -> i32 { return app::m::mk(41).bump(); }\n",
            "app::m::mk",
            1,
            1,
        );
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0]["uri"], root, "{hits:?}");
        assert_eq!(
            (hits[0]["range"]["start"]["line"].as_u64(), hits[0]["range"]["start"]["character"].as_u64()),
            (Some(0), Some(0)),
            "the root file top: {hits:?}"
        );
        // the next segment is the ROOT's `mod m;` edge
        let hits = s.def_in(
            &uri,
            "fn k() -> i32 { return app::m::mk(41).bump(); }\n",
            "app::m::mk",
            1,
            6,
        );
        assert_jump(&hits, &root, rut_new::ROOT, "mod m;", 1, "the mounting edge");
        // the compiler refuses these: a private leaf of m from outside
        // its subtree, the root's re-export row
        for (src, needle, off) in [
            ("fn k() -> i32 { return app::m::secret(); }\n", "app::m::secret", 8),
            ("fn k() -> i32 { return app::mk(41).bump(); }\n", "app::mk", 6),
        ] {
            let hits = s.def_in(&uri, src, needle, 1, off);
            assert!(hits.is_empty(), "[{needle:?}] a refused leaf jumps nowhere: {hits:?}");
        }
    }
}

both_uri_shapes! {
    #[test]
    fn a_root_declared_edge_used_from_a_child_jumps() {
        // DEFECT 1: `mod m;` declared by the ROOT, used from a child
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let m = rut_new::at("src/m/mod.rut");
        let hits = s.def_in(&uri, "fn k() -> i32 { return m::mk(41).bump(); }\n", "m::mk", 1, 3);
        assert_jump(&hits, &m, rut_new::M, "pub fn mk(", 1, "the leaf");
        // the nested module, and the segments that name modules
        let src = "fn k() -> i32 { return m::grid::cell(); }\n";
        let hits = s.def_in(&uri, src, "m::grid::cell", 1, 9);
        assert_jump(
            &hits,
            &rut_new::at("src/m/grid/mod.rut"),
            rut_new::GRID,
            "pub fn cell(",
            1,
            "the nested leaf",
        );
        let hits = s.def_in(&uri, src, "m::grid::cell", 1, 0);
        assert_jump(
            &hits,
            &rut_new::at("src/mod.rut"),
            rut_new::ROOT,
            "mod m;",
            1,
            "the head: the ROOT's edge",
        );
        let hits = s.def_in(&uri, src, "m::grid::cell", 1, 3);
        assert_jump(&hits, &m, rut_new::M, "pub mod grid;", 1, "the mid segment: m's edge");
        // ... from a grandchild, and the ancestor itself through the
        // root's edge
        let sub = rut_new::at("src/child/sub/mod.rut");
        // one world: the child's buffer is the truth, and this test's
        // earlier probes synced other probe texts into it — its own
        // text rides again before the grandchild names it (the old
        // answer read the indexed copy behind the open buffer)
        s.analyze(&uri, rut_new::CHILD_HEAD);
        assert_jump(
            &s.def_in(&sub, "fn k() -> i32 { return m::mk(41).bump(); }\n", "m::mk", 1, 3),
            &m,
            rut_new::M,
            "pub fn mk(",
            1,
            "the root's edge from a grandchild",
        );
        assert_jump(
            &s.def_in(&sub, "fn k() -> i32 { return child::kid(); }\n", "child::kid", 1, 8),
            &rut_new::at("src/child/mod.rut"),
            rut_new::CHILD_HEAD,
            "pub fn kid(",
            1,
            "the ancestor itself",
        );
        // a private leaf jumps nowhere; the `pub use` row (`m::G`) is a
        // reached DECLARATION now — law 5's walk lands it in grid
        let hits = s.def_in(&uri, "fn k() -> i32 { return m::secret(); }\n", "m::secret", 1, 3);
        assert!(hits.is_empty(), "[m::secret]: {hits:?}");
        let hits = s.def_in(&uri, "fn k() -> i32 { return m::G; }\n", "m::G", 1, 3);
        assert_jump(
            &hits,
            &rut_new::at("src/m/grid/mod.rut"),
            rut_new::GRID,
            "pub let G",
            1,
            "the re-export row's target",
        );
    }
}

both_uri_shapes! {
    #[test]
    fn an_intra_package_use_edge_jumps_through_the_module_lane() {
        // DEFECT 2: the file scope's use tier answers the doc's own
        // package edge — the call site AND the use leaf, for a fn, a
        // type, a module let, a nested path, and the own-package name
        // as the head
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let m = rut_new::at("src/m/mod.rut");
        let grid = rut_new::at("src/m/grid/mod.rut");
        let root = rut_new::at("src/mod.rut");
        let src = "use m::{ mk };\nfn k() -> i32 { return mk(41).bump(); }\n";
        assert_jump(&s.def_in(&uri, src, "mk(41)", 1, 0), &m, rut_new::M, "pub fn mk(", 1, "the call site");
        assert_jump(&s.def_in(&uri, src, "{ mk }", 1, 2), &m, rut_new::M, "pub fn mk(", 1, "the use leaf");
        let src = "use m::{ Counter };\nfn k(c: Counter) -> i32 { return c.bump(); }\n";
        assert_jump(
            &s.def_in(&uri, src, "c: Counter", 1, 3),
            &m,
            rut_new::M,
            "pub struct Counter",
            1,
            "the type at the annotation",
        );
        assert_jump(&s.def_in(&uri, src, "{ Counter }", 1, 2), &m, rut_new::M, "pub struct Counter", 1, "the type's leaf");
        assert_jump(&s.def_in(&uri, src, ".bump()", 1, 1), &m, rut_new::M, "pub fn bump(", 1, "the member behind the use-bound type");
        let src = "use m::{ STEP };\nfn k() -> i32 { return STEP; }\n";
        assert_jump(&s.def_in(&uri, src, "return STEP", 1, 7), &m, rut_new::M, "pub let STEP", 1, "the module let");
        let src = "use m::grid::{ cell };\nfn k() -> i32 { return cell(); }\n";
        assert_jump(&s.def_in(&uri, src, "cell()", 1, 0), &grid, rut_new::GRID, "pub fn cell(", 1, "the nested path's leaf");
        assert_jump(&s.def_in(&uri, src, "m::grid:", 1, 3), &m, rut_new::M, "pub mod grid;", 1, "the use path's segment");
        let src = "use app::{ LIMIT };\nfn k() -> i32 { return LIMIT; }\n";
        assert_jump(
            &s.def_in(&uri, src, "return LIMIT", 1, 7),
            &root,
            rut_new::ROOT,
            "pub let LIMIT",
            1,
            "the own-pkg head",
        );
        // the pkg head segment of the use path: the root file top
        let hits = s.def_in(&uri, src, "use app:", 1, 4);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0]["uri"], root, "{}", hits[0]["uri"]);
        assert_eq!(hits[0]["range"]["start"]["line"], 0, "the file top: {hits:?}");
    }
}

both_uri_shapes! {
    #[test]
    fn a_use_leaf_the_compiler_refuses_jumps_nowhere() {
        // each a use line the compiler refuses — a private of the
        // module, its own private import, a dep's private fn: no jump
        // at the call site and none on the leaf (import is no
        // visibility bypass)
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        for (src, needles) in [
            ("use m::{ secret };\nfn k() -> i32 { return secret(); }\n", vec![("secret()", 0), ("{ secret }", 2)]),
            ("use m::{ cell };\nfn k() -> i32 { return cell(); }\n", vec![("cell()", 0), ("{ cell }", 2)]),
            ("use m::*;\nfn k() -> i32 { return mk(41); }\n", vec![("mk(41)", 0)]),
            ("use calc::math::{ hidden };\nfn k() -> i32 { return 0; }\n", vec![("{ hidden }", 2)]),
            ("use calc::math::{ cell };\nfn k() -> i32 { return 0; }\n", vec![("{ cell }", 2)]),
        ] {
            for (needle, off) in needles {
                let hits = s.def_in(&uri, src, needle, 1, off);
                assert!(hits.is_empty(), "[{src:?} @ {needle:?}] a refused leaf: {hits:?}");
            }
        }
    }
}

both_uri_shapes! {
    #[test]
    fn a_re_exported_leaf_and_its_qualified_spelling_jump_to_the_declaration() {
        // the own lane's `pub use` row NAMES its target: `m` re-exports
        // `grid::G`, so the leaf (`use m::{ G }`), its spelling on the
        // use line, and the qualified spelling (`m::G`) all jump to
        // the DECLARATION in grid — one walk, one answer.
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let grid = rut_new::at("src/m/grid/mod.rut");
        let leaf = "use m::{ G };\nfn k() -> i32 { return G; }\n";
        assert_jump(
            &s.def_in(&uri, leaf, "return G", 1, 7),
            &grid,
            rut_new::GRID,
            "pub let G",
            1,
            "the re-exported leaf's use site",
        );
        assert_jump(
            &s.def_in(&uri, leaf, "{ G }", 1, 2),
            &grid,
            rut_new::GRID,
            "pub let G",
            1,
            "the leaf's spelling",
        );
        let qualified = "fn k() -> i32 { return m::G; }\n";
        assert_jump(
            &s.def_in(&uri, qualified, "m::G", 1, 3),
            &grid,
            rut_new::GRID,
            "pub let G",
            1,
            "the qualified spelling",
        );
    }
}

both_uri_shapes! {
    #[test]
    fn a_twice_bound_name_jumps_nowhere() {
        // the one-name shape — every registry kind of it: a use site
        // of a name the file binds TWICE by distinct bindings jumps
        // NOWHERE (never a pick between the bindings; the hover lists
        // the candidates), and every decl SITE stays exact (itself)
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        // two use leaves of one name (same-named decls in two
        // packages are legal; the second leaf refuses)
        let math = format!(
            "{}pub fn mk(n: i32) -> Calc {{ return Calc {{ d: n }}; }}\n",
            rut_new::CALC_MATH
        );
        s.index_mod_at(&rut_new::calc("src/math/mod.rut"), "calc", "math", &math);
        let src = "use m::{ mk };\nuse calc::math::{ mk };\nfn k() -> i32 { return mk(41).bump(); }\n";
        for (needle, occ) in [("mk(41)", 1), ("{ mk }", 1), ("{ mk }", 2)] {
            let off = if needle == "mk(41)" { 0 } else { 2 };
            let hits = s.def_in(&uri, src, needle, occ, off);
            assert!(hits.is_empty(), "[{needle} #{occ}] a twice-bound name: {hits:?}");
        }
        // an own decl beside a leaf
        let src = "use m::{ mk };\nfn mk(n: i32) -> i32 { return n; }\nfn k() -> i32 { return mk(41); }\n";
        assert!(
            s.def_in(&uri, src, "mk(41)", 1, 0).is_empty(),
            "an own decl beside a leaf: {:?}",
            s.def_in(&uri, src, "mk(41)", 1, 0)
        );
        let hits = s.def_in(&uri, src, "fn mk(", 1, 3);
        assert_eq!(hits.len(), 1, "the decl itself: {hits:?}");
        assert_target(src, &hits[0], "mk", 2, 2, "the decl site is exact");
        // two own decls — a fn and a class
        let src = "fn mk(n: i32) -> i32 { return n; }\nclass mk {\n    n: i32,\n}\nfn k() -> i32 { return mk(41); }\n";
        assert!(
            s.def_in(&uri, src, "mk(41)", 1, 0).is_empty(),
            "two own decls: {:?}",
            s.def_in(&uri, src, "mk(41)", 1, 0)
        );
        let hits = s.def_in(&uri, src, "fn mk(", 1, 3);
        assert_target(src, &hits[0], "mk", 1, 2, "the fn's own site");
        let hits = s.def_in(&uri, src, "class mk", 1, 6);
        assert_eq!(hits.len(), 1, "the class's own site: {hits:?}");
        assert_target(src, &hits[0], "mk", 2, 2, "the class's own site");
        // a type beside a leaf: the type's own site is itself, a
        // spelling of the name (typeDefinition too) jumps nowhere
        let src = "class Counter {\n    n: i32,\n}\nuse m::{ Counter };\nfn k(c: Counter) -> i32 { return c.bump(); }\n";
        let hits = s.def_in(&uri, src, "class Counter", 1, 6);
        assert_eq!(hits.len(), 1, "the type's own site: {hits:?}");
        assert_target(src, &hits[0], "Counter", 1, 7, "the type's own site is exact");
        assert!(s.def_in(&uri, src, "c: Counter", 1, 3).is_empty(), "a use site: ");
        assert!(s.typedef_in(&uri, src, "c: Counter", 1, 3).is_empty(), "typeDefinition never picks either");
        assert!(s.typedef_in(&uri, src, "{ Counter }", 1, 2).is_empty(), "the leaf too");
        let hits = s.typedef_in(&uri, src, "class Counter", 1, 6);
        assert_target(src, &hits[0], "Counter", 1, 7, "typeDefinition at the type's own site");
    }
}

both_uri_shapes! {
    #[test]
    fn members_are_silent_when_the_annotations_candidates_differ() {
        // an own `class Counter` beside a `use m::{ Counter }` leaf:
        // the file binds the name twice by DISTINCT declarations, so
        // the annotation's candidates differ — the asking file's own
        // declaration is a candidate like any other, and no member
        // picks between them (hover, definition, completion alike)
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let src = "class Counter {\n    n: i32,\n}\nuse m::{ Counter };\nfn k(c: Counter) -> i32 { return c.bump(); }\n";
        let hits = s.def_in(&uri, src, "c.bump", 1, 2);
        assert!(hits.is_empty(), "no pick between the candidates: {hits:?}");
        let h = s.hover_in(&uri, src, "c.bump", 1, 2);
        assert!(no_hover(&h), "no member hover: {h}");
        let items = s.complete_at(&uri, src, "c.", 1);
        assert!(
            !offers(&items, "bump"),
            "no member row: {:?}",
            labels(&items)
        );
    }
}

both_uri_shapes! {
    #[test]
    fn the_receiver_family_behind_a_root_declared_edge_jumps() {
        // the annotated, the inferred, the chained, and the field-typed
        // receiver — the member's decl, through the lane
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let m = rut_new::at("src/m/mod.rut");
        for src in [
            "fn k() -> i32 { let c: m::Counter = m::mk(41); return c.bump(); }\n",
            "fn k() -> i32 { let c = m::mk(41); return c.bump(); }\n",
            "fn k() -> i32 { return m::mk(41).bump(); }\n",
            "struct Holder { c: m::Counter }\nfn k() -> i32 { let h = Holder { c: m::mk(1) }; return h.c.bump(); }\n",
        ] {
            assert_jump(&s.def_in(&uri, src, ".bump()", 1, 1), &m, rut_new::M, "pub fn bump(", 1, "the member");
        }
    }
}

both_uri_shapes! {
    #[test]
    fn a_dep_roots_own_member_jumps_nowhere() {
        // `calc::ctop()` refuses ("unknown name `calc::ctop`"): the
        // pkg-rooted name roots INSIDE the dep's module tree only
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let hits = s.def_in(&uri, "fn k() -> i32 { return calc::ctop(); }\n", "calc::ctop", 1, 6);
        assert!(hits.is_empty(), "the dep root's own member: {hits:?}");
        let hits = s.def_in(
            &uri,
            "fn k() -> i32 { return calc::math::cmk(1).deg(); }\n",
            "calc::math::cmk",
            1,
            12,
        );
        assert_jump(
            &hits,
            &rut_new::calc_uri("src/math/mod.rut"),
            rut_new::CALC_MATH,
            "pub fn cmk(",
            1,
            "the tree path answers",
        );
    }
}

both_uri_shapes! {
    #[test]
    fn a_module_leaf_use_opens_the_module() {
        // `use calc::{ math }` — the leaf names a module: its file top
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let src = "use calc::{ math };\nfn k() -> i32 { return math::cmk(1).deg(); }\n";
        let hits = s.def_in(&uri, src, "{ math }", 1, 2);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0]["uri"], rut_new::calc_uri("src/math/mod.rut"), "{}", hits[0]["uri"]);
        assert_eq!(hits[0]["range"]["start"]["line"], 0, "the module's file top: {hits:?}");
    }
}

both_uri_shapes! {
    #[test]
    fn two_lanes_mounting_one_file_answer_one_target() {
        // the indexer's stamp and the walk's rooted string are ONE
        // file in the store — a jump is never doubled
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let src = "use app::m::{ mk };\nfn k() -> i32 { return mk(41).bump(); }\n";
        assert_jump(
            &s.def_in(&uri, src, "mk(41)", 1, 0),
            &rut_new::at("src/m/mod.rut"),
            rut_new::M,
            "pub fn mk(",
            1,
            "one target for the twice-mounted file",
        );
        let src = "use app::{ LIMIT };\nfn k() -> i32 { return LIMIT; }\n";
        assert_jump(
            &s.def_in(&uri, src, "return LIMIT", 1, 7),
            &rut_new::at("src/mod.rut"),
            rut_new::ROOT,
            "pub let LIMIT",
            1,
            "the root, once",
        );
    }
}

#[test]
fn definition_at_a_tail_s_end_byte_still_rides_the_tail() {
    // DELTA (Act 4 — the classification probes one node back at the
    // same caret): the byte after a qualified tail's last char sits in
    // the statement's own span, so the base tree's probe answered
    // nothing there (RED on base: `[]`); the caret is still ON the
    // tail — the member law answers it, like every mid-tail byte.
    let mut s = S::boot();
    let uri = "file:///t/main.rut";
    let src = "enum Color { Red, Green }\nentry fn main() -> nil {\n    let c = Color::Red;\n}\n";
    let hits = s.def_in(uri, src, "Color::Red", 1, 10);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], uri, "{hits:?}");
    // the enum member's own decl site (line 0, `Red` after `{ `)
    assert_eq!(pos_pair(&hits[0]["range"]["start"]), (0, 13), "{hits:?}");
}

#[test]
fn fn_use_caret_past_the_name_still_answers_the_fn() {
    // END-OF-NAME CARET (Act 4 review): the caret one byte past a
    // callee's last character — `helper(|)` — rides the callee: hover
    // renders it, definition jumps to it, references count it. The
    // base probe answered null/[]/[] there (the byte's node was the
    // enclosing stmt).
    let mut s = S::boot();
    let uri = "file:///t/main.rut";
    let src = "fn helper() -> i32 {\n    return 1;\n}\nentry fn main() -> nil {\n    let x = helper();\n}\n";
    s.analyze(uri, src);
    // the caret past the use-site `helper` (the `(` that follows)
    let (l, c) = S::pos_at_byte(src, byte_of(src, "helper", 2) + 6);
    let md = markdown(s.hover_pos(uri, l, c));
    assert!(
        md.contains("fn helper() -> i32") && md.contains("main.rut:1"),
        "{md}"
    );
    let hits = s.def_pos(uri, l, c);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(pos_pair(&hits[0]["range"]["start"]), (0, 3), "{hits:?}");
    let refs = s.refs_pos(uri, (l, c), true);
    assert_eq!(refs.len(), 2, "{refs:?}");
    assert_eq!(pos_pair(&refs[0]["range"]["start"]), (0, 3), "{refs:?}");
    assert_eq!(pos_pair(&refs[1]["range"]["start"]), (4, 12), "{refs:?}");
}

#[test]
fn local_use_caret_past_the_name_still_answers_the_binding() {
    // END-OF-NAME CARET (Act 4 review): the caret one byte past a
    // local's last character — `return n|;` — rides the binding:
    // hover renders the parameter, definition jumps to its decl,
    // references count it. The base probe answered null/[]/[] there.
    let mut s = S::boot();
    let uri = "file:///t/main.rut";
    let src = "fn go(n: i32) -> i32 {\n    return n;\n}\n";
    s.analyze(uri, src);
    // the caret past the returned `n` (the `;` that follows)
    let (l, c) = S::pos_at_byte(src, byte_of(src, "return n", 1) + 8);
    let md = markdown(s.hover_pos(uri, l, c));
    assert!(md.contains("n: i32") && md.contains("parameter"), "{md}");
    let hits = s.def_pos(uri, l, c);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(pos_pair(&hits[0]["range"]["start"]), (0, 6), "{hits:?}");
    let refs = s.refs_pos(uri, (l, c), true);
    assert_eq!(refs.len(), 2, "{refs:?}");
    assert_eq!(pos_pair(&refs[0]["range"]["start"]), (0, 6), "{refs:?}");
    assert_eq!(pos_pair(&refs[1]["range"]["start"]), (1, 11), "{refs:?}");
}

#[test]
fn receiver_caret_at_the_dot_answers_the_receiver() {
    // END-OF-NAME CARET (Act 4 review): the dot that ends a receiver
    // sits at the receiver's own end byte — `p.|bump()` — and the
    // caret reads the RECEIVER (the single-segment Name arm carries
    // no position check of its own): hover renders the binding,
    // definition jumps to its decl, references count it,
    // typeDefinition names the receiver's type. The base probe
    // answered null/[]/[]/[] there.
    let mut s = S::boot();
    let uri = "file:///t/main.rut";
    let src = "class Box2 {\n    n: i32;\n}\nimpl Box2 {\n    fn bump(self) -> i32 { return self.n; }\n}\nfn go() -> i32 {\n    let p = Box2 { n: 1 };\n    return p.bump();\n}\n";
    s.analyze(uri, src);
    // the caret at `p`'s end byte — the DOT itself
    let (l, c) = S::pos_at_byte(src, byte_of(src, "p.bump", 1) + 1);
    let md = markdown(s.hover_pos(uri, l, c));
    assert!(md.contains("let p: Box2"), "{md}");
    let hits = s.def_pos(uri, l, c);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(pos_pair(&hits[0]["range"]["start"]), (7, 8), "{hits:?}");
    let refs = s.refs_pos(uri, (l, c), true);
    assert_eq!(refs.len(), 2, "{refs:?}");
    assert_eq!(pos_pair(&refs[0]["range"]["start"]), (7, 8), "{refs:?}");
    assert_eq!(pos_pair(&refs[1]["range"]["start"]), (8, 11), "{refs:?}");
    let tys = s.typedef_in(uri, src, "p.bump", 1, 1);
    assert_eq!(tys.len(), 1, "{tys:?}");
    assert_eq!(pos_pair(&tys[0]["range"]["start"]), (0, 6), "{tys:?}");
}

both_uri_shapes! {
#[test]
fn struct_lit_tail_caret_past_the_name_typedefs_the_type() {
    // END-OF-NAME CARET (Act 4 review): the caret one byte past a
    // struct-literal's qualified tail — `gadgets::Widget| { .. }` —
    // rides the tail: typeDefinition names the type. The base probe
    // answered [] there.
    let mut s = S::boot();
    s.index_mod_at(WIDGET_URI, "gadgets", "", WIDGET_SRC);
    let uri = "file:///t/main.rut";
    let src = "use gadgets::{ Widget };\nentry fn main() -> nil {\n    let w = gadgets::Widget { id: 1 };\n}\n";
    s.analyze(uri, src);
    // the caret past the literal's tail (the ` {` that follows)
    let (l, c) = S::pos_at_byte(src, byte_of(src, "gadgets::Widget", 1) + 15);
    let tys = s.typedef_pos(uri, l, c);
    assert_eq!(tys.len(), 1, "{tys:?}");
    assert_eq!(tys[0]["uri"], WIDGET_URI, "{tys:?}");
    assert_eq!(pos_pair(&tys[0]["range"]["start"]), (0, 10), "{tys:?}");
}
}

both_uri_shapes! {
#[test]
fn a_re_exported_name_across_packages_jumps_to_the_declaration() {
    // law 3: a `pub use` of a DEPENDENCY name is a surface row of the
    // re-exporter. The consumer's `use mid::{ helper }` binds it, so
    // both the call site and the use leaf jump to the DECLARING
    // package's declaration (the same answer the compiler's binding
    // lands on), flat and through a walked module.
    const UTIL_SRC: &str = "pub fn helper() -> i32 { return 3; }\n";
    let mut s = S::boot();
    s.index_mod_at(
        "file:///deps/util/src/mod.rut",
        "util",
        "",
        UTIL_SRC,
    );
    s.index_mod_at(
        "file:///deps/mid/src/mod.rut",
        "mid",
        "",
        "pub mod sub;\npub use util::{ helper };\n",
    );
    s.index_mod_at(
        "file:///deps/mid/src/sub/mod.rut",
        "mid",
        "sub",
        "pub use util::{ helper };\n",
    );
    let uri = "file:///t/main.rut";
    // the flat leaf: the call site and the leaf both land in util
    let flat = "use mid::{ helper };\nfn k() -> i32 { return helper(); }\n";
    assert_jump(
        &s.def_in(uri, flat, "return helper", 1, 7),
        "file:///deps/util/src/mod.rut",
        UTIL_SRC,
        "pub fn helper",
        1,
        "the flat leaf's call site",
    );
    assert_jump(
        &s.def_in(uri, flat, "{ helper }", 1, 2),
        "file:///deps/util/src/mod.rut",
        UTIL_SRC,
        "pub fn helper",
        1,
        "the flat leaf's spelling",
    );
    // the walked module's column
    let walked = "use mid::sub::{ helper };\nfn k() -> i32 { return helper(); }\n";
    assert_jump(
        &s.def_in(uri, walked, "return helper", 1, 7),
        "file:///deps/util/src/mod.rut",
        UTIL_SRC,
        "pub fn helper",
        1,
        "the walked leaf's call site",
    );
}
}

#[test]
fn an_own_module_reexport_of_a_dependency_name_jumps_across() {
    // law 3's own-lane hand-off: `b` (a module of app) re-exports a
    // DEPENDENCY's name; the root's `use b::{ helper }` binds the
    // dependency's surface row, so the jump lands on util's decl — the
    // same answer the compiler's synthetic surface binding lands on.
    const UTIL_SRC: &str = "pub fn helper() -> i32 { return 3; }\n";
    let mut s = S::boot();
    s.index_mod_at("file:///deps/util/src/mod.rut", "util", "", UTIL_SRC);
    let root = "file:///deps/app/mod.rut";
    let src = "mod b;\nuse b::{ helper };\nfn k() -> i32 { return helper(); }\n";
    s.index_mod_at(root, "app", "", src);
    s.index_mod_at(
        "file:///deps/app/b.rut",
        "app",
        "b",
        "pub use util::{ helper };\n",
    );
    assert_jump(
        &s.def_in(root, src, "return helper", 1, 7),
        "file:///deps/util/src/mod.rut",
        UTIL_SRC,
        "pub fn helper",
        1,
        "the own-lane foreign edge's call site",
    );
    assert_jump(
        &s.def_in(root, src, "{ helper }", 1, 2),
        "file:///deps/util/src/mod.rut",
        UTIL_SRC,
        "pub fn helper",
        1,
        "the leaf's spelling",
    );
}

both_uri_shapes! {
#[test]
fn a_package_private_re_export_across_packages_jumps_nowhere() {
    // HIGH 2's mirror: only a full `pub use` crosses a package
    // boundary. A `pub(pkg)` re-export stays inside its package (law
    // 2's edge), so a foreign consumer's leaf binds nothing — the same
    // answer the compiler's must-bind gives ("mark it `pub`").
    const UTIL_SRC: &str = "pub fn helper() -> i32 { return 3; }\n";
    let mut s = S::boot();
    s.index_mod_at("file:///deps/util/src/mod.rut", "util", "", UTIL_SRC);
    s.index_mod_at(
        "file:///deps/mid/src/mod.rut",
        "mid",
        "",
        "pub(pkg) use util::{ helper };\n",
    );
    let uri = "file:///t/main.rut";
    let src = "use mid::{ helper };\nfn k() -> i32 { return helper(); }\n";
    assert!(
        s.def_in(uri, src, "return helper", 1, 7).is_empty(),
        "a package-private re-export crosses no package boundary"
    );
    assert!(
        s.def_in(uri, src, "{ helper }", 1, 2).is_empty(),
        "the leaf's spelling binds nothing either"
    );
}
}

// ---- phase 6 task A: one declaration or nothing (F40) ----

both_uri_shapes! {
#[test]
fn a_deps_module_let_jumps_nowhere() {
    // law 6 / F40's definition face: the compiler refuses a module let
    // across a package surface, so go-to-definition lands nowhere —
    // while the same module's `pub` fn (the control) does land
    let (mut s, _io) = rut_new::boot();
    let uri = rut_new::at("src/child/mod.rut");
    let src = "use calc::math::{ CPI };\nfn k() -> f64 { return CPI; }\n";
    let hits = s.def_at(&uri, src, "CPI", 2);
    assert!(
        hits.is_empty(),
        "a dep's module let jumps nowhere: {hits:?}"
    );
    let hits = s.def_at(
        &uri,
        "use calc::math::{ cmk };\nfn k() -> i32 { return cmk(1).deg(); }\n",
        "cmk",
        2,
    );
    assert_eq!(hits.len(), 1, "a dep's pub fn lands: {hits:?}");
}
}
