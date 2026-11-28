//! The prelude's gate — `core`, the ONE ambient index, driven through
//! the envelopes (the prelude mounts at boot; the door never names
//! it): the ambient pair completes plain everywhere, the `pub builtin`
//! names ride the import gate, F12 lands on the TRUE repo-relative
//! source path, and a host mod-tree dep mounts through the dep walk
//! exactly as the store lays it.

mod common;

use common::*;
use std::rc::Rc;

#[test]
fn the_prelude_rides_every_chain_as_core() {
    // the ONE ambient index: `use ⏐` names it (the use-path tier reads
    // the same named set as any dep), and nothing else is special
    let mut s = S::boot();
    let items = s.complete_pos("file:///t/main.rut", "use ", 0, 4);
    assert!(
        offers(&items, "core"),
        "the prelude names itself: {:?}",
        labels(&items)
    );
    // `use core::` — the prelude's own names
    let items = s.complete_pos("file:///t/main.rut", "use core::", 0, 10);
    assert!(offers(&items, "StackTrace"), "{:?}", labels(&items));
}

#[test]
fn the_ambient_pair_completes_plain_and_the_gated_names_wait_for_use() {
    // StackTrace/panic spell `prelude builtin` — they bind in every
    // compilation unit, so their rows are AMBIENT (never an import
    // offer); the import-gated `pub builtin` names (Weak) only ride
    // once the document's `use` names them. The editor mirrors the
    // engine's own ambient split exactly.
    let mut s = S::boot();
    let uri = "file:///t/main.rut";
    let bare = "entry fn main() -> nil {\n    let b = StackTr\n}\n";
    let items = s.complete_at(uri, bare, "StackTr", 1);
    let st = item(&items, "StackTrace");
    assert!(
        st["additionalTextEdits"].is_null(),
        "the ambient row completes plain, no import offer: {st}"
    );
    // panic — the other ambient row
    let bare = "entry fn main() -> nil {\n    pan\n}\n";
    let items = s.complete_at(uri, bare, "pan", 1);
    assert!(
        offers(&items, "panic"),
        "the ambient fn completes: {:?}",
        labels(&items)
    );

    // the gated name: off bare, on after the use
    let bare = "entry fn main() -> nil {\n    let w = Wea\n}\n";
    let items = s.complete_at(uri, bare, "Wea", 1);
    assert!(
        !offers(&items, "Weak"),
        "the `pub builtin` name stays gated: {:?}",
        labels(&items)
    );
    let used = "use core::{ Weak };\nfn main() -> nil {\n    let w = Wea\n}\n";
    let items = s.complete_at(uri, used, "Wea", 1);
    assert!(
        offers(&items, "Weak"),
        "the imported name completes: {:?}",
        labels(&items)
    );
}

#[test]
fn definition_on_a_core_name_jumps_to_the_true_source_path() {
    // the prelude's definition target is the TRUE repo-relative path
    // (the extension resolves it against the workspace), never a label
    let mut s = S::boot();
    const CORE: &str = include_str!("../../../rut/core/src/core.d.rut");
    let src = "entry fn main() -> nil {\n    let b = StackTrace();\n}\n";
    let hits = s.def_in("file:///t/main.rut", src, "StackTrace()", 1, 0);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "rut/core/src/core.d.rut");
    // the range is the declaring row in the REAL source
    let real = CORE.replace("\r\n", "\n");
    let want = byte_of(&real, "StackTrace", 2);
    assert_eq!(
        span_of(&real, &hits[0]),
        (want as usize, want as usize + 10),
        "{hits:?}"
    );
}

#[test]
fn hover_over_a_core_name_renders_the_row() {
    let mut s = S::boot();
    let src = "entry fn main() -> nil {\n    let b = StackTrace();\n}\n";
    let md = markdown(s.hover_in("file:///t/main.rut", src, "StackTrace()", 1, 0));
    assert!(md.contains("StackTrace"), "{md}");
    assert!(
        md.contains("core"),
        "the hover names the exporting pkg: {md}"
    );
}

#[test]
fn a_primitive_member_answers_every_door() {
    // the numerics re-spell's visibility win: the integer widths'
    // numeric methods declare on their OWN primitive bodies, so the
    // ordinary doors answer for `x.wrapping_add` — hover renders the
    // row (verbatim from the prelude doc), completion offers the
    // members, F12 jumps to the declaring row in `core.d.rut`
    let mut s = S::boot();
    const CORE: &str = include_str!("../../../rut/core/src/core.d.rut");

    // hover renders the row
    let src = "entry fn main() -> nil {\n    const x: i32 = 1;\n    x.wrapping_add(2);\n}\n";
    let md = markdown(s.hover_in("file:///t/main.rut", src, ".wrapping_add", 1, 2));
    assert!(
        md.contains("fn wrapping_add(self, y: i32) -> i32"),
        "the verbatim row: {md}"
    );
    assert!(md.contains("in `i32`"), "the owning primitive: {md}");

    // completion offers the members
    let src = "entry fn main() -> nil {\n    const x: i32 = 1;\n    x.\n}\n";
    let items = s.complete_at("file:///t/main.rut", src, "x.", 1);
    assert!(
        offers(&items, "wrapping_add") && offers(&items, "checked_add"),
        "the primitive's members complete: {:?}",
        labels(&items)
    );

    // definition jumps to the declaring row in the prelude
    let src = "entry fn main() -> nil {\n    const x: i32 = 1;\n    x.wrapping_add(2);\n}\n";
    let hits = s.def_in("file:///t/main.rut", src, ".wrapping_add", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "rut/core/src/core.d.rut");
    // the range is the member's name in the REAL source (i32's block:
    // the third `wrapping_add` spelling)
    let real = CORE.replace("\r\n", "\n");
    let want = byte_of(&real, "fn wrapping_add", 3) + "fn ".len() as u32;
    assert_eq!(
        span_of(&real, &hits[0]),
        (want as usize, want as usize + "wrapping_add".len()),
        "{hits:?}"
    );
}

