//! v11 compiled bundles: the mixed closure — linked pkgs ride as
//! `mod.rutc` binaries (bodies + surface, the linking truth; generic
//! owners also ride their generic bodies as templates INSIDE the
//! binary, v22), pkgs the graph
//! never linked (declared-but-unused) and host pkgs ride as source
//! file sets under their content identities — plus the pinned
//! equivalence (compiled bundle vs source directory ⇒ identical
//! linked binaries), determinism, and the refusal matrix.

use std::path::{Path, PathBuf};

use rut_native::pack_dir;
use rut_driver::{
    PkgBody,
};

mod common;
use common::{dep_dir, pkg_named};




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

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-v5-{tag}-{}", std::process::id()));
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
    // the entry keys are repealed: the body is `mod.rut` beside the
    // manifest (the `_entry` arg names the file the caller writes)
    format!(r#"{{"name": "{name}"{extra}}}"#)
}

/// The bundle-shaped spelling: the pack gate's keys ride inside.
fn bundle_manifest(name: &str, _entry: &str, extra: &str) -> String {
    format!(r#"{{"format": "rutbundle", "format_version": 11, "name": "{name}"{extra}}}"#)
}

/// The mixed-closure world: `app` (linkable root) uses `util` (linked
/// → compiled group) and declares `boxy` without using it (declared
/// but unlinked → source group). With instantiation owner-anchored,
/// generic exports and iface params link — a source pkg rides compiled
/// iff the graph linked it, so `boxy`'s class methods trigger nothing:
/// the unused declaration still rides (declared deps publish
/// completely) as a source group a consumer can splice.
fn mixed_world(tag: &str) -> PathBuf {
    let root = scratch(tag);
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &bundle_manifest("app", "mod.rut", r#", "deps": {"util": {"path": "../util"}, "boxy": {"path": "../boxy"}}"#,)
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
    let boxy = root.join("boxy");
    write(&boxy, "rut.jsonc", &manifest("boxy", "mod.rut", ""));
    write(
        &boxy,
        "mod.rut",
        "pub class Holder<T> {\n\
         \x20   v: T;\n\
         }\n\n\
         impl<T> Holder<T> {\n\
         \x20   pub fn make(v: T) -> Self { return Self { v: v }; }\n\
         \x20   pub fn get(self) -> T { return self.v; }\n\
         }\n",
    );
    root
}

fn linked_binary(loaded: &rut_driver::Loaded) -> Vec<u8> {
    let g = rut_driver::RutRun::new()
        .pkgs(loaded)
        .entrypoint(&loaded.root)
        .compile()
        .expect("compile the walk");
    assert!(g.graph.diags.is_empty(), "{:?}", g.graph.diags);
    rut_core::binary::encode(&g.graph.program.expect("linked program"))
}

fn run_entry<R: rut_vm::interp::Ret>(loaded: rut_driver::Loaded, entry: &str) -> R {
    let g = rut_driver::RutRun::new()
        .pkgs(&loaded)
        .pkg(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"))
        .entrypoint(&loaded.root)
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
    let mut vm = rut_vm::interp:: Vm::builder().program(std::rc::Rc::new(flat)).limits(limits).hooks(rut_vm::interp::HostHooks::default()).hosts(rut_vm::interp::HostRegistry::new()).build()
    .expect("vm");
    vm.call::<_, R>(entry, ()).expect("run")
}

#[test]
fn mixed_closure_pack_load_run_equals_the_directory() {
    let root = mixed_world("mixed");
    // THE equivalence, post-identity: the compiled bundle and its
    // source directory are behaviorally identical — same run result.
    // (The v10-era BYTE pin yields to the identity law: a bundle
    // world's groups compile under their mount keys, so the name
    // table spells `util@<fp16>` where the directory world spells
    // `util` — the key is part of the compile input now.)
    let dir_loaded = rut_native::load_dir(&root.join("app")).expect("dir load");
    let from_dir = run_entry::<i64>(dir_loaded, "go");

    let bytes = pack_dir(&root.join("app")).expect("pack");
    let loaded = rut_driver::Pkg::from_bundle(&bytes).expect("load");
    assert_eq!(loaded.root, "app");
    assert_eq!(run_entry::<i64>(loaded.clone(), "go"), from_dir, "dir and bundle run identically");

    // the closure mixed as declared: the linked pkgs compiled, the
    // declared-but-unused one a source group
    assert!(matches!(
        loaded.pkg("app").unwrap().body,
        PkgBody::Compiled(_)
    ));
    assert!(matches!(
        pkg_named(&loaded, "util").body,
        PkgBody::Compiled(_)
    ));
    assert!(matches!(
        pkg_named(&loaded, "boxy").body,
        PkgBody::Source { .. }
    ));
    let names: Vec<String> =
        rut_driver::bundle::parse_bundle(&bytes).unwrap().into_iter().map(|(n, _)| n).collect();
    let util_dir = dep_dir(&names, "util");
    let boxy_dir = dep_dir(&names, "boxy");
    for key in ["rut.jsonc", "rut.scopes", "mod.rutc", &format!("{util_dir}/mod.rutc"), &format!("{boxy_dir}/rut.jsonc"), &format!("{boxy_dir}/mod.rut")] {
        assert!(names.contains(&key.to_string()), "the bundle must carry `{key}`: {names:?}");
    }
    assert!(!names.iter().any(|n| n == &format!("{boxy_dir}/mod.rutc")), "the unused group rides source, not a binary");

    // the ledger names every linked module of the packed closure — by
    // its content identity (the one wire's row spelling)
    let ledger = rut_driver::bundle::parse_bundle(&bytes).unwrap();
    let ledger = ledger.iter().find(|(n, _)| n == "rut.scopes").unwrap();
    let ledger = std::str::from_utf8(&ledger.1).unwrap();
    assert!(ledger.contains("= \"app@"), "{ledger}");
    assert!(ledger.contains("= \"util@"), "{ledger}");
    assert!(ledger.contains("= \"core@e3b0c44298fc1c14\""), "{ledger}");

    // and the mixed world dispatches: the compiled root answers
    let got: i64 = run_entry(loaded, "go");
    assert_eq!(got, 42);
}

#[test]
fn a_source_group_splices_into_a_consumer_session() {
    // the mixed closure's payoff: a consumer compiled AGAINST the
    // loaded bundle splices the source group (boxy) and
    // binds the compiled groups' surfaces — the two kinds meet in one
    // session, no directory in sight
    let root = mixed_world("splice");
    let bytes = pack_dir(&root.join("app")).expect("pack");
    let mut loaded = rut_driver::Pkg::from_bundle(&bytes).expect("load");
    loaded.pkgs.push(
        rut_driver::Pkg::source(
            "consumer",
            "use boxy::{ Holder };\nuse util::{ twice };\n\n\
             entry fn go2() -> i64 {\n\
             \x20   let h = Holder<i64>::make(5);\n\
             \x20   return twice(h.get());\n\
             }\n",
        )
        // the consumer declares the bundle groups it names (the
        // nameability law — mounted-but-undeclared is D5)
        .manifest(rut_driver::Manifest::lib("consumer", &["boxy", "util", "calc"])),
    );
    // the CONSUMER is the root here — a source unit over the bundle's
    // mounted groups, compiled and linked exactly like a directory world
    let g = rut_driver::RutRun::new()
        .pkgs(&loaded)
        .pkg(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"))
        .entrypoint("consumer")
        .compile()
        .unwrap();
    assert!(g.graph.diags.is_empty(), "{:?}", g.graph.diags);
    let flat = rut_core::link::flatten(g.graph.program.expect("linked"));
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp:: Vm::builder().program(std::rc::Rc::new(flat)).limits(limits).hooks(rut_vm::interp::HostHooks::default()).hosts(rut_vm::interp::HostRegistry::new()).build()
    .expect("vm");
    assert_eq!(vm.call::<_, i64>("go2", ()).expect("run"), 10);
}

#[test]
fn same_dir_packs_byte_identical() {
    let root = mixed_world("det");
    let a = pack_dir(&root.join("app")).unwrap();
    let b = pack_dir(&root.join("app")).unwrap();
    assert_eq!(a, b, "same dir => byte-identical v5 bundle");
}

/// Owner-anchored instantiation retired the generic-export and
/// trait-param refusals: a generic root publishes compiled, and its
/// consumers' instantiation requests resolve against the binary's
/// ledger. The pack walk seeds the root's requests into the dep's
/// compile, so the dep's group binary carries every instantiation the
/// closure uses.
#[test]
fn generic_and_iface_param_roots_publish_compiled() {
    // a generic root: the exported template rides the binary, the
    // instantiation the root itself spells compiles inside it, and the
    // ledger names the row
    let root = scratch("genroot");
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 11, "name": "app"}"#,
    );
    write(
        &app,
        "mod.rut",
        "pub struct Pair<A, B> {\n\
         \x20   fst: A;\n\
         \x20   snd: B;\n\
         }\n\n\
         entry fn go() -> i64 {\n\
         \x20   let p = Pair<i64, i64> { fst: 1, snd: 2 };\n\
         \x20   return p.fst + p.snd;\n\
         }\n",
    );
    let bytes = pack_dir(&app).expect("a generic root publishes compiled");
    let names: Vec<String> =
        rut_driver::bundle::parse_bundle(&bytes).unwrap().into_iter().map(|(n, _)| n).collect();
    assert!(names.contains(&"mod.rutc".to_string()), "{names:?}");
    // v22 — the delivery law: generic bodies ride the binary as
    // template rows; NO source rides beside it (the riding law is dead)
    assert!(!names.contains(&"mod.rut".to_string()), "{names:?}");
    let run = |b: &[u8]| {
        let loaded = rut_driver::Pkg::from_bundle(b).expect("load");
        run_entry(loaded, "go")
    };
    let got: i64 = run(&bytes);
    assert_eq!(got, 3);
    let _ = std::fs::remove_dir_all(&root);

    // an interface-typed parameter: the fn crosses and dispatches through
    // the per-type itable fill (the boxing site is here in the same
    // unit) — no splice
    let root = scratch("ifaceroot");
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 11, "name": "app"}"#,
    );
    write(
        &app,
        "mod.rut",
        "pub interface Shape { fn area(self) -> i32; }\n\
         pub fn draw(s: Shape) -> i32 { return s.area(); }\n\n\
         entry fn go() -> i32 {\n\
         \x20   return draw(Square { side: 6 });\n\
         }\n\n\
         struct Square { side: i32; }\n\n\
         impl Square {\n\
         \x20   pub fn area(self) -> i32 { return self.side * self.side; }\n\
         }\n",
    );
    let bytes = pack_dir(&app).expect("an iface-param root publishes compiled");
    let loaded = rut_driver::Pkg::from_bundle(&bytes).expect("load");
    let got: i32 = run_entry(loaded, "go");
    assert_eq!(got, 36);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn stale_and_corrupt_group_binaries_are_refused() {
    let root = mixed_world("refuse");
    let bytes = pack_dir(&root.join("app")).expect("pack");

    // a pre-v16 `.rutc` (no surface section) inside the archive: the
    // decode gate refuses it loudly, before any mount
    let names: Vec<String> = rut_driver::bundle::parse_bundle(&bytes)
        .unwrap()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    let util_bin = format!("{}/mod.rutc", dep_dir(&names, "util"));
    let stale: Vec<(String, Vec<u8>)> = rut_driver::bundle::parse_bundle(&bytes)
        .unwrap()
        .into_iter()
        .map(|(n, mut b)| {
            if n == util_bin {
                // MAGIC + a stale version word, then nothing — the
                // standard version error, not a silent mount
                b = b"RUTC".to_vec();
                b.extend_from_slice(&15u32.to_le_bytes());
            }
            (n, b)
        })
        .collect();
    let err = rut_driver::Pkg::from_bundle(&rut_driver::bundle::write_bundle(&stale).unwrap())
        .unwrap_err()
        .to_string();
    assert!(err.contains(&util_bin), "{err}");
    assert!(err.contains("version"), "{err}");

    // a corrupt `.rutc` payload: flip a byte INSIDE the container's
    // stored group binary payload — the container CRC fires first (the
    // group binary is never even decoded)
    let mut corrupt = bytes.clone();
    let needle = util_bin.as_bytes();
    let at = corrupt
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("the entry name rides the container");
    let data_at = at + needle.len() + 30; // local header: fixed 30 bytes + name
    corrupt[data_at] ^= 0x01;
    let err = rut_driver::Pkg::from_bundle(&corrupt).unwrap_err().to_string();
    assert!(err.contains("CRC"), "{err}");

    // a doctored scope ledger: the group's binary carries its own
    // scope, and the ledger row must AGREE with it (refuse, never
    // guess — a mismatch is a corrupt or doctored bundle)
    let doctored: Vec<(String, Vec<u8>)> = rut_driver::bundle::parse_bundle(&bytes)
        .unwrap()
        .into_iter()
        .map(|(n, mut b)| {
            if n == "rut.scopes" {
                let text = std::str::from_utf8(&b).unwrap().to_string();
                // bump every scope by one — nothing matches anymore
                let mut out = String::new();
                for line in text.lines() {
                    if let Some((l, r)) = line.split_once('=') {
                        let scope: u32 = l.trim().parse().unwrap();
                        out.push_str(&format!("{} = {}\n", scope + 100, r.trim()));
                    }
                }
                b = out.into_bytes();
            }
            (n, b)
        })
        .collect();
    let err = rut_driver::Pkg::from_bundle(&rut_driver::bundle::write_bundle(&doctored).unwrap())
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("ledger") || err.contains("scope"),
        "{err}"
    );
}

