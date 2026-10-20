//! v11 compiled bundles: the mixed closure — linked pkgs ride as
//! `mod.rutc` binaries (bodies + surface, the linking truth; generic
//! owners also ride their generic bodies as templates INSIDE the
//! binary, v22), pkgs the graph
//! never linked (declared-but-unused) and host pkgs ride as source
//! file sets under their content identities — plus the pinned
//! equivalence (compiled bundle vs source directory ⇒ identical
//! linked binaries), determinism, and the refusal matrix.

use std::path::{Path, PathBuf};

use rut_driver::MountedBody;
use rut_native::pack_dir;

mod common;
use common::{dep_dir, pkg_named};

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
    format!(r#"{{"format": "rutbundle", "format_version": 12, "name": "{name}"{extra}}}"#)
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

fn run_entry<R: rut_vm::interp::Ret>(loaded: rut_driver::Walk, entry: &str) -> R {
    let g = loaded
        .pkgs
        .iter()
        .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()))
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
    let loaded = rut_driver::Mounted::from_bundle(&bytes).expect("load");
    assert_eq!(loaded.root, "app");
    assert_eq!(
        run_entry::<i64>(loaded.clone(), "go"),
        from_dir,
        "dir and bundle run identically"
    );

    // the closure mixed as declared: the linked pkgs compiled, the
    // declared-but-unused one a source group
    assert!(matches!(
        loaded.pkg("app").unwrap().body,
        MountedBody::Compiled(_)
    ));
    assert!(matches!(
        pkg_named(&loaded, "util").body,
        MountedBody::Compiled(_)
    ));
    assert!(matches!(
        pkg_named(&loaded, "boxy").body,
        MountedBody::Source { .. }
    ));
    let names: Vec<String> = rut_driver::bundle::parse_bundle(&bytes)
        .unwrap()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    let util_dir = dep_dir(&names, "util");
    let boxy_dir = dep_dir(&names, "boxy");
    for key in [
        "rut.jsonc",
        "rut.scopes",
        "mod.rutc",
        &format!("{util_dir}/mod.rutc"),
        &format!("{boxy_dir}/rut.jsonc"),
        &format!("{boxy_dir}/mod.rut"),
    ] {
        assert!(
            names.contains(&key.to_string()),
            "the bundle must carry `{key}`: {names:?}"
        );
    }
    assert!(
        !names.iter().any(|n| n == &format!("{boxy_dir}/mod.rutc")),
        "the unused group rides source, not a binary"
    );

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
    let mut loaded = rut_driver::Mounted::from_bundle(&bytes).expect("load");
    loaded.pkgs.push(
        rut_driver::Mounted::source(
            "consumer",
            "use boxy::{ Holder };\nuse util::{ twice };\n\n\
             entry fn go2() -> i64 {\n\
             \x20   let h = Holder<i64>::make(5);\n\
             \x20   return twice(h.get());\n\
             }\n",
        )
        // the consumer declares the bundle groups it names (the
        // nameability law — mounted-but-undeclared is D5)
        .manifest(rut_driver::Manifest::lib(
            "consumer",
            &["boxy", "util", "calc"],
        )),
    );
    // the CONSUMER is the root here — a source unit over the bundle's
    // mounted groups, compiled and linked exactly like a directory world
    let g = loaded
        .pkgs
        .iter()
        .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()))
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
    let mut vm = rut_vm::interp::Vm::builder()
        .program(std::rc::Rc::new(flat))
        .limits(limits)
        .hooks(rut_vm::interp::HostHooks::default())
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
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
        r#"{"format": "rutbundle", "format_version": 12, "name": "app"}"#,
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
    let names: Vec<String> = rut_driver::bundle::parse_bundle(&bytes)
        .unwrap()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert!(names.contains(&"mod.rutc".to_string()), "{names:?}");
    // v22 — the delivery law: generic bodies ride the binary as
    // template rows; NO source rides beside it (the riding law is dead)
    assert!(!names.contains(&"mod.rut".to_string()), "{names:?}");
    let run = |b: &[u8]| {
        let loaded = rut_driver::Mounted::from_bundle(b).expect("load");
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
        r#"{"format": "rutbundle", "format_version": 12, "name": "app"}"#,
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
    let loaded = rut_driver::Mounted::from_bundle(&bytes).expect("load");
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
    let err = rut_driver::Mounted::from_bundle(&rut_driver::bundle::write_bundle(&stale).unwrap())
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
    let err = rut_driver::Mounted::from_bundle(&corrupt)
        .unwrap_err()
        .to_string();
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
    let err =
        rut_driver::Mounted::from_bundle(&rut_driver::bundle::write_bundle(&doctored).unwrap())
            .unwrap_err()
            .to_string();
    assert!(err.contains("ledger") || err.contains("scope"), "{err}");
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
            rut_driver::Mounted::source("app", POUCH_APP)
                .manifest(rut_driver::Manifest::lib("app", &["pouch"])),
        )
        .entrypoint("app")
        .compile()
        .expect("reference compile");
    let ref_bytes =
        rut_core::binary::encode(&reference.graph.program.clone().expect("linked program"));
    assert!(!ref_bytes.is_empty());
    assert_eq!(run_compiled::<i32>(reference, "go"), 260);

    // L-ref sanity: the reference lane recompiles to identical bytes
    let recompiled = rut_driver::RutRun::new()
        .pkg(rut_native::tree_pkg("pouch").expect("walk rut/pouch"))
        .pkg(
            rut_driver::Mounted::source("app", POUCH_APP)
                .manifest(rut_driver::Manifest::lib("app", &["pouch"])),
        )
        .entrypoint("app")
        .compile()
        .expect("reference recompile");
    let ref_bytes_again =
        rut_core::binary::encode(&recompiled.graph.program.clone().expect("linked program"));
    assert_eq!(
        ref_bytes_again, ref_bytes,
        "the reference lane is deterministic"
    );

    // (b) the staged lane: pack the closure to an archive, walk +
    // mount it, compile the consumer against its surface
    let bytes = pack_dir(&pouch).expect("pack the closure");
    let archive = rut_driver::Mounted::from_bundle(&bytes).expect("decode the archive");
    assert_eq!(archive.root, "pouch");
    assert!(matches!(
        archive.pkg("pouch").unwrap().body,
        MountedBody::Compiled(_)
    ));
    let staged = archive
        .pkgs
        .iter()
        .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()))
        .pkg(
            rut_driver::Mounted::source("app", POUCH_APP)
                .manifest(rut_driver::Manifest::lib("app", &["pouch"])),
        )
        .entrypoint("app")
        .compile()
        .expect("staged compile");
    assert_eq!(
        run_compiled::<i32>(staged, "go"),
        260,
        "staged == reference"
    );
}

