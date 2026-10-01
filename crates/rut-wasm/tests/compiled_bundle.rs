//! The wasm host's compiled-bundle mount: the session is built from
//! BYTES — `load_bundle_bytes` over an in-memory archive, no
//! filesystem at load — which is exactly the shape a wasm host drives
//! (the bundle rides the JS boundary as bytes, the mount never touches
//! a path). The pack side stays a native concern; this test only
//! exercises the mount half: a decoded `.rutc` root, its compiled
//! groups, and a run through the same Vm path `rut_run` drives.

use std::path::Path;
use std::rc::Rc;

/// A tiny linkable world (root + one compiled dep), packed v5.
fn pack_world() -> Vec<u8> {
    let base = std::env::temp_dir().join(format!("rut-wasm-v5-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let lib = base.join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("rut.json"), r#"{"name": "lib", "entry": {"lib": "./lib.rut"}}"#).unwrap();
    std::fs::write(lib.join("lib.rut"), "pub fn four() -> i64 { return 4; }\n").unwrap();
    let app = base.join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(
        app.join("rut.json"),
        r#"{"format": "rutbundle", "format_version": 7, "name": "app", "entry": {"lib": "./app.rut"}, "deps": {"lib": {"path": "../lib"}}}"#,
    )
    .unwrap();
    std::fs::write(
        app.join("app.rut"),
        "use lib::{ four };\npub fn main() -> i64 { return four(); }\n",
    )
    .unwrap();
    let bytes = rut_driver::pack_dir(&app).expect("pack");
    let _ = std::fs::remove_dir_all(&base);
    bytes
}

#[test]
fn compiled_bundle_mounts_from_bytes_and_runs() {
    let bytes = pack_world();
    // the mount: bytes in, no fs
    let (mut session, root) =
        rut_driver::load_bundle_bytes(&bytes, Path::new("mem")).expect("load");
    assert_eq!(root, "app");
    // the root and its dep mounted as compiled bodies — decoded
    // programs, not sources
    assert!(matches!(session.resolve("app").unwrap().body, rut_driver::ModuleBody::Compiled(_)));
    assert!(matches!(session.resolve("lib").unwrap().body, rut_driver::ModuleBody::Compiled(_)));
    // the ordinary pipeline takes over: mount std, compile the graph
    // (the compiled-mount arm rebases and pushes), link, verify, run
    rut_driver::mount_std(&mut session);
    let g = rut_driver::compile_graph(&session, &root);
    assert!(g.diags.is_empty(), "{:?}", g.diags);
    let flat = rut_core::link::flatten(g.program.expect("linked program"));
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        Rc::new(flat),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    assert_eq!(vm.call::<_, i64>("main", ()).expect("run"), 4);
}
