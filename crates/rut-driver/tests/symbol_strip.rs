//! Compile-time symbol stripping, end to end: the packed closure loads,
//! verifies, and runs identically with names mangled and positions
//! gone; the private `.rutsym` sidecar restores real names and line/col
//! for `StackTrace` symbolication; mixed closures (a source group that
//! can bind a compiled group's surface) refuse; the whole lane is
//! byte-deterministic.
//!
//! The VM changes zero lines here — that is the point: symbolication is
//! lazy, per-index, against the loaded program's interner + `pos`
//! table, and degrades to pc-only text when the positions are gone.

use std::path::{Path, PathBuf};

use rut_driver::PackOpts;
use rut_native::pack_dir_opts;

/// One source pkg over the auto core — the chain's graph. Closure and
/// shape refusals come back as one span-0 diagnostic.
#[allow(dead_code)] // not every suite in this file needs both lanes
fn compiled(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(
        rut_driver::RutRun::new()
            .pkg(rut_driver::Mounted::source(spec, src))
            .entrypoint(spec)
            .compile(),
    )
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
                rut_driver::Mounted::source(spec, src)
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
            diags: vec![rut_lexer::diag::Diag::new(
                rut_lexer::span::Span::new(0, 0),
                e.msg,
            )],
            program: None,
        },
    }
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-strip-{tag}-{}", std::process::id()));
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

fn manifest(name: &str, _entry: &str, extra: &str) -> String {
    format!(r#"{{"name": "{name}"{extra}}}"#)
}

/// The bundle-shaped spelling: the pack gate's keys ride inside.
fn bundle_manifest(name: &str, _entry: &str, extra: &str) -> String {
    format!(r#"{{"format": "rutbundle", "format_version": 12, "name": "{name}"{extra}}}"#)
}

/// The fully-compiled closure: `app` (linkable root) uses `util`
/// (linkable) — no source group anywhere, so `--strip` is legal.
fn fc_world(tag: &str) -> PathBuf {
    let root = scratch(tag);
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &bundle_manifest(
            "app",
            "mod.rut",
            r#", "deps": {"util": {"path": "../util"}}"#,
        ),
    );
    write(
        &app,
        "mod.rut",
        "use util::{ twice };\n\n\
         entry fn go() -> i64 {\n\
         \x20   return twice(21);\n\
         }\n",
    );
    let util = root.join("util");
    write(&util, "rut.jsonc", &manifest("util", "mod.rut", ""));
    write(
        &util,
        "mod.rut",
        "pub fn twice(v: i64) -> i64 {\n\
         \x20   return v * 2;\n\
         }\n",
    );
    root
}

/// `n` filler statements — past the checker inliner's budget, so the
/// chain stays REAL frames (the stack_trace suite's trick).
fn pad(n: usize) -> String {
    (0..n).map(|i| format!("    let p{i} = {i};\n")).collect()
}

/// The stack-trace world: a single linkable root whose padded chain
/// captures and renders a trace.
fn trace_world(tag: &str) -> PathBuf {
    let root = scratch(tag);
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 12, "name": "app"}"#,
    );
    let mut src = String::from("fn boom() -> str {\n");
    src.push_str(&pad(25));
    src.push_str("    let t = capture_stacktrace();\n");
    src.push_str("    return t.render();\n");
    src.push_str("}\n");
    src.push_str("entry fn probe() -> str {\n");
    src.push_str(&pad(25));
    src.push_str("    return boom();\n");
    src.push_str("}\n");
    src.push_str("entry fn go() -> i64 { return 7; }\n");
    write(&app, "mod.rut", &src);
    root
}

/// The mixed closure: `app` uses `util` (both ride compiled); `boxy`
/// (inline, unused by the linkable tree) rides as a SOURCE group whose
/// source `use`s util — exactly the shape strip must refuse.
fn mixed_world(tag: &str) -> PathBuf {
    let root = scratch(tag);
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &bundle_manifest(
            "app",
            "mod.rut",
            r#", "deps": {"util": {"path": "../util"}, "boxy": {"path": "../boxy"}}"#,
        ),
    );
    write(
        &app,
        "mod.rut",
        "use util::{ twice };\n\n\
         entry fn go() -> i64 {\n\
         \x20   return twice(21);\n\
         }\n",
    );
    let util = root.join("util");
    write(&util, "rut.jsonc", &manifest("util", "mod.rut", ""));
    write(
        &util,
        "mod.rut",
        "pub fn twice(v: i64) -> i64 { return v * 2; }\n",
    );
    let boxy = root.join("boxy");
    write(
        &boxy,
        "rut.jsonc",
        &format!(
            "{}",
            manifest(
                "boxy",
                "mod.rut",
                r#", "deps": {"util": {"path": "../util"}}"#
            )
        ),
    );
    write(
        &boxy,
        "mod.rut",
        "use util::{ twice };\n\n\
         pub fn boxed(v: i64) -> i64 {\n\
         \x20   return twice(v) + 1;\n\
         }\n",
    );
    root
}