// ---- the substitution-shape matrix (the F0 falsifier, as a gate) ----

/// The matrix libs' shared preamble: a small heap class (`Box` — the
/// class-shaped T) and the probe's minimal one-slot container
/// (`Cell<T>` — the `?T` pattern). Every shape's lib carries both, so
/// the only per-cell variable is the shape's own generic — the exact
/// isolation the F0 probes ran.
const MATRIX_LIB_PREAMBLE: &str = r#"
pub class Box {
    v: i32;
}

impl Box {
    pub fn new(v: i32) -> Self {
        return Self { v: v };
    }
    pub fn get(self) -> i32 {
        return self.v;
    }
}

pub class Cell<T> {
    v: ?T;
}

impl<T> Cell<T> {
    pub fn put(v: T) -> Self {
        return Self { v: v };
    }
    pub fn get(self) -> T {
        return self.v;
    }
}
"#;

/// One substitution shape × one T-kind: the owner's shape generic
/// (`lib_tail`, appended to the preamble) and the consumer that
/// exercises it (the SAME source in both lanes), with the answer the
/// reference lane pins.
struct MatrixCell {
    tag: &'static str,
    lib_tail: &'static str,
    app: &'static str,
    expect: i32,
}

/// The 7 F0 shapes × 3 T-kinds (`i32` — value repr; `Box` — a heap
/// class; `opaque` — the erased builtin; both ref reprs). S1/S2/S5 are
/// the cells phase F0 measured DIVERGENT (the T→T ref move in a free
/// generic fn's body substituted at a value-repr T — the staged lane
/// retained the prim bits as a heap handle and aborted); the matrix
/// gates the repair: staged == reference on every cell.
const MATRIX: &[MatrixCell] = &[
    // S1 — the free generic identity: a T→T move through the RETURN
    MatrixCell {
        tag: "s01-ident-i32",
        lib_tail: "pub fn ident<T>(v: T) -> T {\n    return v;\n}\n",
        app: r#"
use lib::{ ident };

entry fn main() -> i32 {
    return ident<i32>(3);
}
"#,
        expect: 3,
    },
    MatrixCell {
        tag: "s01-ident-cls",
        lib_tail: "pub fn ident<T>(v: T) -> T {\n    return v;\n}\n",
        app: r#"
use lib::{ ident, Box };

entry fn main() -> i32 {
    let b = ident<Box>(Box::new(7));
    return b.get();
}
"#,
        expect: 7,
    },
    MatrixCell {
        tag: "s01-ident-opq",
        lib_tail: "pub fn ident<T>(v: T) -> T {\n    return v;\n}\n",
        app: r#"
use lib::{ ident };

entry fn main() -> i32 {
    let o = ident<opaque>(opaque(3));
    let ok = if (opaque::downcast<i32>(o) != nil) { 1 } else { 0 };
    return ok;
}
"#,
        expect: 1,
    },
    // S2 — a T→T move WITHOUT a return (the kept local)
    MatrixCell {
        tag: "s02-sink-i32",
        lib_tail: "pub fn sink<T>(v: T) -> nil {\n    let keep: T = v;\n    return;\n}\n",
        app: r#"
use lib::{ sink };

entry fn main() -> i32 {
    sink<i32>(3);
    return 1;
}
"#,
        expect: 1,
    },
    MatrixCell {
        tag: "s02-sink-cls",
        lib_tail: "pub fn sink<T>(v: T) -> nil {\n    let keep: T = v;\n    return;\n}\n",
        app: r#"
use lib::{ sink, Box };

entry fn main() -> i32 {
    sink<Box>(Box::new(7));
    return 1;
}
"#,
        expect: 1,
    },
    MatrixCell {
        tag: "s02-sink-opq",
        lib_tail: "pub fn sink<T>(v: T) -> nil {\n    let keep: T = v;\n    return;\n}\n",
        app: r#"
use lib::{ sink };

entry fn main() -> i32 {
    sink<opaque>(opaque(3));
    return 1;
}
"#,
        expect: 1,
    },
    // S3 — a free generic returning a class parameterized on T
    MatrixCell {
        tag: "s03-wrap-i32",
        lib_tail: "pub fn wrap<T>(v: T) -> Cell<T> {\n    let c: Cell<T> = Cell::put(v);\n    return c;\n}\n",
        app: r#"
use lib::{ Cell, wrap };

entry fn main() -> i32 {
    let c: Cell<i32> = wrap<i32>(3);
    return c.get();
}
"#,
        expect: 3,
    },
    MatrixCell {
        tag: "s03-wrap-cls",
        lib_tail: "pub fn wrap<T>(v: T) -> Cell<T> {\n    let c: Cell<T> = Cell::put(v);\n    return c;\n}\n",
        app: r#"
use lib::{ Cell, wrap, Box };

entry fn main() -> i32 {
    let c: Cell<Box> = wrap<Box>(Box::new(7));
    return c.get().get();
}
"#,
        expect: 7,
    },
    MatrixCell {
        tag: "s03-wrap-opq",
        lib_tail: "pub fn wrap<T>(v: T) -> Cell<T> {\n    let c: Cell<T> = Cell::put(v);\n    return c;\n}\n",
        app: r#"
use lib::{ Cell, wrap };

entry fn main() -> i32 {
    let c: Cell<opaque> = wrap<opaque>(opaque(3));
    let ok = if (opaque::downcast<i32>(c.get()) != nil) { 1 } else { 0 };
    return ok;
}
"#,
        expect: 1,
    },
    // S4 — a class METHOD returning T by value (the class shape that
    // rescued the value-T return in F0)
    MatrixCell {
        tag: "s04-mget-i32",
        lib_tail: "",
        app: r#"
use lib::{ Cell };

entry fn main() -> i32 {
    let c = Cell<i32>::put(3);
    return c.get();
}
"#,
        expect: 3,
    },
    MatrixCell {
        tag: "s04-mget-cls",
        lib_tail: "",
        app: r#"
use lib::{ Cell, Box };

entry fn main() -> i32 {
    let c = Cell<Box>::put(Box::new(7));
    return c.get().get();
}
"#,
        expect: 7,
    },
    MatrixCell {
        tag: "s04-mget-opq",
        lib_tail: "",
        app: r#"
use lib::{ Cell };

entry fn main() -> i32 {
    let c = Cell<opaque>::put(opaque(3));
    let ok = if (opaque::downcast<i32>(c.get()) != nil) { 1 } else { 0 };
    return ok;
}
"#,
        expect: 1,
    },
    // S5 — composition through two generic frames (inherits S1's move)
    MatrixCell {
        tag: "s05-comp-i32",
        lib_tail: "pub fn ident<T>(v: T) -> T {\n    return v;\n}\n\npub fn twice<T>(v: T) -> T {\n    return ident(ident(v));\n}\n",
        app: r#"
use lib::{ twice };

entry fn main() -> i32 {
    return twice<i32>(3);
}
"#,
        expect: 3,
    },
    MatrixCell {
        tag: "s05-comp-cls",
        lib_tail: "pub fn ident<T>(v: T) -> T {\n    return v;\n}\n\npub fn twice<T>(v: T) -> T {\n    return ident(ident(v));\n}\n",
        app: r#"
use lib::{ twice, Box };

entry fn main() -> i32 {
    let b = twice<Box>(Box::new(7));
    return b.get();
}
"#,
        expect: 7,
    },
    MatrixCell {
        tag: "s05-comp-opq",
        lib_tail: "pub fn ident<T>(v: T) -> T {\n    return v;\n}\n\npub fn twice<T>(v: T) -> T {\n    return ident(ident(v));\n}\n",
        app: r#"
use lib::{ twice };

entry fn main() -> i32 {
    let o = twice<opaque>(opaque(3));
    let ok = if (opaque::downcast<i32>(o) != nil) { 1 } else { 0 };
    return ok;
}
"#,
        expect: 1,
    },
    // S6 — store into a [?T]-backed container, read back through it
    MatrixCell {
        tag: "s06-stash-i32",
        lib_tail: r#"
pub class Slot<T> {
    buf: [?T];
    n: i32;
}

impl<T> Slot<T> {
    pub fn empty() -> Self {
        return Self { buf: [nil; 4], n: 0 };
    }
    pub fn put(mut self, v: T) -> nil {
        self.buf[self.n] = v;
        self.n += 1;
    }
    pub fn count(self) -> i32 {
        return self.n;
    }
    pub fn last(self) -> T {
        return self.buf[self.n - 1];
    }
}

pub fn stash<T>(v: T) -> Slot<T> {
    let mut s: Slot<T> = Slot::empty();
    s.put(v);
    s.put(v);
    return s;
}
"#,
        app: r#"
use lib::{ Slot, stash };

entry fn main() -> i32 {
    let s: Slot<i32> = stash<i32>(3);
    return s.count() * 10 + s.last();
}
"#,
        expect: 23,
    },
    MatrixCell {
        tag: "s06-stash-cls",
        lib_tail: r#"
pub class Slot<T> {
    buf: [?T];
    n: i32;
}

impl<T> Slot<T> {
    pub fn empty() -> Self {
        return Self { buf: [nil; 4], n: 0 };
    }
    pub fn put(mut self, v: T) -> nil {
        self.buf[self.n] = v;
        self.n += 1;
    }
    pub fn count(self) -> i32 {
        return self.n;
    }
    pub fn last(self) -> T {
        return self.buf[self.n - 1];
    }
}

pub fn stash<T>(v: T) -> Slot<T> {
    let mut s: Slot<T> = Slot::empty();
    s.put(v);
    s.put(v);
    return s;
}
"#,
        app: r#"
use lib::{ Slot, stash, Box };

entry fn main() -> i32 {
    let s: Slot<Box> = stash<Box>(Box::new(7));
    return s.count() * 10 + s.last().get();
}
"#,
        expect: 27,
    },
    MatrixCell {
        tag: "s06-stash-opq",
        lib_tail: r#"
pub class Slot<T> {
    buf: [?T];
    n: i32;
}

impl<T> Slot<T> {
    pub fn empty() -> Self {
        return Self { buf: [nil; 4], n: 0 };
    }
    pub fn put(mut self, v: T) -> nil {
        self.buf[self.n] = v;
        self.n += 1;
    }
    pub fn count(self) -> i32 {
        return self.n;
    }
    pub fn last(self) -> T {
        return self.buf[self.n - 1];
    }
}

pub fn stash<T>(v: T) -> Slot<T> {
    let mut s: Slot<T> = Slot::empty();
    s.put(v);
    s.put(v);
    return s;
}
"#,
        app: r#"
use lib::{ Slot, stash };

entry fn main() -> i32 {
    let s: Slot<opaque> = stash<opaque>(opaque(3));
    let ok = if (opaque::downcast<i32>(s.last()) != nil) { 1 } else { 0 };
    return s.count() * 10 + ok;
}
"#,
        expect: 21,
    },
    // S7 — T arriving NESTED (a Cell<T> argument), returned by value
    MatrixCell {
        tag: "s07-head-i32",
        lib_tail: "pub fn head<T>(xs: Cell<T>) -> T {\n    return xs.get();\n}\n",
        app: r#"
use lib::{ Cell, head };

entry fn main() -> i32 {
    let c = Cell<i32>::put(3);
    return head<i32>(c);
}
"#,
        expect: 3,
    },
    MatrixCell {
        tag: "s07-head-cls",
        lib_tail: "pub fn head<T>(xs: Cell<T>) -> T {\n    return xs.get();\n}\n",
        app: r#"
use lib::{ Cell, head, Box };

entry fn main() -> i32 {
    let c = Cell<Box>::put(Box::new(7));
    return head<Box>(c).get();
}
"#,
        expect: 7,
    },
    MatrixCell {
        tag: "s07-head-opq",
        lib_tail: "pub fn head<T>(xs: Cell<T>) -> T {\n    return xs.get();\n}\n",
        app: r#"
use lib::{ Cell, head };

entry fn main() -> i32 {
    let c = Cell<opaque>::put(opaque(3));
    let ok = if (opaque::downcast<i32>(head<opaque>(c)) != nil) { 1 } else { 0 };
    return ok;
}
"#,
        expect: 1,
    },
];

