//! H1 (the nmap host experiment's reader): `Vm::opaque_key_payload`
//! classifies an `opaque` box's payload against the closed native-key
//! set — integers and `bool` cross as raw bits, `str`/`bytes` as owned
//! copies, and anything else (floats, user records) reports
//! `Unsupported` naming the type. This test drives it the way the `nmap_host`
//! host fns will: the key box crosses as an `OpaqueRef` param of a
//! registered host fn (the `boxes` pattern, RFC 0023/0026), the body
//! reads the payload through the accessor, and rut sees only the tag.

use std::rc::Rc;

use rut_driver::{Module, Session};
use rut_vm::OpaqueRef;
use rut_vm::interp::{KeyPayload, Vm};

const PKG_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/keypayload");

const SRC: &str = r#"
use keypayload::{ key_class };

entry fn box_i32(k: i32) -> opaque { return opaque(k); }
entry fn box_bool(k: bool) -> opaque { return opaque(k); }
entry fn box_str(k: str) -> opaque { return opaque(k); }
entry fn box_bytes() -> opaque { return opaque(bytes.from([1, 2, 3])); }
entry fn box_f64(k: f64) -> opaque { return opaque(k); }

struct Pt { x: i32; y: i32 }

// the user-defined-key case: a record is a rut value but not a native
// key — the reader must report Unsupported, never re-enter rut
entry fn box_record() -> opaque {
    let p = Pt { x: 3, y: 4 };
    return opaque(p);
}

// the host crossing: rut hands the key box to the host fn, the host
// classifies the payload and reports the kind tag
entry fn classify(c: opaque) -> i64 { return key_class(c); }
"#;

/// kind tags the registered `key_class` body returns (mirrors KeyPayload)
const BITS: i64 = 1;
const STR: i64 = 2;
const BYTES: i64 = 3;
const UNSUPPORTED: i64 = 4;

fn vm_with_host() -> Vm {
    let mut session = Session::new();
    rut_driver::mount_std_core(&mut session);
    rut_driver::mount_dir(&mut session, std::path::Path::new(PKG_DIR)).expect("mount keypayload");
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
    let prog = rut_core::link::flatten(g.program.expect("compile"));
    rut_vm::verify::verify(&prog).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    // bindings BEFORE the Vm (RFC 0025): the body is the nmap shape —
    // an `OpaqueRef` param read through the payload accessor
    let mut hosts = rut_vm::interp::HostRegistry::new();
    rut_vm::register!(hosts, "keypayload::key_class", (OpaqueRef,) -> i64, |vm: &mut Vm,
                                                                              k: OpaqueRef| {
        Ok(match vm.opaque_key_payload(&k)? {
            KeyPayload::Bits { .. } => BITS,
            KeyPayload::Str(_) => STR,
            KeyPayload::Bytes(_) => BYTES,
            KeyPayload::Unsupported(_) => UNSUPPORTED,
        })
    });
    hosts.verify_against(&expected); // keypayload.d.rut ↔ the body
    rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts)
        .unwrap()
}

#[test]
fn key_payload_classifies_through_the_host_crossing() {
    let mut vm = vm_with_host();
    let tag = |vm: &mut Vm, h: &OpaqueRef| -> i64 { vm.call::<_, i64>("classify", (h.clone(),)).unwrap() };

    // the closed set: integers and bool cross as Bits
    let i: OpaqueRef = vm.call("box_i32", (7i32,)).unwrap();
    assert_eq!(tag(&mut vm, &i), BITS);
    let b: OpaqueRef = vm.call("box_bool", (true,)).unwrap();
    assert_eq!(tag(&mut vm, &b), BITS);

    // ref keys cross as owned copies
    let s: OpaqueRef = vm.call("box_str", ("k",)).unwrap();
    assert_eq!(tag(&mut vm, &s), STR);
    let bytes: OpaqueRef = vm.call("box_bytes", ()).unwrap();
    assert_eq!(tag(&mut vm, &bytes), BYTES);

    // outside the closed set: floats and user records report Unsupported —
    // the nmap trap's raw material ("not natively supported — encode the
    // key to `bytes`, or use `nmapset`")
    let f: OpaqueRef = vm.call("box_f64", (1.5f64,)).unwrap();
    assert_eq!(tag(&mut vm, &f), UNSUPPORTED);
    let r: OpaqueRef = vm.call("box_record", ()).unwrap();
    assert_eq!(tag(&mut vm, &r), UNSUPPORTED);

    // re-classifying the same box is stable (a read, not a consume)
    assert_eq!(tag(&mut vm, &i), BITS);
    assert_eq!(tag(&mut vm, &r), UNSUPPORTED);
}

#[test]
fn distinct_boxes_of_the_same_key_stay_distinct_handles() {
    // opaque(..) mints a fresh box per call — two boxes of the equal
    // keys are different handles, and each classifies on its own
    let mut vm = vm_with_host();
    let a: OpaqueRef = vm.call("box_i32", (7i32,)).unwrap();
    let b: OpaqueRef = vm.call("box_i32", (7i32,)).unwrap();
    assert_ne!(a, b, "opaque(..) mints a fresh box per call");
    let tag = |vm: &mut Vm, h: &OpaqueRef| -> i64 { vm.call::<_, i64>("classify", (h.clone(),)).unwrap() };
    assert_eq!(tag(&mut vm, &a), BITS);
    assert_eq!(tag(&mut vm, &b), BITS);
}
