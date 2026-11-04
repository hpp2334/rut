//! The wasm ABI's own gate — the envelopes and the session state the
//! extension drives, asserted through the public `RutLsp` binding the
//! same way the hosts call it — async bridges over the loop, driven on
//! a deterministic single-threaded `LocalPool` (`RutLsp::spawn_native`,
//! the native half of the executor seam; the wasm parks the same loop
//! on `spawn_local`). The crates' other tests cover the queries
//! underneath.

use futures::executor::LocalPool;
use rut_lsp_wasm::RutLsp;

const POUCH_BUNDLE: &[u8] = include_bytes!("../../../dist/std/pouch.rutbundle");
/// the CDN url's own sha256 — the cache filename rut-lsp computes
/// (the URL STRING is hashed, never the bytes)
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
/// every manifest must now spell (pins ride `set_lock`).
const PLUGIN_SHAPE: &str = r#"{
  "format": "rutbundle",
  "format_version": 11,
  "name": "plugin",
  "deps": {
    "server": { "path": "../server" },
    "pouch": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v12/dist/std/pouch.rutbundle" }
  }
}"#;

const POUCH_URL: &str = "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v12/dist/std/pouch.rutbundle";
const POUCH_PIN: &str = "63ddd765287243ef45f23105471ff3eae5c5a0ac1d521a3d22636745fd5e72d8";

/// hand a lock spelling `rows` to `set_lock`
fn set_lock(
    lsp: &RutLsp,
    pool: &mut LocalPool,
    rows: Vec<rut_driver::LockRow>,
) -> serde_json::Value {
    let text = rut_driver::render_lock(&rut_driver::LockFile::from_rows(rows));
    take(pool.run_until(lsp.set_lock(text.to_string())))
}

