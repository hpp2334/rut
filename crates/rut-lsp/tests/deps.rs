//! The deps face's gate — the manifest grammar and the pin ledger,
//! driven through the `parse_manifest` envelope and the dep walk: the
//! dep-table rows (path/url, the cache path's URL hash, the retired
//! keys' loud refusals, the host shapes), the lock's pin stamping (and
//! a missing row staying honest `null`), the dev-deps rows, and the
//! named-dep mounts the walk lays down (impl methods riding the dep
//! index, the surface `pub let` rows, the mod-path stamps).

mod common;

use common::*;
use serde_json::Value;
use std::rc::Rc;

/// The plugin manifest's dep shape, INLINE and unpinned: the real
/// `examples/03-plugin/plugin/rut.jsonc` — the directory grammar's
/// spelling (the pin lives in rut.lock, never the manifest).
const PLUGIN_SHAPE: &str = r#"{
  "name": "plugin",
  "deps": {
    "server": { "path": "../server" },
    "pouch": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v18/dist/std/pouch.zip" }
  }
}"#;

const POUCH_URL: &str = "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v18/dist/std/pouch.zip";
/// the real std-v18 pin — rut.lock's row for POUCH_URL
const POUCH_PIN: &str = "ce5f554cdf6f6dbfe758281e598e18b46158db7be26277b662c9ed3c1a5c71a7";

/// the URL string's own sha256 (the cache filename's law — the test
/// computes the expected name independently)
fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn manifest_rows_path_and_url() {
    let mut s = S::boot();
    let t = s.parse_manifest(PLUGIN_SHAPE);
    assert_eq!(t["name"], "plugin");
    // the entry: one lib source, Impl mode — the body rides the
    // src/mod.rut convention (the entry keys are repealed)
    assert_eq!(
        t["entries"],
        serde_json::json!([{ "path": "src/mod.rut", "mode": "Impl" }]),
        "{}",
        t["entries"]
    );
    // deps: the path row …
    let rows = t["deps"].as_array().unwrap();
    let server = rows.iter().find(|r| r["name"] == "server").unwrap();
    assert_eq!(
        server["source"],
        serde_json::json!({ "kind": "path", "dir": "../server" })
    );
    // … and the url row: UNPINNED at parse (the pin is the lock's row),
    // the cache path computed here from the URL STRING
    let pouch = rows.iter().find(|r| r["name"] == "pouch").unwrap();
    assert_eq!(pouch["source"]["kind"], "url");
    assert_eq!(pouch["source"]["url"], POUCH_URL);
    assert!(
        pouch["source"]["sha256"].is_null(),
        "no lock row ⇒ unpinned: {}",
        pouch["source"]
    );
    assert_eq!(
        pouch["source"]["cache_path"],
        format!(".rut/cache/{}.zip", sha256_hex(POUCH_URL)),
        "{}",
        pouch["source"]
    );
    assert!(t["dev_deps"].as_array().unwrap().is_empty());
}

