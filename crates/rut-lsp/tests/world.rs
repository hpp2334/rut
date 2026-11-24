//! The world's seams, pinned through the public door. Each test records the
//! answer the one-world refactor must preserve, or the answer a named act
//! flips (that act keeps the scenario, replaces the assertion, renames the test).
//!
//! Act 1 (one world) flipped five of the Phase 0 pins in place; the other
//! six record answers that survive the act. The UNSTAMPED cross-package
//! lanes are uri-shape-keyed: under a project dir whose name never spells
//! the package they answer nothing (an `index_file` push carries no
//! stamp.module, and the pkg-name derivation has nothing to grip) — those
//! pins ride `both_uri_shapes!` and record both answers, verbatim.
mod common;
use common::*;

/// is the running shape the one whose project dirs spell the package name?
/// (`spelled` is the identity there; the hiding shape renames the dir to
/// `pN`.) The unstamped cross-package lanes key on that spelling, so their
/// answers are shape-keyed — the pins below record both answers.
fn names_shape() -> bool {
    spelled("file:///ws/a/lib.rut") == "file:///ws/a/lib.rut"
}

both_uri_shapes! {
#[test]
fn the_check_lane_compiles_every_open_files_live_text() {
    let mut s = S::boot();
    let root = "mod util;\nuse app::util::{ f };\nentry fn main() -> nil {\n    let x: i32 = f();\n}\n";
    let disk = "pub fn f() -> i32 { return 1; }\n";
    let live = "pub fn f() -> str { return \"s\"; }\n";
    let (ru, cu) = ("file:///ws/app/mod.rut", "file:///ws/app/util/mod.rut");
    s.index_mod_at(ru, "app", "", root);
    s.index_mod_at(cu, "app", "util", disk);
    assert!(s.analyze(ru, root)["diags"].as_array().unwrap().is_empty());
    s.analyze(cu, live); // the buffer edit; the child file's stamp stands
    // one world: the check lane compiles the OPEN child's buffer — the
    // diagnostic appears before any re-index
    let d = s.analyze(ru, root)["diags"].as_array().unwrap().clone();
    assert_eq!(d.len(), 1, "{d:?}");
    assert!(d[0]["message"].as_str().unwrap().contains("but the initializer is `str`"), "{d:?}");
    assert_eq!(pos_pair(&d[0]["range"]["start"]), (3, 4));
    // the control: re-indexing (the walk's own lane) answers the same
    s.index_mod_at(cu, "app", "util", live);
    let d = s.analyze(ru, root)["diags"].as_array().unwrap().clone();
    assert_eq!(d.len(), 1, "{d:?}");
    assert!(d[0]["message"].as_str().unwrap().contains("but the initializer is `str`"), "{d:?}");
    assert_eq!(pos_pair(&d[0]["range"]["start"]), (3, 4));
}
}

const A_LIB: &str = "file:///ws/a/lib.rut";
const A_DISK: &str = "pub fn make() -> i32 { return 1; }\n";
const A_LIVE: &str =
    "\n\npub fn make() -> str { return \"s\"; }\npub fn extra() -> i32 { return 2; }\n";
const USE_MAKE: &str = "use a::{ make };\nentry fn main() -> nil {\n    let x = make();\n}\n";
const USE_EXTRA: &str = "use a::{ extra };\nentry fn main() -> nil {\n    let x = extra();\n}\n";

