//! The host-param fast lane (the crossing-fastpath plan, phase 1):
//! rut→host crossings read prim params as raw slot bits — no
//! `expect_kind`, no `narrow_i64`, no string compare. These tests drive
//! the lane end to end through the driver pipeline (compile → verify →
//! interpret → adapter): the prim round-trips must hold BITS (including
//! the u64 high half, `2^63..2^64-1`), the borrows must still read
//! their cells through the kind match, the owned shapes must still
//! copy, and the embedder-facing checked reads (`value_in`,
//! `Ret::from_slot`) must trap exactly as before.
//!
//! The nil-ref and payload-token traps are pinned at the adapter level
//! in `rut-vm`'s `boundary::fast_lane_tests` — rut source cannot
//! syntactically put a nil in an `opaque` param slot (the transitive
//! `??T` coercion funnel traps at the deref first), which is precisely
//! why that check is "unproven" defense and stayed.

use std::cell::RefCell;
use std::rc::Rc;

use rut_driver::{Module, Session};
use rut_vm::OpaqueRef;
use rut_vm::interp::Vm;

const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/fastlane");

const SRC: &str = r#"
use fastlane::{ id_i8, id_i64, id_u64, id_f64, id_bool, len_str,
                sum_bytes, keep_str, keep_bytes, make_ref };

// every entry is one rut body around one host crossing — the fast lane
// is the middle of each round-trip
entry fn rt_i8(x: i8) -> i8 { return id_i8(x); }
entry fn rt_i64(x: i64) -> i64 { return id_i64(x); }
entry fn rt_u64(x: u64) -> u64 { return id_u64(x); }
entry fn rt_u64_shr(x: u64) -> u64 { return id_u64(x >> 63); }
entry fn rt_f64(x: f64) -> f64 { return id_f64(x); }
entry fn rt_bool(b: bool) -> bool { return id_bool(b); }
entry fn rt_ref_back(x: i64) -> opaque { return make_ref(); }
entry fn rt_str_len(s: str) -> i64 { return len_str(s); }
entry fn rt_bytes_sum(b: bytes) -> i64 { return sum_bytes(b); }
entry fn rt_str_keep(s: str) -> i64 { return keep_str(s); }
entry fn rt_bytes_keep(b: bytes) -> i64 { return keep_bytes(b); }
"#;

/// The host half: identities for the prims, the borrow reads, the owned
/// copies — all bound infallibly where they can be (the hot lane's
/// shape). `kept_str` captures the owned copy so the test can assert
/// the host RECEIVED the data, not just its length.
fn install_fastlane(hosts: &mut rut_vm::interp::HostRegistry, kept_str: Rc<RefCell<String>>) {
    rut_vm::register!(hosts, "fastlane::id_i8", (i8,) -> i8, |_vm: &mut Vm, x: i8| x);
    rut_vm::register!(hosts, "fastlane::id_i64", (i64,) -> i64, |_vm: &mut Vm, x: i64| x);
    rut_vm::register!(hosts, "fastlane::id_u64", (u64,) -> u64, |_vm: &mut Vm, x: u64| x);
    rut_vm::register!(hosts, "fastlane::id_f64", (f64,) -> f64, |_vm: &mut Vm, x: f64| x);
    rut_vm::register!(hosts, "fastlane::id_bool", (bool,) -> bool, |_vm: &mut Vm, b: bool| b);
    rut_vm::register!(hosts, "fastlane::len_str", (&str,) -> i64, |_vm: &mut Vm, s: &str| -> i64 {
        s.len() as i64
    });
    rut_vm::register!(hosts, "fastlane::sum_bytes", (&[u8],) -> i64, |_vm: &mut Vm, b: &[u8]| -> i64 {
        b.iter().map(|&x| x as i64).sum()
    });
    rut_vm::register!(hosts, "fastlane::keep_str", (String,) -> i64, move |_vm: &mut Vm, s: String| -> i64 {
        let n = s.len() as i64;
        *kept_str.borrow_mut() = s;
        n
    });
    rut_vm::register!(hosts, "fastlane::keep_bytes", (Vec<u8>,) -> i64, |_vm: &mut Vm, v: Vec<u8>| -> i64 {
        v.len() as i64
    });
    rut_vm::register!(hosts, "fastlane::make_ref", () -> OpaqueRef, |vm: &mut Vm| -> OpaqueRef {
        rut_vm::Opaque::alloc(vm, 42i64).expect("alloc").handle().clone()
    });}

fn vm_with_surface(kept_str: Rc<RefCell<String>>) -> Vm {
    let mut session = Session::new();
    rut_driver::mount_std_core(&mut session);
    rut_driver::mount_dir(&mut session, std::path::Path::new(FIXTURE_DIR))
        .expect("mount fastlane");
    let expected = session.expected_host_fns();
    session
        .register_module(
            "app",
            Module { spec: "app".into(), source: Some(SRC.into()), ..Default::default() },
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
    install_fastlane(&mut hosts, kept_str);
    hosts.verify_against(&expected); // the surface ↔ the bodies, pre-boot
    rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts)
        .unwrap()
}

