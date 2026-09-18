//! Host pkgs are real pkgs (RFC 0025/0029, the host-pkgs plan §1): a
//! declaration-only module directory (`entry.type`, no `entry.lib`)
//! lowers its `host fn`s into the mounted surface at load time —
//! `rut/rt/` and 03-plugin's `server/` load from disk, and a consumer
//! compiles against them with no hand-written Rust surface.

use rut_core::types::{TY_I32, TY_NIL, TY_OPAQUE, TY_STR};
use rut_driver::{load_path_session, lower_decl_module, Session};

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
                    "use core::{ Opaque };\n\
                     use server::{ subscribe, emit };\n\
                     pub fn main() -> nil {\n\
                     \x20   let bus: Opaque = Opaque.new(0);\n\
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
                    "use core::{ Opaque };\n\
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
        out.diags.iter().any(|d| d.msg.contains("`Opaque`") || d.msg.contains("argument")),
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
        "pub host fn bad(v: Array<str>) -> str;",
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
