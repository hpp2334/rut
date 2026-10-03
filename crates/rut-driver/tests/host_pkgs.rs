//! Host pkgs are real pkgs (the host-pkgs plan): a
//! declaration-only module directory (`entry.type`, no `entry.lib`)
//! lowers its `host fn`s into the mounted surface at load time —
//! `rut/ink_host/` and 03-plugin's `server/` load from disk, and a
//! consumer compiles against them with no hand-written Rust surface.

use std::path::Path;

use rut_core::types::{TY_I32, TY_NIL, TY_OPAQUE, TY_STR};
use rut_driver::{load_path_session, lower_decl_module};
use rut_vm::OpaqueRef;

const INK_HOST_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/ink_host");
const SERVER_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/03-plugin/server");




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

#[test]
fn ink_host_loads_from_disk_with_its_name_scope() {
    let loaded = load_path_session(std::path::Path::new(INK_HOST_DIR)).expect("rut/ink_host loads");
    assert_eq!(loaded.root, "ink_host");
    let m = loaded.pkg("ink_host").expect("ink_host mounted");
    assert!(
        matches!(m.body, rut_driver::PkgBody::Host { .. }),
        "a host pkg is a host body — no rut source"
    );
    let rut_driver::PkgBody::Host { ref host_funcs, .. } = m.body else { panic!("host body") };
    assert_eq!(
        *host_funcs,
        vec![
            ("create_logger".to_string(), vec![TY_STR], TY_OPAQUE, false),
            ("logger_log".to_string(), vec![TY_OPAQUE, TY_I32, TY_STR], TY_NIL, false),
        ]
    );
}

#[test]
fn server_loads_from_disk() {
    let loaded = load_path_session(std::path::Path::new(SERVER_DIR)).expect("server loads");
    assert_eq!(loaded.root, "server");
    let m = loaded.pkg("server").expect("server mounted");
    let rut_driver::PkgBody::Host { ref host_funcs, .. } = m.body else { panic!("host body") };
    assert_eq!(
        *host_funcs,
        vec![
            ("subscribe".to_string(), vec![TY_OPAQUE, TY_STR, TY_STR], TY_NIL, false),
            ("emit".to_string(), vec![TY_OPAQUE, TY_STR, TY_STR], TY_NIL, false),
        ]
    );
}

#[test]
fn a_consumer_compiles_against_a_loaded_host_pkg() {
    let server = load_path_session(std::path::Path::new(SERVER_DIR)).unwrap();
    let compile_app = |src: &str| {
        rut_driver::RutRun::new()
            .pkgs(&server)
            .pkg(rut_driver::Pkg::source("app", src))
            .entrypoint("app")
            .compile()
            .unwrap()
    };
    let out = compile_app(
        "\n\
         use server::{ subscribe, emit };\n\
         entry fn main() -> nil {\n\
         \x20   let bus: opaque = opaque(0);\n\
         \x20   subscribe(bus, \"join\", \"on_join\");\n\
         \x20   emit(bus, \"join\", \"ada\");\n\
         }\n",
    );
    assert!(out.graph.diags.is_empty(), "diags: {:?}", out.graph.diags);
    assert!(out.graph.program.is_some());

    // a mistyped call is a compile diagnostic, not a call-time surprise
    let out = compile_app(
        "\n\
         use server::{ subscribe };\n\
         entry fn main() -> nil {\n\
         \x20   subscribe(\"not a bus\", \"join\", \"on_join\");\n\
         }\n",
    );
    assert!(
        out.graph.diags.iter().any(|d| d.msg.contains("`opaque`") || d.msg.contains("argument")),
        "the mistyped call must be diagnosed: {:?}",
        out.graph.diags
    );
}

// ---- the declared kind: host-pkg-ness is spelled, never inferred ----

/// A temp module dir from a manifest + file map.
fn make_pkg(base: &Path, files: &[(&str, &str)]) -> std::path::PathBuf {
    let dir = base.join("pkg");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, text) in files {
        std::fs::write(dir.join(name), text).unwrap();
    }
    dir
}

