//! The any arms (nmap-hostvals P3) — the additive boundary surface,
//! proven END TO END through compiled rut: the `.d.rut` grammar admits
//! `any` (host-decl-only), the checker coerces a rut arg of ANY static
//! type into an `any` param (no box, no copy), and the answer writes the
//! caller's V-typed register (the untagged-slot discipline, §0.8 h).
//!
//! The demo host is a one-slot "box" registered AD HOC here — never in
//! rut-std's installers. Its state is a `ValSlot` (the repr P5's store
//! reuses): prim sites cross as raw bits, ref kinds retain the arg's OWN
//! cell; replace releases the old in-crossing; the tag drives every rc
//! decision. A full put/get cycle must close the heap balance exactly.

use rut_core::types::{TyKind, TY_I64, TY_OPAQUE, TY_VAL};
use rut_driver::{lower_decl_module, Module, Session};
use rut_vm::{HostVal, OpaqueRef, TrapKind, ValSlot};

/// The host-decl surface under test: `any` in a param AND an answer.
const DECL: &str = "\
pub host fn box_put(m: opaque, v: any) -> i64;
pub host fn box_get(m: opaque) -> any;
";

#[test]
fn any_spells_in_the_decl_grammar() {
    // the grammar is a text whitelist (crossing_ty) — `any` is +1 arm,
    // mapped to the DISTINCT boot id TY_VAL (never the VM's u32::MAX
    // TY_ANY sentinel)
    let m = lower_decl_module(DECL, "hmap.d.rut").expect("the any rows lower");
    assert_eq!(
        m.host_funcs,
        vec![
            ("box_put".to_string(), vec![TY_OPAQUE, TY_VAL], TY_I64),
            ("box_get".to_string(), vec![TY_OPAQUE], TY_VAL),
        ],
        "the `any` spelling maps to TY_VAL in params and answers"
    );
}

