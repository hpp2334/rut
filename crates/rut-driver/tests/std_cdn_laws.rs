//! The std-CDN laws over the COMMITTED artifacts (`dist/std/`):
//!
//! 1. **Freshness** — `pack_dir(rut/<pkg>)` is byte-identical to the
//!    committed bundle, for all 16 (the Q4 determinism law turned into
//!    a pure equality gate; CI never touches the network).
//! 2. **The one wire number** — every manifest reads `format_version`
//!    10; the manifest's `type` routes the root kind, read from the
//!    committed bytes.
//! 3. **The duplicate-mount law** — two archives in one session:
//!    first-mount-wins, and the scope ledgers namespace per archive
//!    (independently packed bundles never argue about a number).
//! 4. **The generic law, flipped** — the engine's owner-anchored
//!    instantiation law over the CDN: a compiled std bundle whose pkg
//!    RIDES its generic source serves consumer-spelled shapes at the
//!    consumer's link (`Vec<i64>` from a bundle-mounted pouch
//!    compiles and runs). A bundle stripped of its riding source
//!    refuses loudly at mount (re-pack it). Determinism: same bundle +
//!    same consumer ⇒ byte-identical output.
//! 5. **The concrete-lib law** — a compiled concrete-class lib (the
//!    string builder) serves its consumers from the bundle: the
//!    classes' methods cross on the surface's inherent rows, no
//!    instantiation required.
//! 6. **The ambient law** — mount_std + a url-mounted core coexist
//!    (first-mount-wins).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod common;
use common::{block_on, write_lock, Table};

use rut_native::load_dir_with;




/// One source pkg over the auto core — the chain's graph. Closure and
/// shape refusals come back as one span-0 diagnostic.
#[allow(dead_code)] // not every suite in this file needs both lanes
fn compiled(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(rut_driver::RutRun::new().pkg(rut_driver::Pkg::source(spec, src)).entrypoint(spec).compile())
}

/// [`compiled`] with calc offered (the old `mount_std` shape: core
/// auto-rides, `calc` is an ordinary pkg).
#[allow(dead_code)]
fn compiled_std(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(
        rut_driver::RutRun::new()
            // the root DECLARES its calc use (the structural form; the
            // offer stays the walked tree pkg)
            .pkg(
                rut_driver::Pkg::source(spec, src)
                    .manifest(rut_driver::Manifest::lib(spec, &["calc"])),
            )
            .pkg(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"))
            .entrypoint(spec)
            .compile(),
    )
}

#[allow(dead_code)]
fn graph_of(c: Result<rut_driver::Compiled, rut_driver::RunError>) -> rut_driver::GraphOutput {
    match c {
        Ok(c) => c.graph,
        Err(e) => rut_driver::GraphOutput {
            diags: vec![rut_lexer::diag::Diag::new(rut_lexer::span::Span::new(0, 0), e.msg)],
            program: None,
        },
    }
}

fn dist_std() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../dist/std"))
}

/// The 16 pkgs: (tree dir, artifact name). Keep in sync with
/// scripts/pack-std::cjs's PKGS table.
const PKGS: &[(&str, &str)] = &[
    ("pouch", "pouch"),
    ("flow", "flow"),
    ("nmapset", "nmapset"),
    ("strbuild", "strbuild"),
    ("json", "json"),
    ("ink", "ink"),
    ("http", "http"),
    ("futures", "futures"),
    ("ink_host", "ink_host"),
    ("http_host", "http_host"),
    ("nmap_host", "nmap_host"),
    ("strbuild_host", "strbuild_host"),
    ("async_host", "async_host"),
    ("core", "core"),
    ("calc", "calc"),
    ("bench-cross", "bench_cross"),
];

/// One artifact's bytes + its jsDelivr url, keyed by the DEPS KEY a
/// consumer row spells (the manifest name).
fn artifact(key: &str) -> (String, Vec<u8>) {
    let bytes = std::fs::read(dist_std().join(format!("{key}.rutbundle")))
        .unwrap_or_else(|e| panic!("dist/std/{key}.rutbundle: {e}"));
    let url = format!("https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v2/dist/std/{key}.rutbundle");
    (url, bytes)
}

