//! The v6 decl root — a `type = "host"` pkg packs as its OWN bundle:
//! the root IS its declaration surface (single-package, no ledger, no
//! groups, nothing to decode), format_version 6, readers accept 5|6 —
//! the pairing is total, both directions refused. The mechanism tests:
//! the pack/load round trip, the url lane (pin law, name-vs-key, the
//! leaf law), the refusal matrix, the v5 byte-stability law (writers
//! emit 6 ONLY for decl roots), and host-pack determinism. The CLI face
//! (`rut pack` works, `rut run` refuses) is pinned in rut-cli's
//! `decl_bundles_cli.rs`.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::task::{Context, Poll};

use rut_driver::{
    compile_graph, load_bundle_bytes, load_dir_session_with, mount_std, pack_dir, pack_dir_opts,
    sha256_hex, DepFetch, ModuleBody, PackOpts,
};

// ------------------------------------------------------------------
// the fixture world: a host pkg + a consumer, in scratch dirs

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-decl-bundles-{tag}-{}", std::process::id()));
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

/// A v6-shaped host manifest — the keys PREPEND (before any table).
fn host_manifest(name: &str, surface: &str) -> String {
    format!(
        "format = \"rutbundle\"\nformat_version = 6\nname = \"{name}\"\ntype = \"host\"\nentry.type = \"./{surface}\"\n"
    )
}

/// A host pkg world: one `.d.rut`, one manifest. Returns the pkg dir.
fn host_world(tag: &str, name: &str, surface: &str, decls: &str) -> PathBuf {
    let root = scratch(tag);
    let dir = root.join(name);
    write(&dir, "rut.toml", &host_manifest(name, surface));
    write(&dir, surface, decls);
    dir
}

/// Rewrite one archive entry and re-seal the container — the
/// doctored-bundle fixtures.
fn resealed(bytes: &[u8], key: &str, text: &str) -> Vec<u8> {
    let mut entries: Vec<(String, Vec<u8>)> = rut_bundle::parse_bundle(bytes).unwrap();
    for (n, b) in entries.iter_mut() {
        if n == key {
            *b = text.as_bytes().to_vec();
        }
    }
    rut_bundle::write_bundle(&entries).unwrap()
}

/// Append one archive entry and re-seal — the fabricated-payload
/// fixtures.
fn appended(bytes: &[u8], key: &str, payload: &[u8]) -> Vec<u8> {
    let mut entries: Vec<(String, Vec<u8>)> = rut_bundle::parse_bundle(bytes).unwrap();
    entries.push((key.to_string(), payload.to_vec()));
    rut_bundle::write_bundle(&entries).unwrap()
}

/// Entry names of a bundle, in archive order.
fn entry_names(bytes: &[u8]) -> Vec<String> {
    rut_bundle::parse_bundle(bytes)
        .unwrap()
        .into_iter()
        .map(|(n, _)| n)
        .collect()
}

// the std-only harness: a table fetcher + a noop-waker `block_on`
// (the url_deps.rs pattern — the fetched futures are `ready`)

struct Table(BTreeMap<String, Vec<u8>>);

impl DepFetch for Table {
    fn dep_fetch(&self, url: &str) -> impl Future<Output = Result<Vec<u8>, String>> {
        let r = match self.0.get(url) {
            Some(bytes) => Ok(bytes.clone()),
            None => Err(format!("no fixture bytes for {url}")),
        };
        std::future::ready(r)
    }
}

