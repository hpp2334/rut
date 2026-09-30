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
//! 4. **The generic boundary** — the engine's owner-anchored
//!    instantiation law, pinned from the CDN side: a compiled std
//!    bundle serves exactly the instantiations its own pack closure
//!    spelled. A consumer spelling a NEW generic shape (`Vec<i64>`
//!    from a bundle-mounted pouch) refuses loudly — consumer-spelled
//!    generic shapes are the DIRECTORY lane (mount `rut/pouch`), which
//!    is why the examples keep their generic owners on path rows and
//!    take from the CDN what compiled bundles serve: host surfaces
//!    (v6) and concrete-class libs (http, ink, strbuild).
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
        let bundle = rut_bundle::Bundle::parse(&bytes).unwrap();
        let m = rut_bundle::parse_manifest(&bundle.read("rut.toml").unwrap()).unwrap();
        let is_host = m.pkg_type == rut_bundle::PkgType::Host;
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
fn a_compiled_generic_owner_refuses_consumer_spelled_shapes() {
    // THE GENERIC BOUNDARY, pinned from the CDN side: a compiled
    // pouch serves exactly its own pack closure's instantiations — a
    // consumer spelling `Vec<i64>` (prim or own-typed, alike) refuses
    // loudly, naming the owner and the fix. Consumer-spelled generic
    // shapes are the DIRECTORY lane; the CDN lane serves host surfaces
    // and concrete-class libs.
    let (url, bytes) = artifact("pouch");
    let base = std::env::temp_dir().join(format!("rut-std-cdn-wall-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let mut table = BTreeMap::new();
    table.insert(url.clone(), bytes.clone());
    let diags = load_and_compile(
        &base,
        &format!(
            "pouch = {{ url = \"{url}\", sha256 = \"{}\" }}",
            sha256_hex(&bytes)
        ),
        table,
        "use pouch::{ Vec };\n\nentry fn main() -> i32 {\n    let items: Vec<i64> = Vec<i64>.new();\n    items.push(7);\n    return items.len() as i32;\n}\n",
    )
    .expect("the wall world loads");
    let all = diags.join("; ");
    assert!(
        all.contains("was not compiled into `pouch`'s binary"),
        "{all}"
    );
    assert!(all.contains("re-pack with the consumer in the closure"), "{all}");
    let _ = std::fs::remove_dir_all(&base);
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
