//! v5 compiled bundles: the mixed closure — linkable pkgs ride as
//! `.rutc` binaries (bodies + surface, the linking truth), splice-needed
//! deps (inline / generic export / trait-object params) and host pkgs
//! ride as source file sets — plus the pinned equivalence (compiled
//! bundle vs source directory ⇒ identical linked binaries), determinism,
//! and the refusal matrix.
//!
//! The v2/v3 load scenarios this file succeeds (the dep-bundle
//! round trips, the peer groups through an archive, D1) re-launch here
//! on v5: `rut pack` emits v5 only, and the loader reads v5 only —
//! older layouts are refused with the one-line version error.

use std::path::{Path, PathBuf};

use rut_driver::{
    compile_graph, load_bundle_bytes, load_dir_session, mount_std, pack_dir, ModuleBody,
};

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
    format!("name = \"{name}\"\nentry.lib = \"./{entry}\"\n{extra}")
}

/// The mixed-closure world: `app` (linkable root) uses `util` (linkable
/// → compiled group) and declares `boxy` (generic export → source
/// group). The law chains transitively, so a package that USES a
/// generic splices it and itself becomes splice-needed — the clean
/// mixed shape is a generic dep the linkable part of the tree does not
/// absorb, which still rides (declared deps publish completely) as a
/// source group a consumer can splice.
fn mixed_world(tag: &str) -> PathBuf {
    let root = scratch(tag);
    let app = root.join("app");
    write(
        &app,
        "rut.toml",
        &format!(
            "format = \"rutbundle\"\nformat_version = 5\n{}",
            manifest("app", "app.rut", "[deps]\nutil = { path = \"../util\" }\nboxy = { path = \"../boxy\" }\n")
        ),
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
    write(&util, "rut.toml", &manifest("util", "util.rut", ""));
    write(
        &util,
        "util.rut",
        "pub fn twice(v: i64) -> i64 {\n\
         \x20   return v * 2;\n\
         }\n",
    );
    let boxy = root.join("boxy");
    write(&boxy, "rut.toml", &manifest("boxy", "boxy.rut", ""));
    write(
        &boxy,
        "boxy.rut",
        "pub class Holder<T> {\n\
         \x20   v: T;\n\
         }\n\n\
         impl Holder<T> {\n\
         \x20   pub fn make(v: T) -> Self { return Self { v: v }; }\n\
         \x20   pub fn get(self) -> T { return self.v; }\n\
         }\n",
    );
    root
}

fn linked_binary(session: &rut_driver::Session, root: &str) -> Vec<u8> {
    let g = compile_graph(session, root);
    assert!(g.diags.is_empty(), "{:?}", g.diags);
    rut_core::binary::encode(&g.program.expect("linked program"))
}

fn run_entry<R: rut_vm::interp::Ret>(session: rut_driver::Session, root: &str, entry: &str) -> R {
    let mut session = session;
    mount_std(&mut session);
    let g = compile_graph(&session, root);
    assert!(g.diags.is_empty(), "{:?}", g.diags);
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

#[test]
fn mixed_closure_pack_load_run_equals_the_directory() {
    let root = mixed_world("mixed");
    // THE pinned equivalence: the compiled bundle and its source
    // directory link to the identical binary
    let (dir_session, dir_root) = load_dir_session(&root.join("app")).expect("dir load");
    let from_dir = linked_binary(&dir_session, &dir_root);

    let bytes = pack_dir(&root.join("app")).expect("pack");
    let (session, app_root) = load_bundle_bytes(&bytes, Path::new("mem")).expect("load");
    assert_eq!(app_root, "app");
    assert_eq!(linked_binary(&session, &app_root), from_dir, "dir and bundle compile identically");

    // the closure mixed as declared: the linkable pkgs compiled, the
    // generic one a source group
    assert!(matches!(
        session.resolve("app").unwrap().body,
        ModuleBody::Compiled(_)
    ));
    assert!(matches!(
        session.resolve("util").unwrap().body,
        ModuleBody::Compiled(_)
    ));
    assert!(matches!(
        session.resolve("boxy").unwrap().body,
        ModuleBody::Source { .. }
    ));
    let names: Vec<String> =
        rut_bundle::parse_bundle(&bytes).unwrap().into_iter().map(|(n, _)| n).collect();
    for key in ["rut.toml", "rut.scopes", "app.rutc", "util/util.rutc", "boxy/rut.toml", "boxy/boxy.rut"] {
        assert!(names.contains(&key.to_string()), "the bundle must carry `{key}`: {names:?}");
    }
    assert!(!names.iter().any(|n| n == "boxy/boxy.rutc"), "the generic pkg rides source, not a binary");

    // the ledger names every linked module of the packed closure
    let ledger = rut_bundle::parse_bundle(&bytes).unwrap();
    let ledger = ledger.iter().find(|(n, _)| n == "rut.scopes").unwrap();
    let ledger = std::str::from_utf8(&ledger.1).unwrap();
    assert!(ledger.contains("= \"app\""), "{ledger}");
    assert!(ledger.contains("= \"util\""), "{ledger}");

    // and the mixed world dispatches: the compiled root answers
    let got: i64 = run_entry(session, &app_root, "go");
    assert_eq!(got, 42);
}

#[test]
fn a_source_group_splices_into_a_consumer_session() {
    // the mixed closure's payoff: a consumer compiled AGAINST the
    // loaded bundle splices the source group (the generic pkg) and
    // binds the compiled groups' surfaces — the two kinds meet in one
    // session, no directory in sight
    let root = mixed_world("splice");
    let bytes = pack_dir(&root.join("app")).expect("pack");
    let (mut session, app_root) = load_bundle_bytes(&bytes, Path::new("mem")).expect("load");
    mount_std(&mut session);
    session
        .register_module(
            "consumer",
            rut_driver::Module {
                body: ModuleBody::Source {
                    text: "use boxy::{ Holder };\nuse util::{ twice };\n\n\
                           entry fn go2() -> i64 {\n\
                           \x20   let h = Holder<i64>.make(5);\n\
                           \x20   return twice(h.get());\n\
                           }\n"
                        .into(),
                    is_decl: false,
                },
                ..Default::default()
            },
        )
        .unwrap();
    // the CONSUMER is the root here — a source unit over the bundle's
    // mounted groups, compiled and linked exactly like a directory world
    let g = rut_driver::compile_graph(&session, "consumer");
    assert!(g.diags.is_empty(), "{:?}", g.diags);
    let flat = rut_core::link::flatten(g.program.expect("linked"));
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
    assert_eq!(vm.call::<_, i64>("go2", ()).expect("run"), 10);
    let _ = app_root;
}

#[test]
fn same_dir_packs_byte_identical() {
    let root = mixed_world("det");
    let a = pack_dir(&root.join("app")).unwrap();
    let b = pack_dir(&root.join("app")).unwrap();
    assert_eq!(a, b, "same dir => byte-identical v5 bundle");
}

#[test]
fn unsharable_roots_are_refused() {
    // the exact refusal, verbatim, for each splice-law input
    let want = "exports generic types / takes trait-object params / is inline — it cannot be published compiled; share the directory instead";

    // a generic export
    let root = scratch("genroot");
    let app = root.join("app");
    write(
        &app,
        "rut.toml",
        "format = \"rutbundle\"\nformat_version = 5\nname = \"app\"\nentry.lib = \"./app.rut\"\n",
    );
    write(
        &app,
        "app.rut",
        "pub class G<T> { v: T; }\n\nimpl G<T> { pub fn mk(v: T) -> Self { return Self { v: v }; } }\n\nentry fn go() -> i64 { let g = G<i64>.mk(1); return 0; }\n",
    );
    let err = pack_dir(&app).unwrap_err();
    assert!(err.contains(&format!("pack: app {want}")), "{err}");

    // an inline manifest flag
    let root = scratch("inlineroot");
    let app = root.join("app");
    write(
        &app,
        "rut.toml",
        "format = \"rutbundle\"\nformat_version = 5\nname = \"app\"\nentry.lib = \"./app.rut\"\ninline = true\n",
    );
    write(&app, "app.rut", "pub fn f() -> i32 { return 1; }\n");
    let err = pack_dir(&app).unwrap_err();
    assert!(err.contains(&format!("pack: app {want}")), "{err}");

    // a trait-object parameter
    let root = scratch("traitroot");
    let app = root.join("app");
    write(
        &app,
        "rut.toml",
        "format = \"rutbundle\"\nformat_version = 5\nname = \"app\"\nentry.lib = \"./app.rut\"\n",
    );
    write(
        &app,
        "app.rut",
        "pub trait Shape { fn area(self) -> i32; }\npub fn draw(s: Shape) -> i32 { return s.area(); }\n",
    );
    let err = pack_dir(&app).unwrap_err();
    assert!(err.contains(&format!("pack: app {want}")), "{err}");

    // a host pkg has nothing to compile — the same refusal family
    let root = scratch("hostroot");
    let app = root.join("app");
    write(
        &app,
        "rut.toml",
        "format = \"rutbundle\"\nformat_version = 5\nname = \"app\"\nentry.type = \"./app.d.rut\"\n",
    );
    write(&app, "app.d.rut", "pub host fn f() -> i32;\n");
    let err = pack_dir(&app).unwrap_err();
    assert!(err.contains("cannot be published compiled"), "{err}");
}

#[test]
fn peer_deps_on_a_compiled_group_is_a_pack_refusal() {
    // a linkable dep that declares `[peer-deps]` lib files: appending
    // source into a compiled pkg is impossible (the v1-precedent
    // refusal) — the pack refuses instead of downgrading
    let root = scratch("peerref");
    let app = root.join("app");
    write(
        &app,
        "rut.toml",
        &format!(
            "format = \"rutbundle\"\nformat_version = 5\n{}",
            manifest("app", "app.rut", "[deps]\nlibbed = { path = \"../libbed\" }\n")
        ),
    );
    write(&app, "app.rut", "use libbed::{ f };\nentry fn go() -> i32 { return f(); }\n");
    let libbed = root.join("libbed");
    write(
        &libbed,
        "rut.toml",
        &manifest(
            "libbed",
            "libbed.rut",
            "[peer-deps]\np = { path = \"../p\", optional = true, lib = \"./ser_p.rut\" }\n",
        ),
    );
    write(&libbed, "libbed.rut", "pub fn f() -> i32 { return 7; }\n");
    write(&libbed, "ser_p.rut", "// an impl-only group file\n");
    write(&root.join("p"), "rut.toml", &manifest("p", "p.rut", ""));
    write(&root.join("p"), "p.rut", "pub class K { x: i32; }\n");

    let err = pack_dir(&app).unwrap_err();
    assert!(err.contains("libbed"), "{err}");
    assert!(err.contains("[peer-deps]"), "{err}");
    assert!(err.contains("cannot be published compiled"), "{err}");
}

/// The T2 world, re-launched on v5: the peer-gated pkg is INLINE
/// (concrete exports, the real `json` shape), so it rides as a SOURCE
/// group — group file included — and the loader's peer gate appends by
/// presence. Its consumers splice the inline pkg's text, so they stay
/// linkable: the root publishes compiled.
#[test]
fn peer_groups_ride_v5_as_source_groups() {
    let root = scratch("peers");

    // peered: inline pkg + the trait + the peer-gated impl group
    let peered = root.join("peered");
    write(
        &peered,
        "rut.toml",
        &manifest(
            "peered",
            "peered.rut",
            "inline = true\n\n[peer-deps]\ntagger = { path = \"../tagger\", optional = true, lib = \"./ser_tag.rut\" }\n",
        ),
    );
    write(
        &peered,
        "peered.rut",
        "pub trait Tag { fn tag(self) -> str; }\n\n\
         impl Tag for str {\n\
         \x20   fn tag(self) -> str { return self; }\n\
         }\n",
    );
    write(
        &peered,
        // the group file carries its own use: the impl's target joins
        // the declarer's unit through the ordinary scan of the combined
        // source (the orphan rule holds — one unit, one text)
        "ser_tag.rut",
        "use tagger::{ Badge };\n\nimpl Tag for Badge { fn tag(self) -> str { return \"badged\"; } }\n",
    );

    // the peer: a concrete INLINE class (spliced beside the group's
    // impl — one unit, the orphan rule holds; the real json's peers are
    // inline the same way)
    let tagger = root.join("tagger");
    write(&tagger, "rut.toml", &manifest("tagger", "tagger.rut", "inline = true\n"));
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
    write(&base, "rut.toml", &manifest("base", "base.rut", ""));
    write(&base, "base.rut", "pub fn b() -> i32 { return 7; }\n");

    // the consumer: all three in [deps]; the group's impl dispatches
    let app = root.join("app");
    write(
        &app,
        "rut.toml",
        &format!(
            "format = \"rutbundle\"\nformat_version = 5\n{}",
            manifest("app", "app.rut", "[deps]\npeered = { path = \"../peered\" }\ntagger = { path = \"../tagger\" }\nbase = { path = \"../base\" }\n")
        ),
    );
    write(
        &app,
        "app.rut",
        "use peered::{ Tag };\nuse tagger::{ Badge };\nuse base::{ b };\n\n\
         entry fn go() -> str {\n\
         \x20   let b2 = Badge.new();\n\
         \x20   let _ = b();\n\
         \x20   return b2.tag();\n\
         }\n",
    );

    // the directory world first (the load-time peer gate appends there)
    let (dir_session, dir_root) = load_dir_session(&app).expect("dir load");
    let from_dir = linked_binary(&dir_session, &dir_root);

    let bytes = pack_dir(&app).expect("pack");
    let (session, app_root) = load_bundle_bytes(&bytes, Path::new("mem")).expect("load");
    assert_eq!(linked_binary(&session, &app_root), from_dir, "dir and bundle compile identically");

    // peered rode as a source group WITH its group file; the loader's
    // peer gate appended it by presence (tagger is in the closure)
    assert!(matches!(session.resolve("peered").unwrap().body, ModuleBody::Source { .. }));
    let names: Vec<String> =
        rut_bundle::parse_bundle(&bytes).unwrap().into_iter().map(|(n, _)| n).collect();
    assert!(names.contains(&"peered/ser_tag.rut".to_string()), "{names:?}");
    assert!(names.contains(&"base/base.rutc".to_string()), "{names:?}");
    assert!(names.contains(&"tagger/rut.toml".to_string()), "{names:?}");

    // and the group's impl dispatches
    let got: String = run_entry(session, &app_root, "go");
    assert_eq!(got, "badged");
}

#[test]
fn required_peer_absent_is_d1_at_pack() {
    let root = scratch("d1");
    let app = root.join("app");
    write(
        &app,
        "rut.toml",
        &format!(
            "format = \"rutbundle\"\nformat_version = 5\n{}",
            manifest("app", "app.rut", "[deps]\nreq = { path = \"../req\" }\n")
        ),
    );
    write(&app, "app.rut", "use req::{ f };\nentry fn go() -> i32 { return f(); }\n");
    let req = root.join("req");
    write(
        &req,
        "rut.toml",
        &manifest("req", "req.rut", "[peer-deps]\nmissing = { path = \"../missing\" }\n"),
    );
    write(&req, "req.rut", "pub fn f() -> i32 { return 7; }\n");
    let err = pack_dir(&app).unwrap_err();
    assert!(err.contains("requires the peer `missing`"), "{err}");
    assert!(err.contains("peers are not pulled transitively"), "{err}");
}

#[test]
fn stale_and_corrupt_group_binaries_are_refused() {
    let root = mixed_world("refuse");
    let bytes = pack_dir(&root.join("app")).expect("pack");

    // a pre-v16 `.rutc` (no surface section) inside the archive: the
    // decode gate refuses it loudly, before any mount
    let stale: Vec<(String, Vec<u8>)> = rut_bundle::parse_bundle(&bytes)
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
    let err = load_bundle_bytes(&rut_bundle::write_bundle(&stale).unwrap(), Path::new("stale"))
        .unwrap_err();
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
    let err = load_bundle_bytes(&corrupt, Path::new("crc")).unwrap_err();
    assert!(err.contains("CRC"), "{err}");

    // a doctored scope ledger: the group's binary carries its own
    // scope, and the ledger row must AGREE with it (refuse, never
    // guess — a mismatch is a corrupt or doctored bundle)
    let doctored: Vec<(String, Vec<u8>)> = rut_bundle::parse_bundle(&bytes)
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
        load_bundle_bytes(&rut_bundle::write_bundle(&doctored).unwrap(), Path::new("ledger"))
            .unwrap_err();
    assert!(
        err.contains("ledger") || err.contains("scope"),
        "{err}"
    );
}

#[test]
fn doctored_source_group_compiled_kind_refuses_at_load() {
    // the loader-side twin of the pack refusal: a hand-doctored archive
    // whose COMPILED group declares a peer lib cannot mount — appending
    // source into a compiled pkg is impossible
    let root = mixed_world("doctored");
    let bytes = pack_dir(&root.join("app")).expect("pack");
    let doctored: Vec<(String, Vec<u8>)> = rut_bundle::parse_bundle(&bytes)
        .unwrap()
        .into_iter()
        .map(|(n, b)| match n.as_str() {
            // util is a compiled group: give it a peer lib row
            "util/rut.toml" => (
                n,
                b"name = \"util\"\nentry.lib = \"./util.rut\"\n\n[peer-deps]\np = { path = \"../p\", optional = true, lib = \"./ser_p.rut\" }\n"
                    .to_vec(),
            ),
            _ => (n, b),
        })
        .collect();
    let mut doctored = doctored;
    doctored.push(("util/ser_p.rut".into(), b"// an impl-only group\n".to_vec()));
    let err =
        load_bundle_bytes(&rut_bundle::write_bundle(&doctored).unwrap(), Path::new("doctored"))
            .unwrap_err();
    assert!(err.contains("compiled group"), "{err}");
    assert!(err.contains("util"), "{err}");
    assert!(err.contains("peer-deps"), "{err}");
}