both_uri_shapes! {
#[test]
fn cross_file_navigation_reads_the_live_buffer_of_an_open_file() {
    let mut s = S::boot();
    s.index_file(A_LIB, A_DISK);
    s.analyze(A_LIB, A_LIVE); // open and edited: `make` sits on line 2 char 8 and returns str
    if names_shape() {
        // the naming shape: the open file's buffer is the truth — the
        // live signature at its live line
        let md = markdown(s.hover_at("file:///t/one.rut", USE_MAKE, "make()", 1));
        assert!(md.contains("pub fn make() -> str") && md.contains("lib.rut:3"), "{md}");
        let hits = s.def_at("file:///t/one.rut", USE_MAKE, "make()", 1);
        assert_eq!(hits.len(), 1, "{hits:?}");
        // the plan wrote (2, 8) here; the live buffer spells `pub fn `
        // before the name — the declaring ident starts at char 7, the
        // same column the plan's own Before value pinned in the disk
        // text ((0, 7)). Pinned at the tree's answer.
        assert_eq!(pos_pair(&hits[0]["range"]["start"]), (2, 7));
        let md = markdown(s.hover_at("file:///t/two.rut", USE_EXTRA, "extra()", 1));
        assert!(md.contains("pub fn extra() -> i32"), "{md}");
    } else {
        // the hiding shape: the unstamped cross-package lane still answers
        // nothing — an `index_file` push carries no stamp.module and the
        // pkg-name derivation has no `a` to grip (recorded; the lane is a
        // separate plan's business)
        let h = s.hover_at("file:///t/one.rut", USE_MAKE, "make()", 1);
        assert!(no_hover(&h), "{h:?}");
        assert!(s.def_at("file:///t/one.rut", USE_MAKE, "make()", 1).is_empty());
        assert!(no_hover(&s.hover_at("file:///t/two.rut", USE_EXTRA, "extra()", 1)));
    }
}
}

both_uri_shapes! {
#[test]
fn a_closed_file_keeps_its_last_buffer_text() {
    let mut s = S::boot();
    s.index_file(A_LIB, A_DISK);
    s.analyze(A_LIB, A_LIVE);
    s.forget(A_LIB);
    let (l, c) = pos_of(A_LIVE, "make", 1);
    let closed = s.hover_pos(A_LIB, l, c);
    assert!(no_hover(&closed), "{closed:?}"); // stays (both shapes): a closed uri answers null
    if names_shape() {
        // the naming shape: the file keeps the buffer it held at close —
        // the same three live-buffer answers as when it was open
        let md = markdown(s.hover_at("file:///t/one.rut", USE_MAKE, "make()", 1));
        assert!(md.contains("pub fn make() -> str") && md.contains("lib.rut:3"), "{md}");
        let hits = s.def_at("file:///t/one.rut", USE_MAKE, "make()", 1);
        assert_eq!(pos_pair(&hits[0]["range"]["start"]), (2, 7)); // see the sibling pin: the plan's (2, 8) was a miscount
        let md = markdown(s.hover_at("file:///t/two.rut", USE_EXTRA, "extra()", 1));
        assert!(md.contains("pub fn extra() -> i32"), "{md}");
    } else {
        // the hiding shape: the unstamped cross-package lane is still dead —
        // null and [] (recorded; the lane is a separate plan's business)
        let h = s.hover_at("file:///t/one.rut", USE_MAKE, "make()", 1);
        assert!(no_hover(&h), "{h:?}");
        assert!(s.def_at("file:///t/one.rut", USE_MAKE, "make()", 1).is_empty());
        assert!(no_hover(&s.hover_at("file:///t/two.rut", USE_EXTRA, "extra()", 1)));
    }
    assert!(markdown(s.hover_at(A_LIB, A_LIVE, "make", 1)).contains("pub fn make() -> str")); // stays: reopen (both shapes)
}
}

both_uri_shapes! {
#[test]
fn body_over_decl_applies_to_unstamped_surfaces_too() {
    let surface = "pub host fn make() -> i32;\n";
    let body = "pub fn make() -> i32 { return 1; }\n";
    let src = "use pkg::{ make };\nentry fn main() -> nil {\n    let x = make();\n}\n";
    let uri = "file:///t/main.rut";
    let mut s = S::boot();
    s.index_file("file:///deps/pkg/pkg.d.rut", surface);
    s.index_file("file:///deps/pkg/mod.rut", body);
    let hits = s.def_in(uri, src, "{ make }", 1, 2);
    if names_shape() {
        // the naming shape: the unstamped `.d.rut` surface carries its
        // derived Decl mode — the body answers, once
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0]["uri"], "file:///deps/pkg/mod.rut");
        assert_eq!(pos_pair(&hits[0]["range"]["start"]), (0, 7));
    } else {
        // the hiding shape: the unstamped lane is dead — the derivation
        // has no `pkg` to grip (recorded; the lane is a separate plan's
        // business)
        assert!(hits.is_empty(), "{hits:?}");
    }
    let mut s = S::boot();
    s.index_mod_at("file:///deps/pkg/pkg.d.rut", "pkg", "", surface);
    s.index_mod_at("file:///deps/pkg/mod.rut", "pkg", "", body);
    let hits = s.def_in(uri, src, "{ make }", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}"); // stays (both shapes): the stamped law
    assert_eq!(hits[0]["uri"], "file:///deps/pkg/mod.rut");
    assert_eq!(pos_pair(&hits[0]["range"]["start"]), (0, 7));
}
}

