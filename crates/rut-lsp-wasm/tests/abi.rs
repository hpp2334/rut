//! The wasm ABI's own gate — the envelopes and the session state the
//! extension drives, asserted through the public `RutLsp` binding the
//! same way the hosts call it — async bridges over the loop, driven on
//! a deterministic single-threaded `LocalPool` (`RutLsp::spawn_native`,
//! the native half of the executor seam; the wasm parks the same loop
//! on `spawn_local`). The crates' other tests cover the queries
//! underneath.

use rut_lsp_wasm::RutLsp;

/// the CDN url's own cache filename (the URL STRING is hashed, never
/// the bytes) — the row the BOOT-EMPTY ledger leaves unpinned
const POUCH_CACHE: &str =
    ".rut/cache/02bcf87de17b5c095f2c1a768f20f2506a10b9a56a1290ffe4241536e672dbe0.rutbundle";

/// parse an export's JSON answer
fn take(json: String) -> serde_json::Value {
    serde_json::from_str(&json).expect("the envelope is valid JSON")
}

/// The plugin manifest's dep shape, INLINE and unpinned: the real
/// `examples/03-plugin/plugin/rut.jsonc` migrates off the retired
/// manifest `sha256` key in the docs/examples phase — the grammar
/// refuses the key today, so the host-side fixture is the shape
/// every manifest must now spell (pins ride the walk's lock feed).
const PLUGIN_SHAPE: &str = r#"{
  "format": "rutbundle",
  "format_version": 11,
  "name": "plugin",
  "deps": {
    "server": { "path": "../server" },
    "pouch": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v12/dist/std/pouch.rutbundle" }
  }
}"#;

#[test]
fn manifest_envelope_spells_the_dep_table() {
    let (lsp, mut pool) = RutLsp::spawn_native();
    // the BOOT ledger is empty: the url row answers `sha256: null` —
    // the unpinned refusal, never a guessed pin
    let out = take(pool.run_until(lsp.parse_manifest(PLUGIN_SHAPE.to_string())));
    assert_eq!(out["name"], "plugin");
    // the body rides the src/mod.rut convention (the entry keys are repealed)
    assert_eq!(
        out["entries"],
        serde_json::json!([{ "path": "src/mod.rut", "mode": "Impl" }]),
        "the entry carries its mode: {}",
        out["entries"]
    );
    let rows = out["deps"].as_array().unwrap();
    let server = rows.iter().find(|r| r["name"] == "server").unwrap();
    assert_eq!(
        server["source"],
        serde_json::json!({ "kind": "path", "dir": "../server" })
    );
    assert_eq!(server["optional"], false);
    let pouch = rows.iter().find(|r| r["name"] == "pouch").unwrap();
    assert_eq!(pouch["source"]["kind"], "url");
    assert_eq!(pouch["source"]["cache_path"], POUCH_CACHE);
    assert!(
        pouch["source"]["sha256"].is_null(),
        "no lock row ⇒ unpinned: {}",
        pouch["source"]
    );
}

#[test]
fn a_decl_entry_and_the_error_envelope() {
    let (lsp, mut pool) = RutLsp::spawn_native();
    let src = r#"{ "name": "host", "type": "host", "entry": { "type": "./surface.d.rut" } }"#;
    let out = take(pool.run_until(lsp.parse_manifest(src.to_string())));
    assert_eq!(
        out["entries"],
        serde_json::json!([{ "path": "src/surface.d.rut", "mode": "Decl" }]),
        "entry.type indexes in Decl mode"
    );
    // a malformed manifest is the error envelope — the host's hint
    let bad = take(pool.run_until(lsp.parse_manifest("{\"name\": six}".to_string())));
    assert!(
        bad["error"].as_str().unwrap().starts_with("line 1: "),
        "{bad}"
    );
}

