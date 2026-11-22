//! Definition's gate — the four layers (binding pass → decl layer →
//! use graph → the mounted world) plus typeDefinition, driven through
//! the `definition`/`typeDefinition` envelopes and the workspace-scan
//! push lane. Pins: the shadow-aware targets, the body-over-decl
//! preference, the mounted-over-ambient law, the own-package lanes
//! keyed on the stamped twin, the module matcher's derivation, and the
//! misses that stay misses.

mod common;

use common::*;
use serde_json::Value;

const WIDGET_SRC: &str = "class Widget {\n    id: i32;\n}\n";
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
fn unimported_names_keep_the_flat_chain() {
    let mut s = S::boot();
    s.index_file(WIDGET_URI, WIDGET_SRC);
    let src = "entry fn main() -> nil {\n    let w = Widget::new();\n}\n";
    let hits = s.def_in("file:///t/main.rut", src, "= Widget::new();", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], WIDGET_URI);
}

// ---- the pkg's two files: the body over the decl surface ----

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
fn a_doubled_twin_answer_is_one_answer() {
    // an open doc is indexed by its analyze lane AND stamped by the
    // scan lane at the same origin — a name both carry resolved twice.
    // Identical answers are one answer; the ambiguity peek is for
    // DISTINCT targets
    let mut s = S::boot();
    let src = "class Widget {\n    id: i32;\n}\n";
    s.index_file("file:///t/main.rut", src); // the stamped twin
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

#[test]
fn a_pkg_head_with_no_indexed_pkg_is_an_honest_miss() {
    let mut s = S::boot();
    let src = "use ghost::{ Thing };\nentry fn main() -> nil {\n}\n";
    let hits = s.def_at("file:///t/main.rut", src, "use ghost::", 1);
    assert!(hits.is_empty(), "{hits:?}");
}

// ---- the own-package lanes (the stamped twin keys them) ----

const APP_ROOT: &str = "mod helpers;\nentry fn main() -> nil {\n}\n";
const APP_URI: &str = "file:///ws/app/mod.rut";
const HELPERS_SRC: &str = "pub struct Slot {\n    n: i32;\n}\n";
const HELPERS_URI: &str = "file:///ws/app/helpers/mod.rut";

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

#[test]
fn an_edge_click_on_an_unstamped_doc_is_a_miss() {
    // no stamped twin — the own lanes stay off
    let mut s = S::boot();
    s.index_mod_at(HELPERS_URI, "app", "helpers", HELPERS_SRC);
    let hits = s.def_at("file:///ws/scratch.rut", APP_ROOT, "mod helpers;", 1);
    assert!(hits.is_empty(), "{hits:?}");
}

#[test]
fn an_edge_click_on_an_unmounted_child_is_a_miss() {
    // the doc is stamped but the chain carries no index at the child's
    // mod path — an empty jump, never a wrong one
    let mut s = S::boot();
    s.index_mod_at(APP_URI, "app", "", APP_ROOT);
    let hits = s.def_at(APP_URI, APP_ROOT, "mod helpers;", 1);
    assert!(hits.is_empty(), "{hits:?}");
}

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

#[test]
fn an_own_pkg_use_path_resolves_intra_package() {
    // `use app::helpers::{ Slot };` in the app root — the pkg-name
    // head opens the root module (the doc itself) at its top, the
    // segment rides the mounting edge, the leaf lands in the child
    let mut s = S::boot();
    let doc = "mod helpers;\nuse app::helpers::{ Slot };\nentry fn main() -> nil {\n}\n";
    let uri = APP_URI;
    s.index_mod_at(uri, "app", "", doc);
    // the lagging on-disk twin (the doc's own rows must win)
    s.index_mod_at(
        "file:///ws/app/on-disk/mod.rut",
        "app",
        "",
        "mod helpers;\n",
    );
    s.index_mod_at(HELPERS_URI, "app", "helpers", HELPERS_SRC);
    // the pkg-name head: the doc's own file top (never the twin)
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

// ---- the module matcher (the explicit name, then the derivation) ----

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

#[test]
fn the_unnamed_fallback_stays_the_path_derivation() {
    // an unnamed workspace index: the path segment (stem stripped)
    // resolves — `…/gadgets/lib.rut` speaks for `gadgets`,
    // `…/pouch.rut` for `pouch`
    let mut s = S::boot();
    s.index_file(WIDGET_URI, WIDGET_SRC);
    s.index_file("file:///ws/pouch.rut", "class Bag {\n    n: i32;\n}\n");
    let src = "use gadgets::{ Widget };\nfn main() -> nil {\n    let w = Widget::new();\n}\n";
    let hits = s.def_in("file:///t/main.rut", src, "= Widget::new();", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    let src = "use pouch::{ Bag };\nfn main() -> nil {\n    let b = Bag::new();\n}\n";
    let hits = s.def_in("file:///t/main2.rut", src, "= Bag::new();", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///ws/pouch.rut");
}

// ---- the stdlib dep lane ----

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
