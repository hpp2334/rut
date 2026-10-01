//! The std-CDN laws over the COMMITTED artifacts (`dist/std/`):
//!
//! 1. **Freshness** — `pack_dir(rut/<pkg>)` is byte-identical to the
//!    committed bundle, for all 15 (the Q4 determinism law turned into
//!    a pure equality gate; CI never touches the network).
//! 2. **The version pairing** — lib roots at 5 (compiled), host roots
//!    at 6 (decl), read from the committed bytes.
//! 3. **The duplicate-mount law** — two archives in one session:
//!    first-mount-wins, and the scope ledgers namespace per archive
//!    (independently packed bundles never argue about a number).
//! 4. **The generic law, flipped** — the engine's owner-anchored
//!    instantiation law over the CDN: a compiled std bundle whose pkg
//!    RIDES its generic source serves consumer-spelled shapes at the
//!    consumer's link (`Vec<i64>` from a bundle-mounted pouch
//!    compiles and runs; json's peer groups instantiate the consumer's
//!    `T`). A LEGACY bundle — the riding absent — refuses loudly
//!    (re-pack it). Determinism: same bundle + same consumer ⇒
//!    byte-identical output.
//! 5. **The concrete-lib law** — a compiled concrete-class lib (the
//!    string builder) serves its consumers from the bundle: the
//!    classes' methods cross on the surface's inherent rows, no
//!    instantiation required.
//! 6. **The ambient law** — mount_std + a url-mounted core coexist
//!    (first-mount-wins).

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::task::{Context, Poll};

use rut_driver::{
    compile_graph, load_dir_session_with, mount_std, sha256_hex, DepFetch, ModuleBody,
};

fn dist_std() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../dist/std"))
}

/// The 15 pkgs: (tree dir, artifact name). Keep in sync with
/// scripts/pack-std.cjs's PKGS table.
const PKGS: &[(&str, &str)] = &[
    ("pouch", "pouch"),
    ("nmapset", "nmapset"),
    ("strbuild", "strbuild"),
    ("json", "json"),
    ("ink", "ink"),
    ("http", "http"),
    ("async_host", "async_host"),
    ("ink_host", "ink_host"),
    ("http_host", "http_host"),
    ("nmap_host", "nmap_host"),
    ("strbuild_host", "strbuild_host"),
    ("async_engine", "async_engine"),
    ("core", "core"),
    ("calc", "calc"),
    ("bench-cross", "bench_cross"),
];

// the std-only harness (the url_deps.rs pattern)
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

/// One artifact's bytes + its jsDelivr url, keyed by the DEPS KEY a
/// consumer row spells (the manifest name).
fn artifact(key: &str) -> (String, Vec<u8>) {
    let bytes = std::fs::read(dist_std().join(format!("{key}.rutbundle")))
        .unwrap_or_else(|e| panic!("dist/std/{key}.rutbundle: {e}"));
    let url = format!("https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v1/dist/std/{key}.rutbundle");
    (url, bytes)
}

/// Write a consumer app over the given dep rows + source, load it
/// through the fetcher, mount_std, compile — the CDN-consumer lane end
/// to end. Answers the graph diags (empty = compiled).
fn load_and_compile(dir: &Path, rows: &str, table: BTreeMap<String, Vec<u8>>, src: &str) -> Result<Vec<String>, String> {
    let app = dir.join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(
        app.join("rut.toml"),
        format!("name = \"app\"\nentry.lib = \"./app.rut\"\n\n[deps]\n{rows}"),
    )
    .unwrap();
    std::fs::write(app.join("app.rut"), src).unwrap();
    let (mut session, root) = block_on(load_dir_session_with(&app, &Table(table)))?;
    mount_std(&mut session);
    let g = compile_graph(&session, &root);
    Ok(g.diags.iter().map(|d| d.msg.clone()).collect())
}

#[test]
fn every_committed_artifact_is_byte_fresh() {
    for (dir, name) in PKGS {
        let tree = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut")).join(dir);
        let fresh = rut_driver::pack_dir(&tree)
            .unwrap_or_else(|e| panic!("pack rut/{dir}: {e}"));
        let committed = std::fs::read(dist_std().join(format!("{name}.rutbundle")))
            .unwrap_or_else(|e| panic!("dist/std/{name}.rutbundle: {e}"));
        assert_eq!(
            fresh, committed,
            "rut/{dir} repacks differently from the committed dist/std/{name}.rutbundle — \
             run `node scripts/pack-std.cjs && node scripts/pack-std.cjs --pins`"
        );
    }
}