/// The matrix, as a gate: every F0 shape × T-kind cell compiles
/// through BOTH lanes — (a) the reference whole-program lane (the
/// closure as sources, one graph) and (b) the staged lane (the owner
/// packed to an archive, mounted, the consumer compiled against its
/// surface, instantiating from the templates) — and the lanes agree on
/// the run result. The three cells F0 measured divergent (S1/S2/S5 at
/// T=i32: the template's T→T `MovRef` survived substitution and
/// retained the prim bits as a heap handle) are the ones this law was
/// written for; the other eighteen keep it honest.
#[test]
fn staged_and_reference_lanes_agree_across_the_substitution_matrix() {
    for cell in MATRIX {
        let lib_text = format!("{}\n{}", MATRIX_LIB_PREAMBLE, cell.lib_tail);
        // (a) the reference lane: the closure as sources, one graph
        let reference = rut_driver::RutRun::new()
            .pkg(rut_driver::Mounted::source("lib", &lib_text))
            .pkg(
                rut_driver::Mounted::source("app", cell.app)
                    .manifest(rut_driver::Manifest::lib("app", &["lib"])),
            )
            .entrypoint("app")
            .compile()
            .unwrap_or_else(|e| panic!("{}: reference compile: {e:?}", cell.tag));
        assert!(
            reference.graph.diags.is_empty(),
            "{}: reference diags: {:?}",
            cell.tag,
            reference.graph.diags
        );
        let from_dir = run_compiled::<i32>(reference, "main");
        assert_eq!(
            from_dir, cell.expect,
            "{}: the reference lane's own answer",
            cell.tag
        );

        // (b) the staged lane: pack the owner, mount the archive, serve
        // the consumer's instantiation from the templates
        let dir = scratch(cell.tag);
        write(&dir, "rut.jsonc", &bundle_manifest("lib", "mod.rut", ""));
        write(&dir, "mod.rut", &lib_text);
        let bytes = pack_dir(&dir).unwrap_or_else(|e| panic!("{}: pack: {e:?}", cell.tag));
        let _ = std::fs::remove_dir_all(&dir);
        let archive = rut_driver::Mounted::from_bundle(&bytes)
            .unwrap_or_else(|e| panic!("{}: load: {e:?}", cell.tag));
        let staged = archive
            .pkgs
            .iter()
            .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()))
            .pkg(
                rut_driver::Mounted::source("app", cell.app)
                    .manifest(rut_driver::Manifest::lib("app", &["lib"])),
            )
            .entrypoint("app")
            .compile()
            .unwrap_or_else(|e| panic!("{}: staged compile: {e:?}", cell.tag));
        assert!(
            staged.graph.diags.is_empty(),
            "{}: staged diags: {:?}",
            cell.tag,
            staged.graph.diags
        );
        assert_eq!(
            run_compiled::<i32>(staged, "main"),
            from_dir,
            "{}: staged == reference (the L-eq law)",
            cell.tag
        );
    }
}