/// a host mod-tree dep laid as the store lays it (the unpacked tree):
/// a `type = "host"` manifest whose surface is a MOD TREE — the root
/// `.d.rut` carries `pub mod net;`, the child rides `net/mod.d.rut`.
/// The walk mounts the tree Decl-flavored; the rows complete, the
/// surface type-checks, and F12 lands in the real child files.
#[test]
fn a_host_mod_tree_dep_mounts_through_the_walk() {
    let mut io = MemIo::default();
    io.put(
        "/ws/myhost/rut.jsonc",
        "{\n  \"name\": \"myhost\",\n  \"type\": \"host\"\n}\n",
    );
    io.put(
        "/ws/myhost/src/mod.d.rut",
        "pub mod net;\npub host fn legacy_ping() -> bool;\n",
    );
    io.put(
        "/ws/myhost/src/net/mod.d.rut",
        "pub host fn connect(url: str) -> opaque;\n",
    );
    io.put(
        "/ws/proj/rut.jsonc",
        "{\n  \"name\": \"proj\",\n  \"deps\": {\"myhost\": {\"path\": \"/ws/myhost\"}}\n}\n",
    );
    io.put("/ws/proj/src/mod.rut", "fn main() -> nil {\n}\n");
    let io = Rc::new(io);
    let mut s = S::with_io(&io);
    let out = s.resolve(&["/ws/proj"]);
    assert_eq!(
        out["missing"].as_array().unwrap().len(),
        0,
        "a clean mount: {out}"
    );

    // the rows complete through the mounted tree — the flat row and
    // the module child
    let uri = "file:///t/uses-myhost.rut";
    let doc = "use myhost::\nfn main() -> nil {\n}\n";
    let items = s.complete_at(uri, doc, "use myhost::", 1);
    let ls = labels(&items);
    assert!(ls.contains(&"legacy_ping"), "{ls:?}");
    assert!(ls.contains(&"net"), "{ls:?}");

    // the check lane fires at the PROJECT's own entry (the doc's pkg
    // context rides the walk's origin stamp): the host surface really
    // type-checks, and the lane demonstrably runs
    let entry = "/ws/proj/src/mod.rut";
    let used = "use myhost::{ legacy_ping };\nfn main() -> nil {\n    let ok = legacy_ping();\n}\n";
    let a = s.analyze(entry, used);
    assert_eq!(
        a["diags"].as_array().unwrap().len(),
        0,
        "the host surface type-checks: {a}"
    );
    let bad = "fn main() -> nil {\n    let x = 1 + \"nope\";\n}\n";
    let a = s.analyze(entry, bad);
    assert!(
        a["diags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["message"].as_str().unwrap().contains("equal width")),
        "the check lane demonstrably ran: {a}"
    );

    // F12 lands in the real files: the leaf in the child, the segment
    // on the declaring edge
    let netdoc = "use myhost::net::{ connect };\nfn main() -> nil {\n}\n";
    let hits = s.def_in(entry, netdoc, "{ connect }", 1, 2);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///ws/myhost/src/net/mod.d.rut");
    let hits = s.def_in(entry, netdoc, "myhost::net", 1, 9);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///ws/myhost/src/mod.d.rut");
    assert_target(
        "pub mod net;\npub host fn legacy_ping() -> bool;\n",
        &hits[0],
        "net;",
        1,
        3,
        "the declaring edge",
    );

    // the hover rides the same chain: the leaf renders the row's
    // signature from the child file
    let md = markdown(s.hover_in(entry, netdoc, "{ connect }", 1, 2));
    assert!(md.contains("connect"), "{md}");
    assert!(
        md.contains("myhost"),
        "the hover names the exporting pkg: {md}"
    );
}

/// the byte-span assert the file-local targets share
fn assert_target(
    src: &str,
    loc: &serde_json::Value,
    needle: &str,
    occ: usize,
    len: usize,
    what: &str,
) {
    let want = byte_of(src, needle, occ) as usize;
    assert_eq!(span_of(src, loc), (want, want + len), "{what}: {loc}");
}
