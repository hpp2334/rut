//! Completion's gate — every tier through the `complete` envelope:
//! the member tier (impl methods + fields, the foreign-interface and
//! `pub builtin` gates), the bare tier (keyword table, honest foreign
//! scope, the auto-import offers with their insert edits), the
//! use-path tiers (module names, exports, re-exports, the mod-tree
//! walk, the own-package head), the `::` tiers (enum members, dep
//! statics, associated fns, the static gate), the type tier at
//! annotation boundaries, and the dot guards.

mod common;

use common::*;
use serde_json::Value;
use std::rc::Rc;

/// the gadget dep — a mod-carrying pkg stamped through the scan lane:
/// root (`""`) declaring `pub mod layout;` + `mod secret;`; `layout`
/// a `pub mod grid;` child with a pub struct, a private fn, and a
/// `pub(super)` fn; `layout/grid` and `secret` leaves
fn mount_gadget_dep(s: &mut S) {
    s.index_mod_at(
        "file:///deps/gadgets/mod.rut",
        "gadgets",
        "",
        "pub mod layout;\nmod secret;\npub fn tag() -> i32 { return 1; }\n",
    );
    s.index_mod_at(
        "file:///deps/gadgets/layout/mod.rut",
        "gadgets",
        "layout",
        "pub mod grid;\npub struct Column {\n    w: i32;\n}\nfn internal() -> i32 { return 2; }\npub(super) fn for_parent() -> i32 { return 3; }\n",
    );
    s.index_mod_at(
        "file:///deps/gadgets/layout/grid/mod.rut",
        "gadgets",
        "layout/grid",
        "pub struct Cell {\n    x: i32;\n}\npub fn mk() -> Cell { return Cell { x: 1 }; }\n",
    );
    s.index_mod_at(
        "file:///deps/gadgets/secret.rut",
        "gadgets",
        "secret",
        "pub fn open() -> i32 { return 4; }\nfn hidden() -> i32 { return 5; }\n",
    );
}

/// the re-export fixture: `mid`'s root and its `sub` module each `pub
/// use` names off `util` (and keep a private use — the crossed tier's
/// control)
fn mount_mid_dep(s: &mut S) {
    s.index_mod_at(
        "file:///deps/mid/mod.rut",
        "mid",
        "",
        "pub mod sub;\npub use util::{ helper, Widget };\nuse util::{ secret_root };\n",
    );
    s.index_mod_at(
        "file:///deps/mid/sub/mod.rut",
        "mid",
        "sub",
        "pub use util::{ tag };\nuse util::{ secret_sub };\n",
    );
}

/// the std chain the public lane serves: the ambient prelude (mounted
/// at boot) plus the repo's own std sources as ordinary dep fixtures
/// (nothing about std is special). The names the doc's own manifest
/// declares ride the WALK's seeded set (`declare_direct`).
fn mount_std(s: &mut S) {
    s.index_mod_at(
        "rut/pouch/src/mod.rut",
        "pouch",
        "",
        include_str!("../../../rut/pouch/src/mod.rut"),
    );
    s.index_mod_at(
        "rut/nmapset/src/mod.rut",
        "nmapset",
        "",
        include_str!("../../../rut/nmapset/src/mod.rut"),
    );
    s.index_mod_at(
        "rut/calc/src/mod.rut",
        "calc",
        "",
        include_str!("../../../rut/calc/src/mod.rut"),
    );
    s.index_mod_at(
        "rut/json/src/mod.rut",
        "json",
        "",
        include_str!("../../../rut/json/src/mod.rut"),
    );
    s.index_mod_at(
        "rut/ink/src/mod.rut",
        "ink",
        "",
        include_str!("../../../rut/ink/src/mod.rut"),
    );
}