#[test]
fn complete_answers_locals_and_members_at_the_end_of_buffer() {
    let (lsp, mut pool) = RutLsp::spawn_native();
    // plan3 phase 1 (A7): the doc ends EXACTLY at the incomplete
    // stmt — the parser ends the block's span at the EOF token, the
    // cursor sits ON `hi`, and before the scope extension the member
    // tier dropped the receiver (an empty list). The fn body's scope
    // extends through the source end, so the member tier answers
    // through the doc-sync entry — never the bare keyword dump.
    let doc = "class List {\n    items: i32;\n}\nimpl List {\n    fn count(self) -> i32 { return 0; }\n}\nfn go() -> nil {\n    let list = List.new();\n    list.";
    let uri = "file:///ws/eof-member.rut";
    let last = doc.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line = doc[..last].matches('\n').count() as u32;
    let ch = (doc.len() - last) as u32;
    let items = take(pool.run_until(lsp.complete(uri.to_string(), doc.to_string(), line, ch)));
    let labels: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    for want in ["items", "count"] {
        assert!(
            labels.contains(&want),
            "member completion at EOF lacks '{want}': {labels:?}"
        );
    }
    assert!(
        !labels.contains(&"fn"),
        "no keyword dump after a dot at EOF: {labels:?}"
    );

    // the locals tier (A1) at the same moment: `che⏐` after
    // `let mut checked …` — the binding rides the variable kind
    let doc = "fn go() -> nil {\n    let mut checked: str = \" \";\n    che";
    let uri = "file:///ws/eof-locals.rut";
    let last = doc.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line = doc[..last].matches('\n').count() as u32;
    let ch = (doc.len() - last) as u32;
    let items = take(pool.run_until(lsp.complete(uri.to_string(), doc.to_string(), line, ch)));
    let arr = items.as_array().unwrap();
    let labels: Vec<&str> = arr.iter().map(|i| i["label"].as_str().unwrap()).collect();
    assert!(
        labels.contains(&"checked"),
        "the local completes at EOF: {labels:?}"
    );
    let checked = arr.iter().find(|i| i["label"] == "checked").unwrap();
    assert_eq!(checked["kind"], 6, "LSP VARIABLE kind: {}", checked["kind"]);
    // ahead of the keyword table
    let checked_at = arr.iter().position(|i| i["label"] == "checked").unwrap();
    let let_at = arr.iter().position(|i| i["label"] == "let").unwrap();
    assert!(checked_at < let_at, "locals ride ahead of the keywords");
}

#[test]
fn complete_answers_the_type_tier_at_an_annotation() {
    let (lsp, mut pool) = RutLsp::spawn_native();
    // A3 through the bindgen face: `let v: u⏐` — the primitive table
    // rides the prefix, the keyword table / fns / locals stay off
    let doc = "fn go() -> nil {\n    let v: u\n}\n";
    let uri = "file:///ws/type-tier.rut";
    let items = take(pool.run_until(lsp.complete(uri.to_string(), doc.to_string(), 1, 12)));
    let labels: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    for want in ["u8", "u16", "u32", "u64"] {
        assert!(
            labels.contains(&want),
            "primitive `{want}` completes at the annotation: {labels:?}"
        );
    }
    for absent in ["fn", "let", "go"] {
        assert!(
            !labels.contains(&absent),
            "`{absent}` must stay off a type position: {labels:?}"
        );
    }
    // the bare colon boundary — the full tier, no keywords
    let doc = "fn go() -> nil {\n    let v: \n}\n";
    let items = take(pool.run_until(lsp.complete(uri.to_string(), doc.to_string(), 1, 11)));
    let labels: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    assert!(
        labels.contains(&"i32") && labels.contains(&"str"),
        "the bare colon tier: {labels:?}"
    );
    assert!(
        !labels.contains(&"fn"),
        "no keyword table at the boundary: {labels:?}"
    );
}