/// The one-closure fixture for the lane-equivalence golden: the std
/// `pouch` package (self-contained — no deps) plus a consumer whose
/// shape (`Vec<i32>`) exists only because the consumer spells it.
/// The consumer compiles identically in both lanes.
const POUCH_APP: &str = r#"
use pouch::{ Vec };

entry fn go() -> i32 {
    let items: Vec<i32> = Vec<i32>::new();
    items.push(20);
    items.push(items[0] * 2);
    let n = items.len();
    return n * 100 + items[0] + items[1];
}
"#;

fn run_compiled<R: rut_vm::interp::Ret>(g: rut_driver::Compiled, entry: &str) -> R {
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

/// The lane-equivalence golden (staged == reference, L-eq): ONE
/// closure — the std `pouch` package plus a consumer spelling a shape
/// pouch's own closure never spells — compiled through BOTH lanes.
/// (a) The REFERENCE lane: the whole-program compile over the closure
/// as sources, one graph. (b) The STAGED lane: pouch packed to an
/// archive, the archive walked + mounted into a fresh run, the consumer
/// compiled against its surface (instantiating `Vec<i32>` from the
/// templates), linked. The lanes must agree on the run result — a
/// staged build is correct exactly when it equals the reference build.
/// Linked BYTES are pinned per lane (each lane is deterministic against
/// itself — the reference law), not ACROSS lanes: the staged world's
/// compile input carries the archive's mount keys (the identity law),
/// so its name table is not byte-comparable with the directory world's;
/// the run result is the cross-lane contract (byte-level parity of the
/// two lanes' fn inventories is the generic-templates suite's law).
#[test]
fn staged_lane_produces_what_the_reference_lane_would() {
    let tree = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let pouch = tree.join("rut/pouch");

    // (a) the reference lane: the whole closure as sources, one graph
    let dir_pkg = rut_native::tree_pkg("pouch").expect("walk rut/pouch");
    let reference = rut_driver::RutRun::new()
        .pkg(dir_pkg)
        .pkg(
            rut_driver::Pkg::source("app", POUCH_APP)
                .manifest(rut_driver::Manifest::lib("app", &["pouch"])),
        )
        .entrypoint("app")
        .compile()
        .expect("reference compile");
    let ref_bytes = rut_core::binary::encode(
        &reference
            .graph
            .program
            .clone()
            .expect("linked program"),
    );
    assert!(!ref_bytes.is_empty());
    assert_eq!(run_compiled::<i32>(reference, "go"), 260);

    // L-ref sanity: the reference lane recompiles to identical bytes
    let recompiled = rut_driver::RutRun::new()
        .pkg(rut_native::tree_pkg("pouch").expect("walk rut/pouch"))
        .pkg(
            rut_driver::Pkg::source("app", POUCH_APP)
                .manifest(rut_driver::Manifest::lib("app", &["pouch"])),
        )
        .entrypoint("app")
        .compile()
        .expect("reference recompile");
    let ref_bytes_again = rut_core::binary::encode(
        &recompiled
            .graph
            .program
            .clone()
            .expect("linked program"),
    );
    assert_eq!(ref_bytes_again, ref_bytes, "the reference lane is deterministic");

    // (b) the staged lane: pack the closure to an archive, walk +
    // mount it, compile the consumer against its surface
    let bytes = pack_dir(&pouch).expect("pack the closure");
    let archive = rut_driver::Pkg::from_bundle(&bytes).expect("decode the archive");
    assert_eq!(archive.root, "pouch");
    assert!(matches!(
        archive.pkg("pouch").unwrap().body,
        PkgBody::Compiled(_)
    ));
    let staged = rut_driver::RutRun::new()
        .pkgs(&archive)
        .pkg(
            rut_driver::Pkg::source("app", POUCH_APP)
                .manifest(rut_driver::Manifest::lib("app", &["pouch"])),
        )
        .entrypoint("app")
        .compile()
        .expect("staged compile");
    assert_eq!(run_compiled::<i32>(staged, "go"), 260, "staged == reference");
}
