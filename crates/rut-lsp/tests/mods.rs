//! The file-module namespace's gate — the laws the mod-tree walk
//! carries, driven through the envelopes: the cross-package pub gate,
//! the intra-package ancestor walk (any declared edge, visibility
//! gating the leaves), the (pkg, path) pair keys, the compiler-tier
//! visibility predicate (pub/pkg/super/private), the mounted-file
//! display spellings, and the mod-name slot (the disk probe beside the
//! open document — its exact list, the typed prefix, the declared
//! gate, the item-level shape, the doc's flavor, the env/uri gates).

mod common;

use common::*;
use std::rc::Rc;

/// the open `src/mod.rut` beside: an undeclared dir-form child (with
/// a grandchild), a file-form child, the loud ambiguity, a reserved
/// name, an empty folder
fn mod_world() -> MemIo {
    let mut io = MemIo::default();
    io.put(
        "file:///ws/app/src/mod.rut",
        "entry fn main() -> nil {\n}\n",
    );
    io.put(
        "file:///ws/app/src/harness/mod.rut",
        "pub fn run() -> nil {\n}\n",
    );
    io.put(
        "file:///ws/app/src/harness/opencode/mod.rut",
        "pub fn boot() -> nil {\n}\n",
    );
    io.put(
        "file:///ws/app/src/util.rut",
        "pub fn tag() -> i32 { return 1; }\n",
    );
    io.put("file:///ws/app/src/both/mod.rut", "");
    io.put("file:///ws/app/src/both.rut", "");
    io.put("file:///ws/app/src/use/mod.rut", "");
    io.dirs.insert("file:///ws/app/src/empty".to_string());
    io
}

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

both_uri_shapes! {
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
}

both_uri_shapes! {
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
}

both_uri_shapes! {
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
}

// ---- the mod-name slot: the disk probe beside the open document ----
// Nothing is indexed in these tests — the slot never consults the
// chain; the disk beside the document is the whole answer.

#[test]
fn the_mod_name_slot_lists_exactly_the_module_files_beside_the_doc() {
    // the user's shape (`mod ` on the line above the entry — the parser
    // SWALLOWED `entry` as the decl's name), the fence shape (`mod ` at
    // EOF — the dangling `mod` is an ErrorNode), and `pub mod `: the
    // list is exactly the disk's — no keyword, no grandchild, no
    // `both` (the loud ambiguity), no `use` (a reserved word), no
    // `empty`, not the doc's own `mod`
    let mut s = S::with_io(&Rc::new(mod_world()));
    let uri = "file:///ws/app/src/mod.rut";
    for (doc, line, ch) in [
        ("mod \nentry fn main() -> nil {\n}\n", 0, 4),
        ("mod ", 0, 4),
        ("pub mod \nentry fn main() -> nil {\n}\n", 0, 8),
    ] {
        let items = s.complete_pos(uri, doc, line, ch);
        assert_eq!(
            labels(&items),
            ["harness", "util"],
            "the slot's exact list ({line},{ch}): {:?}",
            labels(&items)
        );
        for i in &items {
            assert_eq!(i["kind"], k::MODULE, "module items only: {i}");
        }
        assert_eq!(
            item(&items, "harness")["detail"],
            "file module — `harness/mod.rut`",
            "the dir form spells the loader's child file"
        );
        assert_eq!(
            item(&items, "util")["detail"],
            "file module — `util.rut`",
            "the file form spells the single-file twin"
        );
    }
}

#[test]
fn the_mod_name_slot_filters_on_the_typed_prefix() {
    let mut s = S::with_io(&Rc::new(mod_world()));
    let uri = "file:///ws/app/src/mod.rut";
    let items = s.complete_pos(uri, "mod har\nentry fn main() -> nil {\n}\n", 0, 7);
    assert_eq!(labels(&items), ["harness"], "{:?}", labels(&items));
    let items = s.complete_pos(uri, "mod ut\nentry fn main() -> nil {\n}\n", 0, 6);
    assert_eq!(labels(&items), ["util"], "{:?}", labels(&items));
    let items = s.complete_pos(uri, "mod zz\nentry fn main() -> nil {\n}\n", 0, 6);
    assert!(
        items.is_empty(),
        "a dead prefix is an empty list: {:?}",
        labels(&items)
    );
}