#[test]
fn definition_on_the_mod_edge_names_the_child_file_top() {
    let (lsp, mut pool) = RutLsp::spawn_native();
    // the plan-C lane through the bindgen face: the root file is
    // BOTH a stamped index (its uri carries the (module, mod_path)
    // pair `doc_mods_of` reads) and the analyzed doc; the `helpers`
    // ident in `pub mod helpers;` jumps to the mounted child at 0:0
    let root = "pub mod helpers;\nentry fn main() -> nil {\n}\n";
    let child = "pub struct Slot {\n    n: i32;\n}\n";
    let ru = "file:///ws/edgemod/mod.rut";
    let cu = "file:///ws/edgemod/helpers/mod.rut";
    take(pool.run_until(lsp.index_mod_at(
        ru.to_string(),
        "edgemod".to_string(),
        "".to_string(),
        root.to_string(),
    )));
    take(pool.run_until(lsp.index_mod_at(
        cu.to_string(),
        "edgemod".to_string(),
        "helpers".to_string(),
        child.to_string(),
    )));
    take(pool.run_until(lsp.analyze(ru.to_string(), root.to_string())));
    // the edge ident starts at char 8 (`pub mod `)
    let locs = take(pool.run_until(lsp.definition(ru.to_string(), 0, 8)));
    let arr = locs.as_array().unwrap();
    assert_eq!(arr.len(), 1, "exactly one target: {locs}");
    assert!(
        arr[0]["uri"]
            .as_str()
            .unwrap()
            .ends_with("/edgemod/helpers/mod.rut"),
        "the child uri: {locs}"
    );
    assert_eq!(
        arr[0]["range"],
        serde_json::json!({"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}),
        "the child file top: {locs}"
    );

    // an UNSTAMPED doc with the same text misses (the doc context
    // keys the lane — the scratch uri carries no stamp)
    let scratch = "file:///ws/edgemod-scratch.rut";
    take(pool.run_until(lsp.analyze(scratch.to_string(), root.to_string())));
    let none = take(pool.run_until(lsp.definition(scratch.to_string(), 0, 8)));
    assert!(
        none.as_array().unwrap().is_empty(),
        "unstamped miss: {none}"
    );
}

#[test]
fn complete_syncs_the_doc_and_the_dot_guard_holds() {
    let (lsp, mut pool) = RutLsp::spawn_native();
    // ink rides the dep lane now (the embedded surface died): the
    // repo's own ink source indexes as a named dep through the
    // workspace-push ABI — ctor inference reads `Logger` off it, so
    // `log.` lists the impl methods. The stored doc is STALE on
    // purpose (the tail still short the dot — the analyze debounce's
    // window): the `_in` entry stores the CURRENT text first, so the
    // member tier still answers.
    take(pool.run_until(lsp.index_mod_at(
        "file:///ws/ink/src/mod.rut".to_string(),
        "ink".to_string(),
        String::new(),
        include_str!("../../../rut/ink/src/mod.rut").to_string(),
    )));
    let stale = "fn go() -> nil {\n    let log = Logger(\"h\");\n    log\n}\n";
    let fresh = "fn go() -> nil {\n    let log = Logger(\"h\");\n    log.\n}\n";
    let uri = "file:///ws/sync.rut";
    take(pool.run_until(lsp.analyze(uri.to_string(), stale.to_string())));
    let items = take(pool.run_until(lsp.complete(uri.to_string(), fresh.to_string(), 2, 8)));
    let labels: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    for want in ["new", "debug", "info", "warn", "error"] {
        assert!(
            labels.contains(&want),
            "`{want}` completes through the ctor binding: {labels:?}"
        );
    }
    // the bare tier's free fns never dump into a member position
    assert!(
        !labels.contains(&"abs"),
        "no free-fn dump after a dot: {labels:?}"
    );

    // the dot guard: a receiver that is not a name (`probe().` — the
    // token before the dot is a `)`) answers EMPTY, never the bare
    // keyword/host-fn dump
    let guarded = "fn go() -> nil {\n    let x = probe().\n}\n";
    let none = take(pool.run_until(lsp.complete(uri.to_string(), guarded.to_string(), 1, 20)));
    assert!(none.as_array().unwrap().is_empty(), "{none}");
}

/// plan3 phase 4 (B1+B2): the CHECK LANE through the bindgen face —
/// a doc whose package is STAMPED (`index_mod_at` named it) compiles
/// with its dep closure, and the checker's type errors surface with
/// the right file and range: the bool→str assignment mismatch AND
/// the missing record field, the diagnosis pair. An UNSTAMPED doc
/// with the same text answers parse-only clean — today's behavior,
/// the corpus lock's foundation.
#[test]
fn analyze_runs_the_checker_over_a_stamped_chain() {
    let (lsp, mut pool) = RutLsp::spawn_native();
    // the dep: one mod-tree file mounted under its module NAME
    let dep_src = "pub struct Task {\n    id: u32;\n    title: str;\n    completed: bool;\n}\n";
    let dep_uri = "file:///ws/chk/dep/mod.rut";
    let stamped = take(pool.run_until(lsp.index_mod_at(
        dep_uri.to_string(),
        "chkdep".to_string(),
        "".to_string(),
        dep_src.to_string(),
    )));
    assert!(
        stamped["decls"].as_array().unwrap().is_empty(),
        "{}",
        stamped
    );
    // the doc's own pkg root, stamped the same way the extension's
    // workspace indexer stamps it
    let doc = "use chkdep::{ Task };\nfn main() -> nil {\n    let mut checked: str = \" \";\n    checked = true;\n    let t = Task { title: \"x\", completed: false };\n}\n";
    let app_uri = "file:///ws/chk/app/mod.rut";
    let stamped = take(pool.run_until(lsp.index_mod_at(
        app_uri.to_string(),
        "chkapp".to_string(),
        "".to_string(),
        doc.to_string(),
    )));
    assert!(
        stamped["decls"].as_array().unwrap().is_empty(),
        "{}",
        stamped
    );

    // the analyze lane returns the MERGED diags: both checker sites,
    // `source: "rut"`, each on its own line
    let a = take(pool.run_until(lsp.analyze(app_uri.to_string(), doc.to_string())));
    let diags = a["diags"].as_array().unwrap();
    let mismatch = diags
        .iter()
        .find(|d| {
            d["message"]
                .as_str()
                .unwrap_or("")
                .contains("assignment type mismatch")
        })
        .unwrap_or_else(|| panic!("no assignment-mismatch diag: {a}"));
    let missing = diags
        .iter()
        .find(|d| {
            d["message"]
                .as_str()
                .unwrap_or("")
                .contains("`id` is missing")
        })
        .unwrap_or_else(|| panic!("no missing-field diag: {a}"));
    for d in [mismatch, missing] {
        assert_eq!(d["source"], "rut", "{d}");
    }
    assert_eq!(mismatch["range"]["start"]["line"], 3, "{mismatch}");
    assert_eq!(missing["range"]["start"]["line"], 4, "{missing}");

    // the parse-only fallback: the SAME text at an UNSTAMPED uri is
    // clean (no package graph assembles for it)
    let plain_uri = "file:///ws/chk/plain.rut";
    let plain = take(pool.run_until(lsp.analyze(plain_uri.to_string(), doc.to_string())));
    assert!(
        plain["diags"].as_array().unwrap().is_empty(),
        "an unstamped doc must stay parse-only: {plain}"
    );
}

#[test]
fn index_mod_at_stamps_the_tree_and_feeds_the_use_walk() {
    let (lsp, mut pool) = RutLsp::spawn_native();
    // the root: `pub mod layout;` rides back as the mount walk's
    // next hop (the host reads its children from the envelope)
    let root_src = "pub mod layout;\nmod secret;\npub fn tag() -> i32 { return 1; }\n";
    let uri_root = "file:///ws/gadgets/mod.rut";
    let out = take(pool.run_until(lsp.index_mod_at(
        uri_root.to_string(),
        "gadgets".to_string(),
        "".to_string(),
        root_src.to_string(),
    )));
    assert_eq!(
        out["decls"],
        serde_json::json!([{ "name": "layout", "vis": "pub" }, { "name": "secret", "vis": "mod" }]),
        "{}",
        out["decls"]
    );
    // the child, stamped at its mod path
    let layout_src =
        "pub struct Column {\n    w: i32;\n}\npub fn mk() -> Column { return Column { w: 1 }; }\n";
    let uri_layout = "file:///ws/gadgets/layout/mod.rut";
    let out = take(pool.run_until(lsp.index_mod_at(
        uri_layout.to_string(),
        "gadgets".to_string(),
        "layout".to_string(),
        layout_src.to_string(),
    )));
    assert!(
        out["decls"].as_array().unwrap().is_empty(),
        "{}",
        out["decls"]
    );

    // `use gadgets::layout::⏐` — the walked module's exports (the
    // private `secret` edge walked would be empty)
    let doc = "use gadgets::layout::\nfn main() -> nil {\n}\n";
    let uri = "file:///ws/main.rut";
    take(pool.run_until(lsp.analyze(uri.to_string(), doc.to_string())));
    let items = take(pool.run_until(lsp.complete(uri.to_string(), doc.to_string(), 0, 21)));
    let labels: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    assert!(labels.contains(&"Column"), "{labels:?}");
    assert!(labels.contains(&"mk"), "{labels:?}");
    // the root tier offers the pub mod child, never the private one
    let items = take(pool.run_until(lsp.complete(uri.to_string(), doc.to_string(), 0, 13)));
    let labels: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    assert!(labels.contains(&"layout"), "{labels:?}");
    assert!(
        !labels.contains(&"secret"),
        "a bare `mod` edge never crosses: {labels:?}"
    );
    assert!(
        !labels.contains(&"Column"),
        "child members stay under their path: {labels:?}"
    );
}

/// the actor seam's dead-loop law: a dropped pool (the loop task gone
/// with it) answers the loop-gone ERROR envelope — never a hang, never
/// a panic, and the bridge resolves so a tearing-down page can move on
#[test]
fn a_dropped_loop_answers_the_error_envelope() {
    let (lsp, pool) = RutLsp::spawn_native();
    drop(pool);
    let out = take(futures::executor::block_on(
        lsp.analyze("file:///ws/gone.rut".into(), "fn main() -> nil {}\n".into()),
    ));
    assert!(out["error"].as_str().unwrap().contains("loop"), "{out}");
}