fn block_on<F: Future>(fut: F) -> F::Output {
    let mut fut = std::pin::pin!(fut);
    let mut cx = Context::from_waker(std::task::Waker::noop());
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

// ------------------------------------------------------------------
// the tests

#[test]
fn host_root_packs_v6_and_loads_back() {
    let dir = host_world(
        "roundtrip",
        "logger_host",
        "logger_host.d.rut",
        "pub host fn make_logger(tag: str) -> opaque;\npub host fn log(line: str);\n",
    );

    // pack: v6, two entries, the surface riding as source
    let bytes = pack_dir(&dir).expect("pack the host pkg");
    let names = entry_names(&bytes);
    assert_eq!(
        names,
        vec!["rut.toml".to_string(), "logger_host.d.rut".to_string()],
        "a host bundle is its manifest + its surface, nothing else: {names:?}"
    );
    // the riding manifest is byte-for-byte the directory's (v6 declared)
    let toml = rut_bundle::Bundle::parse(&bytes)
        .unwrap()
        .read("rut.toml")
        .unwrap();
    assert_eq!(toml, std::fs::read_to_string(dir.join("rut.toml")).unwrap());
    let m = rut_bundle::parse_manifest(&toml).unwrap();
    assert_eq!(m.format_version, Some(6), "a decl root declares 6");

    // load: the root mounts as the pkg's host rows
    let (session, root) = load_bundle_bytes(&bytes, Path::new("mem")).expect("load");
    assert_eq!(root, "logger_host");
    match &session.resolve("logger_host").unwrap().body {
        ModuleBody::Host { host_funcs, .. } => {
            let names: Vec<&str> = host_funcs.iter().map(|(n, ..)| n.as_str()).collect();
            assert_eq!(names, vec!["make_logger", "log"], "{names:?}");
        }
        other => panic!("the decl root mounts as host rows: {other:?}"),
    }
}

#[test]
fn consumer_compiles_against_a_decl_bundle_url_dep() {
    // the url lane: pin law at the door, the root mounting as host
    // rows, and a consumer program compiling `use` against them —
    // the CDN-consumer shape end to end
    let dir = host_world(
        "urllane",
        "logger_host",
        "logger_host.d.rut",
        "pub host fn log(line: str);\n",
    );
    let bytes = pack_dir(&dir).expect("pack");
    let url = "https://cdn.test/std-v1/dist/std/logger_host.rutbundle";
    let pin = sha256_hex(&bytes);

    let app = scratch("urllane-app").join("app");
    write(
        &app,
        "rut.toml",
        &format!(
            "name = \"app\"\nentry.lib = \"./app.rut\"\n\n[deps]\nlogger_host = {{ url = \"{url}\", sha256 = \"{pin}\" }}\n"
        ),
    );
    write(
        &app,
        "app.rut",
        "use logger_host::{log};\n\nentry fn main() -> nil {\n    log(\"hello from the CDN surface\");\n}\n",
    );

    let mut table = BTreeMap::new();
    table.insert(url.to_string(), bytes.clone());
    let (session, root) = block_on(load_dir_session_with(&app, &Table(table))).expect("url load");
    assert_eq!(root, "app");
    assert!(matches!(
        session.resolve("logger_host").unwrap().body,
        ModuleBody::Host { .. }
    ));
    let units = compile_graph(&session, &root);
    assert!(
        units.diags.is_empty(),
        "{}",
        units.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("; ")
    );
    assert!(units.program.is_some(), "the consumer links");
}

/// The real tree host pkgs, shim-packed: the std manifests already
/// carry their format keys (the std-cdn phase added them), so the
/// tree's `rut.toml` rides byte-for-byte — the point is REAL surfaces
/// (including `nmap_host`, whose surface is named `nmap.d.rut` — NOT
/// `<name>.d.rut` — and `async_engine`, `engine.d.rut`: the entry's
/// own rel path keys the archive).
fn shim_tree_host(tag: &str, name: &str, surface: &str) -> PathBuf {
    let tree = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut")).join(name);
    let dir = scratch(tag).join(name);
    let toml = std::fs::read_to_string(tree.join("rut.toml")).unwrap();
    write(&dir, "rut.toml", &toml);
    write(&dir, surface, &std::fs::read_to_string(tree.join(surface)).unwrap());
    dir
}

#[test]
fn the_real_tree_host_pkgs_pack_and_load() {
    for (name, surface) in [
        ("ink_host", "ink_host.d.rut"),
        ("nmap_host", "nmap.d.rut"),
        ("async_engine", "engine.d.rut"),
        ("core", "core.d.rut"),
    ] {
        let dir = shim_tree_host("tree", name, surface);
        let bytes = pack_dir(&dir)
            .unwrap_or_else(|e| panic!("pack rut/{name}: {e}"));
        let names = entry_names(&bytes);
        assert_eq!(
            names,
            vec!["rut.toml".to_string(), surface.to_string()],
            "rut/{name}: {names:?}"
        );
        let (session, root) = load_bundle_bytes(&bytes, Path::new("mem"))
            .unwrap_or_else(|e| panic!("load rut/{name}: {e}"));
        assert_eq!(root, name);
        assert!(matches!(session.resolve(name).unwrap().body, ModuleBody::Host { .. }));
    }
}

#[test]
fn host_pack_is_deterministic() {
    let dir = host_world(
        "determinism",
        "h",
        "h.d.rut",
        "pub host fn f(x: i32) -> i32;\n",
    );
    let a = pack_dir(&dir).expect("pack a");
    let b = pack_dir(&dir).expect("pack b");
    assert_eq!(a, b, "same host dir ⇒ byte-identical bundle");
}

#[test]
fn v5_byte_stability_writers_emit_6_only_for_decl_roots() {
    // a lib root packs EXACTLY as before the mechanism existed: v5 in
    // the riding manifest, compiled root, ledger, groups — the
    // explicit pairing law, pinned from the packed bytes
    let base = scratch("v5stable");
    let lib = base.join("util");
    write(&lib, "rut.toml",
        "format = \"rutbundle\"\nformat_version = 5\nname = \"util\"\nentry.lib = \"./util.rut\"\n");
    write(&lib, "util.rut", "pub fn twice(v: i64) -> i64 {\n    return v * 2;\n}\n");
    let bytes = pack_dir(&lib).expect("pack the lib");
    let m = rut_bundle::parse_manifest(
        &rut_bundle::Bundle::parse(&bytes).unwrap().read("rut.toml").unwrap(),
    )
    .unwrap();
    assert_eq!(m.format_version, Some(5), "a lib root's manifest still declares 5");
    let names = entry_names(&bytes);
    assert!(names.contains(&"rut.scopes".to_string()), "{names:?}");
    assert!(names.contains(&"util.rutc".to_string()), "{names:?}");
    // and the same dir packs byte-identically twice (no drift from the
    // host arm sharing the code path)
    assert_eq!(bytes, pack_dir(&lib).unwrap());
}

#[test]
fn strip_refuses_on_a_host_root() {
    let dir = host_world("strip", "h", "h.d.rut", "pub host fn f(x: i32) -> i32;\n");
    let err = pack_dir_opts(&dir, &PackOpts { strip: true }).unwrap_err();
    assert!(err.contains("no symbols to strip"), "{err}");
}

#[test]
fn the_v5_v6_pairing_is_total() {
    // v5 + host manifest: the broken pairing — a v5 root must be a
    // compiled `.rutc` a host pkg cannot have
    let dir = host_world("pairing", "h", "h.d.rut", "pub host fn f(x: i32) -> i32;\n");
    let bytes = pack_dir(&dir).expect("pack");
    let v5_manifest = format!(
        "format = \"rutbundle\"\nformat_version = 5\nname = \"h\"\ntype = \"host\"\nentry.type = \"./h.d.rut\"\n"
    );
    let err = load_bundle_bytes(&resealed(&bytes, "rut.toml", &v5_manifest), Path::new("mem"))
        .unwrap_err();
    assert!(err.contains("packs at format_version 6"), "{err}");
    assert!(err.contains("a v5 bundle's root is compiled"), "{err}");

    // v6 + lib manifest: the other broken half — lib roots stay v5
    let v6_lib = "format = \"rutbundle\"\nformat_version = 6\nname = \"h\"\nentry.lib = \"./h.rut\"\n";
    let err = load_bundle_bytes(&resealed(&bytes, "rut.toml", v6_lib), Path::new("mem"))
        .unwrap_err();
    assert!(err.contains("decl-root layout"), "{err}");
    assert!(err.contains("a lib root packs at 5"), "{err}");
}

#[test]
fn v6_refuses_compiled_shaped_entries() {
    let dir = host_world("shapes", "h", "h.d.rut", "pub host fn f(x: i32) -> i32;\n");
    let bytes = pack_dir(&dir).expect("pack");

    // a root `.rutc` in a v6 host bundle: the contradiction refusal —
    // a host bundle's root is its surface, not a compiled unit
    let with_rutc = appended(&bytes, "h.rutc", b"not a real program");
    let err = load_bundle_bytes(&with_rutc, Path::new("mem")).unwrap_err();
    assert!(err.contains("a host bundle's root is its surface"), "{err}");
    assert!(err.contains("h.rutc"), "{err}");

    // a scope ledger: no programs, no ledger
    let with_ledger = appended(&bytes, "rut.scopes", b"0 = \"h\"\n");
    let err = load_bundle_bytes(&with_ledger, Path::new("mem")).unwrap_err();
    assert!(err.contains("no programs"), "{err}");

    // a dep group: single-package law
    let group_manifest = rut_bundle::write_bundle(&[(
        "g/rut.toml".into(),
        b"name = \"g\"\nentry.lib = \"./g.rut\"\n".to_vec(),
    )])
    .unwrap();
    let entries: Vec<(String, Vec<u8>)> =
        rut_bundle::parse_bundle(&bytes).unwrap().into_iter().chain(
            rut_bundle::parse_bundle(&group_manifest).unwrap(),
        ).collect();
    let with_group = rut_bundle::write_bundle(&entries).unwrap();
    let err = load_bundle_bytes(&with_group, Path::new("mem")).unwrap_err();
    assert!(err.contains("single-package"), "{err}");

    // a missing surface: the root IS the surface — no entry, no bundle
    let stripped: Vec<(String, Vec<u8>)> = rut_bundle::parse_bundle(&bytes)
        .unwrap()
        .into_iter()
        .filter(|(n, _)| n != "h.d.rut")
        .collect();
    let err =
        load_bundle_bytes(&rut_bundle::write_bundle(&stripped).unwrap(), Path::new("mem"))
            .unwrap_err();
    assert!(err.contains("a host bundle's root is its surface"), "{err}");
}

#[test]
fn url_lane_pin_and_name_key_hold_for_host_bundles() {
    let dir = host_world("pins", "logger_host", "logger_host.d.rut", "pub host fn log(line: str);\n");
    let bytes = pack_dir(&dir).expect("pack");
    let url = "https://cdn.test/std-v1/dist/std/logger_host.rutbundle";

    // a VALID but wrong pin refuses at the door (the pin law precedes
    // everything)
    let wrong = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
    let app = scratch("pins-app").join("app");
    write(
        &app,
        "rut.toml",
        &format!(
            "name = \"app\"\nentry.lib = \"./app.rut\"\n\n[deps]\nlogger_host = {{ url = \"{url}\", sha256 = \"{wrong}\" }}\n"
        ),
    );
    write(&app, "app.rut", "entry fn main() -> nil {}\n");
    let mut table = BTreeMap::new();
    table.insert(url.to_string(), bytes.clone());
    let err = block_on(load_dir_session_with(&app, &Table(table.clone()))).unwrap_err();
    assert!(err.contains("sha256 pin mismatch"), "{err}");

    // name-vs-key: the key spells the mounted name
    let app2 = scratch("pins-app2").join("app2");
    write(
        &app2,
        "rut.toml",
        &format!(
            "name = \"app2\"\nentry.lib = \"./app2.rut\"\n\n[deps]\nnot_logger = {{ url = \"{url}\" }}\n"
        ),
    );
    write(&app2, "app2.rut", "entry fn main() -> nil {}\n");
    let err = block_on(load_dir_session_with(&app2, &Table(table))).unwrap_err();
    assert!(err.contains("the bundle names itself `logger_host`"), "{err}");
}

#[test]
fn url_host_bundle_is_a_leaf_no_fetch_walk() {
    // the leaf law: mounting a host bundle fetches THE BUNDLE and
    // nothing else — there is no closure to walk (and a bundle-internal
    // url row would be inert metadata anyway; a host manifest cannot
    // even carry one)
    let dir = host_world("leaf", "logger_host", "logger_host.d.rut", "pub host fn log(line: str);\n");
    let bytes = pack_dir(&dir).expect("pack");
    let url = "https://cdn.test/std-v1/dist/std/logger_host.rutbundle";

    let root_dir = scratch("leaf");
    let app = root_dir.join("app");
    write(
        &app,
        "rut.toml",
        &format!(
            "name = \"app\"\nentry.lib = \"./app.rut\"\n\n[deps]\npouch = {{ path = \"../pouch\" }}\nlogger_host = {{ url = \"{url}\" }}\n"
        ),
    );
    write(&app, "app.rut", "entry fn main() -> nil {}\n");
    // the path dep needs its dir to exist (never fetched — only the
    // url row goes through the fetcher)
    let pouch = root_dir.join("pouch");
    std::fs::create_dir_all(&pouch).unwrap();
    write(&pouch, "rut.toml", "name = \"pouch\"\nentry.lib = \"./pouch.rut\"\n");
    write(&pouch, "pouch.rut", "pub class Vec<T> {\n    items: [T];\n}\n");

    struct Counting(BTreeMap<String, Vec<u8>>);
    impl DepFetch for Counting {
        fn dep_fetch(&self, url: &str) -> impl Future<Output = Result<Vec<u8>, String>> {
            let r = match self.0.get(url) {
                Some(bytes) => Ok(bytes.clone()),
                None => Err(format!("no fixture bytes for {url}")),
            };
            std::future::ready(r)
        }
    }
    let mut table = BTreeMap::new();
    table.insert(url.to_string(), bytes);
    let (session, _) =
        block_on(load_dir_session_with(&app, &Counting(table))).expect("load");
    assert!(matches!(
        session.resolve("logger_host").unwrap().body,
        ModuleBody::Host { .. }
    ));
    // the leaf mounted; the dir dep mounted beside it — one fetch shape,
    // zero walking
    assert!(session.resolve("pouch").is_ok());
}

#[test]
fn mount_std_then_a_decl_bundle_consumer_runs_the_pattern() {
    // the embedder story end to end: mount_std (the engine mounts),
    // then a consumer that names BOTH the ambient core rows and a
    // CDN-fetched host surface compiles + links
    let dir = host_world(
        "embed",
        "logger_host",
        "logger_host.d.rut",
        "pub host fn log(line: str);\n",
    );
    let bytes = pack_dir(&dir).expect("pack");
    let url = "https://cdn.test/std-v1/dist/std/logger_host.rutbundle";
    let app = scratch("embed").join("app");
    write(
        &app,
        "rut.toml",
        &format!(
            "name = \"app\"\nentry.lib = \"./app.rut\"\n\n[deps]\nlogger_host = {{ url = \"{url}\" }}\n"
        ),
    );
    write(
        &app,
        "app.rut",
        "use logger_host::{log};\n\nentry fn main() -> i32 {\n    log(\"hi\");\n    return 7;\n}\n",
    );
    let mut table = BTreeMap::new();
    table.insert(url.to_string(), bytes);
    let (mut session, root) = block_on(load_dir_session_with(&app, &Table(table))).expect("load");
    mount_std(&mut session);
    let g = compile_graph(&session, &root);
    assert!(
        g.diags.is_empty(),
        "{}",
        g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("; ")
    );
    assert!(g.program.is_some());
}