#[test]
fn manifest_envelope_spells_the_dep_table() {
    let (lsp, mut pool) = RutLsp::spawn_native();
    // the lock WITHOUT a pouch row stands first: the url row answers
    // `sha256: null` — the unpinned refusal, never a guessed pin
    let out = set_lock(&lsp, &mut pool, vec![]);
    assert_eq!(out["pins"], 0, "{out}");
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

    // the lock's row stamps the pin — the JSON keeps the `sha256`
    // field, now LOCK-SOURCED (the manifest itself stays unpinned)
    let out = set_lock(
        &lsp,
        &mut pool,
        vec![rut_driver::LockRow {
            pkg: "pouch".into(),
            url: POUCH_URL.into(),
            sha256: POUCH_PIN.into(),
        }],
    );
    assert_eq!(out["pins"], 1, "{out}");
    let out = take(pool.run_until(lsp.parse_manifest(PLUGIN_SHAPE.to_string())));
    let pouch = out["deps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "pouch")
        .unwrap();
    assert_eq!(
        pouch["source"]["sha256"], POUCH_PIN,
        "the pin is lock-sourced"
    );
    assert_eq!(pouch["source"]["cache_path"], POUCH_CACHE);
}

#[test]
fn a_malformed_lock_is_the_error_envelope_and_stands_empty() {
    let (lsp, mut pool) = RutLsp::spawn_native();
    let bad = take(pool.run_until(lsp.set_lock("{\"format\": \"rutlock\", \"format_version\": 1, \"deps\": [{\"pkg\": \"p\", \"url\": \"https://x/p.rutbundle\", \"sha256\": \"nothex\"}]}".to_string())));
    assert!(
        bad["error"].as_str().unwrap().contains("64 hex digits"),
        "{bad}"
    );
    // the empty ledger stands: the url row answers null (the host
    // shows the unpinned refusal), never half-parsed pins
    let out = take(pool.run_until(lsp.parse_manifest(PLUGIN_SHAPE.to_string())));
    let pouch = out["deps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "pouch")
        .unwrap();
    assert!(pouch["source"]["sha256"].is_null(), "{}", pouch["source"]);
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
fn complete_in_answers_locals_and_members_at_the_end_of_buffer() {
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
    let items = take(pool.run_until(lsp.complete_in(uri.to_string(), doc.to_string(), line, ch)));
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
    let items = take(pool.run_until(lsp.complete_in(uri.to_string(), doc.to_string(), line, ch)));
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
fn complete_in_answers_the_type_tier_at_an_annotation() {
    let (lsp, mut pool) = RutLsp::spawn_native();
    // A3 through the bindgen face: `let v: u⏐` — the primitive table
    // rides the prefix, the keyword table / fns / locals stay off
    let doc = "fn go() -> nil {\n    let v: u\n}\n";
    let uri = "file:///ws/type-tier.rut";
    let items = take(pool.run_until(lsp.complete_in(uri.to_string(), doc.to_string(), 1, 12)));
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
    let items = take(pool.run_until(lsp.complete_in(uri.to_string(), doc.to_string(), 1, 11)));
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
fn the_bare_scope_gates_a_dep_until_used() {
    let (lsp, mut pool) = RutLsp::spawn_native();
    // A2 through the bindgen face: a named dep's fn is NOT a plain
    // bare item until the doc `use`s it — the only bare-doc offer
    // rides the import edit, and the direct-dep set gates the tier
    // a root manifest declaring ONLY the doc's own pkg — kitdep is
    // mounted but NOT direct (the transitive-only shape)
    take(pool.run_until(lsp.set_direct_deps("[\"myapp\"]".to_string())));
    pool.run_until(lsp.add_def_named(
        "file:///ws/kitdep/lib.rut".to_string(),
        "kitdep".to_string(),
        "pub fn tag() -> i32 {\n    return 1;\n}\n".to_string(),
    ));
    let doc = "fn main() -> nil {\n}\n";
    let uri = "file:///ws/bare.rut";
    take(pool.run_until(lsp.analyze(uri.to_string(), doc.to_string())));
    let items = take(pool.run_until(lsp.complete(uri.to_string(), 1, 0)));
    let arr = items.as_array().unwrap();
    assert!(
        arr.iter().all(|i| i["label"] != "tag"),
        "a transitive-only dep's fn must not offer at all: {:?}",
        arr.iter().map(|i| &i["label"]).collect::<Vec<_>>()
    );

    // the direct-dep set gains kitdep (the root manifest's row):
    // the bare doc offers `tag` ONLY through the import edit …
    take(pool.run_until(lsp.set_direct_deps("[\"myapp\", \"kitdep\"]".to_string())));
    pool.run_until(lsp.add_def_named(
        "file:///ws/kitdep/lib.rut".to_string(),
        "kitdep".to_string(),
        "pub fn tag() -> i32 {\n    return 1;\n}\n".to_string(),
    ));
    let items = take(pool.run_until(lsp.complete(uri.to_string(), 1, 0)));
    let arr = items.as_array().unwrap();
    let imp = arr
        .iter()
        .find(|i| i["label"] == "tag")
        .expect("the direct dep's fn offers");
    assert!(
        imp["additionalTextEdits"]
            .as_array()
            .is_some_and(|e| !e.is_empty()),
        "the offer carries the use-insert edit: {imp}"
    );

    // … and once the doc `use`s it, `tag` is a PLAIN bare item —
    // exactly one, with no edit riding
    let doc = "use kitdep::{ tag };\nfn main() -> nil {\n}\n";
    let uri = "file:///ws/bare-used.rut";
    take(pool.run_until(lsp.analyze(uri.to_string(), doc.to_string())));
    let items = take(pool.run_until(lsp.complete(uri.to_string(), 1, 0)));
    let arr = items.as_array().unwrap();
    let hits: Vec<&serde_json::Value> = arr.iter().filter(|i| i["label"] == "tag").collect();
    assert_eq!(hits.len(), 1, "one `tag` item: {hits:?}");
    assert!(
        hits[0]["additionalTextEdits"].is_null(),
        "the imported fn completes plain: {}",
        hits[0]
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
    take(pool.run_until(lsp.add_def_mod(
        ru.to_string(),
        "edgemod".to_string(),
        "".to_string(),
        root.to_string(),
    )));
    take(pool.run_until(lsp.add_def_mod(
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
fn add_def_named_feeds_the_use_path_tier() {
    let (lsp, mut pool) = RutLsp::spawn_native();
    // a DIFFERENT module than the mod-tree fixture below — each test
    // owns its own instance, but the fixture stays distinct so the
    // two tests' envelopes never read as one pkg
    // a dep source under its module NAME
    pool.run_until(
        lsp.add_def_named(
            "file:///ws/toolkit/lib.rut".to_string(),
            "toolkit".to_string(),
            "pub class Widget {\n    id: i32;\n}\npub fn tag() -> i32 {\n    return 1;\n}\n"
                .to_string(),
        ),
    );
    let doc = "use toolkit::\nfn main() -> nil {\n}\n";
    let uri = "file:///ws/main.rut";
    take(pool.run_until(lsp.analyze(uri.to_string(), doc.to_string())));
    // `use toolkit::⏐` — the dep's public names
    let items = take(pool.run_until(lsp.complete(uri.to_string(), 0, 13)));
    let labels: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    assert!(
        labels.contains(&"Widget"),
        "the dep type completes: {labels:?}"
    );
    assert!(labels.contains(&"tag"), "the dep fn completes: {labels:?}");
    // `use ⏐` — the module-name tier sees the named index
    let mods = take(pool.run_until(lsp.complete(uri.to_string(), 0, 4)));
    let names: Vec<&str> = mods
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"toolkit"),
        "the module name completes: {names:?}"
    );
}

#[test]
fn complete_in_syncs_the_doc_and_the_dot_guard_holds() {
    let (lsp, mut pool) = RutLsp::spawn_native();
    // ctor inference rides the embedded ink surface: `Logger("h")`
    // types the binding, `log.` lists the impl methods. The stored
    // doc is STALE on purpose (the tail still short the dot — the
    // analyze debounce's window): the `_in` entry stores the
    // CURRENT text first, so the member tier still answers.
    let stale = "fn go() -> nil {\n    let log = Logger(\"h\");\n    log\n}\n";
    let fresh = "fn go() -> nil {\n    let log = Logger(\"h\");\n    log.\n}\n";
    let uri = "file:///ws/sync.rut";
    take(pool.run_until(lsp.analyze(uri.to_string(), stale.to_string())));
    let items = take(pool.run_until(lsp.complete_in(uri.to_string(), fresh.to_string(), 2, 8)));
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
    // the bare tier's host fns never dump into a member position
    assert!(
        !labels.contains(&"abs"),
        "no host-fn dump after a dot: {labels:?}"
    );

    // the dot guard: a receiver that is not a name (`probe().` — the
    // token before the dot is a `)`) answers EMPTY, never the bare
    // keyword/host-fn dump
    let guarded = "fn go() -> nil {\n    let x = probe().\n}\n";
    let none = take(pool.run_until(lsp.complete_in(uri.to_string(), guarded.to_string(), 1, 20)));
    assert!(none.as_array().unwrap().is_empty(), "{none}");
}

#[test]
fn add_bundle_indexes_the_pouch_surface() {
    let (lsp, mut pool) = RutLsp::spawn_native();
    let out = take(pool.run_until(lsp.add_bundle(POUCH_BUNDLE.to_vec())));
    assert_eq!(out["module"], "pouch");
    assert_eq!(out["files"], serde_json::json!(["mod.d.rut"]));
    // a corrupt archive is the error envelope, never silent state
    let err = take(pool.run_until(lsp.add_bundle(b"definitely not a zip".to_vec())));
    assert!(
        err["error"].as_str().unwrap().contains("not a zip"),
        "{err}"
    );

    // the indexed surface resolves: the use name jumps to BOTH pouch
    // indexes — the std surface's true source path AND the bundle's
    let doc = "use pouch::{ Vec };\nfn main() -> nil {\n    let v: Vec<i32> = Vec::new();\n    v.push(1);\n}\n";
    let uri = "file:///ws/uses-pouch.rut";
    take(pool.run_until(lsp.analyze(uri.to_string(), doc.to_string())));
    let locs = take(pool.run_until(lsp.definition(uri.to_string(), 0, 13))); // the use-line `Vec`
    let uris: Vec<&str> = locs
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["uri"].as_str().unwrap())
        .collect();
    assert!(
        uris.contains(&"bundle:pouch/mod.d.rut"),
        "the bundle index answers go-to-definition: {uris:?}"
    );
    // and the member surface hovers through the named module
    let push_ch = doc.lines().nth(3).unwrap().find("push").unwrap() as u32;
    let h = take(pool.run_until(lsp.hover(uri.to_string(), 3, push_ch)));
    assert!(
        h["contents"]["value"].as_str().unwrap().contains("push"),
        "{h}"
    );
}

/// plan3 phase 4 (B1+B2): the CHECK LANE through the bindgen face —
/// a doc whose package is STAMPED (`add_def_mod` named it) compiles
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
    let stamped = take(pool.run_until(lsp.add_def_mod(
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
    let stamped = take(pool.run_until(lsp.add_def_mod(
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
fn add_def_mod_stamps_the_tree_and_feeds_the_use_walk() {
    let (lsp, mut pool) = RutLsp::spawn_native();
    // the root: `pub mod layout;` rides back as the mount walk's
    // next hop (the host reads its children from the envelope)
    let root_src = "pub mod layout;\nmod secret;\npub fn tag() -> i32 { return 1; }\n";
    let uri_root = "file:///ws/gadgets/mod.rut";
    let out = take(pool.run_until(lsp.add_def_mod(
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
    let out = take(pool.run_until(lsp.add_def_mod(
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
    let items = take(pool.run_until(lsp.complete(uri.to_string(), 0, 21)));
    let labels: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    assert!(labels.contains(&"Column"), "{labels:?}");
    assert!(labels.contains(&"mk"), "{labels:?}");
    // the root tier offers the pub mod child, never the private one
    let items = take(pool.run_until(lsp.complete(uri.to_string(), 0, 13)));
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

/// a mod-carrying bundle in the pack writer's shape — the wasm
/// face mounts the rows tree additively
fn kit_bundle() -> Vec<u8> {
    use std::collections::BTreeMap;
    let mods = BTreeMap::from([(
        "layout".to_string(),
        rut_driver::ModSource {
            path: "layout".into(),
            vis: rut_ast::ast::Vis::Pub,
            text: "pub struct Column {\n    w: i32;\n}\n".into(),
            file: "mod.rut".into(),
        },
    )]);
    let rows = rut_driver::mods::rows_json("pub mod layout;\n", &mods);
    rut_driver::bundle::write_bundle(&[
        (
            rut_driver::bundle::files::MANIFEST_NAME.into(),
            br#"{ "name": "kit" }"#.to_vec(),
        ),
        (rut_driver::mods::ROWS_NAME.into(), rows.into_bytes()),
    ])
    .unwrap()
}

#[test]
fn add_bundle_mounts_a_mod_tree() {
    let (lsp, mut pool) = RutLsp::spawn_native();
    let bytes = kit_bundle();
    let out = take(pool.run_until(lsp.add_bundle(bytes.to_vec())));
    assert_eq!(out["module"], "kit");
    assert_eq!(
        out["files"],
        serde_json::json!(["mod.rut", "layout/mod.rut"]),
        "the tree rides the file list: {}",
        out["files"]
    );
    // `use kit::layout::⏐` — the mounted child's exports
    let doc = "use kit::layout::{ Column };\nfn main() -> nil {\n    let c: Column = Column { w: 1 };\n}\n";
    let uri = "file:///ws/uses-kit.rut";
    take(pool.run_until(lsp.analyze(uri.to_string(), doc.to_string())));
    let items = take(pool.run_until(lsp.complete(uri.to_string(), 0, 16)));
    let labels: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    assert!(labels.contains(&"Column"), "{labels:?}");
    // and the segment hover resolves through the dep tree
    let h = take(pool.run_until(lsp.hover(uri.to_string(), 0, 12))); // the `layout` segment
    let md = h["contents"]["value"].as_str().unwrap();
    assert!(md.contains("file module"), "{md}");
    assert!(md.contains("layout/mod.rut"), "{md}");
}

/// the extract lane — `add_bundle`'s unpack WITHOUT the index
/// write: the file list mirrors `bundle_sources` byte-for-byte,
/// every file spells its own mod path (the host never re-derives
/// the mount rule), and the defs store ends the call untouched
#[test]
fn extract_bundle_names_the_surface_and_never_indexes() {
    let (lsp, mut pool) = RutLsp::spawn_native();
    let out = take(pool.run_until(lsp.extract_bundle(POUCH_BUNDLE.to_vec())));
    assert_eq!(out["module"], "pouch");
    assert!(out["namespace"].is_null());
    // the file list mirrors `bundle_sources`' own unpack
    let b = rut_lsp_wasm::deps::bundle_sources(POUCH_BUNDLE).expect("the pouch bundle unpacks");
    let paths: Vec<&str> = out["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    let want: Vec<&str> = b.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, want, "the envelope's files ride bundle_sources'");
    // the generated declaration entry rides, mode + mod path spelled
    // (wire v12 — the entry IS the surface the extension writes back)
    let f = &out["files"][0];
    assert_eq!(f["path"], "mod.d.rut");
    assert_eq!(f["mode"], "Decl");
    assert_eq!(f["mod_path"], "");
    assert!(
        f["src"].as_str().unwrap().contains("class Vec"),
        "the pouch entry rides"
    );
    // NO indexing side effect — the same probe the mount test answers
    // positively (`use kit::layout::⏐` → `Column`), run against a world
    // where only `extract_bundle` ran: the extract lane answers bytes,
    // the index moves only through `add_bundle`
    let doc = "use kit::layout::\nfn main() -> nil {\n}\n";
    let uri = "file:///ws/extract-only.rut";
    take(pool.run_until(lsp.analyze(uri.to_string(), doc.to_string())));
    let items = take(pool.run_until(lsp.complete(uri.to_string(), 0, 16)));
    let labels: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    assert!(
        !labels.contains(&"Column"),
        "the extract lane never writes the index: {labels:?}"
    );

    // a mod tree spells each file's mount path (the root `""`, each
    // child its `<path>`); a corrupt archive stays the error
    // envelope, never bytes
    let bytes = kit_bundle();
    let out = take(pool.run_until(lsp.extract_bundle(bytes.to_vec())));
    let spells: Vec<(&str, &str)> = out["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["path"].as_str().unwrap(), f["mod_path"].as_str().unwrap()))
        .collect();
    assert_eq!(spells, vec![("mod.rut", ""), ("layout/mod.rut", "layout")]);
    let junk = b"definitely not a zip";
    let err = take(pool.run_until(lsp.extract_bundle(junk.to_vec())));
    assert!(
        err["error"].as_str().unwrap().contains("not a zip"),
        "{err}"
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