// ---- the interface-dispatch matrix (the T2 gates) ----
//
// rut mints one interface row per exact type-argument tuple
// (`mk_iface_inst`), so a generic surface's interface operations ride
// the PLACEHOLDER row (`Readable<#T>`), while the consumer's fills and
// dispatches speak the CONCRETE row (`Readable<i32>`). The materializer
// must re-target the template's `CallI` slots and `IsIface` rows
// (T2a — i04/i05 below trapped at the placeholder row's slot before
// the repair) and lay the (payload × iface) fills a fresh compile's
// check phase would demand at its widens — no check phase runs over a
// materialization, and interface widening emits NO op (`widen_to_slot`
// is a no-op: the cell handle flows, dispatch reads the cell's own
// type), so the body's spoken types are the pairing's only witness
// (T2b — i04 trapped at a CORRECT slot on an unfilled row before the
// repair). i01/i02/i03 are the controls: app-side boxing with a
// visible surface, and the pack-time devirtualization of a
// box-then-dispatch on a freshly-built value (no CallI rides the
// template at all).

/// The interface family's shared preamble: a generic interface, the
/// generic class that satisfies it structurally, and a non-generic
/// dispatch sink (its `CallI` names the CONCRETE row — the control's
/// proof that pack-compiled dispatch is slot-correct as packed).
const IFACE_LIB_PREAMBLE: &str = r#"
pub interface Readable<T> {
  fn get(self) -> T;
  fn tag(self) -> i64;
}

