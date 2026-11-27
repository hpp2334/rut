//! The sync lane's gate — the one edit door through the envelopes:
//! `analyze`'s FULL-sync ≡ a cold session's answer for the same text
//! (the incremental repair is invisible), a no-op sync is a no-op,
//! and every query after a change reads the FRESH views (a stale span
//! would decorate yesterday's text).

mod common;

use common::*;

#[test]
fn sync_equals_a_cold_parse() {
    // the edited session's answer ≡ a fresh session's answer for the
    // new text — diags, tokens, and symbols all ride the repaired
    // document (the edit ≡ cold law, observable at the envelope)
    let mut edited = S::boot();
    let mut cold = S::boot();
    let v1 = "fn area(w: i32, h: i32) -> i32 {\n    w * h\n}\n\nfn main() -> nil {\n    let x = area(2, 3);\n    print(x);\n}\n";
    let v2 = "fn area(w: i32, h: i32) -> i32 {\n    w * h\n}\n\nfn main() -> nil {\n    let x = area(20, 30);\n    print(x);\n}\n";
    let uri = "file:///t/sync.rut";
    edited.analyze(uri, v1);
    let after = edited.analyze(uri, v2);
    let fresh = cold.analyze("file:///t/cold.rut", v2);
    assert_eq!(after, fresh, "the edited session answers like a cold one");
    // the fresh views cover the NEW text: the edited literal's token
    // is there (a stale token would carry the old `2`/`3`)
    let toks = decode_tokens(v2, &after);
    assert!(toks.iter().any(|t| t.text == "20"), "{toks:?}");
    assert!(toks.iter().any(|t| t.text == "30"), "{toks:?}");
    assert!(
        !toks.iter().any(|t| t.text == "2," && t.lo > 60),
        "{toks:?}"
    );
}

#[test]
fn a_noop_sync_is_a_noop() {
    let mut s = S::boot();
    let src = "let ok = 1;\n";
    let uri = "file:///t/noop.rut";
    let before = s.analyze(uri, src);
    let after = s.analyze(uri, src);
    assert_eq!(before, after, "tokens, diags, symbols unmoved");
}

#[test]
fn the_queries_answer_over_the_synced_text() {
    // after a sync, a span join reads the NEW text (never yesterday's):
    // the hover decorates the ident where the new text puts it, and a
    // definition target carries the edited literal
    let mut s = S::boot();
    let uri = "file:///t/join.rut";
    let v1 = "let a = 1;\nlet b = 2;\n";
    let v2 = "let a = 1;\nlet b = 22;\n";
    s.analyze(uri, v1);
    // the hover after the sync lands on the NEW `b` row
    let h = s.hover_in(uri, v2, "let b = 22", 1, 4);
    let md = markdown(h);
    assert!(
        md.contains("let b = 22"),
        "the span join answers the new text: {md}"
    );
    // the range is the ident's span over the NEW coordinates
    let h = s.hover_in(uri, v2, "let b = 22", 1, 4);
    let want = byte_of(v2, "let b = 22", 1) + 4;
    assert_eq!(
        span_of(v2, &h),
        (want as usize, want as usize + 1),
        "the hovered token rides the new coordinates"
    );
    // and the fresh views re-derive coherently: the inlay hints hang
    // at the edited literal's line, not yesterday's
    let hs = s.inlay_all(uri, v2);
    assert_eq!(hs.len(), 2, "both lets are unannotated: {hs:?}");
    assert_eq!(
        hs[1]["position"]["line"], 1,
        "the second hint on the synced line: {}",
        hs[1]
    );
}

#[test]
fn an_open_host_child_honours_its_mounts_mode() {
    // F49: opening a host tree's single-file child must answer the tree
    // the mount parsed — Decl. The filename (`tool.rut`) says Impl, so
    // at base the mount's doc carried an Impl parse and the surface
    // syntax squiggled `declaration keyword in an implementation file`.
    use std::rc::Rc;
    let mut io = MemIo::default();
    io.put(
        "/ws/hh/rut.jsonc",
        "{\n  \"name\": \"hh\",\n  \"type\": \"host\"\n}\n",
    );
    io.put("/ws/hh/src/mod.d.rut", "pub mod tool;\n");
    let child = "pub host fn helper() -> i32;\n";
    io.put("/ws/hh/src/tool.rut", child);
    let io = Rc::new(io);
    let mut s = S::with_io(&io);
    let out = s.resolve(&["/ws/hh"]);
    assert_eq!(out["missing"].as_array().unwrap().len(), 0, "{out}");

    let a = s.analyze("file:///ws/hh/src/tool.rut", child);
    let diags = a["diags"].as_array().unwrap();
    assert!(
        diags.is_empty(),
        "the mount's mode is the open document's: {a}"
    );
}

#[test]
fn an_edit_repair_keeps_the_tree_whole() {
    // an edit that breaks and re-parses a decl: the queries answer on
    // the repaired tree (recovery inside the same document, exactly a
    // cold parse's shape)
    let mut s = S::boot();
    let uri = "file:///t/repair.rut";
    let v1 = "fn go() -> i32 {\n    let c = 1;\n    return c;\n}\n";
    let broken = "fn go() -> i32 {\n    let c = ;\n    return c;\n}\n";
    s.analyze(uri, v1);
    let a = s.analyze(uri, broken);
    assert!(
        !a["diags"].as_array().unwrap().is_empty(),
        "the break is diagnosed: {a}"
    );
    // the same break at a fresh uri: the identical answer (edit ≡ cold)
    let mut cold = S::boot();
    let fresh = cold.analyze("file:///t/repair-cold.rut", broken);
    assert_eq!(a, fresh, "the repaired document parses like a cold one");
    // repairing again restores the clean answer
    let healed = s.analyze(uri, v1);
    assert!(healed["diags"].as_array().unwrap().is_empty(), "{healed}");
    let d = s.def_in(uri, v1, "return c", 1, 7);
    assert_eq!(d.len(), 1, "the binding pass rides the healed tree: {d:?}");
}
