//! Host pkgs are real pkgs (RFC 0025/0029, the host-pkgs plan §1): a
//! declaration-only module directory (`entry.type`, no `entry.lib`)
//! lowers its `host fn`s into the mounted surface at load time —
//! `rut/rt/` and 03-plugin's `server/` load from disk, and a consumer
//! compiles against them with no hand-written Rust surface.

use rut_core::types::{TY_I32, TY_NIL, TY_OPAQUE, TY_STR};
use rut_driver::{load_path_session, lower_decl_module, Session};
use rut_vm::OpaqueRef;

const RT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/rt");
const SERVER_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/03-plugin/server");

#[test]
fn rt_loads_from_disk_with_its_historical_scope() {
    let (session, root) =
        load_path_session(std::path::Path::new(RT_DIR)).expect("rut/rt loads");
    assert_eq!(root, "rt");
    let m = session.resolve("rt").expect("rt mounted");
    assert!(m.source.is_none(), "a host pkg has no body source");
    assert!(m.is_decl);
    assert_eq!(m.host_scope.as_deref(), Some("rt:log"));
    assert_eq!(
        m.host_funcs,
        vec![
            ("create_logger".to_string(), vec![TY_STR], TY_OPAQUE),
            ("logger_log".to_string(), vec![TY_OPAQUE, TY_I32, TY_STR], TY_NIL),
        ]
    );
}

#[test]
fn server_loads_from_disk() {
    let (session, root) =
        load_path_session(std::path::Path::new(SERVER_DIR)).expect("server loads");
    assert_eq!(root, "server");
    let m = session.resolve("server").expect("server mounted");
    assert!(m.is_decl);
    assert_eq!(m.host_scope, None, "the registration scope defaults to the name");
    assert_eq!(
        m.host_funcs,
        vec![
            ("subscribe".to_string(), vec![TY_OPAQUE, TY_STR, TY_STR], TY_NIL),
            ("emit".to_string(), vec![TY_OPAQUE, TY_STR, TY_STR], TY_NIL),
        ]
    );
}

#[test]
fn a_consumer_compiles_against_a_loaded_host_pkg() {
    let (mut session, _) = load_path_session(std::path::Path::new(SERVER_DIR)).unwrap();
    rut_driver::mount_std_core(&mut session);
    session
        .register_module(
            "app",
            rut_driver::Module {
                source: Some(
                    "\n\
                     use server::{ subscribe, emit };\n\
                     pub fn main() -> nil {\n\
                     \x20   let bus: opaque = opaque.new(0);\n\
                     \x20   subscribe(bus, \"join\", \"on_join\");\n\
                     \x20   emit(bus, \"join\", \"ada\");\n\
                     }\n"
                        .into(),
                ),
                ..Default::default()
            },
        )
        .unwrap();
    let out = rut_driver::compile_graph(&session, "app");
    assert!(out.diags.is_empty(), "diags: {:?}", out.diags);
    assert!(out.program.is_some());

    // a mistyped call is a compile diagnostic, not a call-time surprise
    session
        .register_module(
            "app",
            rut_driver::Module {
                source: Some(
                    "\n\
                     use server::{ subscribe };\n\
                     pub fn main() -> nil {\n\
                     \x20   subscribe(\"not a bus\", \"join\", \"on_join\");\n\
                     }\n"
                        .into(),
                ),
                ..Default::default()
            },
        )
        .unwrap();
    let out = rut_driver::compile_graph(&session, "app");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("`opaque`") || d.msg.contains("argument")),
        "the mistyped call must be diagnosed: {:?}",
        out.diags
    );
}

#[test]
fn non_crossing_signatures_refuse_at_load() {
    // the compiler-limitation rule (RFC 0023 §1) holds at load: a host
    // signature over anything but the crossing set refuses, naming the
    // offender
    let err = lower_decl_module(
        "pub host fn bad(v: [str]) -> str;",
        "test.d.rut",
    )
    .unwrap_err();
    assert!(
        err.contains("not a crossing type") && err.contains("bad"),
        "the load error names the offender: {err}"
    );

    let err = lower_decl_module("pub host fn takes_ptr(p: *i32);", "test.d.rut").unwrap_err();
    assert!(err.contains("not a crossing type"), "pointer params refuse: {err}");

    // a `self` receiver cannot cross
    let err = lower_decl_module("pub host fn probe(self) -> nil;", "test.d.rut").unwrap_err();
    assert!(err.contains("receiver"), "a self receiver refuses: {err}");

    // and a surface that does not parse refuses with the parse errors
    let err = lower_decl_module("pub host fn broken(;", "test.d.rut").unwrap_err();
    assert!(err.contains("does not parse"), "parse failures refuse: {err}");
}

// ---- the load-time binding contract (RFC 0025): .d.rut ↔ host impl ----
// Registration is PRE-VM now: the registry is built, checked against the
// session's declared surface, and handed to `Vm::new`, which joins it
// against the program's host thunks.