pub class Holder<T> {
  v: T;
}

impl<T> Holder<T> {
  [constructor] pub fn of(v: T) -> Self {
    return Self { v: v };
  }
  pub fn get(self) -> T {
    return self.v;
  }
  pub fn tag(self) -> i64 {
    return 7;
  }
}

pub fn drive(r: Readable<i32>) -> i32 {
  return r.get() + 1000;
}
"#;

const IFACE_MATRIX: &[MatrixCell] = &[
    // i01 — the control: the app boxes at a call arg (a VISIBLE
    // surface — the app's own check demands the fill), the dispatch
    // rides the DECODED non-generic kit body
    MatrixCell {
        tag: "i01-appbox-kitdispatch",
        lib_tail: "",
        app: r#"
use lib::{ Holder, drive };

entry fn main() -> i32 {
    return drive(Holder<i32>::of(3));
}
"#,
        expect: 1003,
    },
    // i02 — the control: boxing and dispatch both in the app body
    MatrixCell {
        tag: "i02-appbox-appdispatch",
        lib_tail: "",
        app: r#"
use lib::{ Holder, Readable };

entry fn main() -> i32 {
    let r: Readable<i32> = Holder<i32>::of(4);
    return r.get() + 100;
}
"#,
        expect: 104,
    },
    // i03 — the control: box + dispatch inside the template — the
    // pack-time check devirtualizes the freshly-boxed value's call
    // into a direct call, so no CallI rides the materialized body
    MatrixCell {
        tag: "i03-kitbox-devirt",
        lib_tail: r#"
pub fn use_it<T>(v: T) -> i64 {
    let r: Readable<T> = Holder<T>::of(v);
    return r.tag();
}
"#,
        app: r#"
use lib::{ use_it };

entry fn main() -> i32 {
    return use_it<i32>(3) as i32;
}
"#,
        expect: 7,
    },
    // i04 — the T2b gate: the boxing rides the MATERIALIZED body (the
    // kit generic returns the interface; the app only annotates), so
    // the (Holder<i32> × Readable<i32>) fill is demanded by no check
    // phase — the materialization itself must lay it. Trapped at a
    // CORRECT slot on an unfilled row before the repair.
    MatrixCell {
        tag: "i04-kitgen-boxfill",
        lib_tail: r#"
pub fn make<T>(v: T) -> Readable<T> {
    return Holder<T>::of(v);
}
"#,
        app: r#"
use lib::{ make, Readable };

entry fn main() -> i32 {
    let r: ?Readable<i32> = make<i32>(5);
    if (r == nil) {
        return -1;
    }
    return r.get() + 100;
}
"#,
        expect: 105,
    },
    // i05 — the T2a gate: the dispatch rides the MATERIALIZED body (a
    // generic interface PARAM — unprovable payload, a true CallI),
    // baked against the placeholder row at pack. Trapped at the
    // placeholder row's slot before the repair.
    MatrixCell {
        tag: "i05-kitdispatch-on-appbox",
        lib_tail: r#"
pub fn consume<T>(r: Readable<T>) -> T {
    return r.get();
}
"#,
        app: r#"
use lib::{ Holder, consume };

entry fn main() -> i32 {
    return consume<i32>(Holder<i32>::of(9));
}
"#,
        expect: 9,
    },
    // i06 — two type arguments: the row re-spelling must substitute
    // every argument slot of the name (`Pair<i32, i64>`), not just the
    // first
    MatrixCell {
        tag: "i06-two-arg-row",
        lib_tail: r#"
pub interface Pair<A, B> {
  fn first(self) -> A;
  fn second(self) -> B;
}

pub class PairBox<A, B> {
  a: A;
  b: B;
}

impl<A, B> PairBox<A, B> {
  [constructor] pub fn of(a: A, b: B) -> Self {
    return Self { a: a, b: b };
  }
  pub fn first(self) -> A {
    return self.a;
  }
  pub fn second(self) -> B {
    return self.b;
  }
}

pub fn pick2<T, U>(p: Pair<T, U>) -> U {
    return p.second();
}
"#,
        app: r#"
use lib::{ PairBox, pick2 };

entry fn main() -> i32 {
    let b: PairBox<i32, i64> = PairBox<i32, i64>::of(3, 9);
    return pick2<i32, i64>(b) as i32;
}
"#,
        expect: 9,
    },
];

