//! url deps — `[deps]` rows naming a remote `.rutbundle` url, an
//! optional `sha256` pin, and the injected remote. The layer
//! split under test: the CALL SITE owns HOW bytes arrive (here: the
//! shared ready-map fixture — no network, no runtime dep), the LOADER
//! owns WHAT they are — the pin is manifest law, verified at the mount
//! door on every load.
//!
//! Fixtures are real packed bundles (`pack_dir`, no net); the consumer
//! worlds mount them through the fetcher. Bundles stay CLOSED: no test
//! fetches at bundle load, and the inert-url-row case proves the
//! loader refuses rather than fetches.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod common;
use common::{block_on, Table};

use rut_driver::{
    bundle::FsSource, compile_graph, load_bundle_bytes, load_dir_session,
    load_dir_session_fetched, mount_std, pack_dir, sha256_hex, ModuleBody,
};

// ------------------------------------------------------------------
// the fixture world: real dirs, real packed bundles

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-url-deps-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, text).unwrap();
}

fn manifest(name: &str, entry: &str, extra: &str) -> String {
    // `extra` is the fragment after the entry object (`, "deps": { … }`)
    format!(r#"{{"format": "rutbundle", "format_version": 9, "name": "{name}", "entry": {{"lib": "./{entry}"}}{extra}}}"#)
}

/// A plain linkable leaf pkg — packs to a one-group compiled bundle.
fn leaf_world(tag: &str, name: &str, body: &str) -> PathBuf {
    let root = scratch(tag);
    let dir = root.join(name);
    write(&dir, "rut.jsonc", &manifest(name, &format!("{name}.rut"), ""));
    write(&dir, &format!("{name}.rut"), body);
    root
}

const UTIL_BODY: &str = "pub fn twice(v: i64) -> i64 {\n    return v * 2;\n}\n";

/// Rewrite one archive entry (the manifest, say) and re-seal the
/// container — the doctored-closure fixtures.
fn resealed(bytes: &[u8], key: &str, text: &str) -> Vec<u8> {
    let mut entries: Vec<(String, Vec<u8>)> = rut_driver::bundle::parse_bundle(bytes).unwrap();
    for (n, b) in entries.iter_mut() {
        if n == key {
            *b = text.as_bytes().to_vec();
        }
    }
    rut_driver::bundle::write_bundle(&entries).unwrap()
}

fn linked_binary(session: &rut_driver::Session, root: &str) -> Vec<u8> {
    let g = compile_graph(session, root);
    assert!(g.diags.is_empty(), "{:?}", g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n"));
    rut_core::binary::encode(&g.program.expect("linked program"))
}

/// Mount + mount_std + compile + flatten + run one entry.
fn run_entry<R: rut_vm::interp::Ret>(session: rut_driver::Session, root: &str, entry: &str) -> R {
    let mut session = session;
    mount_std(&mut session);
    let g = compile_graph(&session, root);
    assert!(g.diags.is_empty(), "{:?}", g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n"));
    let flat = rut_core::link::flatten(g.program.expect("linked program"));
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(flat),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    vm.call::<_, R>(entry, ()).expect("run")
}

/// Does a probe module naming the group's wrapper compile over this
/// session? The wrapper name is the compiled-world observable for "the
/// group mounted" — the impl-registration rows are gone (satisfaction
/// is structural), so the group's exported wrapper is what resolves.
fn probe_compiles(session: &rut_driver::Session, probe: &str) -> bool {
    let mut s = session.clone();
    s.register_module(
        "probe",
        rut_driver::Module {
            body: rut_driver::ModuleBody::Source { text: probe.to_string(), is_decl: false },
            ..Default::default()
        },
    )
    .expect("register probe");
    let g = compile_graph(&s, "probe");
    g.diags.is_empty()
}

const CODEC_POUCH_PROBE: &str = "use codec::{ CodedVec };\nclass P(CodedVec);\nentry fn main() -> i32 { return 0; }\n";

// ------------------------------------------------------------------
// the tests

#[test]
fn url_dep_mounts_compiles_and_equals_the_dir_twin() {
    let root = leaf_world("happy", "util", UTIL_BODY);
    let bytes = pack_dir(&root.join("util")).expect("pack util");
    let url = "https://fixtures.test/util.rutbundle";
    let pin = sha256_hex(&bytes);

    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "app.rut",
            &format!(r#", "deps": {{"util": {{"url": "{url}", "sha256": "{pin}"}}}}"#),
        ),
    );
    write(
        &app,
        "app.rut",
        "use util::{twice};\n\nentry fn go() -> i64 {\n    return twice(21);\n}\n",
    );

    let mut table = BTreeMap::new();
    table.insert(url.to_string(), bytes.clone());
    let (session, app_root) = block_on(rut_driver::load_dir_session_with(&app, &Table::from(table)))
        .expect("url load");
    assert_eq!(app_root, "app");
    // the url dep is a leaf: mounted, not walked — its body is the
    // bundle's compiled root
    assert!(matches!(
        session.resolve("util").unwrap().body,
        ModuleBody::Compiled(_)
    ));
    assert_eq!(run_entry::<i64>(session, "app", "go"), 42);

    // the pinned equivalence: the url lane and the path lane link to
    // the identical binary
    let twin = root.join("app_path");
    write(&twin, "rut.jsonc", &manifest("app", "app.rut", r#", "deps": {"util": {"path": "../util"}}"#));
    write(&twin, "app.rut", "use util::{twice};\n\nentry fn go() -> i64 {\n    return twice(21);\n}\n");
    let (dir_session, dir_root) = load_dir_session(&twin, &FsSource).expect("dir load");
    assert_eq!(
        linked_binary(&dir_session, &dir_root),
        {
            let mut table = BTreeMap::new();
            table.insert(url.to_string(), bytes);
            let (s, r) = block_on(rut_driver::load_dir_session_with(&app, &Table::from(table))).unwrap();
            linked_binary(&s, &r)
        },
        "dir and url lanes compile identically"
    );
}

#[test]
fn pin_mismatch_names_dep_url_and_both_hashes() {
    let root = leaf_world("pin", "util", UTIL_BODY);
    let bytes = pack_dir(&root.join("util")).expect("pack util");
    let url = "https://fixtures.test/util.rutbundle";
    // a VALID but wrong pin: 64 hex digits, not the bytes' hash
    let wrong = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";

    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "app.rut",
            &format!(r#", "deps": {{"util": {{"url": "{url}", "sha256": "{wrong}"}}}}"#),
        ),
    );
    write(&app, "app.rut", "entry fn go() -> i64 {\n    return 1;\n}\n");

    let mut table = BTreeMap::new();
    table.insert(url.to_string(), bytes.clone());
    let err =
        block_on(rut_driver::load_dir_session_with(&app, &Table::from(table.clone())))
            .unwrap_err()
            .to_string();
    assert!(err.contains("sha256 pin mismatch"), "{err}");
    assert!(err.contains("util"), "{err}");
    assert!(err.contains(url), "{err}");
    assert!(err.contains(wrong), "the pinned hash: {err}");
    assert!(err.contains(&sha256_hex(&bytes)), "the fetched bytes' hash: {err}");

    // the law runs on EVERY load — a correct pin passes (the happy
    // path's twin), and the same wrong pin refuses the vendored-map
    // lane identically: the door, not the fetcher, holds the lock
    let err = load_dir_session_fetched(&app, &FsSource, &table).unwrap_err().to_string();
    assert!(err.contains("sha256 pin mismatch"), "{err}");
}

#[test]
fn name_vs_key_refuses() {
    let root = leaf_world("namekey", "util", UTIL_BODY);
    let bytes = pack_dir(&root.join("util")).expect("pack util");
    let url = "https://fixtures.test/util.rutbundle";

    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest("app", "app.rut", &format!(r#", "deps": {{"not_util": {{"url": "{url}"}}}}"#)),
    );
    write(&app, "app.rut", "entry fn go() -> i64 {\n    return 1;\n}\n");

    let mut table = BTreeMap::new();
    table.insert(url.to_string(), bytes);
    let err = block_on(rut_driver::load_dir_session_with(&app, &Table::from(table)))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains(&format!("dep `not_util` points at {url}")),
        "{err}"
    );
    assert!(err.contains("the bundle names itself `util`"), "{err}");
}

#[test]
fn bundle_stays_closed_no_fetch_at_bundle_load() {
    // a url row inside an already-packed bundle is inert metadata: the
    // closure law demands a group, the loader refuses — it NEVER
    // fetches at bundle load (load_bundle_bytes owns no fetcher at all)
    let root = leaf_world("closed", "util", UTIL_BODY);
    let bytes = pack_dir(&root.join("util")).expect("pack util");
    let doctored = resealed(
        &bytes,
        "rut.jsonc",
        &manifest(
            "util",
            "util.rut",
            r#", "deps": {"ghost": {"url": "https://fixtures.test/ghost.rutbundle"}}"#,
        ),
    );

    // the direct bundle lane
    let err = load_bundle_bytes(&doctored, Path::new("mem")).unwrap_err().to_string();
    assert!(
        err.contains("the bundle is missing its `ghost` dependency group"),
        "{err}"
    );

    // the same law through the url mount door (gate 7): the consumer's
    // fetcher only knows `util`'s bytes — a fetch for `ghost` would be
    // the loudest possible failure, and the refusal names the group
    let url = "https://fixtures.test/util.rutbundle";
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest("app", "app.rut", &format!(r#", "deps": {{"util": {{"url": "{url}"}}}}"#)),
    );
    write(&app, "app.rut", "entry fn go() -> i64 {\n    return 1;\n}\n");
    let mut table = BTreeMap::new();
    table.insert(url.to_string(), doctored);
    let err = block_on(rut_driver::load_dir_session_with(&app, &Table::from(table)))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("the bundle is missing its `ghost` dependency group"),
        "{err}"
    );
}

#[test]
fn embedder_mount_outranks_the_url_dep() {
    // first-mount-wins: the fetch may satisfy the row, but the MOUNT
    // never overwrites the embedder's util — the walk skips a name
    // already in the session
    let root = leaf_world("firstwin", "util", UTIL_BODY);
    let bytes = pack_dir(&root.join("util")).expect("pack util");
    let url = "https://fixtures.test/util.rutbundle";
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest("app", "app.rut", &format!(r#", "deps": {{"util": {{"url": "{url}"}}}}"#)),
    );
    write(&app, "app.rut", "entry fn go() -> i64 {\n    return 1;\n}\n");

    let mut session = rut_driver::Session::new();
    session
        .register_module(
            "util",
            rut_driver::Module {
                body: ModuleBody::Source { text: "// the embedder's".into(), is_decl: false },
                ..Default::default()
            },
        )
        .unwrap();
    let mut table = BTreeMap::new();
    table.insert(url.to_string(), bytes);
    let name =
        block_on(rut_driver::mount_dir_with(&mut session, &app, &Table::from(table))).expect("mount");
    assert_eq!(name, "app");
    match &session.resolve("util").unwrap().body {
        ModuleBody::Source { text, .. } => assert_eq!(text, "// the embedder's"),
        other => panic!("the embedder's mount must win: {other:?}"),
    }
}

/// The peer world: `codec` (inline, interface + wrapper-only pouch
/// group, dev-deps for its own build) packed INSIDE `zeta` as a
/// declared-but-
/// unused dep, so it rides the archive as a SOURCE group (the group
/// files ride with it) — the consumer sees codec as an ARCHIVE SOURCE
/// GROUP, and the one unified peer gate must read the group file from
/// the archive.
fn peer_world(tag: &str) -> (PathBuf, Vec<u8>, String) {
    let root = scratch(tag);
    write(
        &root.join("pouch"),
        "rut.jsonc",
        &manifest("pouch", "pouch.rut", r#", "inline": true"#),
    );
    write(
        &root.join("pouch"),
        "pouch.rut",
        "pub class Vec<T> {\n    items: [T];\n}\n\nimpl<T> Vec<T> {\n    pub fn filled(v: T, n: i32) -> Self {\n        let mut items: [T] = [v; n];\n        return Self { items: items };\n    }\n\n    pub fn first(self) -> T {\n        return self.items[0];\n    }\n}\n",
    );
    // codec — the json shape: the interface and public names in the
    // base, the wrapper-only group beside it, dev-deps for its own build
    write(
        &root.join("codec"),
        "rut.jsonc",
        &manifest(
            "codec",
            "codec.rut",
            r#", "inline": true, "peer-deps": {"pouch": {"path": "../pouch", "optional": true, "lib": "./codec_pouch.rut"}}, "dev-deps": {"pouch": {"path": "../pouch"}}"#,
        ),
    );
    write(
        &root.join("codec"),
        "codec.rut",
        "pub interface Coded {\n    fn coded(self) -> str;\n}\n\npub class CodedStr(str);\n\nimpl CodedStr {\n    pub fn coded(self) -> str {\n        return self.inner;\n    }\n}\n\npub fn encode(s: str) -> str {\n    return CodedStr(s).coded();\n}\n",
    );
    write(
        &root.join("codec"),
        "codec_pouch.rut",
        "use pouch::{Vec};\n\npub class CodedVec(Vec<i32>);\n\nimpl CodedVec {\n    pub fn coded(self) -> str {\n        return \"[vec]\";\n    }\n}\n",
    );
    // zeta — packs codec as a declared-but-unused (source) group
    write(
        &root.join("zeta"),
        "rut.jsonc",
        &manifest("zeta", "zeta.rut", r#", "deps": {"codec": {"path": "../codec"}}"#),
    );
    write(&root.join("zeta"), "zeta.rut", "pub fn ping() -> i32 {\n    return 7;\n}\n");
    let bytes = pack_dir(&root.join("zeta")).expect("pack zeta");
    let url = "https://fixtures.test/zeta.rutbundle";
    (root, bytes, url.to_string())
}

#[test]
fn mixed_dir_and_archive_peer_gate() {
    let (root, bytes, url) = peer_world("peer");
    // the consumer: pouch by PATH (mounted first — name order), zeta by
    // url (codec rides the archive as a source group). Presence for
    // codec's peer comes from the dir lane; the group file reads from
    // the archive — one gate, two source kinds.
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "app.rut",
            &format!(r#", "deps": {{"pouch": {{"path": "../pouch"}}, "zeta": {{"url": "{url}"}}}}"#),
        ),
    );
    write(
        &app,
        "app.rut",
        "use codec::{encode};\n\nentry fn go() -> str {\n    return encode(\"x\");\n}\n",
    );

    let mut table = BTreeMap::new();
    table.insert(url.clone(), bytes);
    let (session, app_root) =
        block_on(rut_driver::load_dir_session_with(&app, &Table::from(table.clone())))
            .expect("url load");
    assert_eq!(app_root, "app");
    // zeta: compiled; codec: the archive's source group; pouch: the DIR
    // mount won (first-mount-wins across the two lanes)
    assert!(matches!(
        session.resolve("zeta").unwrap().body,
        ModuleBody::Compiled(_)
    ));
    assert!(matches!(
        session.resolve("codec").unwrap().body,
        ModuleBody::Source { .. }
    ));
    assert!(matches!(
        session.resolve("pouch").unwrap().body,
        ModuleBody::Source { .. }
    ));
    // THE unification proof: the group file was read FROM THE ARCHIVE
    // and compiled into codec's unit (the group's wrapper resolves)
    assert!(
        probe_compiles(&session, CODEC_POUCH_PROBE),
        "the archive group must mount through the unified gate"
    );

    // the absence twin: no pouch dir, codec mounts light — the same
    // gate, inert optional peer, no read
    let app2 = root.join("app2");
    write(
        &app2,
        "rut.jsonc",
        &manifest("app2", "app2.rut", &format!(r#", "deps": {{"zeta": {{"url": "{url}"}}}}"#)),
    );
    write(&app2, "app2.rut", "use codec::{encode};\n\nentry fn go() -> str {\n    return encode(\"x\");\n}\n");
    let (session2, _) = block_on(rut_driver::load_dir_session_with(&app2, &Table::from(table)))
        .expect("light load");
    assert!(
        !probe_compiles(&session2, CODEC_POUCH_PROBE),
        "absent peer stays inert: the group never mounted"
    );
}

#[test]
fn colliding_pack_numberings_namespace_per_archive() {
    // two independently packed bundles whose ledgers assign the SAME
    // scope number to DIFFERENT specs — the normal CDN case (every
    // per-package bundle numbers its own closure) — share one session:
    // each archive's rows shift into a fresh range and its binaries
    // rebase with that map, so neither argues about a number. The
    // consumer compiles over both, each dep resolving through its OWN
    // archive's rows.
    let a = leaf_world("leda", "util_a", UTIL_BODY);
    let b = leaf_world("ledb", "util_b", "pub fn thrice(v: i64) -> i64 {\n    return v * 3;\n}\n");
    let bytes_a = pack_dir(&a.join("util_a")).expect("pack a");
    let bytes_b = pack_dir(&b.join("util_b")).expect("pack b");

    let app = a.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "app.rut",
            r#", "deps": {"util_a": {"url": "https://fixtures.test/a.rutbundle"}, "util_b": {"url": "https://fixtures.test/b.rutbundle"}}"#,
        ),
    );
    write(
        &app,
        "app.rut",
        "use util_a::{twice};\nuse util_b::{thrice};\n\nentry fn go() -> i64 {\n    return twice(1) + thrice(2);\n}\n",
    );

    let mut table = BTreeMap::new();
    table.insert("https://fixtures.test/a.rutbundle".to_string(), bytes_a);
    table.insert("https://fixtures.test/b.rutbundle".to_string(), bytes_b);
    let (session, root) = block_on(rut_driver::load_dir_session_with(&app, &Table::from(table)))
        .expect("colliding numberings namespace per archive");
    assert!(matches!(session.resolve("util_a").unwrap().body, ModuleBody::Compiled(_)));
    assert!(matches!(session.resolve("util_b").unwrap().body, ModuleBody::Compiled(_)));
    let units = rut_driver::compile_units(&session, &root);
    assert!(
        units.diags.is_empty() && units.ok,
        "{}",
        units.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("; ")
    );
    // and the value semantics survive both rebases: 2*1 + 3*2 = 8
    assert_eq!(run_entry::<i64>(session, &root, "go"), 8);
}

#[test]
fn sync_wrapper_refuses_url_dep_loudly() {
    // the old sync names keep their signatures: no fetcher, no future
    // — a url row is the loud error naming the FIX
    let root = leaf_world("nofetch", "util", UTIL_BODY);
    let url = "https://fixtures.test/util.rutbundle";
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest("app", "app.rut", &format!(r#", "deps": {{"util": {{"url": "{url}"}}}}"#)),
    );
    write(&app, "app.rut", "entry fn go() -> i64 {\n    return 1;\n}\n");

    let err = load_dir_session(&app, &FsSource).unwrap_err().to_string();
    assert!(err.contains("this loader has no `dep_fetch`"), "{err}");
    assert!(err.contains("`load_dir_session_with`"), "{err}");
    assert!(err.contains("vendor the dep"), "{err}");
    assert!(err.contains(url), "{err}");
}

#[test]
fn pack_url_dep_rode_along_and_deterministic() {
    let root = leaf_world("pack", "util", UTIL_BODY);
    let bytes = pack_dir(&root.join("util")).expect("pack util");
    let url = "https://fixtures.test/util.rutbundle";
    let pin = sha256_hex(&bytes);

    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "app.rut",
            &format!(r#", "deps": {{"util": {{"url": "{url}", "sha256": "{pin}"}}}}"#),
        ),
    );
    write(
        &app,
        "app.rut",
        "use util::{twice};\n\nentry fn go() -> i64 {\n    return twice(21);\n}\n",
    );

    let mut map = BTreeMap::new();
    map.insert(url.to_string(), bytes);
    let packed = rut_driver::pack_dir_fetched(&app, &map).expect("pack");
    // determinism: same manifest + same pins ⇒ same bytes
    let packed_again = rut_driver::pack_dir_fetched(&app, &map).expect("pack again");
    assert_eq!(packed, packed_again, "same inputs must pack byte-identically");

    // the rode-along group: the output carries util as a compiled group
    // re-encoded from THIS session's units (the one .rutc path), and
    // the url+sha256 rows rode byte-for-byte inside its manifest
    let names: Vec<String> = rut_driver::bundle::parse_bundle(&packed)
        .unwrap()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert!(names.contains(&"util/util.rutc".to_string()), "{names:?}");
    assert!(names.contains(&"app.rutc".to_string()), "{names:?}");
    let out_manifest = rut_driver::bundle::Bundle::parse(&packed)
        .unwrap()
        .read("rut.jsonc")
        .unwrap();
    assert!(out_manifest.contains(&format!(r#""url": "{url}""#)), "{out_manifest}");
    assert!(out_manifest.contains(&pin), "{out_manifest}");

    // the packed artifact runs, straight from the output bytes
    let (session, out_root) = load_bundle_bytes(&packed, Path::new("mem")).expect("load");
    assert_eq!(out_root, "app");
    assert_eq!(run_entry::<i64>(session, "app", "go"), 42);

    // the async lane settles identically through the noop-waker driver
    let via_with = block_on(rut_driver::pack_dir_with(&app, &Table::from(map))).expect("pack_with");
    assert_eq!(packed, via_with);
}

#[test]
fn pack_refuses_archive_source_group_with_dev_deps() {
    // v1 refusal: an archive-backed SOURCE group whose manifest still
    // declares [dev-deps] dirs — those paths never crossed the pack
    // boundary; the group's own-build closure cannot be reproduced
    let (root, bytes, url) = peer_world("devrefuse");
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "app.rut",
            &format!(r#", "deps": {{"pouch": {{"path": "../pouch"}}, "zeta": {{"url": "{url}"}}}}"#),
        ),
    );
    write(
        &app,
        "app.rut",
        "use codec::{encode};\n\nentry fn go() -> str {\n    return encode(\"x\");\n}\n",
    );

    let mut map = BTreeMap::new();
    map.insert(url, bytes);
    let err = rut_driver::pack_dir_fetched(&app, &map).unwrap_err().to_string();
    assert!(
        err.contains("packed dep `codec` needs its own [dev-deps] directories"),
        "{err}"
    );
    assert!(err.contains("vendor this dep"), "{err}");
}

#[test]
fn pack_refuses_unused_compiled_url_dep() {
    // v1 refusal: a declared-but-unused url dep — its compiled root
    // cannot ride (the output's ledger would not carry its pack-time
    // scopes), and the archive keeps no source to re-emit
    let root = leaf_world("unused", "util", UTIL_BODY);
    let bytes = pack_dir(&root.join("util")).expect("pack util");
    let url = "https://fixtures.test/util.rutbundle";
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest("app", "app.rut", &format!(r#", "deps": {{"util": {{"url": "{url}"}}}}"#)),
    );
    write(&app, "app.rut", "entry fn go() -> i64 {\n    return 1;\n}\n");

    let mut map = BTreeMap::new();
    map.insert(url.to_string(), bytes);
    let err = rut_driver::pack_dir_fetched(&app, &map).unwrap_err().to_string();
    assert!(
        err.contains("compiled group `util`"),
        "{err}"
    );
    assert!(err.contains("cannot ride unpackaged"), "{err}");
    assert!(err.contains("vendor this dep"), "{err}");
}