fn load_and_run<R: rut_vm::interp::Ret>(bytes: &[u8], entry: &str, symtab: Option<&[u8]>) -> R {
    // the handle lane: decode + verify ONCE, restore on the handle
    // (before any offer), then offer the SAME archive
    let mut archive = rut_driver::Bundle::parse(bytes).expect("decode");
    if let Some(sym) = symtab {
        let map = rut_core::strip::SymbolMap::from_bytes(sym).expect("sidecar decode");
        // no section may skip: every section's pkg rides this bundle
        let carried = archive.carried_specs();
        for section in &map.sections {
            assert!(
                carried.iter().any(|s| s == &section.spec),
                "no section may skip: {}",
                section.spec
            );
        }
        archive.restore_symbols(&map);
    }
    let root = archive.walk().expect("walk").root;
    let g = rut_driver::RutRun::new()
        .pkg(&archive)
        .pkg(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"))
        .entrypoint(&root)
        .compile()
        .expect("compile the walk");
    assert!(g.graph.diags.is_empty(), "{:?}", g.graph.diags);
    let flat = rut_core::link::flatten(g.graph.program.expect("linked program"));
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::builder()
        .program(std::rc::Rc::new(flat))
        .limits(limits)
        .hooks(rut_vm::interp::HostHooks::default())
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("vm");
    vm.call::<_, R>(entry, ()).expect("run")
}

#[test]
fn stripped_bundle_loads_verifies_and_runs_like_the_plain_pack() {
    let root = fc_world("run");
    let dir = root.join("app");
    let (plain, no_sym) = pack_dir_opts(&dir, &PackOpts::default()).expect("plain pack");
    assert!(no_sym.is_none(), "the default packs no sidecar");
    let (stripped, symtab) = pack_dir_opts(&dir, &PackOpts { strip: true }).expect("strip pack");
    let symtab = symtab.expect("--strip returns the sidecar");
    assert!(!symtab.is_empty());
    // the sidecar is the PRIVATE half — never an entry inside the bundle
    let names: Vec<String> = rut_driver::bundle::parse_bundle(&stripped)
        .unwrap()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert!(!names.iter().any(|n| n.contains("rutsym")), "{names:?}");

    let got: i64 = load_and_run(&plain, "go", None);
    assert_eq!(got, 42);
    // the mangled binary decodes (the loader's gate), verifies, and
    // answers identically — the pin: names/positions are diagnostics,
    // never behavior
    let got: i64 = load_and_run(&stripped, "go", None);
    assert_eq!(got, 42);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn sidecar_restores_names_and_positions_for_the_trace() {
    let root = trace_world("trace");
    let dir = root.join("app");
    let (plain, _) = pack_dir_opts(&dir, &PackOpts::default()).expect("plain pack");
    let (stripped, symtab) = pack_dir_opts(&dir, &PackOpts { strip: true }).expect("strip pack");
    let symtab = symtab.expect("sidecar");

    // the plain pack: real fn names, real line/col
    let full: String = load_and_run(&plain, "probe", None);
    let full_lines: Vec<&str> = full.lines().collect();
    assert_eq!(full_lines.len(), 2, "boom + probe, innermost first: {full}");
    assert!(full_lines[0].starts_with("at boom ("), "{full}");
    assert!(
        full_lines[0].contains(":") && !full_lines[0].contains("@ pc"),
        "{full}"
    );

    // the mangled pack: %N names, pc-only degradation
    let mangled: String = load_and_run(&stripped, "probe", None);
    let mangled_lines: Vec<&str> = mangled.lines().collect();
    assert_eq!(mangled_lines.len(), 2, "{mangled}");
    assert!(
        mangled_lines[0].starts_with("at %") && mangled_lines[0].contains("@ pc"),
        "stripped renders pc-only: {mangled}"
    );

    // the sidecar restores BOTH lanes to the plain pack's exact text
    let map = rut_core::strip::SymbolMap::from_bytes(&symtab).expect("decode");
    let restored: String = load_and_run(&stripped, "probe", Some(&symtab));
    assert_eq!(restored, full, "restoration is exact, frame for frame");
    // re-applying is a no-op: every section matches, twice over (the
    // restore lives on the handle — applied twice before any offer)
    let mut archive = rut_driver::Bundle::parse(&stripped).expect("decode");
    archive.restore_symbols(&map);
    archive.restore_symbols(&map);
    let g = rut_driver::RutRun::new()
        .pkg(&archive)
        .pkg(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"))
        .entrypoint("app")
        .compile()
        .expect("compile the walk");
    assert!(g.graph.diags.is_empty(), "{:?}", g.graph.diags);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn mixed_closure_with_a_source_group_refuses_strip() {
    let root = mixed_world("refuse");
    let dir = root.join("app");
    // without --strip the mixed world packs as always
    pack_dir_opts(&dir, &PackOpts::default()).expect("plain pack rides mixed");
    let err = pack_dir_opts(&dir, &PackOpts { strip: true })
        .unwrap_err()
        .to_string();
    assert!(err.contains("boxy"), "names the source group: {err}");
    assert!(err.contains("util"), "names the compiled dep: {err}");
    assert!(err.contains("fully-compiled"), "says why: {err}");
    assert!(err.contains("--strip"), "names the escape hatch: {err}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn strip_serves_a_generic_owning_closure() {
    // the v22 coverage law: a successful pack's templates cover its
    // generic surface by construction, so `--strip` on a generic-owning
    // closure is legal unconditionally — the pack succeeds, the bundle
    // carries no source, and a consumer-spelled shape still materializes
    // from the stripped binary's templates at link
    let root = scratch("stripgen");
    let lib = root.join("pairz");
    write(&lib, "rut.jsonc", &bundle_manifest("pairz", "mod.rut", ""));
    write(
        &lib,
        "mod.rut",
        "pub struct Pair<A, B> {\n\
         \x20   fst: A;\n\
         \x20   snd: B;\n\
         }\n",
    );
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &bundle_manifest(
            "app",
            "mod.rut",
            r#", "deps": {"pairz": {"path": "../pairz"}}"#,
        ),
    );
    write(
        &app,
        "mod.rut",
        "use pairz::{ Pair };\n\n\
         entry fn go() -> i64 {\n\
         \x20   let p = Pair<i64, str> { fst: 1, snd: \"x\" };\n\
         \x20   return p.fst;\n\
         }\n",
    );
    // the strip pack succeeds — the app is the root pkg, the whole
    // closure strips
    let (bytes, _sidecar) =
        pack_dir_opts(&app, &PackOpts { strip: true }).expect("the stripped generic pack");
    // the bundle carries no source entries (the delivery law: ledger |
    // templates, never source)
    let parsed = rut_driver::bundle::parse_bundle(&bytes).expect("parse");
    assert!(
        !parsed
            .iter()
            .any(|(n, _)| n.ends_with(".rut") && !n.ends_with(".d.rut") && n != "rut.jsonc"),
        "the stripped bundle carries source: {:?}",
        parsed.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
    // and a consumer of the stripped bundle still materializes the
    // consumer-spelled shape from the templates
    let mut world = rut_driver::Mounted::from_bundle(&bytes)
        .expect("mount the stripped bundle")
        .pkgs;
    world.push(
        rut_driver::Mounted::source(
            "app2",
            "use pairz::{ Pair };\n\n\
             entry fn go() -> i64 {\n\
             \x20   let p = Pair<i64, str> { fst: 1, snd: \"x\" };\n\
             \x20   return p.fst;\n\
             }\n",
        )
        .manifest(rut_driver::Manifest::lib("app2", &["pairz"])),
    );
    let c = rut_driver::Walk {
        pkgs: world,
        root: String::new(),
    }
    .pkgs
    .iter()
    .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()))
    .entrypoint("app2")
    .compile()
    .expect("the consumer of the stripped generic bundle compiles");
    assert!(
        c.graph.diags.is_empty(),
        "diags:\n{}",
        c.graph
            .diags
            .iter()
            .map(|d| d.msg.clone())
            .collect::<Vec<_>>()
            .join("\n")
    );
    let flat = rut_core::link::flatten(c.graph.program.expect("linked"));
    rut_vm::verify::verify(&flat).expect("verify");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn strip_packing_is_deterministic() {
    let root = fc_world("det");
    let dir = root.join("app");
    let (b1, s1) = pack_dir_opts(&dir, &PackOpts { strip: true }).expect("pack 1");
    let (b2, s2) = pack_dir_opts(&dir, &PackOpts { strip: true }).expect("pack 2");
    assert_eq!(b1, b2, "same dir => byte-identical bundle");
    assert_eq!(s1, s2, "same dir => byte-identical sidecar");
    let _ = std::fs::remove_dir_all(&root);
}
