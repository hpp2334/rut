//! The glob lanes through the editor — `use pkg::*` resolves through
//! the public door: a glob-imported name completes bare and
//! definition-resolves into the dep, the auto-import tier never
//! re-offers a glob-bound name (the import map's glob rows suppress
//! it), the glob–glob conflict diag reaches analyze through the check
//! lane's merge, and a `pub use pkg::*;` re-export feeds downstream
//! (the use tier lists the globbed names — the star never offers;
//! `use a::*;` binds them, chained globs expanding recursively).

mod common;

use common::*;
use std::rc::Rc;

/// the json dep — a flat pkg whose pub surface a consumer's glob
/// binds (a private fn rides as the leak control)
fn mount_json_dep(s: &mut S) {
    s.index_mod_at(
        "file:///deps/json/mod.rut",
        "json",
        "",
        "pub fn parse(src: str) -> i32 { return 1; }\npub struct Value {\nn: i32;\n}\nfn internal() -> i32 { return 2; }\n",
    );
}

/// the walk's seeded direct-dep set: a root manifest declaring
/// `names` (the auto-import tier's live gate). The deps mount through
/// the ordinary dir lane.
fn declare_direct(names: &[&str]) -> (Rc<MemIo>, String) {
    let mut io = MemIo::default();
    let deps = format!(
        "{{{}}}",
        names
            .iter()
            .map(|n| format!("\"{n}\": {{\"path\": \"/ws/{n}\"}}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    io.put(
        "/ws/proj/rut.jsonc",
        &format!("{{\n  \"name\": \"proj\",\n  \"deps\": {deps}\n}}\n"),
    );
    io.put("/ws/proj/src/mod.rut", "fn main() -> nil {\n}\n");
    for n in names {
        io.put(
            &format!("/ws/{n}/rut.jsonc"),
            &format!("{{\n  \"name\": \"{n}\"\n}}\n"),
        );
        io.put(
            &format!("/ws/{n}/src/mod.rut"),
            &format!("pub fn {n}_tag() -> i32 {{\n    return 1;\n}}\n"),
        );
    }
    (Rc::new(io), "/ws/proj".to_string())
}

// ---- the glob import binds (bare + definition) ----

#[test]
fn a_glob_import_completes_bare_and_resolves_into_the_dep() {
    let mut s = S::boot();
    mount_json_dep(&mut s);
    let uri = "file:///t/main.rut";
    // the bare position mid-typing: the glob-bound fn completes plain
    // (no insert edit — the glob IS the import), the type rides the
    // same surface, the private name never leaks
    let src = "use json::*;\nfn main() -> nil {\n    let n = par\n}\n";
    let items = s.complete_at(uri, src, "    let n = par", 1);
    let ls = labels(&items);
    assert!(ls.contains(&"parse"), "the glob-bound fn completes: {ls:?}");
    let plain = item(&items, "parse");
    assert!(
        plain["additionalTextEdits"].is_null(),
        "the glob-bound fn completes with no insert: {plain}"
    );
    assert!(
        ls.contains(&"Value"),
        "the glob-bound type completes bare too: {ls:?}"
    );
    assert!(
        !ls.contains(&"internal"),
        "a private fn is not part of the globbed surface: {ls:?}"
    );

    // definition through the glob: the use site lands in the dep
    let hits = s.def_in(
        uri,
        "use json::*;\nfn main() -> nil {\n    let n = parse(\"{}\");\n}\n",
        "parse(",
        1,
        1,
    );
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///deps/json/mod.rut");

    // the cursor past the star: the empty-prefix use edge — the tier
    // lists the walked surface's names, the star itself never offers
    let items = s.complete_pos(uri, "use json::*;", 0, 12);
    let ls = labels(&items);
    assert!(ls.contains(&"parse"), "names list past the star: {ls:?}");
    assert!(ls.contains(&"Value"), "{ls:?}");
    assert!(!ls.contains(&"*"), "the star never offers: {ls:?}");
}

// ---- the auto-import suppression ----

both_uri_shapes! {
#[test]
fn a_glob_bound_name_is_not_re_offered_by_auto_import() {
    let (io, root) = declare_direct(&["json"]);
    let mut s = S::with_io(&io);
    s.resolve(&[&root]);
    s.index_mod_at(
        "file:///deps/json/lib.rut",
        "json",
        "",
        "pub fn parse(src: str) -> i32 { return 1; }\n",
    );
    let uri = "file:///t/main.rut";
    // the control: no use in the doc — the only offer rides the
    // auto-import insert
    let bare = "fn main() -> nil {\n    let n = par\n}\n";
    let items = s.complete_at(uri, bare, "    let n = par", 1);
    let imp = import_offer(&items, "parse").expect("the direct dep's fn auto-imports");
    assert_eq!(imp["detail"], "json::parse — import", "{}", imp);
    // with the glob: the name completes plain (the bare tier) and the
    // import offer is gone — the glob row already binds it
    let src = "use json::*;\nfn main() -> nil {\n    let n = par\n}\n";
    let items = s.complete_at(uri, src, "    let n = par", 1);
    assert!(offers(&items, "parse"), "{:?}", labels(&items));
    let plain = item(&items, "parse");
    assert!(plain["additionalTextEdits"].is_null(), "{plain}");
    assert!(
        import_offer(&items, "parse").is_none(),
        "the glob-bound name is never re-offered: {:?}",
        labels(&items)
    );
}
}

// ---- the glob–glob conflict diag through the check lane ----

both_uri_shapes! {
#[test]
fn the_glob_conflict_diag_surfaces_through_analyze() {
    let mut s = S::boot();
    s.index_mod_at(
        "file:///deps/a/mod.rut",
        "a",
        "",
        "pub fn shadow() -> i32 { return 1; }\n",
    );
    s.index_mod_at(
        "file:///deps/b/mod.rut",
        "b",
        "",
        "pub fn shadow() -> i32 { return 2; }\n",
    );
    // the open document IS pkg app's root (the stamp the index lane laid down keys
    // check lane); the two globs bind `shadow` from both heads — the
    // driver's conflict errors at the SECOND use line and the merged
    // set publishes it on this document
    let uri = "file:///ws/app/src/mod.rut";
    let src = "use a::*;\nuse b::*;\nfn main() -> nil {\n    let n = shadow();\n}\n";
    s.index_mod_at(uri, "app", "", src);
    let analysis = s.analyze(uri, src);
    let diags = analysis["diags"].as_array().unwrap();
    let conflict = diags
        .iter()
        .find(|d| {
            d["message"]
                .as_str()
                .is_some_and(|m| m.contains("`shadow` arrives from both `a::*` and `b::*`"))
        })
        .expect("the conflict diag reaches the editor through the merge");
    // the error site is the second use statement's own span
    assert_eq!(
        pos_pair(&conflict["range"]["start"]),
        (1, 0),
        "{}",
        conflict
    );
}
}

// ---- references: the glob-aware reverse edge ----

both_uri_shapes! {
#[test]
fn references_reach_every_glob_importing_file() {
    // two files both `use json::*;` and both call `parse` — references
    // from one finds the use in the OTHER file too, the same reverse
    // edge explicit importers ride (the glob-aware import map; a
    // glob leaf fans no name rows, so the named-leaf read alone made
    // a glob import second-class). A non-importing file contributes
    // nothing.
    let mut s = S::boot();
    s.index_mod_at(
        "file:///deps/json/mod.rut",
        "json",
        "",
        "pub fn parse(src: str) -> i32 { return 1; }\n",
    );
    let one = "use json::*;\nfn main() -> nil {\n    let n = parse(\"{}\");\n}\n";
    let two = "use json::*;\nfn go() -> i32 {\n    return parse(\"x\");\n}\n";
    let three = "fn unrelated() -> i32 {\n    return 0;\n}\n";
    s.index_file("file:///t/one.rut", one);
    s.index_file("file:///t/two.rut", two);
    s.index_file("file:///t/three.rut", three);
    // the query rides the asking doc's call site (include_declaration
    // off — the decl ident rides only that flag)
    let hits = s.refs_in("file:///t/one.rut", one, "parse(", 1, 1, false);
    let mut uris: Vec<&str> = hits
        .iter()
        .map(|l| l["uri"].as_str().expect("every hit names a uri"))
        .collect();
    uris.sort_unstable();
    uris.dedup();
    assert_eq!(
        uris,
        vec!["file:///t/one.rut", "file:///t/two.rut"],
        "both glob importers found, the non-importer absent: {hits:?}"
    );
    // one call site per importer
    assert_eq!(hits.len(), 2, "{hits:?}");
}
}

// ---- the glob re-export feeds downstream ----

both_uri_shapes! {
#[test]
fn chained_glob_reexports_terminate() {
    // a pub-globs b, b pub-globs a — the real worlds are DAGs, but a
    // hand-built host world must never loop the editor
    let mut s = S::boot();
    s.index_mod_at(
        "file:///deps/a/mod.rut",
        "a",
        "",
        "pub use b::*;\npub fn afn() -> i32 { return 1; }\n",
    );
    s.index_mod_at(
        "file:///deps/b/mod.rut",
        "b",
        "",
        "pub use a::*;\npub fn bfn() -> i32 { return 2; }\n",
    );
    let uri = "file:///t/main.rut";
    // the tier lists both surfaces ONCE and returns — the cycle back
    // to the start head re-lists nothing
    let items = s.complete_pos(uri, "use a::{", 0, 8);
    let ls = labels(&items);
    assert!(ls.contains(&"afn"), "{ls:?}");
    assert!(ls.contains(&"bfn"), "{ls:?}");
    for want in ["afn", "bfn"] {
        assert_eq!(
            ls.iter().filter(|l| **l == want).count(),
            1,
            "one `{want}` item in a cyclic world: {ls:?}"
        );
    }
    // and the glob binds without hanging (bfn keyed on b, where it
    // actually resolves)
    let src = "use a::*;\nfn main() -> nil {\n    let x = bf\n}\n";
    let items = s.complete_at(uri, src, "    let x = bf", 1);
    assert!(offers(&items, "bfn"), "{:?}", labels(&items));
}
}

both_uri_shapes! {
#[test]
fn a_glob_reexport_feeds_downstream() {
    let mut s = S::boot();
    // util's pub surface; pkg a pub-globs it (the re-export lane)
    s.index_mod_at(
        "file:///deps/util/mod.rut",
        "util",
        "",
        "pub fn tag() -> i32 { return 1; }\npub struct Widget {\nid: i32;\n}\nfn hidden() -> i32 { return 2; }\n",
    );
    s.index_mod_at("file:///deps/a/mod.rut", "a", "", "pub use util::*;\n");
    let uri = "file:///t/main.rut";
    // the use tier lists the GLOBBED names — the star itself never
    // offers, the private name never crosses
    let items = s.complete_pos(uri, "use a::{", 0, 8);
    let ls = labels(&items);
    assert!(ls.contains(&"tag"), "the globbed fn rides the tier: {ls:?}");
    assert!(ls.contains(&"Widget"), "the globbed type rides too: {ls:?}");
    assert!(!ls.contains(&"*"), "the star is never offered: {ls:?}");
    assert!(
        !ls.contains(&"hidden"),
        "a private fn is not part of the re-exported surface: {ls:?}"
    );

    // and `use a::*;` binds them: bare completion (a plain item, the
    // chained expansion keyed on util — where the name resolves) …
    let src = "use a::*;\nfn main() -> nil {\n    let w = ta\n}\n";
    let items = s.complete_at(uri, src, "    let w = ta", 1);
    let plain = item(&items, "tag");
    assert!(plain["additionalTextEdits"].is_null(), "{plain}");
    // … and definition lands in the declaring pkg
    let hits = s.def_in(
        uri,
        "use a::*;\nfn main() -> nil {\n    let w = tag();\n}\n",
        "tag(",
        1,
        1,
    );
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///deps/util/mod.rut");
}
}