#[test]
fn the_mod_name_slot_never_re_offers_a_declared_child() {
    // a duplicate `mod` is an error, so a name the doc already
    // declares never re-offers — while the decl under the cursor
    // keeps its own typed name alive (the list narrows, never empties,
    // while typing)
    let mut s = S::with_io(&Rc::new(mod_world()));
    let uri = "file:///ws/app/src/mod.rut";
    let items = s.complete_pos(
        uri,
        "mod harness;\nmod \nentry fn main() -> nil {\n}\n",
        1,
        4,
    );
    assert_eq!(
        labels(&items),
        ["util"],
        "harness is declared: {:?}",
        labels(&items)
    );
    let items = s.complete_pos(
        uri,
        "mod harness;\nmod util;\nmod \nentry fn main() -> nil {\n}\n",
        2,
        4,
    );
    assert!(items.is_empty(), "both declared: {:?}", labels(&items));
    let items = s.complete_pos(uri, "mod har\nentry fn main() -> nil {\n}\n", 0, 7);
    assert_eq!(
        labels(&items),
        ["harness"],
        "the typed decl under the cursor never gates itself: {:?}",
        labels(&items)
    );
}

#[test]
fn the_mod_name_slot_is_item_level_and_unterminated_only() {
    // inside a block `mod` is an expression — the bare tier keeps
    // answering; past a complete `mod harness;` the cursor is a new
    // item's bare position; `mod` with no space never left the bare
    // typing edge
    let mut s = S::with_io(&Rc::new(mod_world()));
    let uri = "file:///ws/app/src/mod.rut";
    let items = s.complete_pos(uri, "entry fn main() -> nil {\n    mod \n}\n", 1, 8);
    assert!(
        offers(&items, "fn"),
        "the keyword table answers in a block: {:?}",
        labels(&items)
    );
    assert!(
        !offers(&items, "harness"),
        "the block never probes the disk: {:?}",
        labels(&items)
    );
    let items = s.complete_pos(uri, "mod harness;\n\nentry fn main() -> nil {\n}\n", 1, 0);
    assert!(
        offers(&items, "fn"),
        "past a complete `mod x;` the bare tier answers: {:?}",
        labels(&items)
    );
    assert!(
        !offers(&items, "util"),
        "the slot is out past the `;`: {:?}",
        labels(&items)
    );
    let items = s.complete_pos(uri, "mod", 0, 3);
    assert!(
        offers(&items, "fn"),
        "`mod` with no space stays bare: {:?}",
        labels(&items)
    );
    assert!(
        !offers(&items, "util"),
        "no space, no slot: {:?}",
        labels(&items)
    );
}

#[test]
fn the_mod_name_slot_follows_the_document_flavor() {
    // the flavor is `mode_of`'s — a `.d.rut` doc's children spell
    // `mod.d.rut`, and the single-file twin never answers there
    let mut io = mod_world();
    io.put(
        "file:///ws/app/src/net/mod.d.rut",
        "interface Peer {\n    send(msg: str);\n}\n",
    );
    let mut s = S::with_io(&Rc::new(io));
    let items = s.complete_pos("file:///ws/app/src/mod.d.rut", "mod ", 0, 4);
    assert_eq!(
        labels(&items),
        ["net"],
        "the Decl flavor's list: {:?}",
        labels(&items)
    );
    assert_eq!(
        item(&items, "net")["detail"],
        "file module — `net/mod.d.rut`",
        "the child file spelling follows the flavor"
    );
}

#[test]
fn the_mod_name_slot_needs_a_file_uri_and_an_env() {
    // no env — no probe (an empty list, never the bare dump); a
    // non-`file://` uri — no disk to ask
    let mut s = S::boot();
    let items = s.complete_pos(
        "file:///ws/app/src/mod.rut",
        "mod \nentry fn main() -> nil {\n}\n",
        0,
        4,
    );
    assert!(items.is_empty(), "no env, no list: {:?}", labels(&items));

    let mut s = S::with_io(&Rc::new(mod_world()));
    let items = s.complete_pos(
        "untitled:app/src/mod.rut",
        "mod \nentry fn main() -> nil {\n}\n",
        0,
        4,
    );
    assert!(
        items.is_empty(),
        "no file uri, no list: {:?}",
        labels(&items)
    );
}