/// the std chain AS the io world: the repo's real sources laid under
/// `/ws/<name>/` with name-only manifests, the root declaring all
/// five — the walk mounts each named AND seeds the direct-dep set in
/// one move (the auto-import tier's live gate beside its offers)
fn std_io_world() -> (Rc<MemIo>, String) {
    let mut io = MemIo::default();
    let five = ["pouch", "nmapset", "calc", "json", "ink"];
    let deps = format!(
        "{{{}}}",
        five.iter()
            .map(|n| format!("\"{n}\": {{\"path\": \"/ws/{n}\"}}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    io.put(
        "/ws/proj/rut.jsonc",
        &format!("{{\n  \"name\": \"proj\",\n  \"deps\": {deps}\n}}\n"),
    );
    io.put("/ws/proj/src/mod.rut", "fn main() -> nil {\n}\n");
    let srcs: &[(&str, &str)] = &[
        ("pouch", include_str!("../../../rut/pouch/src/mod.rut")),
        ("nmapset", include_str!("../../../rut/nmapset/src/mod.rut")),
        ("calc", include_str!("../../../rut/calc/src/mod.rut")),
        ("json", include_str!("../../../rut/json/src/mod.rut")),
        ("ink", include_str!("../../../rut/ink/src/mod.rut")),
    ];
    for (n, src) in srcs {
        io.put(
            &format!("/ws/{n}/rut.jsonc"),
            &format!("{{\n  \"name\": \"{n}\"\n}}\n"),
        );
        io.put(&format!("/ws/{n}/src/mod.rut"), src);
    }
    (Rc::new(io), "/ws/proj".to_string())
}

/// the walk's seeded direct-dep set: a root manifest declaring
/// `names` (the auto-import tier's live gate — the compiler's ambient
/// split mirrored). The deps mount through the ordinary dir lane.
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

/// the transitive world — the module-tier twin of the auto-import
/// twin's world: the root declares `http` + `pads` (path deps) while
/// `http`'s OWN manifest declares `http_host` — the walk mounts it
/// (transitive by design; goto-def into http's files needs it), but
/// the root never declared it, so the compiler refuses `use
/// http_host::…` (the D5 diagnostic)
fn transitive_world() -> (Rc<MemIo>, String) {
    let mut io = MemIo::default();
    io.put(
        "/ws/proj/rut.jsonc",
        "{\n  \"name\": \"proj\",\n  \"deps\": {\"http\": {\"path\": \"/ws/http\"}, \"pads\": {\"path\": \"/ws/pads\"}}\n}\n",
    );
    io.put("/ws/proj/src/mod.rut", "fn main() -> nil {\n}\n");
    io.put(
        "/ws/http/rut.jsonc",
        "{\n  \"name\": \"http\",\n  \"deps\": {\"http_host\": {\"path\": \"/ws/http_host\"}}\n}\n",
    );
    io.put(
        "/ws/http/src/mod.rut",
        "pub fn get() -> str {\n    return \"\";\n}\n",
    );
    io.put(
        "/ws/http_host/rut.jsonc",
        "{\n  \"name\": \"http_host\"\n}\n",
    );
    io.put("/ws/http_host/src/mod.rut", "pub fn listen() -> nil {\n}\n");
    io.put("/ws/pads/rut.jsonc", "{\n  \"name\": \"pads\"\n}\n");
    io.put(
        "/ws/pads/src/mod.rut",
        "pub fn pad() -> i32 {\n    return 1;\n}\n",
    );
    (Rc::new(io), "/ws/proj".to_string())
}

#[test]
fn member_completion_lists_impl_methods_and_fields() {
    let mut s = S::boot();
    let src = "struct Counter {\nn: i32;\n}\nimpl Counter {\nfn bump(self) -> i32 { return self.n; }\nfn reset(self) -> nil { }\n}\nfn use_it(c: Counter) -> i32 {\nreturn c.\n}\n";
    let items = s.complete_at("file:///t/main.rut", src, "c.", 1);
    assert!(offers(&items, "bump"), "{:?}", labels(&items));
    assert!(offers(&items, "reset"));
    assert!(
        offers(&items, "n"),
        "fields complete too: {:?}",
        labels(&items)
    );
}

#[test]
fn member_completion_gates_foreign_interface_members() {
    // the foreign interface's members complete only once the doc
    // uses it; the document's own interface needs no gate
    let mut s = S::boot();
    s.index_file(
        "file:///t/greets.rut",
        "interface Greeter {\nfn greet(self) -> nil;\n}\n",
    );

    let gated = "fn go(g: Greeter) -> nil { g. }\n";
    let items = s.complete_at("file:///t/main.rut", gated, "g.", 1);
    assert!(
        !offers(&items, "greet"),
        "unused foreign interface must not complete: {:?}",
        labels(&items)
    );

    let used = "use greets::{ Greeter };\nfn go(g: Greeter) -> nil { g. }\n";
    let items = s.complete_at("file:///t/main.rut", used, "g.", 1);
    assert!(offers(&items, "greet"), "used interface completes");

    // the document's own interface is in scope natively
    let own = "interface Local {\nfn hi(self) -> nil;\n}\nfn go(l: Local) -> nil { l. }\n";
    let items = s.complete_at("file:///t/own.rut", own, "l.", 1);
    assert!(offers(&items, "hi"), "own interface completes");
}

#[test]
fn bare_completion_has_the_keyword_table_and_decls() {
    let mut s = S::boot();
    let src = "interface Shape {\nfn area(self) -> f64;\n}\nentry fn main() -> nil { }\n";
    let items = s.complete_at("file:///t/main.rut", src, "entry fn main() -> nil { }", 1);
    let ls = labels(&items);
    // the final keyword table — the current spellings, no retired ones
    for kw in ["interface", "async", "use", "impl", "let", "fn"] {
        assert!(ls.contains(&kw), "keyword `{kw}` completes: {ls:?}");
    }
    // `trait` retired to an ordinary identifier — the keyword table
    // must not offer it
    assert!(
        !ls.contains(&"trait"),
        "the retired spelling must not complete: {ls:?}"
    );
    assert!(!ls
        .iter()
        .any(|l| matches!(*l, "import" | "suspend" | "from" | "dyn")));
    // decls: the document's interface (as an interface) and fn
    let shape = item(&items, "Shape");
    assert_eq!(shape["kind"], k::INTERFACE, "{}", shape);
    assert!(ls.contains(&"main"), "{ls:?}");
    // methods are reached through a receiver, not named bare
    assert!(!ls.contains(&"area"), "{ls:?}");
}

#[test]
fn bare_completion_gates_pub_builtin_core_names() {
    // the `pub builtin` rows (the disposal pair and the weak
    // reference) complete only when the doc's `use` names them —
    // the compiler's ambient split, mirrored so a completion never
    // offers a name the compile rejects. Ambient core rows
    // (`prelude builtin`) stay ungated.
    let mut s = S::boot();
    let core_src = "pub builtin class Iterable<E> {\nfn next(self) -> ?E;\n}\n\
                    pub builtin class Disposal {\nfn dispose(self, cx: DisposalContext);\n}\n\
                    pub builtin class DisposalContext { }\n\
                    pub builtin class Weak { }\n\
                    prelude builtin class StackTrace { }\n";
    s.index_mod_at("file:///t/core/lib.rut", "core", "", core_src);

    let bare = "entry fn main() -> nil { }\n";
    let items = s.complete_at("file:///t/main.rut", bare, "entry fn main() -> nil { }", 1);
    let ls = labels(&items);
    for absent in ["Disposal", "DisposalContext", "Iterable", "Weak"] {
        assert!(
            !ls.contains(&absent),
            "the gated builtin name `{absent}` must not complete: {ls:?}"
        );
    }
    assert!(
        ls.contains(&"StackTrace"),
        "the ambient row still completes: {ls:?}"
    );

    let imported =
        "use core::{ Disposal, DisposalContext, Iterable, Weak };\nfn main() -> nil { }\n";
    let items = s.complete_at("file:///t/main.rut", imported, "fn main() -> nil { }", 1);
    let ls = labels(&items);
    for want in ["Disposal", "DisposalContext", "Iterable", "Weak"] {
        assert!(
            ls.contains(&want),
            "the imported `{want}` completes: {ls:?}"
        );
    }
}

// ---- the honest bare scope (A2) ----

#[test]
fn a_foreign_dep_fn_rides_only_the_import_tier_until_used() {
    // the dep mounts AND the doc's manifest declares it (the walk's
    // seeded set): bare doc — `tag` is NOT a plain bare item, the
    // only offer is the auto-import one (the `use gadgets::tag;`
    // edit rides it). After the `use`, `tag` IS a plain bare item —
    // and the auto-import tier never re-offers it.
    let (io, root) = declare_direct(&["gadgets"]);
    let mut s = S::with_io(&io);
    s.resolve(&[&root]);
    s.index_mod_at(
        "file:///deps/gadgets/lib.rut",
        "gadgets",
        "",
        "pub struct Widget {\nid: i32;\n}\npub fn tag() -> i32 { return 1; }\n",
    );
    let uri = "file:///t/main.rut";
    let items = s.complete_at(
        uri,
        "entry fn main() -> nil { }\n",
        "entry fn main() -> nil { }",
        1,
    );
    assert!(
        items
            .iter()
            .all(|c| c["label"] != "tag" || c["additionalTextEdits"].is_array()),
        "the dep fn must not complete plain: {:?}",
        labels(&items)
    );
    let imp = import_offer(&items, "tag").expect("the direct dep's fn auto-imports");
    assert_eq!(imp["detail"], "gadgets::tag — import", "{}", imp);

    // after the `use`: a plain bare item, no re-offer
    let items = s.complete_at(
        uri,
        "use gadgets::{ tag };\nentry fn main() -> nil { }\n",
        "entry fn main() -> nil { }",
        1,
    );
    let plain = item(&items, "tag");
    assert!(
        plain["additionalTextEdits"].is_null(),
        "the imported fn completes plain, no insert: {plain}"
    );
    assert!(
        import_offer(&items, "tag").is_none(),
        "never re-offered: {:?}",
        labels(&items)
    );
}

#[test]
fn a_transitive_only_dep_never_auto_imports() {
    // the compiler's closure check refuses `use` of a mounted-but-
    // undeclared package — the auto-import tier mirrors it: a module
    // NOT in the doc's seeded dep set never rides the tier
    let (io, root) = declare_direct(&[]); // the root declares nothing
    let mut s = S::with_io(&io);
    s.resolve(&[&root]);
    s.index_mod_at(
        "file:///deps/gadgets/lib.rut",
        "gadgets",
        "",
        "pub struct Widget {\nid: i32;\n}\npub fn tag() -> i32 { return 1; }\n",
    );
    let items = s.complete_at(
        "file:///t/main.rut",
        "entry fn main() -> nil { }\n",
        "entry fn main() -> nil { }",
        1,
    );
    assert!(
        import_offer(&items, "tag").is_none(),
        "a transitive-only dep's fn must not auto-import: {:?}",
        labels(&items)
    );
    assert!(
        !offers(&items, "tag"),
        "and no plain offer either: {:?}",
        labels(&items)
    );
}

#[test]
fn a_transitive_only_dep_never_rides_the_use_module_tier() {
    // the module-name tier's twin of `a_transitive_only_dep_never_auto_imports`:
    // `use <cursor>` competes on the SAME direct-dep gate — `http`'s
    // own dep `http_host` mounts into the chain (transitive by
    // design), but the compiler refuses `use http_host::…`
    // (nameability is declaration-scoped, the D5 diagnostic), so the
    // tier never OFFERS it — a plain offer the compile would reject
    // is a trap. The declared `pads` is the positive control.
    let (io, root) = transitive_world();
    let mut s = S::with_io(&io);
    s.resolve(&[&root]);
    let uri = "file:///t/main.rut";
    // `use ⏐` — the nameable set: the two declared deps + ambient core
    let items = s.complete_pos(uri, "use ", 0, 4);
    let ls = labels(&items);
    assert!(ls.contains(&"http"), "the declared dep offers: {ls:?}");
    assert!(
        ls.contains(&"pads"),
        "the second declared dep offers: {ls:?}"
    );
    assert!(
        ls.contains(&"core"),
        "the ambient prelude rides its origin: {ls:?}"
    );
    assert!(
        !ls.contains(&"http_host"),
        "a transitive-only dep never offers on the module tier: {ls:?}"
    );
    // `use h⏐` — the prefix filters through the gate
    let items = s.complete_pos(uri, "use h", 0, 5);
    let ls = labels(&items);
    assert!(ls.contains(&"http"), "{ls:?}");
    assert!(!ls.contains(&"http_host"), "{ls:?}");
    // typing the transitive name in full still never offers
    let items = s.complete_pos(uri, "use http_host", 0, 13);
    assert!(
        !offers(&items, "http_host"),
        "the mounted-but-undeclared module stays off: {:?}",
        labels(&items)
    );
    // the positive control: the second declared dep still offers
    let items = s.complete_pos(uri, "use p", 0, 5);
    assert!(
        offers(&items, "pads"),
        "a declared dep still offers: {:?}",
        labels(&items)
    );
}

#[test]
fn a_name_reachable_two_ways_offers_exactly_one_item() {
    // A4: `tag` completes bare (the doc's `use` names it) AND the
    // auto-import tier would offer it — the merged list retains the
    // plain one, exactly one item by the label
    let (io, root) = declare_direct(&["gadgets"]);
    let mut s = S::with_io(&io);
    s.resolve(&[&root]);
    s.index_mod_at(
        "file:///deps/gadgets/lib.rut",
        "gadgets",
        "",
        "pub struct Widget {\nid: i32;\n}\npub fn tag() -> i32 { return 1; }\n",
    );
    let items = s.complete_at(
        "file:///t/main.rut",
        "use gadgets::{ tag };\nentry fn main() -> nil { }\n",
        "entry fn main() -> nil { }",
        1,
    );
    let hits: Vec<&Value> = items.iter().filter(|c| c["label"] == "tag").collect();
    assert_eq!(hits.len(), 1, "one `tag` item: {:?}", labels(&items));
    assert!(
        hits[0]["additionalTextEdits"].is_null(),
        "the plain (bare) item wins: {}",
        hits[0]
    );
}

// ---- the brace-form use-path shapes (A6 regression pins) ----

#[test]
fn the_brace_form_rides_the_names_tier() {
    // `use gadgets::{⏐` — exports; the comma form too; the prefix
    // filters; an unmounted head stays an honest empty. The dep is
    // the FLAT shape (Widget + tag at the root).
    let mut s = S::boot();
    s.index_mod_at(
        "file:///deps/gadgets/lib.rut",
        "gadgets",
        "",
        "pub struct Widget {\nid: i32;\n}\npub fn tag() -> i32 { return 1; }\n",
    );
    let uri = "file:///t/main.rut";
    for doc in ["use gadgets::{", "use gadgets::{ tag, "] {
        let items = s.complete_pos(uri, doc, 0, doc.len() as u32);
        let ls = labels(&items);
        assert!(
            ls.contains(&"Widget"),
            "exports complete in `{doc}`: {ls:?}"
        );
        assert!(ls.contains(&"tag"), "{ls:?}");
    }
    // `use gadgets::{ Widget, ⏐` — still the full exports
    let items = s.complete_pos(uri, "use gadgets::{ Widget, ", 0, 25);
    assert!(offers(&items, "tag"), "{:?}", labels(&items));
    // `use gadgets::{ Widget, ta⏐` — the prefix filters
    let items = s.complete_pos(uri, "use gadgets::{ Widget, ta", 0, 25);
    let ls = labels(&items);
    assert!(ls.contains(&"tag"), "{ls:?}");
    assert!(
        !ls.contains(&"Widget"),
        "the prefix filters the brace form: {ls:?}"
    );
    // an unmounted head stays an honest empty
    let items = s.complete_pos(uri, "use ghost::{", 0, 12);
    assert!(items.is_empty(), "{:?}", labels(&items));
}

#[test]
fn string_receiver_completes_str_surface() {
    // std index ahead of the doc: `str`'s builtin contract supplies
    // the member list
    let mut s = S::boot();
    s.index_file(
        "file:///t/core.d.rut",
        "prelude builtin primitive str {\nfn len(self) -> i32;\n}\n",
    );
    let src = "fn f(s: str) -> i32 { return s. }\n";
    let items = s.complete_at("file:///t/main.rut", src, "s.", 1);
    assert!(offers(&items, "len"), "{:?}", labels(&items));
}

// ---- the use-path tiers ----

#[test]
fn use_path_completion_lists_the_chain_module_names() {
    // `use ⏐` — module names from the chain's NAMEABLE indexes: the
    // root manifest's own rows (the walk seeded the direct set — std
    // is not special; no hardcoded table) plus the ambient prelude
    let (io, root) = std_io_world();
    let mut s = S::with_io(&io);
    s.resolve(&[&root]);
    let uri = "file:///t/main.rut";
    let items = s.complete_pos(uri, "use ", 0, 4);
    let ls = labels(&items);
    for want in ["pouch", "core", "nmapset", "calc", "json"] {
        assert!(ls.contains(&want), "module `{want}` completes: {ls:?}");
    }
    let pouch = item(&items, "pouch");
    assert_eq!(pouch["kind"], k::MODULE, "{}", pouch);
    // the partial prefix filters
    let items = s.complete_pos(uri, "use po", 0, 6);
    let ls = labels(&items);
    assert!(ls.contains(&"pouch"), "{ls:?}");
    assert!(!ls.contains(&"core"), "{ls:?}");
    // and the keyword tier stays OUT of the use path
    assert!(!ls.contains(&"fn"), "{ls:?}");
}

#[test]
fn use_path_completion_lists_the_modules_exports() {
    // `use pouch::V⏐` — the module's public names; the brace form
    // rides the same tier; the pub-less internals never offer
    let mut s = S::boot();
    mount_std(&mut s);
    let uri = "file:///t/main.rut";
    let items = s.complete_pos(uri, "use pouch::V", 0, 12);
    assert!(offers(&items, "Vec"), "{:?}", labels(&items));
    let items = s.complete_pos(uri, "use pouch::{V", 0, 13);
    assert!(offers(&items, "Vec"), "{:?}", labels(&items));
    // nmapset's pub names
    let items = s.complete_pos(uri, "use nmapset::H", 0, 14);
    let ls = labels(&items);
    assert!(ls.contains(&"HashMap"), "{ls:?}");
    assert!(ls.contains(&"HashSet"), "{ls:?}");
}

#[test]
fn use_path_completion_lists_the_dep_lets_and_mods() {
    // a dep index (the calc_host shape: surface `pub let` rows) + a
    // mod-carrying dep (the calc shape): the names tier offers the
    // lets and the dep's `pub mod` children
    let mut s = S::boot();
    s.index_mod_at(
        "file:///deps/calc_host.d.rut",
        "calc_host",
        "",
        "pub host fn sqrt(x: f64) -> f64;\npub let PI: f64 = 3.141592653589793;\n",
    );
    s.index_mod_at("file:///deps/calc/mod.rut", "calc", "", "pub mod math;\n");
    s.index_mod_at(
        "file:///deps/calc/math.rut",
        "calc",
        "math",
        "pub use calc_host::{ PI };\npub fn sqrt(x: f64) -> f64 { return x; }\n",
    );
    let uri = "file:///t/main.rut";
    let items = s.complete_pos(uri, "use calc_host::", 0, 15);
    let ls = labels(&items);
    assert!(ls.contains(&"sqrt"), "the module's fns complete: {ls:?}");
    assert!(ls.contains(&"PI"), "the surface lets complete: {ls:?}");
    let pi = item(&items, "PI");
    assert_eq!(pi["kind"], k::CONSTANT, "{}", pi);
    // the dep's mod tree walks: `use calc::` offers the `pub mod` child
    let items = s.complete_pos(uri, "use calc::", 0, 10);
    assert!(
        offers(&items, "math"),
        "the pub mod child completes: {:?}",
        labels(&items)
    );
}

// ---- the file-module walk (use-path + position tiers) ----

#[test]
fn use_path_completion_walks_the_dep_mod_tree() {
    // `use gadgets::⏐` — the root's exports PLUS its pub mod
    // children; the private `secret` edge never offers; the child's
    // members live under their path (no Column leak)
    let mut s = S::boot();
    mount_gadget_dep(&mut s);
    let uri = "file:///t/main.rut";
    let items = s.complete_pos(uri, "use gadgets::", 0, 13);
    let ls = labels(&items);
    assert!(
        ls.contains(&"layout"),
        "the pub mod child completes: {ls:?}"
    );
    assert!(ls.contains(&"tag"), "the root's fn completes: {ls:?}");
    assert!(
        !ls.contains(&"secret"),
        "a bare `mod` edge never crosses: {ls:?}"
    );
    assert!(
        !ls.contains(&"Column"),
        "child members stay under their path: {ls:?}"
    );
    let layout = item(&items, "layout");
    assert_eq!(layout["kind"], k::MODULE, "{}", layout);
    assert!(
        layout["detail"]
            .as_str()
            .unwrap()
            .contains("layout/mod.rut"),
        "{}",
        layout
    );

    // the prefix filters the tier
    let items = s.complete_pos(uri, "use gadgets::la", 0, 15);
    let ls = labels(&items);
    assert!(ls.contains(&"layout"), "{ls:?}");
    assert!(!ls.contains(&"tag"), "{ls:?}");

    // `use gadgets::layout::⏐` — the walked module's exports, the
    // pub gate on fns now binding (crossed), plus its pub mod children
    let items = s.complete_pos(uri, "use gadgets::layout::", 0, 21);
    let ls = labels(&items);
    assert!(ls.contains(&"Column"), "{ls:?}");
    assert!(ls.contains(&"grid"), "the nested pub mod completes: {ls:?}");
    assert!(
        !ls.contains(&"for_parent"),
        "only `pub` leaf names cross packages: {ls:?}"
    );
    assert!(
        !ls.contains(&"internal"),
        "a private fn never crosses: {ls:?}"
    );

    // deeper: `use gadgets::layout::grid::⏐`
    let items = s.complete_pos(uri, "use gadgets::layout::grid::", 0, 27);
    let ls = labels(&items);
    assert!(ls.contains(&"Cell"), "{ls:?}");
    assert!(ls.contains(&"mk"), "{ls:?}");

    // the brace form rides the same walk
    let items = s.complete_pos(uri, "use gadgets::layout::{C", 0, 23);
    assert!(offers(&items, "Column"), "{:?}", labels(&items));

    // a private edge is a dead end; a ghost is a miss — both empty
    let items = s.complete_pos(uri, "use gadgets::secret::", 0, 21);
    assert!(items.is_empty(), "{:?}", labels(&items));
    let items = s.complete_pos(uri, "use gadgets::ghost::", 0, 20);
    assert!(items.is_empty(), "{:?}", labels(&items));
}

#[test]
fn position_path_completion_offers_the_module_members_by_tier() {
    // the open document IS the gadgets root (stamped through the scan
    // lane): `mod layout;` + `mod secret;` declared in it. Positions
    // qualify with `.` (`let c: layout.Column`) — the real spelling
    let mut s = S::boot();
    mount_gadget_dep(&mut s);
    let uri = "file:///deps/gadgets/main.rut";
    let stamp = |s: &mut S, src: &str| {
        s.index_mod_at(uri, "gadgets", "", src);
    };

    let doc = "mod layout;\nmod secret;\nentry fn main() -> nil {\n    let c: layout.\n}\n";
    stamp(&mut s, doc);
    let items = s.complete_at(uri, doc, "let c: layout.", 1);
    let ls = labels(&items);
    assert!(ls.contains(&"Column"), "the pub type completes: {ls:?}");
    assert!(
        ls.contains(&"grid"),
        "any declared edge walks intra-package: {ls:?}"
    );
    assert!(
        ls.contains(&"for_parent"),
        "pub(super) reaches the parent (the root): {ls:?}"
    );
    assert!(
        !ls.contains(&"internal"),
        "a private fn of a child stays hidden: {ls:?}"
    );

    // the `::` spelling rides the same tier (the plan's letter)
    let doc = "mod layout;\nmod secret;\nentry fn main() -> nil {\n    let c: layout::\n}\n";
    stamp(&mut s, doc);
    let items = s.complete_at(uri, doc, "let c: layout::", 1);
    assert!(offers(&items, "Column"), "{:?}", labels(&items));

    // a child module's own file: `secret`'s members from the root —
    // pub yes, private no
    let doc = "mod layout;\nmod secret;\nentry fn main() -> nil {\n    let f: secret.\n}\n";
    stamp(&mut s, doc);
    let items = s.complete_at(uri, doc, "let f: secret.", 1);
    let ls = labels(&items);
    assert!(ls.contains(&"open"), "{ls:?}");
    assert!(
        !ls.contains(&"hidden"),
        "a private fn of a sibling is not visible: {ls:?}"
    );

    // the own-pkg head names the root module: `gadgets.` from the
    // root offers the root's own surface + children
    let doc = "mod layout;\nmod secret;\npub fn root_fn() -> i32 { return 9; }\nentry fn main() -> nil {\n    let t: gadgets.\n}\n";
    stamp(&mut s, doc);
    let items = s.complete_at(uri, doc, "let t: gadgets.", 1);
    let ls = labels(&items);
    assert!(ls.contains(&"root_fn"), "{ls:?}");
    assert!(ls.contains(&"layout"), "{ls:?}");

    // a deeper doc (layout/grid), stamped at its own uri beside the
    // mounted tree: a sibling via the parent scope, the pkg head
    // reaching the root's decls
    let grid_uri = "file:///t/grid.rut";
    let grid_doc = "pub struct Cell {\n    x: i32;\n}\nentry fn main() -> nil {\n    let c: layout.Column\n}\n";
    s.index_mod_at(grid_uri, "gadgets", "layout/grid", grid_doc);
    let items = s.complete_at(grid_uri, grid_doc, "let c: layout.", 1);
    let ls = labels(&items);
    assert!(
        ls.contains(&"Column"),
        "the sibling-of-parent resolves: {ls:?}"
    );
    // grid is a descendant of layout: layout's PRIVATES are visible
    // (the declaring module + descendants law)
    let grid_doc = "pub struct Cell {\n    x: i32;\n}\nentry fn main() -> nil {\n    let f: layout.internal\n}\n";
    s.index_mod_at(grid_uri, "gadgets", "layout/grid", grid_doc);
    let items = s.complete_at(grid_uri, grid_doc, "let f: layout.", 1);
    let ls = labels(&items);
    assert!(
        ls.contains(&"internal"),
        "a descendant reads the parent's privates: {ls:?}"
    );

    // no doc mods (unknown provenance): the position tier stays off —
    // a plain type receiver falls through to the member tier (empty
    // for an unknown receiver — never wrong names)
    let flat_uri = "file:///t/flat.rut";
    let flat = "mod layout;\nmod secret;\nentry fn main() -> nil {\n    let c: layout.\n}\n";
    let items = s.complete_at(flat_uri, flat, "let c: layout.", 1);
    let ls = labels(&items);
    assert!(!ls.contains(&"Column"), "no doc mods, no mod tier: {ls:?}");
}

#[test]
fn the_use_path_tier_answers_an_own_package_head() {
    // the open document IS the gadgets root (its stamped twin is the
    // ONE root in the world): `use gadgets::layout::⏐` rides the own
    // lane — the position machinery's visible-members gate, so
    // `pub(super)` reaches the root where the cross-package tier's
    // pub-crossing law stays shut
    let mut s = S::boot();
    let uri = "file:///deps/gadgets/main.rut";
    s.index_mod_at(
        "file:///deps/gadgets/layout/mod.rut",
        "gadgets",
        "layout",
        "pub mod grid;\npub struct Column {\n    w: i32;\n}\nfn internal() -> i32 { return 2; }\npub(super) fn for_parent() -> i32 { return 3; }\n",
    );
    // the open document IS the gadgets root (its stamped twin keys the
    // own lane; the doc carries the use it types)
    let doc =
        "pub mod layout;\npub mod secret;\nuse gadgets::layout::\nentry fn main() -> nil {\n}\n";
    s.index_mod_at(uri, "gadgets", "", doc);
    let items = s.complete_at(uri, doc, "use gadgets::layout::", 1);
    let ls = labels(&items);
    assert!(ls.contains(&"Column"), "{ls:?}");
    assert!(
        ls.contains(&"for_parent"),
        "the own lane gates by visibility, not the pub-crossing law: {ls:?}"
    );
    assert!(ls.contains(&"grid"), "the walked child rides too: {ls:?}");
    assert!(
        !ls.contains(&"internal"),
        "a private fn stays hidden: {ls:?}"
    );

    // the bare own-pkg head: the doc's own rows are the scope (the
    // freshness law) — the declared children offer
    let doc = "pub mod layout;\npub mod secret;\nuse gadgets::\nentry fn main() -> nil {\n}\n";
    s.index_mod_at(uri, "gadgets", "", doc);
    let items = s.complete_at(uri, doc, "use gadgets::", 1);
    let ls = labels(&items);
    assert!(ls.contains(&"layout"), "the own root's edges offer: {ls:?}");
    assert!(ls.contains(&"secret"), "{ls:?}");

    // an own mod edge head — `use layout::⏐` from the root walks the
    // ancestor spelling to the same tier
    let doc = "pub mod layout;\nuse layout::\nentry fn main() -> nil {\n}\n";
    s.index_mod_at(uri, "gadgets", "", doc);
    let items = s.complete_at(uri, doc, "use layout::", 1);
    let ls = labels(&items);
    assert!(
        ls.contains(&"Column"),
        "the own mod edge head rides the same tier: {ls:?}"
    );
    assert!(ls.contains(&"grid"), "{ls:?}");
}

// ---- the `pub use` re-export tier ----

#[test]
fn use_path_completion_offers_the_reexported_names() {
    // `use mid::{ ⏐` — the `pub use`'s names joined the module's
    // surface: they offer like any export, the detail naming the edge
    // (`re-export of util::..`); the cap law picks the icon. A
    // private use's name never offers — the crossing law speaks on
    // flat and walked paths alike.
    let mut s = S::boot();
    mount_mid_dep(&mut s);
    let uri = "file:///t/main.rut";
    let items = s.complete_pos(uri, "use mid::{", 0, 10);
    let ls = labels(&items);
    assert!(ls.contains(&"helper"), "the fn-shaped re-export: {ls:?}");
    assert!(ls.contains(&"Widget"), "the type-shaped re-export: {ls:?}");
    let helper = item(&items, "helper");
    assert!(
        helper["detail"]
            .as_str()
            .unwrap()
            .contains("re-export of util::helper"),
        "{}",
        helper
    );
    assert_eq!(helper["kind"], k::FUNCTION, "{}", helper);
    let widget = item(&items, "Widget");
    assert_eq!(widget["kind"], k::CLASS, "{}", widget);
    assert!(!ls.contains(&"secret_root"), "{ls:?}");
}

#[test]
fn a_walked_module_offers_only_pub_reexports() {
    // `use mid::sub::{ ⏐` — a crossed path: only `pub` re-export
    // edges cross packages, exactly the fns/lets gate
    let mut s = S::boot();
    mount_mid_dep(&mut s);
    let items = s.complete_pos("file:///t/main.rut", "use mid::sub::{", 0, 15);
    let ls = labels(&items);
    assert!(ls.contains(&"tag"), "the pub re-export crosses: {ls:?}");
    assert!(
        !ls.contains(&"secret_sub"),
        "a private use's name never crosses: {ls:?}"
    );
}

// ---- the type-head tier (after `Type::`) ----

#[test]
fn colon2_completion_lists_the_enum_members() {
    // `Color::⏐` — the enum head's member list rides name resolution,
    // not a receiver dot; the typed prefix filters the member list
    let mut s = S::boot();
    let uri = "file:///t/main.rut";
    let src = "enum Color { Red, Green }\nentry fn main() -> nil {\n    let c = Color::\n}\n";
    let items = s.complete_at(uri, src, "Color::", 1);
    let ls = labels(&items);
    assert!(ls.contains(&"Red"), "{ls:?}");
    assert!(ls.contains(&"Green"), "{ls:?}");
    let red = item(&items, "Red");
    assert_eq!(red["kind"], k::ENUM_MEMBER, "{}", red);
    let src = "enum Color { Red, Green }\nentry fn main() -> nil {\n    let c = Color::R\n}\n";
    let items = s.complete_at(uri, src, "Color::R", 1);
    let ls = labels(&items);
    assert!(ls.contains(&"Red"), "{ls:?}");
    assert!(!ls.contains(&"Green"), "{ls:?}");
}

#[test]
fn colon2_completion_lists_the_dep_module_statics() {
    // `math::⏐` over the calc dep's stamped module index — the
    // module's pub fns and its re-exported consts
    let mut s = S::boot();
    s.index_mod_at("file:///deps/calc/mod.rut", "calc", "", "pub mod math;\n");
    s.index_mod_at(
        "file:///deps/calc/math.rut",
        "calc",
        "math",
        "pub use calc_host::{ PI };\npub fn sqrt(x: f64) -> f64 { return x; }\n",
    );
    let uri = "file:///t/main.rut";
    let src = "use calc::{ math };\nentry fn main() -> nil {\n    let x = math::\n}\n";
    let items = s.complete_at(uri, src, "math::", 1);
    let ls = labels(&items);
    assert!(ls.contains(&"sqrt"), "the module's fns complete: {ls:?}");
    assert!(
        ls.contains(&"PI"),
        "the pub use's re-exported names complete: {ls:?}"
    );
    let sqrt = item(&items, "sqrt");
    assert_eq!(sqrt["kind"], k::FUNCTION, "{}", sqrt);
    let pi = item(&items, "PI");
    assert_eq!(
        pi["kind"],
        k::CLASS,
        "the cap law picks the icon until real resolution: {}",
        pi
    );
}

#[test]
fn colon2_completion_lists_the_type_head_associated_fns() {
    // `Counter::⏐` — the statics + the inherent impl block's fns
    // (the owner string IS the type's name); instance fields stay
    // OFF — they ride a receiver dot, never `::`
    let mut s = S::boot();
    let src = "class Counter {\nn: i32;\npub static COUNT: i32;\n}\nimpl Counter {\nfn bump(self) -> i32 { return self.n; }\nfn mk() -> Counter { return Counter { n: 0 }; }\n}\nentry fn main() -> nil {\n    let c = Counter::\n}\n";
    let items = s.complete_at("file:///t/main.rut", src, "Counter::", 1);
    let ls = labels(&items);
    assert!(ls.contains(&"bump"), "{ls:?}");
    assert!(ls.contains(&"mk"), "{ls:?}");
    assert!(ls.contains(&"COUNT"), "the static rides the head: {ls:?}");
    let count = item(&items, "COUNT");
    assert_eq!(count["kind"], k::FIELD, "{}", count);
    assert!(
        !ls.contains(&"n"),
        "instance fields ride a receiver: {ls:?}"
    );
}

#[test]
fn colon2_static_gate_rides_the_modifier_flag_not_the_render() {
    // the static gate reads the field's own modifier flags, never a
    // prefix string-match on the rendered slice — an odd spelling can
    // no longer hide a static
    let mut s = S::boot();
    let src = "class Counter {\npub  static COUNT: i32;\nstatic OTHER: i32;\n}\nentry fn main() -> nil {\n    let c = Counter::\n}\n";
    let items = s.complete_at("file:///t/main.rut", src, "Counter::", 1);
    let ls = labels(&items);
    assert!(
        ls.contains(&"COUNT"),
        "the oddly-spelled static rides the flag: {ls:?}"
    );
    assert!(
        ls.contains(&"OTHER"),
        "the bare static rides the flag: {ls:?}"
    );
}

#[test]
fn colon2_interface_head_and_deeper_walks_offer_nothing() {
    // an interface head: its signatures ride receivers, never `::` —
    // the known head is FINAL (empty), never the bare tier; a known
    // head with a non-matching prefix: filtered empty; a type head is
    // not a namespace: nothing descends past a member
    let mut s = S::boot();
    let uri = "file:///t/main.rut";
    let src = "interface Greeter {\nfn greet(self) -> nil;\n}\nentry fn main() -> nil {\n    let g = Greeter::\n}\n";
    let items = s.complete_at(uri, src, "Greeter::", 1);
    assert!(items.is_empty(), "{:?}", labels(&items));
    let src = "enum Color { Red, Green }\nentry fn main() -> nil {\n    let c = Color::X\n}\n";
    let items = s.complete_at(uri, src, "Color::X", 1);
    assert!(items.is_empty(), "{:?}", labels(&items));
    let src = "enum Color { Red, Green }\nentry fn main() -> nil {\n    let c = Color::Red::\n}\n";
    let items = s.complete_at(uri, src, "Color::Red::", 1);
    assert!(items.is_empty(), "{:?}", labels(&items));
}

// ---- the type tier at annotation boundaries (A3) ----

#[test]
fn the_type_tier_answers_a_struct_field_annotation() {
    // `id: u⏐` — the primitive table rides the prefix filter; the
    // keyword table, the doc's fns, and the fn-locals stay OFF
    // (values never ride a type position)
    let mut s = S::boot();
    let src =
        "struct Slot {\nid: u\n}\nfn go(ready: bool) -> nil {\n    let count = 1;\n    count\n}\n";
    let items = s.complete_at("file:///t/main.rut", src, "id: u", 1);
    let ls = labels(&items);
    for want in ["u8", "u16", "u32", "u64"] {
        assert!(
            ls.contains(&want),
            "primitive `{want}` completes at the annotation: {ls:?}"
        );
    }
    assert!(!ls.contains(&"i32"), "the prefix filters the tier: {ls:?}");
    assert!(
        !ls.contains(&"use"),
        "no keyword tier at a type position: {ls:?}"
    );
    assert!(!ls.contains(&"fn") && !ls.contains(&"let"), "{ls:?}");
    assert!(!ls.contains(&"go"), "no fns at a type position: {ls:?}");
    assert!(
        !ls.contains(&"ready") && !ls.contains(&"count"),
        "no locals at a type position: {ls:?}"
    );
    let u32 = item(&items, "u32");
    assert_eq!(u32["kind"], k::CLASS, "{}", u32);
}

#[test]
fn the_type_tier_answers_the_bare_colon_boundary() {
    // `let c: ⏐` — the lone colon: the full tier, the doc's own types
    // included, never keywords or fns
    let mut s = S::boot();
    let src = "enum Color { Red }\nentry fn main() -> nil {\n    let c: \n}\n";
    let items = s.complete_at("file:///t/main.rut", src, "let c:", 1);
    let ls = labels(&items);
    for want in ["bool", "str", "f64", "i32", "nil"] {
        assert!(ls.contains(&want), "primitive `{want}` completes: {ls:?}");
    }
    let color = item(&items, "Color");
    assert_eq!(color["kind"], k::CLASS, "{}", color);
    assert!(
        !ls.contains(&"fn") && !ls.contains(&"let"),
        "no keywords: {ls:?}"
    );
    assert!(!ls.contains(&"main"), "no fns: {ls:?}");
}

#[test]
fn type_completion_details_are_the_verbatim_heads() {
    // the detail label rides the row's verbatim `head` — the keyword,
    // the spelled modifiers and the generics exactly as written
    let mut s = S::boot();
    let src = "pub class Stack<T> {\nitems: Vec<T>;\n}\nenum Color { Red, Green }\nentry fn main() -> nil { let s: \n}\n";
    let items = s.complete_at("file:///t/main.rut", src, "let s:", 1);
    let stack = item(&items, "Stack");
    assert_eq!(stack["detail"], "pub class Stack<T>", "{}", stack);
    let color = item(&items, "Color");
    assert_eq!(color["detail"], "enum Color", "{}", color);
}

#[test]
fn the_type_tier_answers_the_arrow_boundary() {
    // `fn f() -> ⏐` — the return annotation rides the same tier …
    // with the partial ident prefix-filtering
    let mut s = S::boot();
    let uri = "file:///t/main.rut";
    let src = "struct Thing {\nx: i32;\n}\nfn f() -> \n";
    let items = s.complete_at(uri, src, "-> ", 1);
    let ls = labels(&items);
    assert!(ls.contains(&"i32") && ls.contains(&"Thing"), "{ls:?}");
    assert!(
        !ls.contains(&"fn") && !ls.contains(&"f"),
        "no keywords, no fns: {ls:?}"
    );
    let src = "struct Thing {\nx: i32;\n}\nfn f() -> T\n";
    let items = s.complete_at(uri, src, "-> T", 1);
    let ls = labels(&items);
    assert!(ls.contains(&"Thing"), "{ls:?}");
    assert!(
        !ls.contains(&"i32"),
        "the prefix filters the primitives: {ls:?}"
    );
}

#[test]
fn the_type_tier_answers_a_param_annotation() {
    // `fn f(x: u⏐` — parameter annotations are type positions too
    let mut s = S::boot();
    let src = "fn f(x: u) -> nil { }\n";
    let items = s.complete_at("file:///t/main.rut", src, "x: u", 1);
    let ls = labels(&items);
    assert!(ls.contains(&"u32") && ls.contains(&"u64"), "{ls:?}");
    assert!(!ls.contains(&"fn"), "{ls:?}");
}

#[test]
fn the_type_tier_enumerates_exactly_the_primitive_table() {
    // the tier's primitive rows ARE the parser's one PRIMITIVE_TYS
    // table — never a duplicated, drifting list
    let mut s = S::boot();
    let src = "entry fn main() -> nil {\n    let c: \n}\n";
    let items = s.complete_at("file:///t/main.rut", src, "let c:", 1);
    let mut got: Vec<&str> = labels(&items)
        .into_iter()
        .filter(|l| rut_ast::parser::is_primitive_ty(l))
        .collect();
    got.sort_unstable();
    let mut want = rut_ast::parser::PRIMITIVE_TYS.to_vec();
    want.sort_unstable();
    assert_eq!(got, want, "the tier's primitives are exactly PRIMITIVE_TYS");
}

#[test]
fn the_type_tier_offers_the_ambient_and_imported_foreign_types() {
    // a foreign doc: its own types plus the ambient builtin row
    // complete; the `pub builtin` gated row completes only once the
    // document's `use` names it
    let mut s = S::boot();
    s.index_mod_at(
        "file:///t/core/lib.rut",
        "core",
        "",
        "pub builtin class Weak { }\nprelude builtin class StackTrace { }\n",
    );
    let uri = "file:///t/main.rut";
    let bare = "entry fn main() -> nil {\n    let e: \n}\n";
    let items = s.complete_at(uri, bare, "let e:", 1);
    let ls = labels(&items);
    assert!(
        ls.contains(&"StackTrace"),
        "the ambient builtin type completes: {ls:?}"
    );
    assert!(
        !ls.contains(&"Weak"),
        "the gated builtin type stays off: {ls:?}"
    );
    let used = "use core::{ Weak };\nentry fn main() -> nil {\n    let e: \n}\n";
    let items = s.complete_at(uri, used, "let e:", 1);
    assert!(
        offers(&items, "Weak"),
        "the imported builtin type completes: {:?}",
        labels(&items)
    );
}

// ---- the auto-import tier ----

#[test]
fn auto_import_offers_foreign_names_with_the_insert_edit() {
    let (io, root) = std_io_world();
    let mut s = S::with_io(&io);
    s.resolve(&[&root]);
    let uri = "file:///t/main.rut";
    let src = "entry fn main() -> nil {\n    let v = Ve\n}\n";
    let items = s.complete_at(uri, src, "Ve", 1);
    let imp = import_offer(&items, "Vec").expect("Vec auto-imports from pouch");
    assert_eq!(imp["detail"], "pouch::Vec — import", "{}", imp);
    assert_eq!(
        imp["sortText"], "~Vec",
        "the tier sorts after the locals: {}",
        imp
    );
    // the insert: no uses, no leading comments → top of the body, a
    // fresh line, over the SAME normalized coordinates
    let edits = imp["additionalTextEdits"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    let range = &edits[0]["range"];
    assert_eq!(pos_pair(&range["start"]), (0, 0));
    assert_eq!(range["start"], range["end"], "an insertion");
    assert_eq!(edits[0]["newText"], "use pouch::Vec;\n");
}

#[test]
fn auto_import_lands_after_the_last_leading_use() {
    let (io, root) = std_io_world();
    let mut s = S::with_io(&io);
    s.resolve(&[&root]);
    let uri = "file:///t/main.rut";
    let src = "use core::{ NAN };\n\nentry fn main() -> nil {\n    let v = Ve\n}\n";
    let items = s.complete_at(uri, src, "Ve", 1);
    let imp = import_offer(&items, "Vec").expect("Vec auto-imports");
    let edits = imp["additionalTextEdits"].as_array().unwrap();
    // right after the leading use's `;` — column 18 of line 0 — with
    // the fresh use on the next line
    assert_eq!(pos_pair(&edits[0]["range"]["start"]), (0, 18));
    assert_eq!(edits[0]["newText"], "\nuse pouch::Vec;");
}

#[test]
fn auto_import_skips_leading_comments() {
    let (io, root) = std_io_world();
    let mut s = S::with_io(&io);
    s.resolve(&[&root]);
    let uri = "file:///t/main.rut";
    let src = "// the header prose\n\nentry fn main() -> nil {\n    let v = Ve\n}\n";
    let items = s.complete_at(uri, src, "Ve", 1);
    let imp = import_offer(&items, "Vec").expect("Vec auto-imports");
    let edits = imp["additionalTextEdits"].as_array().unwrap();
    // top of the file BODY — after the comment block and the blank
    assert_eq!(pos_pair(&edits[0]["range"]["start"]), (2, 0));
    assert_eq!(edits[0]["newText"], "use pouch::Vec;\n");
}

#[test]
fn already_imported_names_are_not_re_offered() {
    let (io, root) = std_io_world();
    let mut s = S::with_io(&io);
    s.resolve(&[&root]);
    let uri = "file:///t/main.rut";
    let src = "use pouch::{ Vec };\nentry fn main() -> nil {\n    let v = Ve\n}\n";
    let items = s.complete_at(uri, src, "Ve", 1);
    // the name still completes (the bare tier), but NO import edit
    // rides it — the document already imports it
    assert!(
        import_offer(&items, "Vec").is_none(),
        "the already-imported name must not re-offer: {:?}",
        labels(&items)
    );
}

// ---- constructor-call inference + the dot guard ----

#[test]
fn constructor_call_binding_completes_the_ink_surface() {
    // `let log = Logger("h")` — the constructor-call form types the
    // binding from the indexed class, so `log.` lists the impl
    // methods (new/debug/info/warn/error)
    let (io, root) = std_io_world();
    let mut s = S::with_io(&io);
    s.resolve(&[&root]);
    let doc = "fn go() -> nil {\n    let log = Logger(\"h\");\n    log.\n}\n";
    let items = s.complete_at("file:///t/main.rut", doc, "log.", 1);
    let ls = labels(&items);
    for want in ["new", "debug", "info", "warn", "error"] {
        assert!(
            ls.contains(&want),
            "`{want}` completes through the ctor binding: {ls:?}"
        );
    }
    // the bare tier's free fns never dump into a member position
    // (calc's `abs` rides the same fixture chain)
    assert!(!ls.contains(&"abs"), "no free-fn dump after a dot: {ls:?}");
}

#[test]
fn dot_guard_unknown_receiver_is_empty() {
    // `someUnknown.` — the receiver resolves to nothing: an EMPTY
    // list, never the keyword/host-fn dump (a miss is an empty list)
    let mut s = S::boot();
    mount_std(&mut s);
    let doc = "fn go() -> nil {\n    someUnknown.\n}\n";
    let items = s.complete_at("file:///t/main.rut", doc, "someUnknown.", 1);
    assert!(
        items.is_empty(),
        "an unresolved receiver completes nothing: {:?}",
        labels(&items)
    );
}

#[test]
fn dot_guard_non_name_receiver_is_empty() {
    // `probe().` — the token before the dot is not a name the tiers
    // understand; the text-backed dot guard keeps the bare tier out
    let mut s = S::boot();
    let doc = "fn go() -> nil {\n    let x = probe().\n}\n";
    let items = s.complete_at("file:///t/main.rut", doc, "probe().", 1);
    assert!(items.is_empty(), "{:?}", labels(&items));
}

// ---- the locals tier (plan3 phase 1, A1) ----

#[test]
fn locals_complete_at_a_bare_position_ahead_of_the_keywords() {
    // `che⏐` mid-block: the fn-local `checked` and the param `ready`
    // ride their own kinds, ahead of the keyword table
    let mut s = S::boot();
    let src = "fn go(ready: bool) -> nil {\n    let checked: str = \" \";\n    che\n}\n";
    let items = s.complete_at("file:///t/main.rut", src, "    che", 1);
    let ls = labels(&items);
    assert!(ls.contains(&"checked"), "the local completes: {ls:?}");
    assert!(ls.contains(&"ready"), "the param completes: {ls:?}");
    let checked = item(&items, "checked");
    assert_eq!(checked["kind"], k::VARIABLE, "{}", checked);
    let detail = checked["detail"].as_str().unwrap();
    assert!(
        detail.contains("let binding") && detail.contains("str"),
        "{detail}"
    );
    let ready = item(&items, "ready");
    assert_eq!(
        ready["kind"],
        k::VARIABLE,
        "the parameter rides the variable icon: {}",
        ready
    );
    // ahead of the keyword table
    let checked_at = items.iter().position(|c| c["label"] == "checked").unwrap();
    let let_kw_at = items.iter().position(|c| c["label"] == "let").unwrap();
    assert!(checked_at < let_kw_at, "locals ride ahead of the keywords");
}

#[test]
fn locals_offer_loop_variables_where_their_scope_reaches() {
    // the for-of var inside its body; the for-c counter inside the
    // cond/update too (its scope is the whole loop statement)
    let mut s = S::boot();
    let uri = "file:///t/main.rut";
    let src = "fn go(xs: [i32]) -> nil {\n    for (let v of xs) {\n        vv\n    }\n}\n";
    let items = s.complete_at(uri, src, "        vv", 1);
    let v = item(&items, "v");
    assert_eq!(v["kind"], k::VARIABLE, "{}", v);
    assert!(
        v["detail"].as_str().unwrap().contains("loop variable"),
        "{}",
        v
    );

    let src = "fn go(n: i32) -> nil {\n    let sum = 0;\n    for (let i = 0; i < n; i += su) {\n        sum += i;\n    }\n}\n";
    // `su` inside the UPDATE — the counter `i` and the outer `sum`
    // both still in scope
    let items = s.complete_at(uri, src, "i += su", 1);
    let ls = labels(&items);
    assert!(
        ls.contains(&"i"),
        "the counter completes in the update: {ls:?}"
    );
    assert!(ls.contains(&"sum"), "{ls:?}");
    assert!(ls.contains(&"n"), "the param reaches the loop: {ls:?}");
    let i = item(&items, "i");
    assert!(i["detail"].as_str().unwrap().contains("i32"), "{}", i);
}

#[test]
fn locals_shadow_inside_and_exit_their_scope() {
    // inside the if-block the INNER `flag` wins (one entry per name,
    // the innermost in-scope declaration rides); a binding declared
    // INSIDE the block is gone past it, while the fn-wide param rides
    let mut s = S::boot();
    let uri = "file:///t/main.rut";
    let src = "fn go(ready: bool) -> nil {\n    let flag: str = \"a\";\n    if (ready) {\n        let flag: i32 = 1;\n        fl\n    }\n    fl\n}\n";
    let items = s.complete_at(uri, src, "        fl", 1);
    let flag = item(&items, "flag");
    assert!(
        flag["detail"].as_str().unwrap().contains("i32"),
        "the inner shadow wins inside its scope: {}",
        flag
    );
    let src = "fn go(ready: bool) -> nil {\n    if (ready) {\n        let tmp: i32 = 1;\n        tm\n    }\n    tm\n}\n";
    let items = s.complete_at(uri, src, "        tm", 1);
    assert!(
        offers(&items, "tmp"),
        "inside its block the local completes"
    );
    let items = s.complete_at(uri, src, "    tm", 2);
    let ls = labels(&items);
    assert!(
        !ls.contains(&"tmp"),
        "the binding does not leak past its block: {ls:?}"
    );
    assert!(
        ls.contains(&"ready"),
        "the fn-wide param rides past the block: {ls:?}"
    );
}

#[test]
fn locals_skip_their_own_declaration_ident() {
    // the cursor inside the declaring ident — `let ch⏐e` — never
    // offers the name being typed; a use after the decl does
    let mut s = S::boot();
    let uri = "file:///t/main.rut";
    let src = "fn go() -> nil {\n    let che = 1;\n}\n";
    let items = s.complete_at(uri, src, "let ch", 1);
    let ls = labels(&items);
    assert!(
        !ls.contains(&"che"),
        "the declaration's own ident stays off"
    );
    assert!(!ls.contains(&"ch"), "the half-typed name stays off: {ls:?}");
    let src = "fn go() -> nil {\n    let che = 1;\n    che\n}\n";
    let items = s.complete_at(uri, src, "    che", 1);
    assert!(offers(&items, "che"), "{:?}", labels(&items));
}

// ---- the end-of-buffer scope fix (plan3 phase 1, A7) ----

/// the List probe doc's head — a type with a field and an impl
/// method for the member tier to answer with
const EOF_LIST_HEAD: &str = "\
class List {
    items: i32;
}
impl List {
    fn count(self) -> i32 { return 0; }
}
";

#[test]
fn member_completion_survives_the_end_of_buffer() {
    // THE regression: the text ends EXACTLY at the incomplete stmt
    let mut s = S::boot();
    let uri = "file:///t/main.rut";
    let src = format!("{EOF_LIST_HEAD}fn go() -> nil {{\n    let list = List.new();\n    list.");
    let items = s.complete_at(uri, &src, "    list.", 1);
    let ls = labels(&items);
    assert!(ls.contains(&"items"), "the field completes at EOF: {ls:?}");
    assert!(ls.contains(&"count"), "the method completes at EOF: {ls:?}");
    assert!(!ls.contains(&"fn"), "no keyword dump after a dot: {ls:?}");

    // one trailing newline — the same hole (the block span still
    // ends at the EOF token)
    let src = format!("{EOF_LIST_HEAD}fn go() -> nil {{\n    let list = List.new();\n    list.\n");
    let items = s.complete_at(uri, &src, "    list.", 1);
    let ls = labels(&items);
    assert!(
        ls.contains(&"items") && ls.contains(&"count"),
        "the member list survives the newline: {ls:?}"
    );

    // last stmt before `}` — the closed-block shape guards today's
    // behavior behind the fix
    let src =
        format!("{EOF_LIST_HEAD}fn go() -> nil {{\n    let list = List.new();\n    list.\n}}\n");
    let items = s.complete_at(uri, &src, "    list.", 1);
    let ls = labels(&items);
    assert!(ls.contains(&"items") && ls.contains(&"count"), "{ls:?}");
}

#[test]
fn locals_complete_at_the_end_of_buffer() {
    // A1 at EOF (post-A7): the doc ends mid-statement
    let mut s = S::boot();
    let uri = "file:///t/main.rut";
    let src = "fn go() -> nil {\n    let checked: str = \" \";\n    che";
    let items = s.complete_at(uri, src, "    che", 1);
    let ls = labels(&items);
    assert!(
        ls.contains(&"checked"),
        "the local completes at EOF: {ls:?}"
    );

    // the receiver itself still resolves for member completion when
    // the receiver is the LAST thing before the cursor
    let src = format!("{EOF_LIST_HEAD}fn go() -> nil {{\n    let list = List.new();\n    li");
    let items = s.complete_at(uri, &src, "    li", 1);
    assert!(
        offers(&items, "list"),
        "the local completes at EOF: {:?}",
        labels(&items)
    );
}

#[test]
fn the_end_of_buffer_extension_never_leaks_a_closed_block() {
    // a REAL `}` ends the scope exactly as before: the cursor past it
    // (at the source end) sees no fn-local — no leak out of the
    // closed block, with or without a trailing newline
    let mut s = S::boot();
    let uri = "file:///t/main.rut";
    for (src, needle, occ) in [
        (
            "fn go() -> nil {\n    let checked: str = \" \";\n}\n",
            "}\n",
            1usize,
        ),
        ("fn go() -> nil {\n    let checked: str = \" \";\n}", "}", 1),
    ] {
        let items = s.complete_at(uri, src, needle, occ);
        let ls = labels(&items);
        assert!(
            !ls.contains(&"checked"),
            "no leak past a closed block ({src:?}): {ls:?}"
        );
    }
    // and a closed first fn never leaks into an unterminated second
    let src = "fn a() -> nil {\n    let alpha: i32 = 1;\n}\nfn b() -> nil {\n    let beta: i32 = 2;\n    be";
    let items = s.complete_at(uri, src, "    be", 1);
    let ls = labels(&items);
    assert!(
        ls.contains(&"beta"),
        "the unterminated fn's local completes: {ls:?}"
    );
    assert!(
        !ls.contains(&"alpha"),
        "the closed fn's local stays in: {ls:?}"
    );
}

#[test]
fn member_completion_steps_through_self_ret_chains() {
    // the builder-chain law: an impl method's `-> Self` rung answers
    // the impl's target type, so the next dot enumerates the same
    // surface — the single ring and the multi-ring chain alike
    let mut s = S::boot();
    s.index_mod_at(
        "file:///deps/things/mod.rut",
        "things",
        "",
        "pub class Thing {\nc: i32;\n}\nimpl Thing {\npub fn m(self) -> Self { return self; }\npub fn done(self) -> i32 { return 1; }\n}\n",
    );
    let uri = "file:///t/main.rut";
    let src = "use things::{ Thing };\nentry fn main() -> nil {\n    Thing().m().\n}\n";
    let items = s.complete_at(uri, src, "Thing().m().", 1);
    let ls = labels(&items);
    assert!(
        ls.contains(&"m") && ls.contains(&"done"),
        "the `-> Self` rung completes the impl surface: {ls:?}"
    );

    // `m` answers `Self`, so the second ring steps to the same type
    let src = "use things::{ Thing };\nentry fn main() -> nil {\n    Thing().m().m().\n}\n";
    let items = s.complete_at(uri, src, "Thing().m().m().", 1);
    let ls = labels(&items);
    assert!(
        ls.contains(&"m") && ls.contains(&"done"),
        "the chain steps through both rungs: {ls:?}"
    );
}

#[test]
fn http_builder_chain_completes_after_self_steps() {
    // the regression lock for the report: the REAL std http lane
    // mounted as the dep (its cross-`use`s are inert here — the
    // member tier never walks them), the user's statement verbatim.
    // Every `-> Self` rung steps the receiver: the dot after
    // `.header(..)` offers the builder's surface — and NOT a
    // `Request`-only member, proving the type actually stepped to
    // `RequestBuilder` rather than dumping a bare name
    let mut s = S::boot();
    s.index_mod_at(
        "rut/http/src/mod.rut",
        "http",
        "",
        include_str!("../../../rut/http/src/mod.rut"),
    );
    let uri = "file:///t/main.rut";
    let src = "use http::HttpClient;\nentry fn main() -> nil {\n    let r = HttpClient().request()\n        .header(\"Content-Type\", \"application/json\").\n}\n";
    let items = s.complete_at(uri, src, "application/json\").", 1);
    let ls = labels(&items);
    for want in ["method", "url", "header", "body", "build"] {
        assert!(
            ls.contains(&want),
            "`{want}` rides the builder surface: {ls:?}"
        );
    }
    assert!(
        !ls.contains(&"send"),
        "a `Request`-only member must not offer: {ls:?}"
    );
}