#[test]
fn the_artifact_set_spells_the_version_pairing() {
    // lib roots at 5 (compiled), host roots at 6 (decl) — read from the
    // committed bytes, the exact pairing the reader refuses to break
    for (dir, name) in PKGS {
        let bytes = std::fs::read(dist_std().join(format!("{name}.rutbundle"))).unwrap();
        let bundle = rut_driver::bundle::Bundle::parse(&bytes).unwrap();
        let m = rut_driver::bundle::parse_manifest(&bundle.read("rut.toml").unwrap()).unwrap();
        let is_host = m.pkg_type == rut_driver::bundle::PkgType::Host;
        assert_eq!(
            m.format_version,
            Some(if is_host { 6 } else { 5 }),
            "rut/{dir}: the riding manifest's version must match its kind"
        );
        if is_host {
            // single-package: manifest + surface, nothing else
            let names: Vec<&str> = bundle.entries().iter().map(|(n, _)| n.as_str()).collect();
            assert_eq!(names.len(), 2, "rut/{name}: {names:?}");
        } else {
            assert!(
                bundle.bytes("rut.scopes").is_some(),
                "rut/{name}: a compiled root carries the ledger"
            );
        }
    }
}

#[test]
fn the_six_pin_set_loads_and_duplicates_first_mount_wins() {
    // 06-github-viewer-cli's pin set, loaded over the committed
    // artifacts: two archives (json's and http's) carry the SAME
    // strbuild (+ strbuild_host) groups — first-mount-wins mounts one
    // body, and the per-archive ledger namespacing means neither
    // archive argues about a scope number. The consumer binds json's
    // Writer class + trait from the compiled binary over a LOCAL impl
    // (the concrete surface law — no owner-anchored instantiation).
    let mut table = BTreeMap::new();
    let mut rows = String::new();
    for key in ["async_host", "http", "json", "nmapset", "pouch"] {
        let (url, bytes) = artifact(key);
        rows.push_str(&format!(
            "{key} = {{ url = \"{url}\", sha256 = \"{}\" }}\n",
            sha256_hex(&bytes)
        ));
        table.insert(url, bytes);
    }
    let base = std::env::temp_dir().join(format!("rut-std-cdn-six-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let diags = load_and_compile(
        &base,
        &rows,
        table,
        // json's compiled rows: the Writer class binds from the binary;
        // the trait registers; the impl is THIS app's (local dispatch)
        "use json::{ EncodeJsonError, JsonSerialize, JsonWriter };\n\nclass Item { name: str; n: i64; }\n\nimpl JsonSerialize for Item {\n    fn encode(self, mut w: JsonWriter) -> ?EncodeJsonError {\n        w.begin_object();\n        w.key(\"name\");\n        w.write_str(self.name);\n        w.end_object();\n        return nil;\n    }\n}\n\nentry fn main() -> str {\n    let mut w = JsonWriter.new();\n    let it = Item { name: \"a\", n: 1 };\n    let e = it.encode(w);\n    if (e != nil) { return \"err\"; }\n    return w.finish();\n}\n",
    )
    .expect("the six-pin world loads");
    assert!(diags.is_empty(), "{}", diags.join("; "));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_legacy_bundle_without_ridden_source_refuses_consumer_shapes() {
    // THE GENERIC BOUNDARY, the legacy arm: a bundle that predates
    // generic-source riding carries no source beside its binary — a
    // consumer spelling a NEW shape (`Vec<i64>` from a pouch packed
    // without the riding) refuses loudly, naming the owner and the fix.
    // The fixture is the honest legacy artifact: the fresh pack, minus
    // its riding source entries, re-written into an archive.
    let (url, bytes) = artifact("pouch");
    let parsed = rut_driver::bundle::parse_bundle(&bytes).expect("parse the committed pouch");
    assert!(
        parsed.iter().any(|(n, _)| n == "pouch.rut"),
        "the fresh pack rides its source (the riding law moved; this fixture strips it)"
    );
    let legacy: Vec<(String, Vec<u8>)> = parsed
        .into_iter()
        .filter(|(n, _)| !(n.ends_with(".rut") && n != "rut.toml"))
        .collect();
    let legacy_bytes =
        rut_driver::bundle::write_bundle(&legacy).expect("re-write the legacy archive");
    let base = std::env::temp_dir().join(format!("rut-std-cdn-wall-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let mut table = BTreeMap::new();
    table.insert(url.clone(), legacy_bytes.clone());
    let diags = load_and_compile(
        &base,
        &format!(
            "pouch = {{ url = \"{url}\", sha256 = \"{}\" }}",
            sha256_hex(&legacy_bytes)
        ),
        table,
        "use pouch::{ Vec };\n\nentry fn main() -> i32 {\n    let items: Vec<i64> = Vec<i64>.new();\n    items.push(7);\n    return items.len() as i32;\n}\n",
    )
    .expect("the legacy world loads");
    let all = diags.join("; ");
    assert!(
        all.contains("was not compiled into `pouch`'s binary"),
        "{all}"
    );
    assert!(all.contains("no generic source rides"), "{all}");
    assert!(all.contains("predates generic-source riding"), "{all}");
    let _ = std::fs::remove_dir_all(&base);
}

/// One consumer world over the committed CDN artifacts: rows + source →
/// (diags, linked program, the session). Empty diags ⇒ compiled.
fn load_compile_run(
    dir: &Path,
    rows: &str,
    table: BTreeMap<String, Vec<u8>>,
    src: &str,
) -> Result<(Vec<String>, Option<rut_core::binary::Program>, rut_driver::Session), String> {
    let app = dir.join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(
        app.join("rut.toml"),
        format!("name = \"app\"\nentry.lib = \"./app.rut\"\n\n[deps]\n{rows}"),
    )
    .unwrap();
    std::fs::write(app.join("app.rut"), src).unwrap();
    let (mut session, root) = block_on(load_dir_session_with(&app, &Table(table)))?;
    mount_std(&mut session);
    let g = compile_graph(&session, &root);
    let diags: Vec<String> = g.diags.iter().map(|d| d.msg.clone()).collect();
    Ok((diags, g.program, session))
}

#[test]
fn a_consumer_spelled_shape_compiles_from_a_bundle_mounted_pouch() {
    // THE FLIPPED LAW: a compiled pouch whose source rides serves
    // consumer-spelled shapes at the consumer's LINK — the ridden text
    // lowers in the consumer's session, the monomorphized body
    // compiles with owner = the pkg's spec, and the linked program
    // runs. Nothing persists: the `.rutc` bytes stay pack-time.
    let (url, bytes) = artifact("pouch");
    let base = std::env::temp_dir().join(format!("rut-std-cdn-vec-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let mut table = BTreeMap::new();
    table.insert(url.clone(), bytes.clone());
    let (diags, program, _session) = load_compile_run(
        &base,
        &format!(
            "pouch = {{ url = \"{url}\", sha256 = \"{}\" }}",
            sha256_hex(&bytes)
        ),
        table,
        "use pouch::{ Vec };\n\nentry fn main() -> i32 {\n    let items: Vec<i64> = Vec<i64>.new();\n    items.push(7);\n    items.push(9);\n    return items.len() as i32;\n}\n",
    )
    .expect("the CDN-pouch world loads");
    assert!(diags.is_empty(), "{}", diags.join("; "));
    let prog = rut_core::link::flatten(program.expect("linked"));
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(prog),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    let out: i32 = vm.call("main", ()).expect("run");
    assert_eq!(out, 2, "the load-time-compiled body runs");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_json_peer_shape_compiles_with_the_consumer_type() {
    // json's peer groups inherit the riding law: `impl JsonSerialize
    // for Vec<T>` is itself generic — the group impl file rides, and
    // the consumer's `T` instantiates at the link. The consumer type
    // impls BOTH json traits (the container impl set is fixed; the
    // vtable fills demand both), and the round trip runs through
    // json's own generic entries — whose bodies are `f`-kind requests
    // the recompiled owner also serves.
    let mut table = BTreeMap::new();
    let mut rows = String::new();
    for key in ["json", "pouch", "nmapset"] {
        let (url, bytes) = artifact(key);
        rows.push_str(&format!(
            "{key} = {{ url = \"{url}\", sha256 = \"{}\" }}\n",
            sha256_hex(&bytes)
        ));
        table.insert(url, bytes);
    }
    let base = std::env::temp_dir().join(format!("rut-std-cdn-json-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let src = r#"use json::{ JsonSerialize, JsonDeserialize, JsonWriter, JsonReader,
             EncodeJsonError, DecodeJsonError, encodeJson, decodeJson };
use pouch::{ Vec };

class Todo { name: str; n: i64; }

impl JsonSerialize for Todo {
    fn encode(self, mut w: JsonWriter) -> ?EncodeJsonError {
        w.begin_object();
        w.key("name");
        w.write_str(self.name);
        w.key("n");
        w.write_i64(self.n);
        w.end_object();
        return nil;
    }
}

impl JsonDeserialize for Todo {
    fn decode(mut r: JsonReader) -> (?Self, ?DecodeJsonError) {
        r.begin_object();
        let mut name: ?str = nil;
        let mut n: ?i64 = nil;
        while (r.more()) {
            let k = r.next_key();
            when (k) {
                "name" -> { name = r.read_str(); },
                "n" -> { n = r.read_i64(); },
                else -> { r.skip_value(); },
            }
            if (r.failed()) { return (nil, r.error()); }
        }
        r.end_object();
        if (r.failed()) { return (nil, r.error()); }
        return (Todo { name: name, n: n }, nil);
    }
}

entry fn main() -> i32 {
    let mut v: Vec<Todo> = Vec<Todo>.new();
    v.push(Todo { name: "write the book", n: 1 });
    v.push(Todo { name: "ship the CDN", n: 2 });
    let (text, e) = encodeJson(v);
    if (e != nil) { return -1; }
    let (back, e2) = decodeJson<Vec<Todo>>(text);
    if (e2 != nil) { return -2; }
    return back.len() as i32;
}
"#;
    let (diags, program, session) = load_compile_run(&base, &rows, table, src)
        .expect("the json peer world loads");
    assert!(diags.is_empty(), "{}", diags.join("; "));
    let prog = rut_core::link::flatten(program.expect("linked"));
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(10_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    // the writer's accumulator rides strbuild_host (json's closure
    // carries it) — the embedder half binds its bodies
    let ctx = session.host_pkg_context();
    let mut hosts = rut_vm::interp::HostRegistry::new();
    hosts.install_host_pkg(&ctx, rut_std::strbuild::pkg());
    hosts.install_host_pkg(&ctx, rut_std::math::pkg());
    hosts.install_host_pkg(&ctx, rut_std::nmap::pkg());
    hosts.verify_against(&session.expected_host_fns());
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(prog),
        &limits,
        rut_vm::interp::HostHooks::default(),
        hosts,
    )
    .expect("vm");
    let out: i32 = vm.call("main", ()).expect("run");
    assert_eq!(out, 2, "the consumer type round-trips through json's container rows");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn mixed_dir_and_archive_of_one_pkg_first_mount_wins() {
    // the same pkg offered by BOTH lanes: json's url row mounts first
    // (name order), its archive's `strbuild` group wins, and the app's
    // own `strbuild` path row is skipped — one body, the compiled one
    // — and the program runs over the winner
    let mut table = BTreeMap::new();
    let mut rows = String::new();
    for key in ["json", "pouch", "nmapset"] {
        let (url, bytes) = artifact(key);
        rows.push_str(&format!(
            "{key} = {{ url = \"{url}\", sha256 = \"{}\" }}\n",
            sha256_hex(&bytes)
        ));
        table.insert(url, bytes);
    }
    let tree = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut")).join("strbuild");
    rows.push_str(&format!("strbuild = {{ path = \"{}\" }}\n", tree.display()));
    let base = std::env::temp_dir().join(format!("rut-std-cdn-mixed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (diags, program, session) = load_compile_run(
        &base,
        &rows,
        table,
        "use strbuild::{ StringBuilder };\n\nentry fn main() -> str {\n    let mut b = StringBuilder.new();\n    b.append(\"mixed\");\n    return b.build();\n}\n",
    )
    .expect("the mixed world loads");
    assert!(diags.is_empty(), "{}", diags.join("; "));
    // the ARCHIVE group won (json mounts before the strbuild row) —
    // the mounted body is the decoded binary, not the dir source
    assert!(
        matches!(session.resolve("strbuild").unwrap().body, ModuleBody::Compiled(_)),
        "first-mount-wins keeps the archive's compiled group"
    );
    let prog = rut_core::link::flatten(program.expect("linked"));
    rut_vm::verify::verify(&prog).expect("verify");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn the_load_time_compile_is_deterministic() {
    // same bundle + same consumer ⇒ byte-identical output: the
    // on-demand compilation is the deterministic compiler over the same
    // inputs — no clocks, no iteration-order hazards
    let (url, bytes) = artifact("pouch");
    let src = "use pouch::{ Vec };\n\nentry fn main() -> i32 {\n    let items: Vec<i64> = Vec<i64>.new();\n    items.push(7);\n    return items.len() as i32;\n}\n";
    let mut outs: Vec<Vec<u8>> = Vec::new();
    for run in 0..2 {
        let base =
            std::env::temp_dir().join(format!("rut-std-cdn-det-{}-{run}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mut table = BTreeMap::new();
        table.insert(url.clone(), bytes.clone());
        let (diags, program, _session) = load_compile_run(
            &base,
            &format!(
                "pouch = {{ url = \"{url}\", sha256 = \"{}\" }}",
                sha256_hex(&bytes)
            ),
            table,
            src,
        )
        .expect("the determinism world loads");
        assert!(diags.is_empty(), "{}", diags.join("; "));
        let prog = program.expect("linked");
        outs.push(rut_core::binary::encode(&prog));
        let _ = std::fs::remove_dir_all(&base);
    }
    assert_eq!(outs[0], outs[1], "same bundle + same consumer ⇒ same bytes");
}

#[test]
fn a_concrete_class_lib_serves_from_the_bundle() {
    // strbuild: a compiled lib whose classes are CONCRETE — the
    // methods cross on the surface's inherent rows, the builder's
    // bodies ride the host pkg's rows (bound by the embedder below) —
    // the exact shape the CDN lane delivers for lib pkgs
    let (url, bytes) = artifact("strbuild");
    let base = std::env::temp_dir().join(format!("rut-std-cdn-sb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let mut table = BTreeMap::new();
    table.insert(url.clone(), bytes);
    let app = base.join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(
        app.join("rut.toml"),
        format!(
            "name = \"app\"\nentry.lib = \"./app.rut\"\n\n[deps]\nstrbuild = {{ url = \"{url}\", sha256 = \"{}\" }}\n",
            sha256_hex(&std::fs::read(dist_std().join("strbuild.rutbundle")).unwrap())
        ),
    )
    .unwrap();
    std::fs::write(
        app.join("app.rut"),
        "use strbuild::{ StringBuilder };\n\nentry fn main() -> str {\n    let mut b = StringBuilder.new();\n    b.append(\"hello, \");\n    b.append(\"cdn\");\n    return b.build();\n}\n",
    )
    .unwrap();
    let (mut session, root) =
        block_on(load_dir_session_with(&app, &Table(table))).expect("strbuild bundle loads");
    mount_std(&mut session);
    // the builder's host rows demand bodies — the embedder half
    // (math too: mount_std mounted calc's surface)
    let ctx = session.host_pkg_context();
    let mut hosts = rut_vm::interp::HostRegistry::new();
    hosts.install_host_pkg(&ctx, rut_std::math::pkg());
    hosts.install_host_pkg(&ctx, rut_std::strbuild::pkg());
    hosts.verify_against(&session.expected_host_fns());
    let g = compile_graph(&session, &root);
    assert!(
        g.diags.is_empty(),
        "{}",
        g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("; ")
    );
    let prog = g.program.expect("linked");
    rut_vm::verify::verify(&rut_core::link::flatten(prog.clone())).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(std::rc::Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts)
        .expect("vm");
    let out: String = vm.call("main", ()).expect("run");
    assert_eq!(out, "hello, cdn");
    // and strbuild_host rode the archive as the decl-surface group
    assert!(matches!(
        session.resolve("strbuild_host").unwrap().body,
        ModuleBody::Host { .. }
    ));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ambient_std_and_a_url_std_bundle_coexist() {
    // the ambient redundancy note: mount_std already mounts core/calc;
    // pinning core's CDN bundle on top is a HARMLESS doubled mount
    // (first-mount-wins — the same law as any duplicate) and custom
    // hosts that do not ambient-mount get the rows from the bundle
    let (url, bytes) = artifact("core");
    let base = std::env::temp_dir().join(format!("rut-std-cdn-core-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let mut table = BTreeMap::new();
    table.insert(url.clone(), bytes.clone());
    let diags = load_and_compile(
        &base,
        &format!(
            "core = {{ url = \"{url}\", sha256 = \"{}\" }}",
            sha256_hex(&bytes)
        ),
        table,
        "entry fn main() -> i32 {\n    return 7;\n}\n",
    )
    .expect("core loads");
    assert!(diags.is_empty(), "{}", diags.join("; "));
    let _ = std::fs::remove_dir_all(&base);
}