#[test]
fn prim_round_trips_hold_bits_including_the_u64_high_half() {
    let kept = Rc::new(RefCell::new(String::new()));
    let mut vm = vm_with_surface(kept);
    // the narrow widths at their extremes — the old `narrow_i64` would
    // have re-proved these fits per call; the fast lane reinterprets
    assert_eq!(vm.call::<_, i8>("rt_i8", (i8::MIN,)).unwrap(), i8::MIN);
    assert_eq!(vm.call::<_, i8>("rt_i8", (i8::MAX,)).unwrap(), i8::MAX);
    assert_eq!(vm.call::<_, i64>("rt_i64", (i64::MIN,)).unwrap(), i64::MIN);
    assert_eq!(vm.call::<_, i64>("rt_i64", (-1i64,)).unwrap(), -1);
    // u64: `2^63..2^64-1` cross with the sign bit set in the slot — the
    // raw-bit read must keep every bit (no sign extension anywhere)
    for v in [
        u64::MAX,
        1u64 << 63,
        (1u64 << 63) + 12345,
        0xDEAD_BEEF_CAFE_BABE,
    ] {
        assert_eq!(vm.call::<_, u64>("rt_u64", (v,)).unwrap(), v, "bits lost for {v:#x}");
    }
    // and the bits are USABLE rut-side: the shift sees the high word
    assert_eq!(vm.call::<_, u64>("rt_u64_shr", (u64::MAX,)).unwrap(), 1);
    assert_eq!(vm.call::<_, u64>("rt_u64_shr", (1u64 << 63,)).unwrap(), 1);
    assert_eq!(vm.call::<_, u64>("rt_u64_shr", (1u64 << 62,)).unwrap(), 0);
    // floats/bool/char: the raw union reads
    assert_eq!(vm.call::<_, f64>("rt_f64", (1.5e300,)).unwrap(), 1.5e300);
    assert_eq!(vm.call::<_, f64>("rt_f64", (-0.0,)).unwrap() == 0.0, true);
    assert_eq!(vm.call::<_, bool>("rt_bool", (true,)).unwrap(), true);
    assert_eq!(vm.call::<_, bool>("rt_bool", (false,)).unwrap(), false);
}

#[test]
fn borrow_params_read_their_cells_zero_copy() {
    let kept = Rc::new(RefCell::new(String::new()));
    let mut vm = vm_with_surface(kept);
    assert_eq!(vm.call::<_, i64>("rt_str_len", ("hello".to_string(),)).unwrap(), 5);
    assert_eq!(vm.call::<_, i64>("rt_str_len", (String::new(),)).unwrap(), 0);
    // byte length, not char count — the octets are what crossed
    assert_eq!(vm.call::<_, i64>("rt_str_len", ("héllo".to_string(),)).unwrap(), 6);
    assert_eq!(vm.call::<_, i64>("rt_bytes_sum", (vec![1u8, 2, 3, 250],)).unwrap(), 256);
    assert_eq!(vm.call::<_, i64>("rt_bytes_sum", (Vec::new(),)).unwrap(), 0);
}

#[test]
fn owned_params_arrive_as_copies_the_host_keeps() {
    let kept = Rc::new(RefCell::new(String::new()));
    let mut vm = vm_with_surface(kept.clone());
    assert_eq!(vm.call::<_, i64>("rt_str_keep", ("ada".to_string(),)).unwrap(), 3);
    assert_eq!(*kept.borrow(), "ada", "the host body received the owned copy");
    assert_eq!(vm.call::<_, i64>("rt_bytes_keep", (vec![7u8; 9],)).unwrap(), 9);
}

#[test]
fn embedder_wrong_shape_traps_unchanged() {
    let kept = Rc::new(RefCell::new(String::new()));
    let mut vm = vm_with_surface(kept);
    // ARG side (value_in — untouched): an integer where the entry takes
    // a str is a trap naming both sides, before any slot exists
    let err = vm.call::<_, i64>("rt_str_len", (7i64,)).unwrap_err();
    assert!(err.msg.contains("an integer") && err.msg.contains("str"), "{}", err.msg);
    // RET side (Ret::from_slot — the CHECKED read, untouched): the host
    // answers opaque, the embedder asked i64 — the boundary trap names both
    let err = vm.call::<_, i64>("rt_ref_back", (5i64,)).unwrap_err();
    assert!(err.msg.contains("boundary:"), "{}", err.msg);
    assert!(err.msg.contains("`opaque`") && err.msg.contains("`i64`"), "{}", err.msg);
    // the right shape still crosses: the box arrives, payload intact
    let back: rut_vm::Opaque<i64> = vm.call("rt_ref_back", (5i64,)).unwrap();
    assert_eq!(back.with(|v| *v).unwrap(), 42i64);
}
