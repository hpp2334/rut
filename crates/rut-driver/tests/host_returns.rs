//! The legal host returns (the answer-lane phase): `-> ?str`,
//! `-> ?bytes`, `-> ?opaque`, and plain `-> bytes` lower from the
//! `.d.rut` surface, verify against their `Option<T>` bindings by boot-id
//! identity, and cross back through compiled rut — the Some/nil shapes,
//! the minted boxes, and the exact heap balance.

use rut_core::types::{TY_BYTES, TY_I64, TY_OPT_BYTES, TY_OPT_OPAQUE, TY_OPT_STR};
use rut_driver::{lower_decl_module, Module, Session};

/// The host-decl surface under test: one row per new answer lane.
const DECL: &str = "\
pub host fn qstr_pick() -> ?str;
pub host fn qbytes_pick() -> ?bytes;
pub host fn qopaque_pick() -> ?opaque;
pub host fn bytes_give() -> bytes;
";

#[test]
fn the_answer_lanes_spell_in_the_decl_grammar() {
    // `-> ?T` renders through ty_text's `?` shape and maps to its fixed
    // boot `Opt` row; plain `bytes` keeps its own boot id
    let m = lower_decl_module(DECL, "rets.d.rut").expect("the answer rows lower");
    assert_eq!(
        m.host_funcs,
        vec![
            ("qstr_pick".to_string(), vec![], TY_OPT_STR),
            ("qbytes_pick".to_string(), vec![], TY_OPT_BYTES),
            ("qopaque_pick".to_string(), vec![], TY_OPT_OPAQUE),
            ("bytes_give".to_string(), vec![], TY_BYTES),
        ],
        "each answer lane maps to its boot row"
    );
}

#[test]
fn a_q_param_still_refuses() {
    // the lanes are ANSWER-position: params keep the plain crossing
    // table (no Option param impls exist host-side to bind them)
    let err = lower_decl_module("pub host fn f(x: ?str);", "rets.d.rut")
        .expect_err("a ?str param refuses");
    assert!(err.contains("not a crossing type"), "{err}");
}

#[test]
fn nesting_beyond_the_answer_optionals_still_refuses() {
    // the `?` shape is the ONE legal nesting, one level deep — a nested
    // optional and a qualified path both refuse naming their text
    let err = lower_decl_module("pub host fn f() -> ??str;", "rets.d.rut")
        .expect_err("a nested optional refuses");
    assert!(err.contains("??str"), "{err}");
    let err = lower_decl_module("pub host fn f() -> ?Vec<?str>;", "rets.d.rut")
        .expect_err("a generic optional refuses");
    // ty_text never renders generic args (the pre-existing house shape) —
    // the refusal names the `?` head it saw
    assert!(err.contains("?Vec"), "{err}");
    let err = lower_decl_module("pub host fn f() -> ?i32;", "rets.d.rut")
        .expect_err("?i32 has no answer lane");
    assert!(err.contains("?i32"), "{err}");
}

#[test]
fn verify_against_names_the_q_rows_on_drift() {
    // the RFC 0025 join is by boot-id identity: a `-> ?str` row against
    // a `-> f64` binding panics naming BOTH shapes, the `?` spelled
    let mut s = Session::new();
    let module = lower_decl_module("pub host fn pick() -> ?str;", "rets.d.rut").unwrap();
    s.register_module("rets", module).unwrap();
    let expected = s.expected_host_fns();
    assert_eq!(
        expected.get("rets::pick"),
        Some(&(vec![], TY_OPT_STR)),
        "the mounted surface carries the boot ?str row"
    );
    let mut hosts = rut_vm::interp::HostRegistry::new();
    rut_vm::register!(
        hosts,
        "rets::pick",
        () -> f64,
        |_vm: &mut rut_vm::interp::Vm| 0.0,
    );
    let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        hosts.verify_against(&expected);
    }))
    .expect_err("a drifted answer lane panics");
    let msg = err
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    assert!(msg.contains("signature drift"), "{msg}");
    assert!(msg.contains("-> ?str"), "the pkg side names the lane: {msg}");
    assert!(msg.contains("-> f64"), "the binding side names the drift: {msg}");
}

