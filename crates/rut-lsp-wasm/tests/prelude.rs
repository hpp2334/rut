//! The prelude's tests — `core`, the ONE ambient index. The library
//! embeds nothing but the engine's own prelude; every other std pkg
//! reaches a test as an ordinary FIXTURE through the public dep lane
//! (`deps::index_dep` / `index_dep_mod` over the repo's own sources —
//! tests embedding fixture sources is the normal shape, the LIBRARY
//! embedding them was the repealed second channel). The myhost tree
//! below ports the old surface's mod-tree assertions onto the same
//! lane the walk mounts through.

use rut_lsp_wasm::deps::{index_dep, index_dep_mod};
use rut_lsp_wasm::hover::DefIndex;
use rut_lsp_wasm::prelude;
use std::collections::HashSet;

/// the myhost fixture tree — the repo's own host pkg, mounted the way
/// the dep walk mounts it: one index per file, the (module, mod_path)
/// pair keyed exactly as the walk stamps it (the root at `""`)
const MYHOST: &str = include_str!("../../../rut/myhost/src/myhost.d.rut");
const MYHOST_NET: &str = include_str!("../../../rut/myhost/src/net/mod.d.rut");
const MYHOST_NET_TLS: &str = include_str!("../../../rut/myhost/src/net/tls/mod.d.rut");

fn myhost_chain() -> Vec<DefIndex> {
    vec![
        index_dep_mod("myhost", "rut/myhost/src/myhost.d.rut", "", MYHOST),
        index_dep_mod("myhost", "rut/myhost/src/net/mod.d.rut", "net", MYHOST_NET),
        index_dep_mod(
            "myhost",
            "rut/myhost/src/net/tls/mod.d.rut",
            "net/tls",
            MYHOST_NET_TLS,
        ),
    ]
}

/// one doc artifact — the (normalized src, toks, ast) triple from ONE
/// parse, read through the doc's doors (the test lane's replacement
/// for the hand-spelled `normalize → lex → doc_ast` triple; doc
/// grafting ON — the editor lane's shape)
fn doc_of(src: &str, mode: rut_ast::parser::Mode) -> rut_semantic::ParsedDoc {
    rut_semantic::ParsedDoc::parse(mode, src)
}

#[test]
fn the_prelude_index_stamps_the_manifest_shape() {
    // the ONE ambient index: origin `core`, the TRUE repo-relative
    // source path (the definition layer jumps there), the module
    // name (the use-path tier reads the same named set as any dep),
    // and the Decl mode. Namespace-free — the head mechanism is the
    // manifest's, and no manifest ever delivers core.
    let idx = prelude::index();
    assert_eq!(idx.origin, "core");
    assert_eq!(idx.src_path.as_deref(), Some("rut/core/src/core.d.rut"));
    assert_eq!(idx.module.as_deref(), Some("core"));
    assert_eq!(idx.mode, Some(rut_ast::parser::Mode::Decl));
    assert!(
        !idx.fns.is_empty() && !idx.types.is_empty(),
        "the prelude carries rows"
    );

    // the ambient rule the auto-import tier DERIVES at read: origin
    // `core` ⇒ offered — the prelude rides every chain as the
    // ambient dep lane (the same shape a direct dep's mount answers
    // with), no manifest row and no set membership asked. The
    // direct-dep gate keys the MOUNTED dep walks; the prelude is not
    // one of them (a walked dep's origin is always a uri, never the
    // bare label, so no collision). Driven over a core-origin
    // fixture with a plain pub row (the real prelude's pub rows are
    // `pub builtin` — gated — and its ambient ones already complete
    // plain), the doc's dep set EMPTY.
    let doc = "entry fn main() -> nil { }\n";
    let ddoc = doc_of(doc, rut_ast::parser::Mode::Impl);
    let (d2, toks, ast) = (ddoc.src(), ddoc.toks(), ddoc.ast());
    let di = rut_lsp_wasm::hover::index(&ddoc);
    let core = index_dep(
        "core",
        "core",
        "pub fn ambient_offered() -> i32 { return 1; }\n",
    );
    let idxs = [&di, &core];
    let items = rut_lsp_wasm::completion::complete(
        &idxs,
        toks,
        ast,
        d2.len() as u32,
        d2,
        &rut_lsp_wasm::mods::DocMods::default(),
        &HashSet::new(),
    );
    assert!(
        items
            .iter()
            .any(|c| c.label == "ambient_offered" && !c.additional_text_edits.is_empty()),
        "origin `core` rides the auto-import tier with an empty dep set: {items:?}"
    );
}

#[test]
fn the_ambient_pair_completes_plain() {
    // StackTrace/panic spell `prelude builtin` — they bind in every
    // compilation unit, so their rows are AMBIENT (never pub_gated,
    // never an import offer); the import-gated `pub builtin` names
    // (Weak) ride `pub_gated` instead. The editor mirrors the
    // engine's own ambient split exactly.
    let idx = prelude::index();
    let st = idx.ty("StackTrace").expect("StackTrace indexes");
    assert!(st.ambient, "StackTrace is an ambient row");
    let panic = idx
        .fns
        .iter()
        .find(|f| f.name == "panic")
        .expect("panic indexes");
    assert!(panic.ambient, "panic is an ambient row");
    assert!(
        !idx.pub_gated
            .iter()
            .any(|n| n == "StackTrace" || n == "panic"),
        "the ambient pair never rides the import gate: {:?}",
        idx.pub_gated
    );
    assert!(
        idx.pub_gated.iter().any(|n| n == "Weak"),
        "the `pub builtin` spelling rides the gate: {:?}",
        idx.pub_gated
    );
}

