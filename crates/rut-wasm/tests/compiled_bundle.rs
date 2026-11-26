//! The compiled-bundle mount from BYTES: the driver's bundle walk over
//! an in-memory archive, no filesystem at load — a decoded `.rutc`
//! root, its compiled groups, and a run through the same `Vm` path
//! `RutEngine::run` drives. No rut-wasm API is exercised here: the
//! test composes the driver's mount, the native pack (host-side — the
//! wasm lane cannot pack) and the vm's verify/run shapes, which is the
//! stack the wasm engine sits on.

use std::rc::Rc;

/// A tiny linkable world (root + one compiled dep), packed.
fn pack_world() -> Vec<u8> {
    let base = rut_testkit::Scratch::new("wasm-bundle");
    let lib = base.join("lib");
    std::fs::create_dir_all(lib.join("src")).unwrap();
    std::fs::write(lib.join("rut.jsonc"), r#"{"name": "lib"}"#).unwrap();
    std::fs::write(
        lib.join("src/mod.rut"),
        "pub fn four() -> i64 { return 4; }\n",
    )
    .unwrap();
    let app = base.join("app");
    std::fs::create_dir_all(app.join("src")).unwrap();
    std::fs::write(
        app.join("rut.jsonc"),
        r#"{"name": "app", "deps": {"lib": {"path": "../lib"}}}"#,
    )
    .unwrap();
    std::fs::write(
        app.join("src/mod.rut"),
        "use lib::{ four };\nentry fn main() -> i64 { return four(); }\n",
    )
    .unwrap();
    rut_native::pack_dir(&app).expect("pack")
}

#[test]
fn compiled_bundle_mounts_from_bytes_and_runs() {
    let bytes = pack_world();
    // the mount: bytes in, no fs — decode once, walk the yield
    let bundle = rut_driver::Bundle::parse(&bytes).expect("load");
    let loaded = bundle.walk().expect("load");
    assert_eq!(loaded.root, "app");
    // the root and its dep mounted as compiled bodies — decoded
    // programs, not sources. The dep group mounts under its content
    // identity, so the lookup goes by the bare name half
    assert!(matches!(
        loaded.pkg("app").unwrap().body,
        rut_driver::MountedBody::Compiled(_)
    ));
    let lib = loaded
        .pkgs
        .iter()
        .find(|p| rut_driver::bundle::parse_identity(&p.spec).map(|(n, _)| n) == Some("lib"))
        .expect("the lib group mounted");
    assert!(matches!(lib.body, rut_driver::MountedBody::Compiled(_)));
    // the ordinary pipeline takes over: compile the chain (the
    // compiled-mount arm rebases and pushes), link, verify, run
    let g = loaded
        .pkgs
        .iter()
        .fold(rut_driver::RutRun::new(), |r, p| r.mount(p.clone()))
        .mount(rut_native::tree_pkg("calc_host").expect("the toolchain tree's rut/calc_host"))
        .entrypoint(&loaded.root)
        .compile()
        .expect("compile the walk");
    assert!(g.graph.diags.is_empty(), "{:?}", g.graph.diags);
    let flat = rut_core::link::flatten(g.graph.program.expect("linked program"));
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::builder()
        .program(Rc::new(flat))
        .limits(limits)
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("vm");
    assert_eq!(vm.call::<_, i64>("main", ()).expect("run"), 4);
}
