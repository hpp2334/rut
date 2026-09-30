//! 01-sort — a Rust app embedding rut.
//!
//! `sort.rut` is the application: a sorting library — five algorithms
//! (insertion / bubble / selection / quicksort / merge sort) behind one
//! `entry fn` dispatcher. This file is the embedder: compile, verify,
//! then drive the library — the host owns the session, holds the opaque
//! bank, and reads results back as one JSON array string (`serialize`);
//! `Vec<i32>` itself never crosses.

use std::rc::Rc;

fn main() {
    // the manifest lane: this dir's `rut.toml` carries the deps, the
    // load mounts the closure and runs the mount passes — then the same
    // embedder half as before (mount, compile, verify, drive)
    let (mut session, root) =
        rut_driver::load_dir_session(std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
            .expect("load the module dir");
    // the app's libs: core+calc (engine) — `calc` is ambient, and
    // `pouch` rode the manifest
    rut_driver::mount_std(&mut session);
    let g = rut_driver::compile_graph(&session, &root);
    assert!(g.diags.is_empty());
    let prog = g.program.expect("no binary emitted");
    rut_vm::verify::verify(&prog).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(5_000_000),
        heap_limit_bytes: Some(8 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let ctx = session.host_pkg_context();
    let mut hosts = rut_vm::interp::HostRegistry::new();
    hosts.install_host_pkg(&ctx, rut_std::math::pkg());
    hosts.verify_against(&session.expected_host_fns()); // calc: .d.rut ↔ bodies
    let mut vm = rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts).unwrap();


    // the session: one bank, exact values in, JSON out — typed
    let c: rut_vm::OpaqueRef = vm.call("create", ()).unwrap();
    let ser = |vm: &mut rut_vm::interp::Vm| -> String {
        vm.call::<_, String>("serialize", (c.clone(),)).unwrap()
    };

    // an exact host-supplied input, sorted by hand-picked algorithms
    for x in [5i32, 2, 9, 2] {
        vm.call::<_, ()>("push", (c.clone(), x)).unwrap();
    }
    println!("pushed: {}", ser(&mut vm));
    vm.call::<_, String>("sort", (c.clone(), "insertion")).unwrap();
    println!("after insertion: {}", ser(&mut vm));

    // the full sweep: same deterministic input, every algorithm, fuel
    // per call (the session runs under budgets)
    println!("fill(16, seed=42), each algorithm:");
    for algo in ["insertion", "bubble", "selection", "quick", "merge"] {
        vm.call::<_, ()>("fill", (c.clone(), 16u32, 42u32)).unwrap();
        let before = vm.fuel_used;
        vm.call::<_, String>("sort", (c.clone(), algo)).unwrap();
        let sorted: bool = vm.call("is_sorted", (c.clone(),)).unwrap();
        println!("  {algo:<9} {sorted}  fuel {:>6}  {}", vm.fuel_used - before, ser(&mut vm));
    }

    // unknown names are values, not traps
    println!("sort(bogus) = {:?}", vm.call::<_, String>("sort", (c.clone(), "bogus")).unwrap());

    println!("fuel used: {} of {:?}", vm.fuel_used, limits.fuel);
}