#[test]
fn the_lock_pins_the_url_rows_and_a_missing_row_stays_none() {
    // the walk-root rut.lock's rows stamp the url rows (parse_manifest
    // reads the ledger the walk fed); a url the lock does not name
    // stays `null` — the walk's install-recipe refusal
    let mut io = MemIo::default();
    io.put("/ws/proj/src/mod.rut", "fn main() -> nil {\n}\n");
    io.put("/ws/proj/rut.jsonc", PLUGIN_SHAPE);
    io.put(
        "/ws/proj/rut.lock",
        &rut_driver::render_lock(&rut_driver::LockFile::from_rows(vec![
            rut_driver::LockRow {
                pkg: "pouch".into(),
                url: POUCH_URL.into(),
                sha256: POUCH_PIN.into(),
            },
        ])),
    );
    let io = Rc::new(io);
    let mut s = S::with_io(&io);
    let out = s.resolve(&["/ws/proj"]);
    assert_eq!(
        out["missing"].as_array().unwrap().len(),
        1,
        "the store is cold: {out}"
    );
    // the ledger stands: parse_manifest now stamps the pinned row
    let t = s.parse_manifest(PLUGIN_SHAPE);
    let pouch = t["deps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "pouch")
        .unwrap();
    assert_eq!(
        pouch["source"]["sha256"], POUCH_PIN,
        "the pin is lock-sourced: {}",
        pouch
    );
    assert_eq!(
        pouch["source"]["cache_path"],
        format!(".rut/cache/{}.zip", sha256_hex(POUCH_URL))
    );
    // a session that never walked: the same manifest answers unpinned
    let mut s = S::boot();
    let t = s.parse_manifest(PLUGIN_SHAPE);
    let pouch = t["deps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "pouch")
        .unwrap();
    assert!(
        pouch["source"]["sha256"].is_null(),
        "no lock row ⇒ unpinned: {}",
        pouch
    );
}

#[test]
fn a_malformed_lock_hints_the_error_not_silent_pins() {
    // the walk surfaces the lock's parse error as the hint — the
    // ledger CLEARS (the honest unpinned refusal, never half-parsed)
    let url_manifest = r#"{"name": "p", "deps": {"pouch": {"url": "https://x/p.zip"}}}"#;
    let mut io = MemIo::default();
    io.put("/ws/proj/src/mod.rut", "fn main() -> nil {\n}\n");
    io.put("/ws/proj/rut.jsonc", url_manifest);
    io.put(
        "/ws/proj/rut.lock",
        r#"{"format": "rutlock", "format_version": 1, "deps": [{"pkg": "p", "url": "https://x/p.zip", "sha256": "nothex"}]}"#,
    );
    let io = Rc::new(io);
    let mut s = S::with_io(&io);
    let _out = s.resolve(&["/ws/proj"]);
    let notes = io.notes.borrow();
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(
        notes.iter().any(|n| n.contains("64 hex digits")),
        "the parse error rides the hint: {notes:?}"
    );
    assert!(
        notes.iter().any(|n| n.contains("no sha256 pin")),
        "the cleared ledger's unpinned refusal: {notes:?}"
    );
    // the cleared ledger stands: the url row answers unpinned
    let mut s = S::with_io(&io);
    let t = s.parse_manifest(url_manifest);
    let pouch = t["deps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "pouch")
        .unwrap();
    assert!(pouch["source"]["sha256"].is_null(), "{}", pouch);
}

#[test]
fn the_retired_keys_refuse_loudly_and_calc_host_parses() {
    // the consts RETIRED from the manifest: the ONE grammar refuses
    // the key loudly, naming the surface spelling — no lane
    // grandfathered. The real calc_host manifest — the surface lets
    // are the one spelling now — parses clean.
    let mut s = S::boot();
    let calc_host = include_str!("../../../rut/calc_host/rut.jsonc");
    let t = s.parse_manifest(calc_host);
    assert_eq!(t["name"], "calc_host");
    assert_eq!(
        t["entries"],
        serde_json::json!([{ "path": "src/calc.d.rut", "mode": "Decl" }]),
        "the host surface: decl mode, no body — {}",
        t["entries"]
    );
    // the refusal names the fix: the surface `pub let` row
    let bad = s.parse_manifest(
        r#"{"name": "calc_host", "type": "host", "consts": {"PI": 3.141592653589793}}"#,
    );
    let err = bad["error"].as_str().unwrap();
    assert!(
        err.contains("consts: retired")
            && err.contains("declare `pub let NAME: f64 = <literal>;` in the surface"),
        "the refusal names the fix: {err}"
    );
    // the wire pair parses on the ONE grammar — the packer stamps it
    // and the bundle reader gates on it (legal-optional)
    let t = s.parse_manifest(r#"{"name": "x", "format": "rutbundle", "format_version": 13}"#);
    assert_eq!(t["name"], "x", "{t}");
    // the retired `namespace` key refuses loudly, naming the spellings
    // that replaced it
    let bad = s.parse_manifest(&calc_host.replace(
        "\"type\": \"host\",",
        "\"type\": \"host\",\n  \"namespace\": \"Math\",",
    ));
    let err = bad["error"].as_str().unwrap();
    assert!(
        err.contains("namespace: retired") && err.contains("delete the key"),
        "{err}"
    );
}

#[test]
fn dev_descriptor_rows_ride() {
    let mut s = S::boot();
    let t = s.parse_manifest(
        r#"{
  "name": "json",
  "dev-deps": { "nmapset": { "path": "../nmapset" } }
}"#,
    );
    let rows = t["dev_deps"].as_array().unwrap();
    let nmapset = rows
        .iter()
        .find(|r| r["name"] == "nmapset")
        .expect("dev row");
    assert_eq!(
        nmapset["source"],
        serde_json::json!({ "kind": "path", "dir": "../nmapset" })
    );
}

#[test]
fn a_bad_manifest_is_the_error() {
    let mut s = S::boot();
    let bad = s.parse_manifest("{\"name\": six}");
    let err = bad["error"].as_str().unwrap();
    assert!(err.starts_with("line 1: "), "{err}");
}

#[test]
fn cache_path_hashes_the_url_not_the_bytes() {
    // the filename is sha256 of the URL STRING — deterministic,
    // computable before any fetch (a `.rutbundle` url carries its own
    // extension)
    let mut s = S::boot();
    let t = s.parse_manifest(
        r#"{"name": "p", "deps": {"pouch": {"url": "https://example.com/pouch.rutbundle"}}}"#,
    );
    let pouch = t["deps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "pouch")
        .unwrap();
    assert_eq!(
        pouch["source"]["cache_path"],
        format!(
            ".rut/cache/{}.rutbundle",
            sha256_hex("https://example.com/pouch.rutbundle")
        ),
        "{}",
        pouch
    );
}

// ---- the named-dep mounts the walk lays down ----

#[test]
fn a_host_surface_mount_indexes_the_lets_and_fns() {
    // the walk mounts a host pkg's `.d.rut` surface named: the flat
    // fns AND the surface `pub let` rows ride the ordinary paths (the
    // `use calc_host::{ PI }` form indexes exactly as authored source;
    // `inf` spells the non-finite literal)
    let mut io = MemIo::default();
    io.put(
        "/ws/calchost/rut.jsonc",
        "{\n  \"name\": \"calchost\",\n  \"type\": \"host\"\n}\n",
    );
    io.put(
        "/ws/calchost/src/mod.d.rut",
        "pub host fn sqrt(x: f64) -> f64;\npub let PI: f64 = 3.141592653589793;\npub let INFINITY: f64 = inf;\n",
    );
    io.put(
        "/ws/proj/rut.jsonc",
        "{\n  \"name\": \"proj\",\n  \"deps\": {\"calchost\": {\"path\": \"/ws/calchost\"}}\n}\n",
    );
    io.put("/ws/proj/src/mod.rut", "fn main() -> nil {\n}\n");
    let io = Rc::new(io);
    let mut s = S::with_io(&io);
    let out = s.resolve(&["/ws/proj"]);
    assert_eq!(out["missing"].as_array().unwrap().len(), 0, "{out}");

    // the brace tier offers the fns and the lets (PI a constant)
    let uri = "file:///t/main.rut";
    let items = s.complete_pos(uri, "use calchost::{", 0, 15);
    let ls = labels(&items);
    assert!(ls.contains(&"sqrt"), "the surface fns complete: {ls:?}");
    assert!(ls.contains(&"PI"), "the surface lets complete: {ls:?}");
    assert!(
        ls.contains(&"INFINITY"),
        "the inf literal indexes like any float: {ls:?}"
    );
    let pi = item(&items, "PI");
    assert_eq!(pi["kind"], k::CONSTANT, "{}", pi);
    // the verbatim head rides the detail: the spelled `pub` + the row
    let detail = pi["detail"].as_str().unwrap();
    assert!(
        detail.starts_with("pub let PI: f64 = 3.141592653589793"),
        "the verbatim row: {detail}"
    );

    // the leaf resolves into the surface (F12 lands in the dep's file)
    let hits = s.def_in(
        uri,
        "use calchost::{ PI };\nfn main() -> nil {\n}\n",
        "{ PI }",
        1,
        2,
    );
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///ws/calchost/src/mod.d.rut");
}

#[test]
fn dep_impl_methods_ride_the_dep_index() {
    // THE member law: a dep source's inherent impl methods ride the
    // dep index, so a consumer's `log.info(…)` click resolves through
    // the impl fallback to the dep index's fn — the ink shape that
    // stranded every scaffold's method jump
    let mut s = S::boot();
    const INK: &str = concat!(
        "use ink_host::{ create_logger, logger_log };\n",
        "pub class Logger {\n",
        "    internal: opaque;\n",
        "}\n",
        "impl Logger {\n",
        "    pub fn info(self, msg: str) -> nil { logger_log(self.internal, 1, msg); }\n",
        "}\n",
    );
    s.index_mod_at("file:///deps/ink/src/mod.rut", "ink", "", INK);
    let consumer =
        "use ink::{ Logger };\nentry fn main() {\n    let log = Logger(\"hello\");\n    log.info(\"hi\");\n}\n";
    let uri = "file:///t/main.rut";
    let hits = s.def_in(uri, consumer, "log.info", 1, 4);
    assert_eq!(hits.len(), 1, "the .info click resolves: {hits:?}");
    assert_eq!(hits[0]["uri"], "file:///deps/ink/src/mod.rut");
    assert_target(INK, &hits[0], "info(self", 1, 4, "the dep's impl fn");
}

// ---- the check lane's degraded retry (the folder-only e2e pin's
// native twin): a doc whose dep closure is unresolved used to silence
// the WHOLE semantic set — the run's closure refusal carries no spans,
// and `Err` meant `Vec::new()`. Silence is the bug: the doc's own name
// errors must survive. ----

/// the self-contained ink surface (the nested e2e fixture's shape — no
/// dangling heads of its own, so the full world assembles)
const CLEAN_INK: &str = "pub class Logger {\n    internal: opaque;\n}\nimpl Logger {\n    pub fn info(self, msg: str) -> nil {\n        return;\n    }\n}\n";

#[test]
fn an_unresolvable_closure_keeps_the_docs_own_errors() {
    // the headline pin: the doc names `ink`, nothing mounts it — the
    // run refuses — and the DEGRADED retry still answers the doc's own
    // unknowns: `y` at its site, the unnameable `Logger` at its call.
    // The use statement itself stays clean (the head is blanked
    // span-preserving in the degraded copy), and nothing lands outside
    // the doc's true misses.
    let mut s = S::boot();
    let doc = "use ink::{ Logger };\nentry fn main() {\n    let log = Logger(\"x\");\n    let x = y;\n}\n";
    let uri = "file:///ws/app/src/mod.rut";
    s.index_mod_at(uri, "app", "", doc);
    let a = s.analyze(uri, doc);
    let diags = a["diags"].as_array().unwrap();
    assert_eq!(diags.len(), 2, "the two true misses, no more: {a}");

    // the headline: the doc's OWN unknown name squiggles at its line
    let (y_line, y_ch) = pos_of(doc, "= y;", 1);
    let (y_line, y_ch) = (i64::from(y_line), i64::from(y_ch) + 2); // the needle is `= y;` — the ident starts +2
    let hits: Vec<&Value> = diags
        .iter()
        .filter(|d| d["message"].as_str().unwrap().contains("`y`"))
        .collect();
    assert_eq!(hits.len(), 1, "one `y` squiggle: {a}");
    assert!(
        hits[0]["message"]
            .as_str()
            .unwrap()
            .contains("unknown name"),
        "the doc's own unknown name: {a}"
    );
    assert_eq!(pos_pair(&hits[0]["range"]["start"]), (y_line, y_ch), "{a}");
    assert_eq!(
        pos_pair(&hits[0]["range"]["end"]),
        (y_line, y_ch + 1),
        "{a}"
    );

    // the unmounted head's honest miss: `Logger` squiggles at its CALL
    // (a true squiggle — the head really is unnameable), never on the
    // use statement
    let (call_line, call_ch) = pos_of(doc, "Logger(\"x\")", 1);
    let (call_line, call_ch) = (i64::from(call_line), i64::from(call_ch));
    let hits: Vec<&Value> = diags
        .iter()
        .filter(|d| d["message"].as_str().unwrap().contains("Logger"))
        .collect();
    assert_eq!(hits.len(), 1, "one `Logger` squiggle: {a}");
    assert!(
        hits[0]["message"].as_str().unwrap().contains("unknown"),
        "the unnameable head's miss: {a}"
    );
    assert_eq!(
        pos_pair(&hits[0]["range"]["start"]),
        (call_line, call_ch),
        "{a}"
    );
    assert!(
        diags.iter().all(|d| d["range"]["start"]["line"] != 0),
        "the use line stays clean: {a}"
    );
}

#[test]
fn a_resolved_closure_rides_the_ok_branch() {
    // the Ok branch is untouched: the same doc against a RESOLVABLE
    // closure answers exactly today's set — the constructor-call lint
    // (a ctor-less class called directly) beside the doc's own unknown
    // name, the dep's surface binding clean around them
    let mut s = S::boot();
    s.index_mod_at("file:///deps/ink/src/mod.rut", "ink", "", CLEAN_INK);
    let doc = "use ink::{ Logger };\nentry fn main() {\n    let log = Logger(\"x\");\n    log.info(\"hi\");\n    let x = y;\n}\n";
    let uri = "file:///ws/app/src/mod.rut";
    s.index_mod_at(uri, "app", "", doc);
    let a = s.analyze(uri, doc);
    let diags = a["diags"].as_array().unwrap();
    assert_eq!(diags.len(), 2, "the Ok branch's answer, unchanged: {a}");
    let (y_line, y_ch) = pos_of(doc, "= y;", 1);
    let hits: Vec<&Value> = diags
        .iter()
        .filter(|d| d["message"].as_str().unwrap().contains("`y`"))
        .collect();
    assert_eq!(hits.len(), 1, "the doc's own unknown name: {a}");
    assert_eq!(
        pos_pair(&hits[0]["range"]["start"]),
        (i64::from(y_line), i64::from(y_ch) + 2),
        "the doc's own unknown name at its site: {a}"
    );
    assert!(
        diags.iter().any(|d| d["message"]
            .as_str()
            .unwrap()
            .contains("constructs through its class methods")),
        "the ctor-call lint rides the resolved world, as today: {a}"
    );
}

#[test]
fn a_dirty_doc_degrades_without_duplication() {
    // a syntax-damaged doc beside a dangling use: the closure gate
    // skips the dirty file, the compile reports the syntax, and the
    // merge dedupes the TRUE duplicates (same range AND message) — the
    // degraded set only ever ADDS distinct sites
    let mut s = S::boot();
    let doc = "use ink::{ Logger };\nentry fn main() {\n    let x = ;\n    let z = q;\n}\n";
    let uri = "file:///ws/app/src/mod.rut";
    s.index_mod_at(uri, "app", "", doc);
    let a = s.analyze(uri, doc);
    let diags = a["diags"].as_array().unwrap();
    assert_eq!(diags.len(), 2, "the syntax set, exactly once: {a}");
    assert!(
        diags
            .iter()
            .all(|d| d["message"].as_str().unwrap().contains("expected")),
        "the parse lane's own diags: {a}"
    );
}

#[test]
fn mod_path_stamps_spell_the_tree() {
    // the mod-path stamps ride the scan lane's envelope: the root's
    // `pub mod layout;` edge answers the walk's next hop, the child's
    // rows complete under their path, and the completion detail spells
    // the mounted display
    let mut s = S::boot();
    let root = s.index_mod_at(
        "file:///deps/gadgets/mod.rut",
        "gadgets",
        "",
        "pub mod layout;\npub fn tag() -> i32 { return 1; }\n",
    );
    assert_eq!(
        root["decls"],
        serde_json::json!([{ "name": "layout", "vis": "pub" }]),
        "{}",
        root
    );
    let child = s.index_mod_at(
        "file:///deps/gadgets/layout/mod.rut",
        "gadgets",
        "layout",
        "pub struct Column {\n    w: i32;\n}\n",
    );
    assert!(child["decls"].as_array().unwrap().is_empty(), "{}", child);
    // `use gadgets::` — the walkable child's row spells the mounted
    // display; the child's exports complete under their path
    let items = s.complete_pos("file:///t/main.rut", "use gadgets::", 0, 13);
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
    let items = s.complete_pos("file:///t/main.rut", "use gadgets::layout::", 0, 21);
    assert!(
        offers(&items, "Column"),
        "the child's exports complete: {:?}",
        labels(&items)
    );
}

#[test]
fn a_dep_without_lets_indexes_none() {
    // a lib dep carries no surface lets — the brace tier offers its
    // classes and fns, never invented const rows
    let mut s = S::boot();
    s.index_mod_at(
        "file:///deps/pouch.rut",
        "pouch",
        "",
        "pub class Vec<T> {\n}\n",
    );
    let items = s.complete_pos("file:///t/main.rut", "use pouch::{", 0, 13);
    let ls = labels(&items);
    assert!(ls.contains(&"Vec"), "{ls:?}");
    assert!(!ls.contains(&"PI"), "no invented lets: {ls:?}");
}

// ---- the host manifest shapes ----

#[test]
fn a_host_manifest_without_entry_type_discovers_the_default_root() {
    // the lib-parity amendment: `src/mod.d.rut` under the `src/` shell
    // is the auto-discovered root — the dep table carries it as the
    // Decl entry exactly like a spelled surface; a lib manifest with
    // an entry spells no default root
    let mut s = S::boot();
    let t = s.parse_manifest(r#"{ "name": "h", "type": "host" }"#);
    assert_eq!(
        t["entries"],
        serde_json::json!([{ "path": "src/mod.d.rut", "mode": "Decl" }]),
        "the default root: {}",
        t["entries"]
    );
    // a spelled surface still wins — under the shell as well
    let t =
        s.parse_manifest(r#"{ "name": "h", "type": "host", "entry": { "type": "./h.d.rut" } }"#);
    assert_eq!(
        t["entries"],
        serde_json::json!([{ "path": "src/h.d.rut", "mode": "Decl" }]),
        "{}",
        t["entries"]
    );
    // a host manifest marks the table through its entry: the surface
    // indexes in Decl mode, no lib body row rides
    let t = s.parse_manifest(
        r#"{ "name": "myhost", "type": "host", "entry": { "type": "./myhost.d.rut" } }"#,
    );
    assert_eq!(
        t["entries"],
        serde_json::json!([{ "path": "src/myhost.d.rut", "mode": "Decl" }]),
        "{}",
        t["entries"]
    );
    // a lib manifest's spelled surface rides WITH the body row (the
    // convention's root module follows)
    let t = s.parse_manifest(r#"{ "name": "s", "type": "lib", "entry": { "type": "./s.d.rut" } }"#);
    assert_eq!(
        t["entries"],
        serde_json::json!([
            { "path": "src/s.d.rut", "mode": "Decl" },
            { "path": "src/mod.rut", "mode": "Impl" },
        ]),
        "{}",
        t["entries"]
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
