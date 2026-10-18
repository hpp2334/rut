//! The intent-level public API (L6): the offered world is bundles +
//! host bodies, over ONE verb — `RutRun::pkg(..)` and the
//! [`rut_driver::Offer`] classification. The archive handle
//! ([`rut_driver::Bundle`]) decodes + verifies once and relocates +
//! mounts per world; `pack_inline_pkg` assembles ephemeral source
//! without compiling; strip restore lives on the handle; the retired
//! names keep working one release under `#[deprecated]`.

use std::path::Path;

use rut_driver::Mounted;
use rut_native::pack_dir;

fn scratch(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("rut-intent-{tag}-{}", std::process::id()));
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

/// `util` — a linkable leaf pkg — plus an `app` that uses it: the
/// smallest compiled archive with a dep group.
fn util_world(tag: &str) -> std::path::PathBuf {
    let root = scratch(tag);
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 11, "name": "app", "deps": {"util": {"path": "../util"}}}"#,
    );
    write(
        &app,
        "mod.rut",
        "use util::{ twice };\n\nentry fn go() -> i64 {\n    return twice(21);\n}\n",
    );
    let util = root.join("util");
    write(&util, "rut.jsonc", r#"{"name": "util"}"#);
    write(&util, "mod.rut", "pub fn twice(v: i64) -> i64 {\n    return v * 2;\n}\n");
    root
}

/// The flat text of the linked root program — the lane-equality probe
/// (dir-compiled vs archive-mounted must spell the same program).
fn linked_binary(compiled: &rut_driver::Compiled) -> Vec<u8> {
    rut_core::binary::encode(
        &rut_core::link::flatten(compiled.graph.program.clone().expect("linked")),
    )
}