#[test]
fn a_lib_surfaces_host_fns_are_refused_with_the_fix() {
    // the lib-surface law: `host fn` text lives only in
    // `type = "host"` pkgs. A surface that PARSES and declares host
    // fns is the loud error, naming the row and the fix.
    let base = std::env::temp_dir().join(format!("rut-lib-surface-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let dir = make_pkg(
        &base,
        &[
            ("rut.jsonc", r#"{"name": "s", "type": "lib", "entry": {"type": "./s.d.rut"}}"#),
            ("s.d.rut", "pub host fn ping(x: i32) -> i32;\n"),
        ],
    );
    let err = load_path_session(&dir).unwrap_err().to_string();
    assert!(err.contains("s.d.rut"), "{err}");
    assert!(err.contains("`host fn ping`"), "{err}");
    assert!(err.contains("`type = \"host\"`"), "{err}");

    // a surface that does not PARSE stays inert (doc-only, as today) —
    // the refusal fires only on a surface that parses into host rows
    let dir = make_pkg(
        &base,
        &[
            ("rut.jsonc", r#"{"name": "s", "type": "lib", "entry": {"type": "./s.d.rut"}}"#),
            ("s.d.rut", "this is not rut source at all <<<\n"),
        ],
    );
    let loaded = load_path_session(&dir).unwrap();
    assert_eq!(loaded.root, "s");
    assert!(matches!(
        loaded.pkg("s").unwrap().body,
        rut_driver::PkgBody::Source { is_decl: true, .. }
    ));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn the_surface_only_dev_state_mounts_as_a_decl_unit() {
    // a `type = "lib"` pkg with a surface and no body is the dev
    // state: a decl unit — no host rows, nothing exported (use sites
    // resolve-miss, correctly)
    let base = std::env::temp_dir().join(format!("rut-dev-state-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let dir = make_pkg(
        &base,
        &[
            ("rut.jsonc", r#"{"name": "s", "type": "lib", "entry": {"type": "./s.d.rut"}}"#),
            ("s.d.rut", "/// documented surface, no body yet.\n"),
        ],
    );
    let loaded = load_path_session(&dir).unwrap();
    assert_eq!(loaded.root, "s");
    let m = loaded.pkg("s").unwrap();
    assert!(
        matches!(&m.body, rut_driver::PkgBody::Source { is_decl: true, .. }),
        "the dev state is a decl unit, not a host body: {:?}",
        m.body
    );
    // a consumer's use of it resolve-misses — the loud, correct answer
    // (the pkg resolves; the ITEM misses as the ordinary unknown name)
    let out = rut_driver::RutRun::new()
        .pkgs(&loaded)
        .pkg(rut_driver::Pkg::source("app", "use s::{ ping };\nentry fn main() -> nil { ping(1); }\n"))
        .entrypoint("app")
        .compile()
        .unwrap();
    assert!(
        out.graph
            .diags
            .iter()
            .any(|d| d.msg.contains("cannot resolve") || d.msg.contains("unknown")),
        "the use site must resolve-miss: {:?}",
        out.graph.diags
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn non_crossing_signatures_refuse_at_load() {
    // the compiler-limitation rule holds at load: a host
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

    let err = lower_decl_module("pub host fn takes_ptr(p: ?i32);", "test.d.rut").unwrap_err();
    assert!(err.contains("not a crossing type"), "pointer params refuse: {err}");

    // a `self` receiver cannot cross
    let err = lower_decl_module("pub host fn probe(self) -> nil;", "test.d.rut").unwrap_err();
    assert!(err.contains("receiver"), "a self receiver refuses: {err}");

    // and a surface that does not parse refuses with the parse errors
    let err = lower_decl_module("pub host fn broken(;", "test.d.rut").unwrap_err();
    assert!(err.contains("does not parse"), "parse failures refuse: {err}");
}

// ---- the load-time binding contract: .d.rut ↔ host impl ----
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
    let loaded = load_path_session(std::path::Path::new(SERVER_DIR)).unwrap();
    rut_driver::declared_host_fns(&loaded.pkgs)
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
    // the error names the fn (declared ⊆ bound, at boot)
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
        pos: vec![],
        host_id: Some(host_key),
    });
    let limits = rut_vm::interp::Limits::default();
    let err = match rut_vm::interp:: Vm::builder().program(std::rc::Rc::new(prog.clone())).limits(limits.clone()).hooks(rut_vm::interp::HostHooks::default()).hosts(rut_vm::interp::HostRegistry::new()).build() {
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
    assert!(rut_vm::interp:: Vm::builder().program(std::rc::Rc::new(prog)).limits(limits.clone()).hooks(rut_vm::interp::HostHooks::default()).hosts(hosts).build()
    .is_ok());
}

// ---- the installer lane: `host_pkg_context` + `install_host_pkg` ----

#[test]
fn host_pkg_context_partitions_per_pkg_and_expands_async() {
    // the ctx partitions the mounted host pkgs' rows by scope (the pkg
    // name — the registration naming has no override) and expands async
    // rows into their family
    use rut_core::types::{TY_BOOL, TY_STR};

    let ink_host = lower_decl_module(
        include_str!("../../../rut/ink_host/ink_host.d.rut"),
        "ink_host.d.rut",
    )
    .expect("ink_host surface")
    .named("ink_host");
    let mut engine = rut_driver::Pkg::host(
        "engine",
        vec![("probe".to_string(), vec![TY_STR], TY_BOOL, true)],
    );
    engine.body = rut_driver::PkgBody::Host {
        host_funcs: vec![("probe".to_string(), vec![TY_STR], TY_BOOL, true)],
        consts: vec![],
        native_types: vec![],
        native_fns: vec![],
        native_impls: vec![],
    };
    let world = vec![ink_host, engine];
    let ctx = rut_driver::host_pkg_ctx(&world);
    // the partition: per-scope rows, scoped from the spec
    let ink = ctx.rows_of("ink_host").expect("ink_host partitioned");
    assert!(ink.contains_key("create_logger"));
    assert!(ink.contains_key("logger_log"));
    assert!(!ink.contains_key("__launch"), "rows never cross scopes");
    let engine = ctx.rows_of("engine").expect("engine partitioned");
    // the async expansion: one decl row → the five-row family
    for suffix in ["", "__start", "__yield", "__take", "__cancel"] {
        assert!(engine.contains_key(&format!("probe{suffix}")), "probe{suffix}");
    }
    assert!(ctx.is_mounted("engine"));
    assert!(!ctx.is_mounted("nmap_host"));
    assert_eq!(
        ctx.scopes().collect::<Vec<_>>(),
        vec!["engine", "ink_host"],
        "the scopes iterate in name order"
    );

    // the raw-lane compatibility: declared_host_fns IS the flatten
    assert_eq!(rut_driver::declared_host_fns(&world), ctx.flatten());

    // the pkg contract pins re-run per-pkg: the ink_host pkg satisfies
    // its scope's rows exactly
    let mut hosts = rut_vm::interp::HostRegistry::new();
    hosts.install_host_pkg(&ctx, rut_std::logger::pkg(|_| {}));
    // a pkg whose installer left a row out panics naming ITS scope —
    // the engine rows are declared and nobody installed them here, so
    // verify_against (the net) names the pkg's row
    let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        hosts.verify_against(&ctx.flatten());
    }))
    .err()
    .and_then(|e| e.downcast_ref::<String>().cloned())
    .expect("verify panics on the uninstalled engine rows");
    assert!(
        err.contains("engine::") && err.contains("never bound"),
        "{err}"
    );
}

#[test]
fn a_missing_installer_panics_naming_the_pkg_and_blanket_installs_boot_clean() {
    // the e2e contract: TWO host pkgs mounted, one installer missing —
    // the net (`Vm::new`) panics naming the pkg; installing BOTH over a
    // session that mounts a subset boots clean (the inert-merge law)
    use rut_vm::interp::{HostRegistry, Limits, Vm};

    let ink_host = lower_decl_module(
        include_str!("../../../rut/ink_host/ink_host.d.rut"),
        "ink_host.d.rut",
    )
    .expect("ink_host surface")
    .named("ink_host");
    let server = lower_decl_module(
        "pub host fn subscribe(bus: opaque, topic: str, handler: str) -> nil;\n",
        "server.d.rut",
    )
    .expect("server surface")
    .named("server");
    let world = vec![
        ink_host,
        server,
        rut_driver::Pkg::source(
            "app",
            "use ink_host::{ create_logger };\nuse server::{ subscribe };\nentry fn main() -> opaque {\n    let log = create_logger(\"t\");\n    let bus: opaque = opaque(0);\n    subscribe(bus, \"join\", \"on_join\");\n    return log;\n}\n",
        ),
    ];
    let compiled = rut_driver::RutRun::new()
        .pkgs(&rut_driver::Loaded { pkgs: world.clone(), root: String::new() })
        .entrypoint("app")
        .compile()
        .unwrap();
    assert!(compiled.graph.diags.is_empty(), "{:?}", compiled.graph.diags);
    let prog = std::rc::Rc::new(rut_core::link::flatten(compiled.graph.program.expect("linked")));
    let limits = Limits::default();

    // ONE installer (ink_host's) ran; server's never did — the boot
    // refuses naming the missing pkg's row (the net: today's join)
    let ctx = rut_driver::host_pkg_ctx(&world);
    let mut hosts = HostRegistry::new();
    hosts.install_host_pkg(&ctx, rut_std::logger::pkg(|_| {}));
    let err = match Vm::builder().program(prog.clone()).limits(limits.clone()).hooks(rut_vm::interp::HostHooks::default()).hosts(hosts).build() {
        Err(t) => t,
        Ok(_) => panic!("a missing installer must not boot the program"),
    };
    assert!(err.msg.contains("server::subscribe"), "{}", err.msg);
    assert!(err.msg.contains("never bound"), "{}", err.msg);

    // the blanket install: BOTH pkgs installed (server's rows bound as
    // inert extras — the program never mounts a `server` use), boots clean
    let mut hosts = HostRegistry::new();
    hosts.install_host_pkg(&ctx, rut_std::logger::pkg(|_| {}));
    rut_vm::register!(
        hosts,
        "server::subscribe",
        (OpaqueRef, &str, &str) -> (),
        |_vm: &mut rut_vm::interp::Vm, _b: OpaqueRef, _t: &str, _h: &str| ()
    );
    let mut vm = Vm::builder().program(prog).limits(limits.clone()).hooks(rut_vm::interp::HostHooks::default()).hosts(hosts).build()
        .expect("the blanket install boots");
    let _: OpaqueRef = vm.call("main", ()).expect("main");
}
