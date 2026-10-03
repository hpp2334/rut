//! H1 (the nmap host experiment's reader): `Vm::opaque_key_payload`
//! classifies an `opaque` box's payload against the closed native-key
//! set — integers and `bool` cross as raw bits, `str`/`bytes` as owned
//! copies, and anything else (floats, user records) reports
//! `Unsupported` naming the type. This test drives it the way the `nmap_host`
//! host fns will: the key box crosses as an `OpaqueRef` param of a
//! registered host fn (the `boxes` pattern), the body
//! reads the payload through the accessor, and rut sees only the tag.

use std::rc::Rc;


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




/// One source pkg over the auto core — the chain's graph. Closure and
/// shape refusals come back as one span-0 diagnostic.
#[allow(dead_code)] // not every suite in this file needs both lanes
fn compiled(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(rut_driver::RutRun::new().pkg(rut_driver::Pkg::source(spec, src)).entrypoint(spec).compile())
}

/// [`compiled`] with calc offered (the old `mount_std` shape: core
/// auto-rides, `calc` is an ordinary pkg).
#[allow(dead_code)]
fn compiled_std(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(
        rut_driver::RutRun::new()
            .pkg(rut_driver::Pkg::source(spec, src))
            .pkg(rut_driver::calc_pkg())
            .entrypoint(spec)
            .compile(),
    )
}

#[allow(dead_code)]
fn graph_of(c: Result<rut_driver::Compiled, rut_driver::RunError>) -> rut_driver::GraphOutput {
    match c {
        Ok(c) => c.graph,
        Err(e) => rut_driver::GraphOutput {
            diags: vec![rut_lexer::diag::Diag::new(rut_lexer::span::Span::new(0, 0), e.msg)],
            program: None,
        },
    }
}

fn vm_with_host() -> Vm {
    let mut loaded = rut_driver::dir_pkgs(std::path::Path::new(PKG_DIR)).expect("mount keypayload");
    let expected = rut_driver::declared_host_fns(&loaded.pkgs);
    loaded.pkgs.push(rut_driver::Pkg::source("app", SRC));
    let g = rut_driver::RutRun::new()
        .pkgs(&loaded)
        .entrypoint("app")
        .compile()
        .unwrap();
    assert!(
        g.graph.diags.is_empty(),
        "{}",
        g.graph.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
    let prog = rut_core::link::flatten(g.graph.program.expect("compile"));
    rut_vm::verify::verify(&prog).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    // bindings BEFORE the Vm: the body is the nmap shape —
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
    rut_vm::interp:: Vm::builder().program(Rc::new(prog)).limits(limits).hooks(rut_vm::interp::HostHooks::default()).hosts(hosts).build()
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