both_uri_shapes! {
#[test]
fn hover_is_silent_on_an_ambiguous_module_let() {
    let mut s = S::boot();
    let src = "let A: i32 = 1;\nlet A: i32 = 2;\nfn f() -> i32 {\n    return A;\n}\n";
    let uri = "file:///t/amb.rut";
    let md = markdown(s.hover_in(uri, src, "return A;", 1, 7));
    assert_eq!(
        md,
        "multiple definitions:\n- `let A: i32 = 1;` — module let — file:///t/amb.rut:1\n- `let A: i32 = 2;` — module let — file:///t/amb.rut:2",
        "{md}"
    ); // stays: delta 5 realized by 21332ebe — hover lists the candidates (Act 3 renames this pin)
    // stays (HEAD's law, beyond the plan's After): 21332ebe's
    // `one_name_refused` gate — an ambiguous name jumps NOWHERE (never a
    // pick between the bindings; the hover lists them). The plan's Before
    // was 2 targets; Act 3's unified walk re-answers this.
    let amb = s.def_in(uri, src, "return A;", 1, 7);
    assert!(amb.is_empty(), "{amb:?}");
    let src2 = "fn g() -> i32 { return 1; }\nfn g() -> i32 { return 2; }\nfn f() -> i32 {\n    return g();\n}\n";
    assert!(markdown(s.hover_in(uri, src2, "return g();", 1, 7)).contains("multiple definitions"));
}
}

both_uri_shapes! {
#[test]
fn definition_answers_nothing_on_a_middle_path_segment() {
    let mut s = S::boot();
    let doc = "mod layout;\nentry fn main() -> i32 { return layout::span(); }\n";
    let uri = "file:///ws/app/mod.rut";
    s.index_mod_at("file:///ws/app/layout/mod.rut", "app", "layout", "pub fn span() -> i32 { return 1; }\n");
    let md = markdown(s.hover_in(uri, doc, "layout::span", 1, 2));
    assert!(md.contains("layout/mod.rut") && md.contains("file module"), "{md}"); // stays
    assert!(s.def_in(uri, doc, "layout::span", 1, 2).is_empty()); // Act 3: the module file top, (0, 0)
    assert_eq!(s.def_in(uri, doc, "layout::span", 1, 9).len(), 1); // stays
}
}

both_uri_shapes! {
#[test]
fn the_own_package_head_answers_inside_a_flat_package() {
    let uri = "file:///ws/flat/mod.rut";
    let doc = "pub fn x() -> i32 { return 1; }\nuse flat::{ x };\nentry fn main() -> nil {\n    let a = x();\n}\n";
    let mut s = S::boot();
    s.index_mod_at(uri, "flat", "", "pub fn x() -> i32 { return 1; }\n");
    assert!(markdown(s.hover_in(uri, doc, "use flat::", 1, 4)).contains("file module")); // Act 2: no hover
    let head = s.def_in(uri, doc, "use flat::", 1, 4);
    assert_eq!(head.len(), 1, "{head:?}"); // Act 2: empty
    assert_eq!(pos_pair(&head[0]["range"]["start"]), (0, 0));
    // recorded drift: the use leaf answers NOTHING at HEAD. The plan's
    // Before had one hit here and the plan's delta-7 After kept one hit
    // ("still resolves through the cross-package lane"); d9cd1f00's
    // identity exclusion dropped the open doc from the cross-package
    // sweep, and this scenario's only `flat` file IS the doc (a sibling
    // file does not help — the leaf's name lives in the doc). Act 2
    // rewrites this pin.
    let leaf = s.def_in(uri, doc, "{ x }", 1, 2);
    assert!(leaf.is_empty(), "{leaf:?}");
    let typing = "pub fn x() -> i32 { return 1; }\nuse flat::\nentry fn main() -> nil {\n}\n";
    let items = s.complete_at(uri, typing, "use flat::", 1);
    let ls = labels(&items);
    for want in ["entry", "main", "x"] {
        assert!(ls.contains(&want), "{ls:?}"); // Act 2: only `x`
    }
}
}