/// The interface-dispatch matrix, as a gate: every cell compiles
/// through BOTH lanes (the reference whole-program lane and the staged
/// pack → bundle → consume lane) and the lanes agree on the run
/// result — the same L-eq law as the substitution matrix, armed for
/// the shape that matrix never covered (an interface dispatch touching
/// a consumer-instantiated generic).
#[test]
fn staged_and_reference_lanes_agree_across_the_interface_dispatch_matrix() {
    for cell in IFACE_MATRIX {
        let lib_text = format!("{IFACE_LIB_PREAMBLE}\n{}", cell.lib_tail);
        // (a) the reference lane: the closure as sources, one graph
        let reference = rut_driver::RutRun::new()
            .pkg(rut_driver::Mounted::source("lib", &lib_text))
            .pkg(
                rut_driver::Mounted::source("app", cell.app)
                    .manifest(rut_driver::Manifest::lib("app", &["lib"])),
            )
            .entrypoint("app")
            .compile()
            .unwrap_or_else(|e| panic!("{}: reference compile: {e:?}", cell.tag));
        assert!(
            reference.graph.diags.is_empty(),
            "{}: reference diags: {:?}",
            cell.tag,
            reference.graph.diags
        );
        let from_dir = run_compiled::<i32>(reference, "main");
        assert_eq!(
            from_dir, cell.expect,
            "{}: the reference lane's own answer",
            cell.tag
        );

        // (b) the staged lane: pack the owner, mount the archive,
        // serve the consumer's instantiation from the templates
        let dir = scratch(cell.tag);
        write(&dir, "rut.jsonc", &bundle_manifest("lib", "mod.rut", ""));
        write(&dir, "mod.rut", &lib_text);
        let bytes = pack_dir(&dir).unwrap_or_else(|e| panic!("{}: pack: {e:?}", cell.tag));
        let _ = std::fs::remove_dir_all(&dir);
        let archive = rut_driver::Mounted::from_bundle(&bytes)
            .unwrap_or_else(|e| panic!("{}: load: {e:?}", cell.tag));
        let staged = archive
            .pkgs
            .iter()
            .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()))
            .pkg(
                rut_driver::Mounted::source("app", cell.app)
                    .manifest(rut_driver::Manifest::lib("app", &["lib"])),
            )
            .entrypoint("app")
            .compile()
            .unwrap_or_else(|e| panic!("{}: staged compile: {e:?}", cell.tag));
        assert!(
            staged.graph.diags.is_empty(),
            "{}: staged diags: {:?}",
            cell.tag,
            staged.graph.diags
        );
        assert_eq!(
            run_compiled::<i32>(staged, "main"),
            from_dir,
            "{}: staged == reference (the L-eq law)",
            cell.tag
        );
    }
}

