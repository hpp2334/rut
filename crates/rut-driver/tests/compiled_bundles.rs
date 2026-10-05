//! v5 compiled bundles: the mixed closure — linkable pkgs ride as
//! `.rutc` binaries (bodies + surface, the linking truth), splice-needed
//! deps (inline / generic export / interface-typed params) and host pkgs
//! ride as source file sets — plus the pinned equivalence (compiled
//! bundle vs source directory ⇒ identical linked binaries), determinism,
//! and the refusal matrix.
//!
//! The v2/v3 load scenarios this file succeeds (the dep-bundle
//! round trips, the peer groups through an archive, D1) re-launch here
//! on v5: `rut pack` emits v5 only, and the loader reads v5 only —
//! older layouts are refused with the one-line version error.

use std::path::{Path, PathBuf};

use rut_native::pack_dir;
use rut_driver::{
    PkgBody,
};




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
            .pkg(rut_driver::Pkg::source(spec, src))
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

fn manifest(name: &str, entry: &str, extra: &str) -> String {
    format!(r#"{{"name": "{name}", "entry": {{"lib": "./{entry}"}}{extra}}}"#)
}

/// The bundle-shaped spelling: the pack gate's keys ride inside.
fn bundle_manifest(name: &str, entry: &str, extra: &str) -> String {
    format!(r#"{{"format": "rutbundle", "format_version": 10, "name": "{name}", "entry": {{"lib": "./{entry}"}}{extra}}}"#)
}

/// The mixed-closure world: `app` (linkable root) uses `util` (linkable
/// → compiled group) and declares `boxy` (an explicitly `inline`d pkg →
/// source group). With instantiation owner-anchored, generic exports
/// link — the one splice trigger left is the `inline` flag (class
/// methods cross no surface yet, so a method-bearing pkg keeps sharing
/// its source). The clean mixed shape is an inline dep the linkable
/// part of the tree does not absorb, which still rides (declared deps
/// publish completely) as a source group a consumer can splice.
fn mixed_world(tag: &str) -> PathBuf {
    let root = scratch(tag);
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &bundle_manifest("app", "app.rut", r#", "deps": {"util": {"path": "../util"}, "boxy": {"path": "../boxy"}}"#,)
    );
    write(
        &app,
        "app.rut",
        "use util::{ twice };\n\n\
         entry fn go() -> i64 {\n\
         \x20   return twice(21);\n\
         }\n",
    );
    let util = root.join("util");
    write(&util, "rut.jsonc", &manifest("util", "util.rut", ""));
    write(
        &util,
        "util.rut",
        "pub fn twice(v: i64) -> i64 {\n\
         \x20   return v * 2;\n\
         }\n",
    );
    let boxy = root.join("boxy");
    write(&boxy, "rut.jsonc", &manifest("boxy", "boxy.rut", r#", "inline": true"#));
    write(
        &boxy,
        "boxy.rut",
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
    // THE pinned equivalence: the compiled bundle and its source
    // directory link to the identical binary
    let dir_loaded = rut_native::load_dir(&root.join("app")).expect("dir load");
    let from_dir = linked_binary(&dir_loaded);

    let bytes = pack_dir(&root.join("app")).expect("pack");
    let loaded = rut_driver::Pkg::from_bundle(&bytes).expect("load");
    assert_eq!(loaded.root, "app");
    assert_eq!(linked_binary(&loaded), from_dir, "dir and bundle compile identically");

    // the closure mixed as declared: the linkable pkgs compiled, the
    // explicitly inlined one a source group
    assert!(matches!(
        loaded.pkg("app").unwrap().body,
        PkgBody::Compiled(_)
    ));
    assert!(matches!(
        loaded.pkg("util").unwrap().body,
        PkgBody::Compiled(_)
    ));
    assert!(matches!(
        loaded.pkg("boxy").unwrap().body,
        PkgBody::Source { .. }
    ));
    let names: Vec<String> =
        rut_driver::bundle::parse_bundle(&bytes).unwrap().into_iter().map(|(n, _)| n).collect();
    for key in ["rut.jsonc", "rut.scopes", "app.rutc", "util/util.rutc", "boxy/rut.jsonc", "boxy/boxy.rut"] {
        assert!(names.contains(&key.to_string()), "the bundle must carry `{key}`: {names:?}");
    }
    assert!(!names.iter().any(|n| n == "boxy/boxy.rutc"), "the inline pkg rides source, not a binary");

    // the ledger names every linked module of the packed closure
    let ledger = rut_driver::bundle::parse_bundle(&bytes).unwrap();
    let ledger = ledger.iter().find(|(n, _)| n == "rut.scopes").unwrap();
    let ledger = std::str::from_utf8(&ledger.1).unwrap();
    assert!(ledger.contains("= \"app\""), "{ledger}");
    assert!(ledger.contains("= \"util\""), "{ledger}");

    // and the mixed world dispatches: the compiled root answers
    let got: i64 = run_entry(loaded, "go");
    assert_eq!(got, 42);
}

#[test]
fn a_source_group_splices_into_a_consumer_session() {
    // the mixed closure's payoff: a consumer compiled AGAINST the
    // loaded bundle splices the source group (the inline pkg) and
    // binds the compiled groups' surfaces — the two kinds meet in one
    // session, no directory in sight
    let root = mixed_world("splice");
    let bytes = pack_dir(&root.join("app")).expect("pack");
    let mut loaded = rut_driver::Pkg::from_bundle(&bytes).expect("load");
    loaded.pkgs.push(rut_driver::Pkg::source(
        "consumer",
        "use boxy::{ Holder };\nuse util::{ twice };\n\n\
         entry fn go2() -> i64 {\n\
         \x20   let h = Holder<i64>.make(5);\n\
         \x20   return twice(h.get());\n\
         }\n",
    ));
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
        r#"{"format": "rutbundle", "format_version": 10, "name": "app", "entry": {"lib": "./app.rut"}}"#,
    );
    write(
        &app,
        "app.rut",
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
    assert!(names.contains(&"app.rutc".to_string()), "{names:?}");
    // generic-source riding: the exported template crosses compiled AND
    // the source rides beside it, so consumer-spelled shapes stay
    // servable at load
    assert!(names.contains(&"app.rut".to_string()), "{names:?}");
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
        r#"{"format": "rutbundle", "format_version": 10, "name": "app", "entry": {"lib": "./app.rut"}}"#,
    );
    write(
        &app,
        "app.rut",
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
fn peer_deps_on_a_compiled_group_ride_the_binary() {
    // presence-gated compiled rows: a compiled dep that declares
    // `[peer-deps]` publishes with whatever rows its pack-time closure
    // gated in — no source append, no downgrade. The group file itself
    // does not travel; the rows live in the declarer's binary.
    let root = scratch("peerref");
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &bundle_manifest("app", "app.rut", r#", "deps": {"libbed": {"path": "../libbed"}, "p": {"path": "../p"}}"#,)
    );
    write(&app, "app.rut", "use libbed::{ f };\nentry fn go() -> i32 { return f(); }\n");
    let libbed = root.join("libbed");
    write(
        &libbed,
        "rut.jsonc",
        &manifest(
            "libbed",
            "libbed.rut",
            r#", "peer-deps": {"p": {"path": "../p", "optional": true, "lib": "./ser_p.rut"}}"#,
        ),
    );
    write(&libbed, "libbed.rut", "pub fn f() -> i32 { return 7; }\n");
    write(&libbed, "ser_p.rut", "// an impl-only group file\n");
    write(&root.join("p"), "rut.jsonc", &manifest("p", "p.rut", ""));
    write(&root.join("p"), "p.rut", "pub class K { x: i32; }\n");

    // the pack succeeds: the closure has the peer, the gate recorded the
    // group, and the group's rows compiled into libbed's unit (the file
    // here is impl-only — no rows to carry; the shape is the pin)
    let bytes = pack_dir(&app).expect("peer-deps publish compiled");
    let names: Vec<String> =
        rut_driver::bundle::parse_bundle(&bytes).unwrap().into_iter().map(|(n, _)| n).collect();
    assert!(names.contains(&"libbed/libbed.rutc".to_string()), "{names:?}");
    assert!(!names.contains(&"libbed/ser_p.rut".to_string()), "{names:?}");
    let loaded = rut_driver::Pkg::from_bundle(&bytes).expect("load");
    let got: i32 = run_entry(loaded, "go");
    assert_eq!(got, 7);
    let _ = std::fs::remove_dir_all(&root);
}

/// The T2 world, re-launched on v5: the peer-gated pkg is INLINE
/// (concrete exports, the real `json` shape), so it rides as a SOURCE
/// group — group file included — and the loader's peer gate appends by
/// presence. Its consumers splice the inline pkg's text, so they stay
/// linkable: the root publishes compiled.
#[test]
fn peer_groups_ride_v5_as_compiled_rows() {
    // presence-gated compiled rows: the peer gate reads the group at
    // pack time (the peer is in the closure) and the group's impl rows
    // compile into the declarer's OWN binary. The group file does not
    // travel, the mounted body stays compiled, and the dispatch works
    // — the directory world and the bundle world compile identically.
    let root = scratch("peers");

    // peered: the interface + the peer-gated wrapper group (the json
    // group shape: a wrapper class per peer type, its inherent impl
    // carries the members)
    let peered = root.join("peered");
    write(
        &peered,
        "rut.jsonc",
        &manifest(
            "peered",
            "peered.rut",
            r#", "peer-deps": {"tagger": {"path": "../tagger", "optional": true, "lib": "./ser_tag.rut"}}"#,
        ),
    );
    write(
        &peered,
        "peered.rut",
        "pub interface Tag { fn tag(self) -> str; }\n\n\
         pub class TagStr(str);\n\
         impl TagStr {\n\
         \x20   pub fn tag(self) -> str { return self.inner; }\n\
         }\n",
    );
    write(
        &peered,
        // the group file carries its own use: the wrapper's field type
        // joins the declarer's unit through the ordinary scan of the
        // combined source (one unit, one text)
        "ser_tag.rut",
        "use tagger::{ Badge };\n\n\
         pub class TagBadge(Badge);\n\
         impl TagBadge { pub fn tag(self) -> str { return \"badged\"; } }\n",
    );

    // the peer: a concrete class
    let tagger = root.join("tagger");
    write(&tagger, "rut.jsonc", &manifest("tagger", "tagger.rut", ""));
    write(
        &tagger,
        "tagger.rut",
        "pub class Badge { x: i32; }\n\n\
         impl Badge {\n\
         \x20   pub fn new() -> Self { return Self { x: 0 }; }\n\
         }\n",
    );

    // a linkable dep on the side: the compiled group in the same world
    let base = root.join("base");
    write(&base, "rut.jsonc", &manifest("base", "base.rut", ""));
    write(&base, "base.rut", "pub fn b() -> i32 { return 7; }\n");

    // the consumer: all three in [deps]; the group's impl dispatches
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &bundle_manifest("app", "app.rut", r#", "deps": {"peered": {"path": "../peered"}, "tagger": {"path": "../tagger"}, "base": {"path": "../base"}}"#,)
    );
    write(
        &app,
        "app.rut",
        "use peered::{ TagBadge };\nuse tagger::{ Badge };\nuse base::{ b };\n\n\
         entry fn go() -> str {\n\
         \x20   let b2 = Badge.new();\n\
         \x20   let _ = b();\n\
         \x20   return TagBadge(b2).tag();\n\
         }\n",
    );

    // the directory world first (the load-time peer gate rides the same
    // compile — the rows land in the declarer's unit there too)
    let dir_loaded = rut_native::load_dir(&app).expect("dir load");
    let from_dir = linked_binary(&dir_loaded);

    let bytes = pack_dir(&app).expect("pack");
    let loaded = rut_driver::Pkg::from_bundle(&bytes).expect("load");
    assert_eq!(linked_binary(&loaded), from_dir, "dir and bundle compile identically");

    // the group rides INSIDE the declarer's compiled binary: the body
    // is compiled, no source group travels
    assert!(matches!(loaded.pkg("peered").unwrap().body, PkgBody::Compiled(_)));
    let names: Vec<String> =
        rut_driver::bundle::parse_bundle(&bytes).unwrap().into_iter().map(|(n, _)| n).collect();
    assert!(!names.contains(&"peered/ser_tag.rut".to_string()), "{names:?}");
    assert!(names.contains(&"base/base.rutc".to_string()), "{names:?}");
    assert!(names.contains(&"tagger/tagger.rutc".to_string()), "{names:?}");

    // and the group's wrapper dispatches
    let got: String = run_entry(loaded, "go");
    assert_eq!(got, "badged");
}

#[test]
fn required_peer_absent_is_d1_at_pack() {
    let root = scratch("d1");
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &bundle_manifest("app", "app.rut", r#", "deps": {"req": {"path": "../req"}}"#,)
    );
    write(&app, "app.rut", "use req::{ f };\nentry fn go() -> i32 { return f(); }\n");
    let req = root.join("req");
    write(
        &req,
        "rut.jsonc",
        &manifest("req", "req.rut", r#", "peer-deps": {"missing": {"path": "../missing"}}"#,)
    );
    write(&req, "req.rut", "pub fn f() -> i32 { return 7; }\n");
    let err = pack_dir(&app).unwrap_err().to_string();
    assert!(err.contains("requires the peer `missing`"), "{err}");
    assert!(err.contains("peers are not pulled transitively"), "{err}");
}

#[test]
fn stale_and_corrupt_group_binaries_are_refused() {
    let root = mixed_world("refuse");
    let bytes = pack_dir(&root.join("app")).expect("pack");

    // a pre-v16 `.rutc` (no surface section) inside the archive: the
    // decode gate refuses it loudly, before any mount
    let stale: Vec<(String, Vec<u8>)> = rut_driver::bundle::parse_bundle(&bytes)
        .unwrap()
        .into_iter()
        .map(|(n, mut b)| {
            if n == "util/util.rutc" {
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
    assert!(err.contains("util/util.rutc"), "{err}");
    assert!(err.contains("version"), "{err}");

    // a corrupt `.rutc` payload: flip a byte INSIDE the container's
    // stored util.rutc payload — the container CRC fires first (the
    // group binary is never even decoded)
    let mut corrupt = bytes.clone();
    let needle = b"util/util.rutc";
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