/// The magic-lane `server` bodies: the closure's Rust shape IS the
/// `.d.rut` row — `(OpaqueRef, &str, &str) -> ()`. `second` swaps emit's
/// second param type to fabricate a runtime drift for the panic tests
/// (a Rust-side drift cannot compile anymore — decision 7's point).
fn server_registry(emit_second: Option<rut_core::types::TypeId>) -> rut_vm::interp::HostRegistry {
    let mut hosts = rut_vm::interp::HostRegistry::new();
    rut_vm::register!(
        hosts,
        "server::subscribe",
        (OpaqueRef, &str, &str) -> (),
        |_vm: &mut rut_vm::interp::Vm, _bus: OpaqueRef, _t: &str, _h: &str| (),
    );
    match emit_second {
        // the surface's shape — `(OpaqueRef, str, str)`
        None | Some(TY_STR) => rut_vm::register!(
            hosts,
            "server::emit",
            (OpaqueRef, &str, &str) -> (),
            |_vm: &mut rut_vm::interp::Vm, _bus: OpaqueRef, _t: &str, _h: &str| (),
        ),
        // a fabricated runtime drift: i64 where the surface declares str
        // (a RUST-side drift cannot compile anymore — that is decision 7)
        Some(TY_I64) => rut_vm::register!(
            hosts,
            "server::emit",
            (OpaqueRef, i64, &str) -> (),
            |_vm: &mut rut_vm::interp::Vm, _bus: OpaqueRef, _n: i64, _h: &str| (),
        ),
        Some(_) => unreachable!(),
    }
    hosts
}

fn server_expected() -> rut_vm::interp::ExpectedHostFns {
    let (session, _) = load_path_session(std::path::Path::new(SERVER_DIR)).unwrap();
    session.expected_host_fns()
}

#[test]
fn a_matching_binding_table_verifies() {
    let expected = server_expected();
    assert_eq!(expected.len(), 2, "exactly the two server fns: {expected:?}");
    assert!(expected.contains_key("server::subscribe"));
    let hosts = server_registry(Some(TY_STR));
    hosts.verify_against(&expected); // no panic — the contract holds
}

#[test]
#[should_panic(expected = "declared by a mounted package but never bound")]
fn a_missing_body_panics_early() {
    let expected = server_expected();
    // `emit` never registered — the panic names it, before the Vm exists
    let mut hosts = rut_vm::interp::HostRegistry::new();
    rut_vm::register!(
        hosts,
        "server::subscribe",
        (OpaqueRef, &str, &str) -> (),
        |_vm: &mut rut_vm::interp::Vm, _bus: OpaqueRef, _t: &str, _h: &str| (),
    );
    hosts.verify_against(&expected);
}

#[test]
#[should_panic(expected = "signature drift")]
fn a_drifted_signature_panics_early() {
    use rut_core::types::TY_I64;
    let expected = server_expected();
    // the binding took an i64 where the surface declares a str
    let hosts = server_registry(Some(TY_I64));
    hosts.verify_against(&expected);
}

#[test]
#[should_panic(expected = "declared by no mounted package")]
fn an_undeclared_binding_panics_early() {
    let expected = server_expected();
    // a body for a fn no .d.rut declares — a typo'd binding caught here
    // instead of trapping mid-run
    let mut hosts = server_registry(Some(TY_STR));
    // a body for a fn no .d.rut declares — a typo'd binding caught here
    rut_vm::register!(
        hosts,
        "server::emits",
        (OpaqueRef, &str, &str) -> (),
        |_vm: &mut rut_vm::interp::Vm, _bus: OpaqueRef, _t: &str, _h: &str| (),
    );
    hosts.verify_against(&expected);
}

#[test]
fn the_vm_new_join_refuses_an_unbound_thunk() {
    // a program with ONE host thunk; an empty registry cannot boot it —
    // the error names the fn (RFC 0025: declared ⊆ bound, at boot)
    let mut prog = rut_core::binary::Program::default();
    let fname = prog.interner.intern("probe");
    let host_key = prog.interner.intern("server::probe");
    prog.funcs.push(rut_core::binary::FuncCode {
        name: fname,
        params: vec![TY_STR],
        ret: TY_NIL,
        is_method: false,
        n_captures: 0,
        regs: vec![],
        argv: vec![],
        labels: vec![],
        code: vec![],
        spans: vec![],
        host_id: Some(host_key),
    });
    let limits = rut_vm::interp::Limits::default();
    let err = match rut_vm::interp::Vm::new(
        std::rc::Rc::new(prog.clone()),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    ) {
        Err(t) => t,
        Ok(_) => panic!("an empty registry must not boot a program with host thunks"),
    };
    assert!(
        err.msg.contains("server::probe") && err.msg.contains("never bound"),
        "the boot error names the unbound thunk: {}",
        err.msg
    );
    // with the body registered, the same program boots
    let mut hosts = rut_vm::interp::HostRegistry::new();
    rut_vm::register!(hosts, "server::probe", (&str,) -> (), |_vm: &mut rut_vm::interp::Vm, _s: &str| ());
    assert!(rut_vm::interp::Vm::new(
        std::rc::Rc::new(prog),
        &limits,
        rut_vm::interp::HostHooks::default(),
        hosts
    )
    .is_ok());
}