both_uri_shapes! {
#[test]
fn an_open_scratch_file_is_invisible_to_other_documents() {
    let (a, b) = ("file:///t/a.rut", "file:///t/b.rut");
    let bsrc = "class Box2 {\n    n: i32;\n}\n";
    let asrc = "entry fn main() -> nil {\n    let b = Box2(1);\n}\n";
    let mut s = S::boot();
    s.analyze(b, bsrc);
    let md = markdown(s.hover_at(a, asrc, "b =", 1));
    assert!(md.contains("let b\n") && !md.contains("Box2"), "never mounted: {md}");
    s.index_file(b, bsrc);
    assert!(markdown(s.hover_at(a, asrc, "b =", 1)).contains("let b: Box2"));
    s.forget(b);
    assert!(markdown(s.hover_at(a, asrc, "b =", 1)).contains("let b: Box2"), "the index lane's copy stands");
}
}

both_uri_shapes! {
#[test]
fn a_stale_index_push_never_clobbers_an_open_buffer() {
    let uri = "file:///ws/a/lib.rut";
    let mut s = S::boot();
    s.analyze(uri, "pub fn make() -> str { return \"s\"; }\n");
    s.index_file(uri, "pub fn make() -> i32 { return 1; }\n");
    // recorded: the plan's probe hovered the `fn` KEYWORD (0, 4) and
    // expected the decl — but that position answers null at HEAD and
    // answered null at ca434c85 too (the decl-site lanes key on the
    // NAME's position; the pin's own spelling never held). The
    // scenario's intent is pinned at the name (0, 7): the open buffer
    // is the truth after a stale push.
    let kw = s.hover_pos(uri, 0, 4);
    assert!(no_hover(&kw), "{kw:?}");
    let md = markdown(s.hover_pos(uri, 0, 7));
    assert!(md.contains("pub fn make() -> str"), "{md}"); // stays (both shapes)
}
}

both_uri_shapes! {
#[test]
fn self_hovers_the_enclosing_type_and_defines_nothing() {
    let uri = "file:///t/self.rut";
    let doc = "class P {\n    n: i32,\n}\nimpl P {\n    fn mk() -> Self {\n        return Self { n: 1 };\n    }\n}\n";
    let mut s = S::boot();
    let md = markdown(s.hover_in(uri, doc, "-> Self", 1, 3));
    assert!(md.contains("class P {") && md.contains("self.rut:1"), "{md}");
    assert!(s.def_in(uri, doc, "-> Self", 1, 3).is_empty());
}
}

both_uri_shapes! {
#[test]
fn reference_scans_resolve_each_file_under_its_own_package_context() {
    let mut s = S::boot();
    s.index_mod_at("file:///ws/app/mod.rut", "app", "", "mod helper;\nmod x;\n");
    let hsrc = "pub fn two() -> i32 { return 2; }\n";
    s.index_mod_at("file:///ws/app/helper/mod.rut", "app", "helper", hsrc);
    // x imports `two` (so the scan reaches it) AND names it through an own-edge
    // qualified path, which only x's own package context resolves
    s.index_mod_at(
        "file:///ws/app/x/mod.rut",
        "app",
        "x",
        "use app::helper::{ two };\nentry fn main() -> i32 {\n    return helper::two();\n}\n",
    );
    let hits = s.refs_in("file:///ws/app/helper/mod.rut", hsrc, "two", 1, 0, false);
    // stays: delta 9 realized by d9cd1f00 and kept by the one-world store —
    // each scanned file resolves under its own stamp (the plan's Before
    // was one site under a fake "unknown" context; this is Act 1's
    // renamed pin's answer)
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(hits.iter().all(|l| l["uri"] == "file:///ws/app/x/mod.rut"), "{hits:?}");
    let starts: Vec<(i64, i64)> = hits.iter().map(|l| pos_pair(&l["range"]["start"])).collect();
    assert_eq!(starts, [(0, 19), (2, 19)], "{hits:?}");
}
}
