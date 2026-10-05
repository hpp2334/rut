//! The wasm host's compiled-bundle mount: the session is built from
//! BYTES — `load_bundle_bytes` over an in-memory archive, no
//! filesystem at load — which is exactly the shape a wasm host drives
//! (the bundle rides the JS boundary as bytes, the mount never touches
//! a path). The pack side stays a native concern; this test only
//! exercises the mount half: a decoded `.rutc` root, its compiled
//! groups, and a run through the same Vm path `rut_run` drives.

use std::rc::Rc;

/// A tiny linkable world (root + one compiled dep), packed v5.
fn pack_world() -> Vec<u8> {
    let base = std::env::temp_dir().join(format!("rut-wasm-v5-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let lib = base.join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("rut.jsonc"), r#"{"name": "lib", "entry": {"lib": "./lib.rut"}}"#).unwrap();
    std::fs::write(lib.join("lib.rut"), "pub fn four() -> i64 { return 4; }\n").unwrap();
    let app = base.join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(
        app.join("rut.jsonc"),
        r#"{"format": "rutbundle", "format_version": 10, "name": "app", "entry": {"lib": "./app.rut"}, "deps": {"lib": {"path": "../lib"}}}"#,
    )
    .unwrap();
    std::fs::write(
        app.join("app.rut"),
        "use lib::{ four };\nentry fn main() -> i64 { return four(); }\n",
    )
    .unwrap();
    let bytes = rut_native::pack_dir(&app).expect("pack");
    let _ = std::fs::remove_dir_all(&base);
    bytes
}

#[test]
fn compiled_bundle_mounts_from_bytes_and_runs() {
    let bytes = pack_world();
    // the mount: bytes in, no fs
    let loaded = rut_driver::Pkg::from_bundle(&bytes).expect("load");
    assert_eq!(loaded.root, "app");
    // the root and its dep mounted as compiled bodies — decoded
    // programs, not sources
    assert!(matches!(loaded.pkg("app").unwrap().body, rut_driver::PkgBody::Compiled(_)));
    assert!(matches!(loaded.pkg("lib").unwrap().body, rut_driver::PkgBody::Compiled(_)));
    // the ordinary pipeline takes over: compile the chain (the
    // compiled-mount arm rebases and pushes), link, verify, run
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
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp:: Vm::builder().program(Rc::new(flat)).limits(limits).hooks(rut_vm::interp::HostHooks::default()).hosts(rut_vm::interp::HostRegistry::new()).build()
    .expect("vm");
    assert_eq!(vm.call::<_, i64>("main", ()).expect("run"), 4);
}