/// The interface-row find-or-mint is deterministic: minting order is
/// first-touch through the request-driven materialization cascade, so
/// the same (bundle, app) pair links to byte-identical merged binaries
/// across independent sessions — the lane's reproducibility law, with
/// the T2 shapes in play (minted rows, re-targeted slots, laid fills).
#[test]
fn staged_iface_materialization_is_deterministic() {
    let lib_text = format!(
        "{IFACE_LIB_PREAMBLE}\n{}",
        r#"
pub fn make<T>(v: T) -> Readable<T> {
    return Holder<T>::of(v);
}

pub fn consume<T>(r: Readable<T>) -> T {
    return r.get();
}
"#
    );
    let app = r#"
use lib::{ Holder, make, consume, Readable };

entry fn main() -> i32 {
    let r: ?Readable<i32> = make<i32>(5);
    if (r == nil) {
        return -1;
    }
    return consume<i32>(Holder<i32>::of(r.get()));
}
"#;
    let dir = scratch("iface-det");
    write(&dir, "rut.jsonc", &bundle_manifest("lib", "mod.rut", ""));
    write(&dir, "mod.rut", &lib_text);
    let bytes = pack_dir(&dir).expect("pack");
    let _ = std::fs::remove_dir_all(&dir);

    let mut encodings: Vec<Vec<u8>> = Vec::new();
    for _ in 0..2 {
        let archive = rut_driver::Mounted::from_bundle(&bytes).expect("load");
        let staged = archive
            .pkgs
            .iter()
            .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()))
            .pkg(
                rut_driver::Mounted::source("app", app)
                    .manifest(rut_driver::Manifest::lib("app", &["lib"])),
            )
            .entrypoint("app")
            .compile()
            .expect("staged compile");
        assert!(staged.graph.diags.is_empty(), "{:?}", staged.graph.diags);
        encodings.push(rut_core::binary::encode(
            &staged.graph.program.clone().expect("linked program"),
        ));
    }
    assert_eq!(
        encodings[0], encodings[1],
        "two independent staged sessions link byte-identically"
    );
}