/// One run over the decoded handle: parse once, offer many. Two runs
/// over ONE `Bundle` compile byte-identically, and the archive lane
/// RUNS like the dir lane — the decode happened once, the relocation
/// per world.
#[test]
fn bundle_parse_once_offers_many_across_runs() {
    let root = util_world("reuse");
    let dir = root.join("app");
    let bytes = pack_dir(&dir).expect("pack the closure");
    let bundle = rut_driver::Bundle::parse(&bytes).expect("decode + verify once");

    let dir_run = rut_native::load_dir(&dir)
        .expect("walk the dir")
        .pkgs
        .iter()
        .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()))
        .entrypoint("app")
        .compile()
        .expect("the dir lane compiles");

    let mut last: Option<Vec<u8>> = None;
    for _ in 0..2 {
        let compiled = rut_driver::RutRun::new()
            .pkg(&bundle) // Offer::Lib — the ONE verb over the handle
            .entrypoint("app")
            .compile()
            .expect("the archive lane compiles");
        let bin = linked_binary(&compiled);
        if let Some(prev) = &last {
            assert_eq!(&bin, prev, "each run of one handle is byte-identical");
        }
        last = Some(bin);
    }
    // and the archive lane answers what the dir lane answers
    assert_eq!(
        run_entry(last.unwrap()),
        run_entry(linked_binary(&dir_run)),
        "archive == dir, run for run"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Boot a linked binary and call its entry — the run-behavior probe.
fn run_entry(binary: Vec<u8>) -> i64 {
    let flat = rut_core::link::flatten(
        rut_core::binary::decode(&binary).expect("decode the linked binary"),
    );
    rut_vm::verify::verify(&flat).expect("verify");
    let mut vm = rut_vm::interp::Vm::builder()
        .program(std::rc::Rc::new(flat))
        .limits(rut_vm::interp::Limits::default())
        .hooks(rut_vm::interp::HostHooks::default())
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("boot");
    vm.call("go", ()).expect("run")
}

/// `pkg(..)` over a host pkg's bodies — the `Offer::Host` arm of the
/// classification — installs them like the old `host_pkg` did.
#[test]
fn pkg_host_offers_the_host_bodies() {
    let mut calc = rut_vm::HostPkg::new("calc");
    calc.register::<_, (i64,), i64, _>("twice", |_vm: &mut rut_vm::interp::Vm, v: i64| v * 2);
    let compiled = rut_driver::RutRun::new()
        .pkg(Mounted::source(
            "app",
            "use calc::{ twice };\n\nentry fn go() -> i64 {\n    return twice(20);\n}\n",
        )
        .manifest(rut_driver::Manifest::lib("app", &["calc"])))
        .pkg(Mounted::host(
            "calc",
            vec![rut_driver::HostRow::root(
                "twice",
                vec![rut_core::types::TY_I64],
                rut_core::types::TY_I64,
                false,
            )],
        ))
        .pkg(calc)
        .entrypoint("app")
        .compile()
        .expect("host rows bind");
    assert!(
        compiled.graph.diags.is_empty(),
        "{:?}",
        compiled.graph.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>()
    );
    let flat = rut_core::link::flatten(compiled.graph.program.expect("linked"));
    let limits = rut_vm::interp::Limits::default();
    let mut vm = rut_vm::interp::Vm::builder()
        .program(std::rc::Rc::new(flat))
        .limits(limits)
        .hooks(rut_vm::interp::HostHooks::default())
        .hosts(compiled.hosts)
        .build()
        .expect("boot");
    let got: i64 = vm.call("go", ()).expect("run");
    assert_eq!(got, 40);
}

/// `pack_inline_pkg` assembles a one-member source bundle — no
/// compile, no container — and a run compiles the member at the
/// offer. The linked program equals the same source offered as
/// mounted data (the legacy bridge).
#[test]
fn pack_inline_pkg_assembles_and_the_run_compiles() {
    let src = "entry fn go() -> i64 {\n    return 6 * 7;\n}\n";
    let inline = rut_driver::pack_inline_pkg("app", src);
    // assembly, never compilation: the handle carries text, not bytes
    let via_handle = rut_driver::RutRun::new()
        .pkg(&inline)
        .entrypoint("app")
        .compile()
        .expect("the run compiles the member");
    let via_legacy = rut_driver::RutRun::new()
        .pkg(Mounted::source("app", src))
        .entrypoint("app")
        .compile()
        .expect("the legacy offer compiles");
    assert_eq!(
        linked_binary(&via_handle),
        linked_binary(&via_legacy),
        "one member, one program — the offer shape cannot matter"
    );
}

/// The legacy surface still works, one release of deprecation: the
/// `Mounted` data, the `Walk` yield, `from_bundle`, `pkgs`,
/// `host_pkg`, and `bundle_walk_bytes` all function exactly as before.
#[test]
#[allow(deprecated)]
fn the_retired_names_still_function() {
    let root = util_world("legacy");
    let dir = root.join("app");
    let bytes = pack_dir(&dir).expect("pack the closure");

    // the byte walk: the retired spelling of decode + mount (groups
    // mount under their content identities — the root under its name)
    let walked: rut_driver::Loaded = rut_driver::bundle_walk_bytes(&bytes).expect("walk");
    assert_eq!(walked.root, "app");
    assert_eq!(walked.pkgs.len(), 2, "the root + its util group");

    // from_bundle and the pkgs/host_pkg offering pair
    let loaded = rut_driver::Mounted::from_bundle(&bytes).expect("load");
    let compiled = rut_driver::RutRun::new()
        .pkgs(&loaded)
        .entrypoint(&loaded.root)
        .compile()
        .expect("compile");
    assert!(compiled.graph.program.is_some());

    // the pkg data constructor, through the deprecated alias
    let data: rut_driver::Pkg = rut_driver::Pkg::source("solo", "entry fn go() -> i64 { return 1; }");
    let compiled = rut_driver::RutRun::new()
        .pkg(data)
        .entrypoint("solo")
        .compile()
        .expect("compile");
    assert!(compiled.graph.program.is_some());
    let _ = std::fs::remove_dir_all(&root);
}

/// A decoded archive whose per-world mount refuses defers the error to
/// `.compile()` — the chain stays fluent, the refusal stays loud. The
/// construction: a doctored scope ledger (every row bumped off the
/// binaries' own scopes) passes the decode gate — the ledger's shape
/// is legal, every binary decodes, every fingerprint agrees — and
/// refuses at the mount, where the rows must AGREE with what the
/// binaries carry.
#[test]
fn a_mount_refusal_surfaces_at_compile_not_at_offer() {
    let root = util_world("defer");
    let dir = root.join("app");
    let bytes = pack_dir(&dir).expect("pack");
    let doctored: Vec<(String, Vec<u8>)> = rut_driver::bundle::parse_bundle(&bytes)
        .unwrap()
        .into_iter()
        .map(|(n, mut b)| {
            if n == "rut.scopes" {
                let text = std::str::from_utf8(&b).unwrap().to_string();
                let mut out = String::new();
                for line in text.lines() {
                    if let Some((l, r)) = line.split_once('=') {
                        let scope: u32 = l.trim().parse().unwrap();
                        out.push_str(&format!("{span} = {rest}\n", span = scope + 100, rest = r.trim()));
                    }
                }
                b = out.into_bytes();
            }
            (n, b)
        })
        .collect();
    let sealed = rut_driver::bundle::write_bundle(&doctored).unwrap();
    // the decode gate passes — the handle builds
    let bundle = rut_driver::Bundle::parse(&sealed).expect("decode (the ledger's shape is legal)");
    // the offer is infallible; the refusal surfaces at .compile()
    let err = rut_driver::RutRun::new()
        .pkg(&bundle)
        .entrypoint("app")
        .compile()
        .expect_err("the doctored ledger refuses at the mount");
    assert!(err.msg.contains("ledger"), "{err}");
    let _ = std::fs::remove_dir_all(&root);
}