#[test]
fn the_host_tree_mounts_root_and_children_through_the_dep_lane() {
    // the depth-2 shape the old surface pinned (deps.rs's
    // `the_decl_entry_indexes_whole` covers the one-level bundle
    // form): root + child + grandchild, each with its mod path, the
    // root carrying the `pub mod net;` edge and its flat row, the
    // children their own
    let idxs = myhost_chain();
    let paths: Vec<&str> = idxs
        .iter()
        .map(|i| i.mod_path.as_deref().unwrap_or("??"))
        .collect();
    assert_eq!(paths, vec!["", "net", "net/tls"], "{paths:?}");
    assert_eq!(idxs[0].origin, "rut/myhost/src/myhost.d.rut");
    assert_eq!(idxs[1].origin, "rut/myhost/src/net/mod.d.rut");
    assert_eq!(idxs[2].origin, "rut/myhost/src/net/tls/mod.d.rut");
    // the root carries the edge and its flat row; the children their own
    assert!(idxs[0].mods.iter().any(|m| m.name == "net"));
    assert!(idxs[0].fns.iter().any(|f| f.name == "legacy_ping"));
    assert!(idxs[1].mods.iter().any(|m| m.name == "tls"));
    assert!(idxs[1].fns.iter().any(|f| f.name == "connect"));
    assert!(idxs[2].fns.iter().any(|f| f.name == "handshake"));
    // every row of the pkg carries the manifest-shape stamps
    for i in &idxs {
        assert_eq!(i.module.as_deref(), Some("myhost"));
        assert_eq!(i.mode, Some(rut_ast::parser::Mode::Decl));
    }
}

#[test]
fn definition_over_a_qualified_host_use_jumps_into_the_tree() {
    // F12 on the leaf of `use myhost::net::{ connect }` lands in
    // `net/mod.d.rut` (the mounted module's own file); F12 on the
    // `net` segment lands on the declaring `pub mod net;` edge
    let src = "use myhost::net::{ connect };\n\nentry fn main() -> nil {\n    let _ = connect(\"x\");\n}\n";
    let ddoc = doc_of(src, rut_ast::parser::Mode::Impl);
    let (normalized, toks, ast) = (ddoc.src(), ddoc.toks(), ddoc.ast());
    let mut doc_idx = rut_lsp_wasm::hover::index(&ddoc);
    let doc_uri = "file:///t/main.rut";
    doc_idx.origin = doc_uri.to_string();
    let idxs = myhost_chain();
    let chain: Vec<&DefIndex> = std::iter::once(&doc_idx).chain(idxs.iter()).collect();
    let dm = rut_lsp_wasm::mods::DocMods::default();
    let ctx = rut_lsp_wasm::definition::Ctx {
        doc_uri,
        toks,
        ast,
        idxs: &chain,
        doc_mods: &dm,
    };
    // the leaf: the `connect` inside the braces
    let leaf = normalized.find("connect").expect("the leaf") as u32;
    let hits = rut_lsp_wasm::definition::definition(&ctx, leaf);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(
        hits[0].uri, "rut/myhost/src/net/mod.d.rut",
        "the leaf's file"
    );
    // the segment: `net` in the use path — the declaring edge
    let seg = normalized.find("net").expect("the segment") as u32;
    let hits = rut_lsp_wasm::definition::definition(&ctx, seg);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(
        hits[0].uri, "rut/myhost/src/myhost.d.rut",
        "the edge's file"
    );
}

#[test]
fn hover_over_a_qualified_host_use_renders_the_row() {
    // the hover face rides the same chain: the leaf renders the
    // row's signature from the child file
    let src = "use myhost::net::{ connect };\n";
    let ddoc = doc_of(src, rut_ast::parser::Mode::Impl);
    let (normalized, toks, ast) = (ddoc.src(), ddoc.toks(), ddoc.ast());
    let mut doc_idx = rut_lsp_wasm::hover::index(&ddoc);
    let doc_uri = "file:///t/main.rut";
    doc_idx.origin = doc_uri.to_string();
    let idxs = myhost_chain();
    let chain: Vec<&DefIndex> = std::iter::once(&doc_idx).chain(idxs.iter()).collect();
    let leaf = normalized.find("connect").expect("the leaf") as u32;
    let h = rut_lsp_wasm::hover::hover(
        &chain,
        toks,
        ast,
        &rut_lsp_wasm::mods::DocMods::default(),
        leaf,
    )
    .expect("the leaf hovers");
    assert!(h.markdown.contains("connect"), "{}", h.markdown);
    assert!(
        h.markdown.contains("myhost"),
        "the hover names the exporting pkg: {}",
        h.markdown
    );
}

/// `index_dep` over one repo std source — the fixture lane the
/// completion/hover/definition tests mount (the same public door the
/// walk rides; nothing about std is special anymore)
#[test]
fn a_std_source_mounts_as_an_ordinary_dep() {
    // pouch — a root-module lib pkg: the file parses in Impl mode
    // (the filename convention), the module is named, and the class
    // rows the use-path tiers read are simply there
    let idx = index_dep(
        "pouch",
        "rut/pouch/src/mod.rut",
        include_str!("../../../rut/pouch/src/mod.rut"),
    );
    assert_eq!(idx.module.as_deref(), Some("pouch"));
    assert_eq!(idx.mode, Some(rut_ast::parser::Mode::Impl));
    assert_eq!(idx.origin, "rut/pouch/src/mod.rut");
    assert!(idx.ty("Vec").is_some(), "Vec indexes through the dep lane");
    // directness is NOT an index fact — nothing to assert here: the
    // auto-import tier derives it at read from the doc's dep set
    // (the transitive negative lives in tests/completion.rs)
}