/// The registered bodies behind [`DECL`], switchable between the Some
/// and the all-nil flavor (the shared Rc<Cell<bool>>).
fn registry(some: std::rc::Rc<std::cell::Cell<bool>>) -> rut_vm::interp::HostRegistry {
    let mut hosts = rut_vm::interp::HostRegistry::new();
    let s = some.clone();
    rut_vm::register!(hosts, "rets::qstr_pick", () -> Option<String>,
        move |_vm: &mut rut_vm::interp::Vm| {
            Ok(if s.get() { Some("ada".to_string()) } else { None })
        },
    );
    let s = some.clone();
    rut_vm::register!(hosts, "rets::qbytes_pick", () -> Option<Vec<u8>>,
        move |_vm: &mut rut_vm::interp::Vm| {
            Ok(if s.get() { Some(vec![9u8, 8]) } else { None })
        },
    );
    let s = some.clone();
    rut_vm::register!(hosts, "rets::qopaque_pick", () -> Option<rut_vm::Opaque<i64>>,
        move |vm: &mut rut_vm::interp::Vm| {
            Ok(if s.get() { Some(rut_vm::Opaque::alloc(vm, 7i64)?) } else { None })
        },
    );
    rut_vm::register!(hosts, "rets::bytes_give", () -> Vec<u8>,
        move |_vm: &mut rut_vm::interp::Vm| Ok(vec![1u8, 2, 3]),
    );
    hosts
}

/// Mount the host pkg + its consumer, run `main`, answer the result and
/// the heap balance (the answers must release exactly with the frame).
fn run_app(src: &str, some: bool) -> (i64, u64) {
    let mut s = Session::new();
    rut_driver::mount_std_core(&mut s);
    let module = lower_decl_module(DECL, "rets.d.rut").expect("the answer rows lower");
    s.register_module("rets", module).unwrap();
    s.register_module(
        "app",
        Module { source: Some(src.into()), ..Default::default() },
    )
    .unwrap();
    let out = rut_driver::compile_graph(&s, "app");
    assert!(out.diags.is_empty(), "the answer-lane consumer compiles: {:?}", out.diags);
    let prog = out.program.expect("linked program");
    let flat = rut_core::link::flatten(prog);
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let hosts = registry(std::rc::Rc::new(std::cell::Cell::new(some)));
    // the load-time contract (RFC 0025): the lanes verify by identity
    hosts.verify_against(&s.expected_host_fns());
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(flat),
        &limits,
        rut_vm::interp::HostHooks::default(),
        hosts,
    )
    .expect("vm");
    let base = vm.heap_usage();
    let answer: i64 = vm.call("main", ()).expect("run");
    (answer, vm.heap_usage() - base)
}

#[test]
fn the_some_shapes_cross_back_through_compiled_rut() {
    // every lane answers Some: the nil tests fall through and the plain
    // bytes answer's length rides home (3 octets)
    let (answer, delta) = run_app(
        "use rets::{ qstr_pick, qbytes_pick, qopaque_pick, bytes_give };\n\
         pub fn main() -> i64 {\n\
         \x20   let s = qstr_pick();\n\
         \x20   if (s == nil) { return 1; }\n\
         \x20   let b = qbytes_pick();\n\
         \x20   if (b == nil) { return 2; }\n\
         \x20   let o = qopaque_pick();\n\
         \x20   if (o == nil) { return 3; }\n\
         \x20   let raw = bytes_give();\n\
         \x20   return raw.len() as i64;\n\
         }\n",
        true,
    );
    assert_eq!(answer, 3, "every Some crossed back; bytes carried its octets");
    assert_eq!(delta, 0, "the answers release exactly with the frame");
}

#[test]
fn the_nil_shapes_answer_the_flat_nil_through_compiled_rut() {
    // every lane answers None: the first `== nil` test takes the early
    // return — the flat nil is what the caller's test reads
    let (answer, delta) = run_app(
        "use rets::{ qstr_pick, qbytes_pick, qopaque_pick, bytes_give };\n\
         pub fn main() -> i64 {\n\
         \x20   let s = qstr_pick();\n\
         \x20   if (s == nil) { return 1; }\n\
         \x20   return 0;\n\
         }\n",
        false,
    );
    assert_eq!(answer, 1, "the miss crossed as the flat nil");
    assert_eq!(delta, 0);
}

#[test]
fn the_boot_rows_keep_their_positions() {
    // guard of the boot-table append: the pre-existing ids keep their
    // rows and names (nothing reordered)
    assert_eq!(TY_I64, 8);
    assert_eq!(TY_BYTES, 15);
    assert_eq!(TY_OPT_STR, 19);
}
