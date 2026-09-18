//! 01-sort — a Rust app embedding rut.
//!
//! `sort.rut` is the application: a sorting library — five algorithms
//! (insertion / bubble / selection / quicksort / merge sort) behind one
//! `entry fn` dispatcher. This file is the embedder: compile, verify,
//! then drive the library — the host owns the session, holds the opaque
//! bank, and reads results back as one JSON array string (`serialize`);
//! `Vec<i32>` itself never crosses (RFC 0023 §2).

use std::rc::Rc;
use rut_vm::heap::Value;

fn main() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/sort.rut")).unwrap();
    // the app's libs: core+calc (engine) and `pouch` — a third-party pkg
    // the app declares, mounted from the toolchain tree
    let mut session = rut_driver::Session::new();
    rut_driver::mount_std(&mut session);
    rut_driver::mount_dir(
        &mut session,
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/pouch")),
    )
    .expect("mount pouch");
    let out = rut_driver::compile_module_in(&mut session, &src, rut_parser::Mode::Impl, "sort");
    assert!(out.diags.is_empty());
    let prog = rut_core::binary::decode(out.binary.as_deref().unwrap()).unwrap();
    rut_vm::verify::verify(&prog).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(5_000_000),
        heap_limit_bytes: Some(8 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut hosts = rut_vm::interp::HostRegistry::new();
    rut_std::math::install_std_math(&mut hosts);
    hosts.verify_against(&session.expected_host_fns()); // calc: .d.rut ↔ bodies
    let mut vm = rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts).unwrap();


    // the session: one bank, exact values in, JSON out
    let Value::Opaque(c) = vm.call("create", &[]).unwrap() else { unreachable!() };
    let ser = |vm: &mut rut_vm::interp::Vm| -> String {
        match vm.call("serialize", &[Value::Opaque(c.clone())]).unwrap() {
            Value::Str(s) => s,
            v => unreachable!("{v:?}"),
        }
    };

    // an exact host-supplied input, sorted by hand-picked algorithms
    for x in [5, 2, 9, 2] {
        vm.call("push", &[Value::Opaque(c.clone()), Value::I64(x)]).unwrap();
    }
    println!("pushed: {}", ser(&mut vm));
    vm.call("sort", &[Value::Opaque(c.clone()), Value::Str("insertion".into())]).unwrap();
    println!("after insertion: {}", ser(&mut vm));

    // the full sweep: same deterministic input, every algorithm, fuel
    // per call (RFC 0040 — the session runs under budgets)
    println!("fill(16, seed=42), each algorithm:");
    for algo in ["insertion", "bubble", "selection", "quick", "merge"] {
        vm.call("fill", &[Value::Opaque(c.clone()), Value::I64(16), Value::I64(42)]).unwrap();
        let before = vm.fuel_used;
        vm.call("sort", &[Value::Opaque(c.clone()), Value::Str(algo.into())]).unwrap();
        let sorted = match vm.call("is_sorted", &[Value::Opaque(c.clone())]).unwrap() {
            Value::Bool(b) => b,
            v => unreachable!("{v:?}"),
        };
        println!("  {algo:<9} {sorted}  fuel {:>6}  {}", vm.fuel_used - before, ser(&mut vm));
    }

    // unknown names are values, not traps
    println!("sort(bogus) = {:?}", vm.call("sort", &[Value::Opaque(c.clone()), Value::Str("bogus".into())]).unwrap());

    println!("fuel used: {} of {:?}", vm.fuel_used, limits.fuel);
}