#[test]
fn rut_source_cannot_name_any() {
    // host-decl-only: the decl surface accepts the spelling, the rut
    // resolver does not — a type position in .rut source refuses
    let mut s = Session::new();
    rut_driver::mount_std_core(&mut s);
    s.register_module(
        "app",
        Module {
            source: Some("pub fn main() -> nil {\n    let x: any = 1;\n}".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let out = rut_driver::compile_graph(&s, "app");
    assert!(
        !out.diags.is_empty(),
        "`any` is host-decl-only — rut source must refuse it"
    );
}

/// The demo box: ONE slot of `ValSlot` state. Put decodes the any-arg
/// per the plan's law (prim → the bits as-is; ref → retain the arg's own
/// cell), releases the replaced value in-crossing, and stores. Get takes
/// the value out: the miss answers `None` (the flat nil), a prim answer
/// moves the bits, a ref answer releases the map's retain and hands the
/// cell over — the register takes its own rc in `call_host`'s write-back.
fn demo_registry(
    state: std::rc::Rc<std::cell::RefCell<Option<ValSlot>>>,
) -> rut_vm::interp::HostRegistry {
    let mut hosts = rut_vm::interp::HostRegistry::new();
    let put_state = state.clone();
    rut_vm::register!(
        hosts,
        "hmap::box_put",
        (OpaqueRef, HostVal) -> i64,
        move |vm: &mut rut_vm::interp::Vm, _m: OpaqueRef, hv: HostVal| {
            // the ARG law (the plan's decode, verbatim shape) — rut types
            // live in the VM, never Rust: the site's TyKind reads through
            // the program's own table, never cloned
            let vs = match vm.prog.types.kind(hv.ty) {
                TyKind::Prim(_) => ValSlot::Bits(hv.slot), // the 8 bytes move as-is
                _ => {
                    vm.retain(hv.slot); // ref kinds: the arg's OWN cell
                    ValSlot::Ref(hv.slot)
                }
            };
            let mut slot = put_state.borrow_mut();
            if let Some(old) = slot.replace(vs) {
                if let ValSlot::Ref(old) = old {
                    vm.release(old); // replace: the old value releases in-crossing
                }
            }
            1
        },
    );
    let get_state = state;
    rut_vm::register!(
        hosts,
        "hmap::box_get",
        (OpaqueRef,) -> Option<ValSlot>,
        move |vm: &mut rut_vm::interp::Vm, _m: OpaqueRef| {
            match get_state.borrow_mut().take() {
                None => Ok(None), // the miss: the flat nil
                Some(ValSlot::Bits(s)) => Ok(Some(ValSlot::Bits(s))),
                Some(ValSlot::Ref(s)) => {
                    vm.release(s); // take: the map's retain goes back
                    Ok(Some(ValSlot::Ref(s))) // the register takes its own rc
                }
                // §0.8 g: the placeholder is a caller bug, never a nil
                Some(ValSlot::Empty) => Err(rut_vm::Trap::new(
                    TrapKind::Invalid,
                    "box_get over an empty slot (§0.8 g)",
                )),
            }
        },
    );
    hosts
}

/// Mount the host pkg + its consumer, run `main`, answer the result and
/// the heap balance (a full put/get cycle must close exactly on the base).
fn run_app<R: rut_vm::interp::Ret>(src: &str) -> (R, u64) {
    let mut s = Session::new();
    rut_driver::mount_std_core(&mut s);
    let module = lower_decl_module(DECL, "hmap.d.rut").expect("the any rows lower");
    s.register_module("hmap", module).unwrap();
    s.register_module(
        "app",
        Module { source: Some(src.into()), ..Default::default() },
    )
    .unwrap();
    let out = rut_driver::compile_graph(&s, "app");
    assert!(out.diags.is_empty(), "the any-lane consumer compiles: {:?}", out.diags);
    let prog = out.program.expect("linked program");
    let flat = rut_core::link::flatten(prog);
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let state = std::rc::Rc::new(std::cell::RefCell::new(None));
    let hosts = demo_registry(state);
    // the load-time contract (RFC 0025) — the any row verifies under its
    // own spelling
    hosts.verify_against(&s.expected_host_fns());
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(flat),
        &limits,
        rut_vm::interp::HostHooks::default(),
        hosts,
    )
    .expect("vm");
    let base = vm.heap_usage();
    let answer: R = vm.call("main", ()).expect("run");
    (answer, vm.heap_usage() - base)
}

#[test]
fn the_round_trip_through_the_arm_prims_and_refs() {
    // prims cross as bits (no cell), refs as their OWN cell (aliased);
    // a full cycle leaves the heap exactly where it started
    let (answer, delta) = run_app::<String>(
        "use hmap::{ box_put, box_get };\n\
         pub fn main() -> str {\n\
         \x20   let m: opaque = opaque(0);\n\
         \x20   let a: i64 = 42;\n\
         \x20   box_put(m, a);\n\
         \x20   let v: i64 = box_get(m);\n\
         \x20   let s: str = \"ada\";\n\
         \x20   box_put(m, s);\n\
         \x20   let t: str = box_get(m);\n\
         \x20   if (v != 42) { return \"bad-v\"; }\n\
         \x20   return t;\n\
         }\n",
    );
    assert_eq!(answer, "ada", "the ref value crossed and reads as itself");
    assert_eq!(delta, 0, "the put/get cycle closes the heap balance exactly");
}

#[test]
fn a_narrow_v_register_reads_the_same_bits() {
    // i64 in → i32-V-shaped register out: the answer write performs no
    // width conversion — the dst register's own static type (the
    // caller's V) gives the 8 bytes their meaning
    let (answer, delta) = run_app::<i32>(
        "use hmap::{ box_put, box_get };\n\
         pub fn main() -> i32 {\n\
         \x20   let m: opaque = opaque(0);\n\
         \x20   let a: i64 = 7;\n\
         \x20   box_put(m, a);\n\
         \x20   let v: i32 = box_get(m);\n\
         \x20   return v;\n\
         }\n",
    );
    assert_eq!(answer, 7, "the i64 bits landed in the i32 register unconverted");
    assert_eq!(delta, 0);
}

#[test]
fn a_miss_answers_the_flat_nil() {
    // get over an empty box: the host answers None — the flat nil, the
    // zero word, in the caller's V register (a prim V reads it as 0)
    let (answer, delta) = run_app::<i64>(
        "use hmap::box_get;\n\
         pub fn main() -> i64 {\n\
         \x20   let m: opaque = opaque(0);\n\
         \x20   let v: i64 = box_get(m);\n\
         \x20   return v + 5;\n\
         }\n",
    );
    assert_eq!(answer, 5, "the miss crossed as the flat nil (0 + 5 = 5)");
    assert_eq!(delta, 0);
}

#[test]
fn verify_against_names_any_on_drift() {
    // the RFC 0025 contract check names the any row from the CONST (the
    // boot row's name is the shell, never read): the pkg declares
    // `(opaque) -> any`, a drifted `-> f64` binding must say so
    let mut s = Session::new();
    let module = lower_decl_module(DECL, "hmap.d.rut").unwrap();
    s.register_module("hmap", module).unwrap();
    let expected = s.expected_host_fns();
    assert_eq!(
        expected.get("hmap::box_get"),
        Some(&(vec![TY_OPAQUE], TY_VAL)),
        "the mounted surface carries TY_VAL for the any answer"
    );
    let mut hosts = rut_vm::interp::HostRegistry::new();
    rut_vm::register!(
        hosts,
        "hmap::box_get",
        (OpaqueRef,) -> f64,
        |_vm: &mut rut_vm::interp::Vm, _m: OpaqueRef| 0.0,
    );
    let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        hosts.verify_against(&expected);
    }))
    .expect_err("a drifted any row panics");
    let msg = err
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    assert!(msg.contains("signature drift"), "{msg}");
    assert!(msg.contains("-> any"), "the pkg side names the any row: {msg}");
    assert!(msg.contains("-> f64"), "the binding side names the drift: {msg}");
}
