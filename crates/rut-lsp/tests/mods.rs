//! The file-module namespace's gate — the laws the mod-tree walk
//! carries, driven through the envelopes: the cross-package pub gate,
//! the intra-package ancestor walk (any declared edge, visibility
//! gating the leaves), the (pkg, path) pair keys, the compiler-tier
//! visibility predicate (pub/pkg/super/private), and the mounted-file
//! display spellings.

mod common;

use common::*;

/// the app tree: a root declaring `pub mod layout;` + `mod secret;`,
/// layout a pub struct + a `pub(pkg)` struct + a private fn, grid a
/// leaf child
fn mount_app(s: &mut S) {
    s.index_mod_at(
        "file:///ws/app/mod.rut",
        "app",
        "",
        "pub mod layout;\nmod secret;\npub fn root_fn() -> i32 { return 1; }\n",
    );
    s.index_mod_at(
        "file:///ws/app/layout/mod.rut",
        "app",
        "layout",
        "pub mod grid;\npub struct Shown {\n    w: i32;\n}\npub(pkg) struct Hidden {\n    h: i32;\n}\nfn internal() -> i32 { return 2; }\n",
    );
    s.index_mod_at(
        "file:///ws/app/layout/grid/mod.rut",
        "app",
        "layout/grid",
        "pub struct Cell {\n    x: i32;\n}\n",
    );
    s.index_mod_at(
        "file:///ws/app/secret.rut",
        "app",
        "secret",
        "pub fn open() -> i32 { return 4; }\n",
    );
}

#[test]
fn the_cross_package_walk_gates_pub_edges() {
    // through the completion envelope: `use app::⏐` offers the pub
    // edge and the root's fns, never the bare `mod secret`; the walk
    // through secret is a dead end; an unknown segment or package is
    // an honest empty
    let mut s = S::boot();
    mount_app(&mut s);
    let uri = "file:///t/main.rut";
    let items = s.complete_pos(uri, "use app::", 0, 9);
    let ls = labels(&items);
    assert!(
        ls.contains(&"layout"),
        "the pub mod child completes: {ls:?}"
    );
    assert!(ls.contains(&"root_fn"), "the root's fn completes: {ls:?}");
    assert!(
        !ls.contains(&"secret"),
        "a bare `mod` edge never crosses: {ls:?}"
    );
    let items = s.complete_pos(uri, "use app::secret::", 0, 17);
    assert!(
        items.is_empty(),
        "the private edge is a dead end: {:?}",
        labels(&items)
    );
    let items = s.complete_pos(uri, "use app::ghost::", 0, 16);
    assert!(
        items.is_empty(),
        "an unknown segment is a miss: {:?}",
        labels(&items)
    );
    let items = s.complete_pos(uri, "use pouch::", 0, 11);
    assert!(
        items.is_empty(),
        "an unknown package is a miss: {:?}",
        labels(&items)
    );
}

#[test]
fn the_position_head_walks_ancestors_and_the_pkg_name() {
    // from layout/grid: a sibling resolves through the parent scope;
    // the own-pkg head names the root; an unknown pkg name misses.
    // One probe per stamp — the doc carries the one statement it types.
    let mut s = S::boot();
    mount_app(&mut s);
    let grid_uri = "file:///t/grid.rut";

    // the sibling-of-parent resolves
    let doc =
        "pub struct Cell {\n    x: i32;\n}\nentry fn main() -> nil {\n    let s: layout.Shown\n}\n";
    s.index_mod_at(grid_uri, "app", "layout/grid", doc);
    let items = s.complete_at(grid_uri, doc, "let s: layout.", 1);
    assert!(
        offers(&items, "Shown"),
        "the sibling resolves: {:?}",
        labels(&items)
    );

    // the own-pkg head names the root: app's own surface + children
    let doc = "pub struct Cell {\n    x: i32;\n}\nentry fn main() -> nil {\n    let t: app.\n}\n";
    s.index_mod_at(grid_uri, "app", "layout/grid", doc);
    let items = s.complete_at(grid_uri, doc, "let t: app.", 1);
    let ls = labels(&items);
    assert!(
        ls.contains(&"root_fn"),
        "the pkg head reaches the root: {ls:?}"
    );
    assert!(ls.contains(&"layout"), "{ls:?}");

    // an unknown pkg name: a miss (no mod tier answers)
    let doc = "pub struct Cell {\n    x: i32;\n}\nentry fn main() -> nil {\n    let u: pouch.\n}\n";
    s.index_mod_at(grid_uri, "app", "layout/grid", doc);
    let items = s.complete_at(grid_uri, doc, "let u: pouch.", 1);
    assert!(
        !offers(&items, "Cell"),
        "an unknown pkg resolves nothing: {:?}",
        labels(&items)
    );
}

#[test]
fn the_vis_predicate_mirrors_the_compiler_tiers() {
    // pub reaches everywhere, pub(pkg) the whole package, private the
    // declaring module + descendants — read through the position tier
    let mut s = S::boot();
    mount_app(&mut s);
    // from the root: layout's pub + pub(pkg) rows visible, the
    // private fn not
    let root_uri = "file:///ws/app/mod.rut";
    let root_doc =
        "pub mod layout;\nmod secret;\nentry fn main() -> nil {\n    let a: layout.\n}\n";
    s.index_mod_at(root_uri, "app", "", root_doc);
    let items = s.complete_at(root_uri, root_doc, "let a: layout.", 1);
    let ls = labels(&items);
    assert!(ls.contains(&"Shown"), "pub reaches the package: {ls:?}");
    assert!(
        ls.contains(&"Hidden"),
        "pub(pkg) reaches the whole package: {ls:?}"
    );
    assert!(
        ls.contains(&"grid"),
        "any declared edge walks intra-package: {ls:?}"
    );
    assert!(
        !ls.contains(&"internal"),
        "a private fn of a child stays hidden: {ls:?}"
    );
}

#[test]
fn the_pair_keys_and_the_single_file_display() {
    // the (pkg, path) pair keys the walk — another pkg's file at the
    // same path never answers (the edge-click law lives in
    // tests/definition.rs); the display spells the mounted file's
    // REAL name (a single-file child rides its origin basename)
    let mut s = S::boot();
    mount_app(&mut s);
    let doc = "pub mod layout;\npub mod secret;\nentry fn main() -> nil { }\n";
    let uri = "file:///ws/app/other.rut";
    s.index_mod_at(uri, "app", "", doc);
    // the secret edge's hover spells the single-file child's real name
    let h = s.hover_at(uri, doc, "secret", 1);
    let md = markdown(h);
    assert!(md.contains("file module — `secret.rut`"), "{md}");
    // the dir-form child keeps the loader's spelling
    let h = s.hover_at(uri, doc, "layout", 1);
    let md = markdown(h);
    assert!(md.contains("file module — `layout/mod.rut`"), "{md}");
    // an unindexed child falls back to the dir-form guess
    let doc2 = "pub mod layout;\npub mod ghost;\nentry fn main() -> nil { }\n";
    s.index_mod_at(uri, "app", "", doc2);
    let h = s.hover_at(uri, doc2, "ghost", 1);
    let md = markdown(h);
    assert!(md.contains("file module — `ghost/mod.rut`"), "{md}");
}
