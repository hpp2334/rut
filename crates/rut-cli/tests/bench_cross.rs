//! The `bench_cross` benchmark pkg (the crossing-fastpath plan, phase
//! 0): the two nops crossing end to end the way the bench row drives
//! them — rut code calls the host surface, the host answers, the result
//! feeds a rut accumulator. The bodies are the identity / a four-way
//! sum, so an equality check against the inline rut twins also proves
//! the crossing preserves bits exactly (including negatives).
//!
//! Covered per the phase: both `.d.rut` surfaces (the committed
//! `rut/bench-cross` pkg and this fixture mirror) declare the SAME
//! contract and both compile against the one binding set; the host
//! answers round-trip; the inline twins agree with the host on a sweep
//! of values (the checksum law the bench row relies on).

use std::rc::Rc;

use rut_driver::{Module, ModuleBody, Session};
use rut_vm::interp::Vm;

const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/bench-cross");
const COMMITTED_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/bench-cross");

const SRC: &str = r#"
use bench_cross::{ nop, nop4 };

// loop B's twins in the bench row: the SAME body shape as the host
// nops, so A-B isolates the crossing, not the work
fn nop_rut(x: i64) -> i64 {
    return x;
}

fn nop4_rut(a: i64, b: i64, c: i64, d: i64) -> i64 {
    return a + b + c + d;
}

entry fn host_nop(x: i64) -> i64 { return nop(x); }

entry fn host_nop4(a: i64, b: i64, c: i64, d: i64) -> i64 { return nop4(a, b, c, d); }

// a sweep the test asserts at zero: the host crossing and the inline
// twin must agree bit-for-bit on every value (the bench row's checksum
// law — host loop and rut loop accumulate the same sums)
entry fn twins_agree(n: i64) -> i64 {
    let mut fails: i64 = 0;
    let mut i: i64 = 0;
    while (i < n) {
        let x = i * 7919 - 1000003;
        if (nop(x) != nop_rut(x)) { fails += 1; }
        if (nop4(x, x + 1, x + 2, x + 3) != nop4_rut(x, x + 1, x + 2, x + 3)) { fails += 10; }
        i += 1;
    }
    return fails;
}
"#;

/// Mount ONE bench_cross surface (fixture or committed pkg), bind the
/// bodies, boot the Vm. Bindings before the Vm, contract
/// checked by `verify_against` at boot.
fn vm_with_surface(pkg_dir: &str) -> Vm {
    let mut session = Session::new();
    rut_driver::mount_std_core(&mut session);
    rut_driver::mount_dir(&mut session, std::path::Path::new(pkg_dir))
        .expect("mount bench_cross");
    let ctx = session.host_pkg_context();
    session
        .register_module(
            "app",
            Module { spec: "app".into(), body: ModuleBody::Source { text: SRC.into(), is_decl: false }, ..Default::default() },
        )
        .unwrap();
    let g = rut_driver::compile_graph(&session, "app");
    assert!(
        g.diags.is_empty(),
        "{}",
        g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
    let prog = g.program.expect("compile");
    rut_vm::verify::verify(&prog).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(20_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut hosts = rut_vm::interp::HostRegistry::new();
    hosts.install_host_pkg(&ctx, rut_std::bench_cross::pkg());
    hosts.verify_against(&ctx.flatten()); // the surface ↔ the bodies
    rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts)
        .unwrap()
}

/// The committed pkg and the fixture mirror must declare the SAME
/// contract — one binding set serves both, so a drift between the two
/// surfaces is a boot panic or this assert, never a silent skew.
#[test]
fn both_surfaces_declare_the_same_contract() {
    let mut a = Session::new();
    rut_driver::mount_std_core(&mut a);
    rut_driver::mount_dir(&mut a, std::path::Path::new(FIXTURE_DIR)).unwrap();
    let mut b = Session::new();
    rut_driver::mount_std_core(&mut b);
    rut_driver::mount_dir(&mut b, std::path::Path::new(COMMITTED_DIR)).unwrap();
    let ctx_a = a.host_pkg_context();
    let ctx_b = b.host_pkg_context();
    assert_eq!(ctx_a.flatten(), ctx_b.flatten());
    // and the binding set satisfies the contract exactly
    let mut hosts = rut_vm::interp::HostRegistry::new();
    hosts.install_host_pkg(&ctx_b, rut_std::bench_cross::pkg());
    hosts.verify_against(&ctx_b.flatten());
}

#[test]
fn nops_round_trip_through_the_crossing() {
    let mut vm = vm_with_surface(FIXTURE_DIR);
    assert_eq!(vm.call::<_, i64>("host_nop", (7i64,)).unwrap(), 7);
    assert_eq!(vm.call::<_, i64>("host_nop", (-5i64,)).unwrap(), -5);
    assert_eq!(vm.call::<_, i64>("host_nop4", (1i64, 2, 3, 4)).unwrap(), 10);
    assert_eq!(vm.call::<_, i64>("host_nop4", (-1i64, 1, -2, 2)).unwrap(), 0);
    // the committed pkg drives identically (same contract, same bodies)
    let mut vm = vm_with_surface(COMMITTED_DIR);
    assert_eq!(vm.call::<_, i64>("host_nop", (7i64,)).unwrap(), 7);
    assert_eq!(vm.call::<_, i64>("host_nop4", (1i64, 2, 3, 4)).unwrap(), 10);
}

#[test]
fn host_and_inline_twin_agree_on_a_sweep() {
    let mut vm = vm_with_surface(FIXTURE_DIR);
    assert_eq!(vm.call::<_, i64>("twins_agree", (10_000i64,)).unwrap(), 0);
}