/// Write a consumer app over the given dep rows + source, load it
/// through the fetcher, mount_std, compile — the CDN-consumer lane end
/// to end. Answers the graph diags (empty = compiled).
fn load_and_compile(dir: &Path, rows: &str, table: BTreeMap<String, Vec<u8>>, src: &str) -> Result<Vec<String>, String> {
    let app = dir.join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(
        app.join("rut.jsonc"),
        format!(r#"{{"name": "app", "deps": {{{}}}}}"#, rows.trim_end_matches(|c: char| c == ',' || c.is_whitespace())),
    )
    .unwrap();
    std::fs::write(app.join("mod.rut"), src).unwrap();
    write_lock(&app, &table);
    let loaded = block_on(load_dir_with(&app, &Table::from(table))).map_err(|e| e.to_string())?;
    let g = rut_driver::RutRun::new()
        .pkgs(&loaded)
        .pkg(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"))
        .entrypoint(&loaded.root)
        .compile()
        .map_err(|e| e.to_string())?;
    Ok(g.graph.diags.iter().map(|d| d.msg.clone()).collect())
}

#[test]
fn every_committed_artifact_is_byte_fresh() {
    for (dir, name) in PKGS {
        let tree = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut")).join(dir);
        let fresh = rut_native::pack_dir(&tree)
            .unwrap_or_else(|e| panic!("pack rut/{dir}: {e}"));
        let committed = std::fs::read(dist_std().join(format!("{name}.rutbundle")))
            .unwrap_or_else(|e| panic!("dist/std/{name}.rutbundle: {e}"));
        assert_eq!(
            fresh, committed,
            "rut/{dir} repacks differently from the committed dist/std/{name}.rutbundle — \
             run `node scripts/pack-std::cjs && node scripts/pack-std::cjs --pins`"
        );
    }
}

#[test]
fn the_artifact_set_spells_the_one_wire_number() {
    // every manifest packs format_version 11 — the ONE wire number;
    // the manifest's `type` routes the root kind (read from the
    // committed bytes)
    for (dir, name) in PKGS {
        let bytes = std::fs::read(dist_std().join(format!("{name}.rutbundle"))).unwrap();
        let bundle = rut_driver::bundle::Bundle::parse(&bytes).unwrap();
        let m = rut_driver::bundle::parse_manifest(&bundle.read("rut.jsonc").unwrap()).unwrap();
        let is_host = m.pkg_type == rut_driver::bundle::PkgType::Host;
        assert_eq!(
            m.format_version, Some(11),
            "rut/{dir}: the manifest packs the one wire number"
        );
        if is_host {
            // single-package: manifest + surface, plus the rows envelope
            // when the surface is a mod tree (flat pkgs stay the
            // two-entry shape — the freshness gate proves the flat law;
            // the http_host pilot is the committed tree, its root the
            // default `mod.d.rut` spelling)
            let names: Vec<&str> = bundle.entries().iter().map(|(n, _)| n.as_str()).collect();
            if names.contains(&rut_driver::mods::ROWS_NAME) {
                assert_eq!(names.len(), 3, "rut/{name}: {names:?}");
                assert!(names.contains(&"mod.d.rut"), "rut/{name}: {names:?}");
            } else {
                assert_eq!(names.len(), 2, "rut/{name}: {names:?}");
            }
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
    for key in ["futures", "http", "json", "nmapset", "pouch"] {
        let (url, bytes) = artifact(key);
        rows.push_str(&format!("\"{key}\": {{\"url\": \"{url}\"}},\n"));
        table.insert(url, bytes);
    }
    let base = std::env::temp_dir().join(format!("rut-std-cdn-six-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let diags = load_and_compile(
        &base,
        &rows,
        table,
        // json's compiled rows: the Writer class binds from the binary;
        // the interface crosses on the surface; the satisfaction is THIS
        // app's own members (structural, local dispatch)
        "use json::{ EncodeJsonError, JsonSerialize, JsonWriter };\n\nclass Item { name: str; n: i64; }\n\nimpl Item {\n    pub fn encode(self, mut w: JsonWriter) -> ?EncodeJsonError {\n        w.begin_object();\n        w.key(\"name\");\n        w.write_str(self.name);\n        w.end_object();\n        return nil;\n    }\n}\n\nentry fn main() -> str {\n    let mut w = JsonWriter::new();\n    let it = Item { name: \"a\", n: 1 };\n    let e = it.encode(w);\n    if (e != nil) { return \"err\"; }\n    return w.finish();\n}\n",
    )
    .expect("the six-pin world loads");
    assert!(diags.is_empty(), "{}", diags.join("; "));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_generic_owning_bundle_without_template_bodies_refuses_at_mount() {
    // THE GENERIC BOUNDARY, the mount seam (v22): a compiled pkg whose
    // surface exports generics must carry its template bodies INSIDE
    // the binary — the delivery law has no third lane. The fixture is
    // the honest torn artifact: the fresh pack (binary-only — no source
    // rides), its templates section emptied, re-written into an
    // archive. The mount refuses LOUDLY (the earliest point the decoded
    // program is in hand), naming the fix.
    let (url, bytes) = artifact("pouch");
    let parsed = rut_driver::bundle::parse_bundle(&bytes).expect("parse the committed pouch");
    assert!(
        !parsed.iter().any(|(n, _)| n == "mod.rut"),
        "the fresh pack carries no source (the delivery law: ledger | templates)"
    );
    let torn: Vec<(String, Vec<u8>)> = parsed
        .into_iter()
        .map(|(n, b)| {
            if n == "mod.rutc" {
                let mut prog =
                    rut_core::binary::decode(&b).expect("decode the pouch module binary");
                prog.templates.clear();
                prog.template_stubs.clear();
                (n, rut_core::binary::encode(&prog))
            } else {
                (n, b)
            }
        })
        .collect();
    let torn_bytes =
        rut_driver::bundle::write_bundle(&torn).expect("re-write the torn archive");
    let base = std::env::temp_dir().join(format!("rut-std-cdn-wall-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let mut table = BTreeMap::new();
    table.insert(url.clone(), torn_bytes.clone());
    let err = load_and_compile(
        &base,
        &format!("\"pouch\": {{\"url\": \"{url}\"}}"),
        table,
        "use pouch::{ Vec };\n\nentry fn main() -> i32 {\n    let items: Vec<i64> = Vec<i64>::new();\n    items.push(7);\n    return items.len() as i32;\n}\n",
    )
    .expect_err("the torn bundle refuses at mount");
    assert!(err.contains("`pouch` owns an open generic surface"), "{err}");
    assert!(err.contains("no template bodies"), "{err}");
    assert!(err.contains("re-pack the directory"), "{err}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_doctored_bundle_missing_one_template_row_refuses_loudly() {
    // the anti-doctoring seam's FINE form: a bundle whose templates
    // section merely LOSES one row (a doctored `Vec::push`) decodes and
    // mounts — the binary looks whole — but the consumer-spelled shape
    // the removed row served is answered by neither the pack-time
    // ledger nor the remaining templates, and the link refuses loudly
    // instead of fabricating a bodyless method row (the v22 delivery
    // law: ledger | templates | refusal — never a fabricated `Method
    // { fid: 0 }`)
    let (url, bytes) = artifact("pouch");
    let parsed = rut_driver::bundle::parse_bundle(&bytes).expect("parse the committed pouch");
    let torn: Vec<(String, Vec<u8>)> = parsed
        .into_iter()
        .map(|(n, b)| {
            if n == "mod.rutc" {
                let mut prog =
                    rut_core::binary::decode(&b).expect("decode the pouch module binary");
                // drop exactly one method-body template row: Vec::push
                let before = prog.templates.len();
                prog.templates
                    .retain(|t| match &t.kind {
                        rut_core::binary::TemplateKind::Method { data, name } => {
                            !(prog.interner.name(*data) == "Vec"
                                && prog.interner.name(*name) == "push")
                        }
                        _ => true,
                    });
                assert_eq!(prog.templates.len(), before - 1, "the push row left");
                (n, rut_core::binary::encode(&prog))
            } else {
                (n, b)
            }
        })
        .collect();
    let torn_bytes =
        rut_driver::bundle::write_bundle(&torn).expect("re-write the doctored archive");
    let base = std::env::temp_dir().join(format!("rut-std-cdn-row-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let mut table = BTreeMap::new();
    table.insert(url.clone(), torn_bytes.clone());
    let diags = load_and_compile(
        &base,
        &format!("\"pouch\": {{\"url\": \"{url}\"}}"),
        table,
        "use pouch::{ Vec };\n\nentry fn main() -> i32 {\n    let items: Vec<i64> = Vec<i64>::new();\n    items.push(7);\n    return items.len() as i32;\n}\n",
    )
    .expect("the world loads — the refusal is the compile's diag");
    let err = diags.join("; ");
    assert!(err.contains("served by neither"), "{err}");
    assert!(err.contains("push"), "{err}");
    assert!(err.contains("re-pack the directory"), "{err}");
    let _ = std::fs::remove_dir_all(&base);
}

/// One consumer world over the committed CDN artifacts: rows + source →
/// (diags, linked program, the session). Empty diags ⇒ compiled.
fn load_compile_run(
    dir: &Path,
    rows: &str,
    table: BTreeMap<String, Vec<u8>>,
    src: &str,
) -> Result<(Vec<String>, Option<rut_core::binary::Program>, rut_driver::Loaded), String> {
    let app = dir.join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(
        app.join("rut.jsonc"),
        format!(r#"{{"name": "app", "deps": {{{}}}}}"#, rows.trim_end_matches(|c: char| c == ',' || c.is_whitespace())),
    )
    .unwrap();
    std::fs::write(app.join("mod.rut"), src).unwrap();
    write_lock(&app, &table);
    let loaded = block_on(load_dir_with(&app, &Table::from(table))).map_err(|e| e.to_string())?;
    let g = rut_driver::RutRun::new()
        .pkgs(&loaded)
        .pkg(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"))
        .entrypoint(&loaded.root)
        .compile()
        .map_err(|e| e.to_string())?;
    let diags: Vec<String> = g.graph.diags.iter().map(|d| d.msg.clone()).collect();
    Ok((diags, g.graph.program, loaded))
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
        &format!("\"pouch\": {{\"url\": \"{url}\"}}"),
        table,
        "use pouch::{ Vec };\n\nentry fn main() -> i32 {\n    let items: Vec<i64> = Vec<i64>::new();\n    items.push(7);\n    items.push(9);\n    return items.len() as i32;\n}\n",
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
    let mut vm = rut_vm::interp:: Vm::builder().program(std::rc::Rc::new(prog)).limits(limits).hooks(rut_vm::interp::HostHooks::default()).hosts(rut_vm::interp::HostRegistry::new()).build()
    .expect("vm");
    let out: i32 = vm.call("main", ()).expect("run");
    assert_eq!(out, 2, "the load-time-compiled body runs");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn mixed_dir_and_archive_of_one_pkg_bind_per_row() {
    // the same pkg offered by BOTH lanes: json's url archive carries a
    // `strbuild` group (mounted under its CONTENT IDENTITY) and the
    // app's own `strbuild` path row mounts the directory under ITS
    // mount key — two mounts of one name coexist side by side, and the
    // consumer binds its OWN row's body. First-mount-wins is
    // per-identity exact dedup, never per-name.
    let mut table = BTreeMap::new();
    let mut rows = String::new();
    for key in ["json", "pouch", "nmapset"] {
        let (url, bytes) = artifact(key);
        rows.push_str(&format!("\"{key}\": {{\"url\": \"{url}\"}},\n"));
        table.insert(url, bytes);
    }
    let tree = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut")).join("strbuild");
    rows.push_str(&format!("\"strbuild\": {{\"path\": \"{}\"}},\n", tree.display()));
    let base = std::env::temp_dir().join(format!("rut-std-cdn-mixed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (diags, program, session) = load_compile_run(
        &base,
        &rows,
        table,
        "use strbuild::{ StringBuilder };\n\nentry fn main() -> str {\n    let mut b = StringBuilder::new();\n    b.append(\"mixed\");\n    return b.build();\n}\n",
    )
    .expect("the mixed world loads");
    assert!(diags.is_empty(), "{}", diags.join("; "));
    // the app's path row bound the DIRECTORY (its own row's body); the
    // archive's compiled group rode beside it under its identity
    assert!(
        matches!(session.pkg("strbuild").unwrap().body, rut_driver::PkgBody::Source { .. }),
        "the consumer binds its own row's mount (the dir source)"
    );
    assert!(
        session
            .pkgs
            .iter()
            .any(|p| rut_driver::bundle::parse_identity(&p.spec).map(|(n, _)| n) == Some("strbuild")),
        "the archive's strbuild group mounted under its content identity: {:?}",
        session.pkgs.iter().map(|p| p.spec.clone()).collect::<Vec<_>>()
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
    let src = "use pouch::{ Vec };\n\nentry fn main() -> i32 {\n    let items: Vec<i64> = Vec<i64>::new();\n    items.push(7);\n    return items.len() as i32;\n}\n";
    let mut outs: Vec<Vec<u8>> = Vec::new();
    for run in 0..2 {
        let base =
            std::env::temp_dir().join(format!("rut-std-cdn-det-{}-{run}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mut table = BTreeMap::new();
        table.insert(url.clone(), bytes.clone());
        let (diags, program, _session) = load_compile_run(
            &base,
            &format!("\"pouch\": {{\"url\": \"{url}\"}}"),
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
        app.join("rut.jsonc"),
        format!(r#"{{"name": "app", "deps": {{"strbuild": {{"url": "{url}"}}}}}}"#),
    )
    .unwrap();
    std::fs::write(
        app.join("mod.rut"),
        "use strbuild::{ StringBuilder };\n\nentry fn main() -> str {\n    let mut b = StringBuilder::new();\n    b.append(\"hello, \");\n    b.append(\"cdn\");\n    return b.build();\n}\n",
    )
    .unwrap();
    write_lock(&app, &table);
    let session =
        block_on(load_dir_with(&app, &Table::from(table))).expect("strbuild bundle loads");
    // the builder's host rows demand bodies — the embedder half
    // (calc offered beside; the core prelude auto-rides)
    let mut world = session.pkgs.clone();
    world.push(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"));
    let ctx = rut_driver::host_pkg_ctx(&world);
    let mut hosts = rut_vm::interp::HostRegistry::new();
    hosts.install_host_pkg(&ctx, rut_std::math::pkg());
    hosts.install_host_pkg(&ctx, rut_std::strbuild::pkg());
    hosts.verify_against(&ctx.flatten());
    let g = rut_driver::RutRun::new()
        .pkgs(&session)
        .pkg(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"))
        .entrypoint(&session.root)
        .compile()
        .expect("compile the walk");
    assert!(
        g.graph.diags.is_empty(),
        "{}",
        g.graph.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("; ")
    );
    let prog = g.graph.program.expect("linked");
    rut_vm::verify::verify(&rut_core::link::flatten(prog.clone())).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp:: Vm::builder().program(std::rc::Rc::new(prog)).limits(limits).hooks(rut_vm::interp::HostHooks::default()).hosts(hosts).build()
        .expect("vm");
    let out: String = vm.call("main", ()).expect("run");
    assert_eq!(out, "hello, cdn");
    // and strbuild_host rode the archive as the decl-surface group —
    // under its content identity (the mount key), rows registered
    // under the pkg's bare name
    let sbh = common::pkg_named(&session, "strbuild_host");
    assert!(rut_driver::bundle::is_identity(&sbh.spec), "the group mounts under its identity: {}", sbh.spec);
    assert!(matches!(&sbh.body, rut_driver::PkgBody::Host { .. }));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ambient_std_and_a_url_std_bundle_coexist() {
    // the redundancy note, post-ambient-tier (Q2-3: nothing ambient-
    // mounts calc anymore — only declared rows mount; core still
    // auto-offers): this host's calc offer beside a pinned core CDN
    // bundle is a HARMLESS doubled mount of core (first-mount-wins —
    // the same law as any duplicate) and custom hosts that offer none
    // of the std get the rows from the bundle
    let (url, bytes) = artifact("core");
    let base = std::env::temp_dir().join(format!("rut-std-cdn-core-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let mut table = BTreeMap::new();
    table.insert(url.clone(), bytes.clone());
    let diags = load_and_compile(
        &base,
        &format!("\"core\": {{\"url\": \"{url}\"}}"),
        table,
        "entry fn main() -> i32 {\n    return 7;\n}\n",
    )
    .expect("core loads");
    assert!(diags.is_empty(), "{}", diags.join("; "));
    let _ = std::fs::remove_dir_all(&base);
}
