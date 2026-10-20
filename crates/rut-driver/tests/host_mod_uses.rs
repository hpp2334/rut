//! The host `.d.rut` MOD TREE, phase 2: QUALIFIED BINDING — a consumer
//! spells `use myhost::net::{ connect }` (deep paths included,
//! `myhost::net::tls::handshake`) and the leaf binds through the
//! externs gate against the dep surface's path-qualified row; calling
//! it executes the embedder-registered body end to end. The same
//! visibility law as lib deps holds: `pub mod` edges (a bare `mod` is
//! invisible from outside), `pub host fn` leaves (a bare `host fn` is
//! private), and each leaf must be declared in the walked module.
//!
//! Phase 1's flat lane rides unchanged: `use myhost::{ legacy_ping }`
//! binds the root row exactly as before, and flat host pkgs are
//! byte-identical.

use std::collections::BTreeMap;

use rut_vm::interp::Vm;
use rut_vm::HostPkg;

fn mod_source(path: &str, vis: rut_ast::ast::Vis, text: &str) -> rut_driver::ModSource {
    rut_driver::ModSource {
        path: path.to_string(),
        vis,
        text: text.to_string(),
        file: "mod.rut".to_string(),
    }
}

/// The plan's target shape, hand-lowered: root + net + net/tls.
fn myhost() -> rut_driver::Mounted {
    let root = "pub mod net;\npub host fn legacy_ping() -> bool;\n";
    let mut children = BTreeMap::new();
    children.insert(
        "net".to_string(),
        mod_source(
            "net",
            rut_ast::ast::Vis::Pub,
            "pub mod tls;\npub host fn connect(url: str) -> i64;\n",
        ),
    );
    children.insert(
        "net/tls".to_string(),
        mod_source(
            "net/tls",
            rut_ast::ast::Vis::Pub,
            "pub host fn handshake(c: i64) -> i64;\n",
        ),
    );
    rut_driver::lower_decl_module(root, &children, "myhost.d.rut")
        .expect("lowers")
        .named("myhost")
}

fn graph_of(c: Result<rut_driver::Compiled, rut_driver::RunError>) -> rut_driver::GraphOutput {
    match c {
        Ok(c) => c.graph,
        Err(e) => rut_driver::GraphOutput {
            diags: vec![rut_lexer::diag::Diag::new(
                rut_lexer::span::Span::new(0, 0),
                e.msg,
            )],
            program: None,
        },
    }
}

fn compile(host: rut_driver::Mounted, src: &str) -> rut_driver::GraphOutput {
    let head = host.spec.clone();
    graph_of(
        rut_driver::RutRun::new()
            .pkg(host)
            .pkg(
                // the app DECLARES its use (the host surface's spec)
                rut_driver::Mounted::source("app", src)
                    .manifest(rut_driver::Manifest::lib("app", &[&head])),
            )
            .entrypoint("app")
            .compile(),
    )
}

#[test]
fn a_qualified_host_use_binds_and_calls() {
    let out = compile(
        myhost(),
        "use myhost::net::{ connect };\n\nentry fn main() -> i64 {\n    return connect(\"hello\");\n}\n",
    );
    assert!(out.diags.is_empty(), "diags: {:?}", out.diags);
    assert!(out.program.is_some());
}

#[test]
fn a_deep_single_name_spelling_binds() {
    // `use myhost::net::tls::handshake;` — the deep path, the leaf
    // popped into names by the use grammar
    let out = compile(
        myhost(),
        "use myhost::net::tls::handshake;\n\nentry fn main() -> i64 {\n    return handshake(7);\n}\n",
    );
    assert!(out.diags.is_empty(), "diags: {:?}", out.diags);
}

#[test]
fn a_flat_root_row_binds_unchanged() {
    // phase 1's lane: the flat use of a ROOT row is today's shape
    let out = compile(
        myhost(),
        "use myhost::{ legacy_ping };\n\nentry fn main() -> nil {\n    let _ = legacy_ping();\n}\n",
    );
    assert!(out.diags.is_empty(), "diags: {:?}", out.diags);
}

#[test]
fn a_flat_use_never_reaches_into_the_tree() {
    // the mod row's surface spelling is `net::connect` — a FLAT use of
    // the bare leaf must not bind it (the qualified law's other half).
    // It never binds, and the use-line leaf gate now says so where the
    // import is written: myhost's ROOT module has no `connect` (the
    // row lives in `net`, the flat spelling walks no tree)
    let out = compile(
        myhost(),
        "use myhost::{ connect };\n\nentry fn main() -> nil {\n    let _ = connect(\"x\");\n}\n",
    );
    assert!(
        out.diags
            .iter()
            .any(|d| d.msg.contains("`myhost` has no `connect` — unknown import")),
        "the flat spelling of a mod row is rejected at the use line: {:?}",
        out.diags
    );
}

#[test]
fn a_wrong_leaf_is_diagnosed_against_the_walked_module() {
    let out = compile(
        myhost(),
        "use myhost::net::{ nope };\n\nentry fn main() -> nil {}\n",
    );
    assert!(
        out.diags.iter().any(|d| d
            .msg
            .contains("`nope` is not declared in `myhost`'s module `net`")),
        "the wrong leaf is named: {:?}",
        out.diags
    );
    // deep: the walked module is the child's child
    let out = compile(
        myhost(),
        "use myhost::net::tls::{ nope };\n\nentry fn main() -> nil {}\n",
    );
    assert!(
        out.diags.iter().any(|d| d
            .msg
            .contains("`nope` is not declared in `myhost`'s module `net/tls`")),
        "the deep wrong leaf names the walked module: {:?}",
        out.diags
    );
}

#[test]
fn a_bare_host_fn_leaf_does_not_cross() {
    // `host fn` (no `pub`) in the child: the row is collected (the
    // declaration EXISTS) but is not `pub` — the leaf gate refuses
    let host = {
        let root = "pub mod net;\n";
        let mut children = BTreeMap::new();
        children.insert(
            "net".to_string(),
            mod_source(
                "net",
                rut_ast::ast::Vis::Pub,
                "host fn connect(url: str) -> i64;\n",
            ),
        );
        rut_driver::lower_decl_module(root, &children, "myhost.d.rut")
            .expect("lowers")
            .named("myhost")
    };
    let out = compile(
        host,
        "use myhost::net::{ connect };\n\nentry fn main() -> nil {\n    let _ = connect(\"x\");\n}\n",
    );
    assert!(
        out.diags
            .iter()
            .any(|d| d.msg.contains("`connect` in `myhost::net` is not `pub`")),
        "a bare `host fn` leaf is private: {:?}",
        out.diags
    );
}

#[test]
fn a_bare_mod_edge_is_invisible_from_outside() {
    let host = {
        let root = "mod net;\npub host fn legacy_ping() -> bool;\n";
        let mut children = BTreeMap::new();
        children.insert(
            "net".to_string(),
            mod_source(
                "net",
                rut_ast::ast::Vis::Self_,
                "pub host fn connect(url: str) -> i64;\n",
            ),
        );
        rut_driver::lower_decl_module(root, &children, "myhost.d.rut")
            .expect("lowers")
            .named("myhost")
    };
    let out = compile(
        host,
        "use myhost::net::{ connect };\n\nentry fn main() -> nil {\n    let _ = connect(\"x\");\n}\n",
    );
    assert!(
        out.diags.iter().any(|d| d
            .msg
            .contains("`net` is not visible from outside the package")
            && d.msg.contains("`myhost`'s mod.d.rut")),
        "the edge gate names the surface file: {:?}",
        out.diags
    );
}

#[test]
fn a_qualified_host_call_runs_the_registered_body() {
    // the e2e: a rut unit imports `myhost::net::{ connect }` and CALLS
    // it; the mock row (`net::connect`) executes — the registered
    // body's answer comes back through the bound leaf
    let mut pkg = HostPkg::new("myhost");
    pkg.register::<_, (), bool, _>("legacy_ping", |_vm: &mut Vm| true);
    pkg.register::<_, (&str,), i64, _>("net::connect", |_vm: &mut Vm, url: &str| url.len() as i64);
    pkg.register::<_, (i64,), i64, _>("net::tls::handshake", |_vm: &mut Vm, c: i64| c * 2);
    let compiled = rut_driver::RutRun::new()
        .pkg(myhost())
        .pkg(pkg.build())
        .pkg(
            // the app DECLARES its use (the myhost host surface)
            rut_driver::Mounted::source(
                "app",
                "use myhost::net::{ connect };\nuse myhost::net::tls::handshake;\nuse myhost::{ legacy_ping };\n\nentry fn main() -> i64 {\n    let _ = legacy_ping();\n    return handshake(connect(\"abc\"));\n}\n",
            )
            .manifest(rut_driver::Manifest::lib("app", &["myhost"])),
        )
        .entrypoint("app")
        .compile()
        .expect("compiles");
    assert!(
        compiled.graph.diags.is_empty(),
        "{:?}",
        compiled.graph.diags
    );
    let prog = std::rc::Rc::new(rut_core::link::flatten(
        compiled.graph.program.expect("linked"),
    ));
    let mut vm = rut_vm::interp::Vm::builder()
        .program(prog)
        .limits(rut_vm::interp::Limits::default())
        .hooks(rut_vm::interp::HostHooks::default())
        .hosts(compiled.hosts)
        .build()
        .expect("boots");
    let n: i64 = vm.call("main", ()).expect("runs");
    assert_eq!(
        n, 6,
        "connect(\"abc\") = 3, handshake(3) = 6 — the mock bodies ran"
    );
}

#[test]
fn an_async_mod_row_binds_through_the_qualified_spelling() {
    // `host async fn` under a mod: the row family registers on the
    // path-qualified name and the qualified use binds the leaf (the
    // call weaves into the host future at the async call site)
    let host = {
        let root = "pub mod net;\n";
        let mut children = BTreeMap::new();
        children.insert(
            "net".to_string(),
            mod_source(
                "net",
                rut_ast::ast::Vis::Pub,
                "pub host async fn probe(k: str) -> i64;\n",
            ),
        );
        rut_driver::lower_decl_module(root, &children, "fixture.d.rut")
            .expect("lowers")
            .named("fixture")
    };
    let futures = rut_native::dir_pkgs(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rut/futures")
            .as_path(),
    )
    .expect("the futures pkg walks");
    let out = graph_of(
        futures
            .pkgs
            .iter()
            .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()))
            .pkg(host)
            .pkg(
                // the app DECLARES its uses (futures + the fixture surface)
                rut_driver::Mounted::source(
                    "app",
                    "use futures::{ launch_future };\nuse fixture::net::{ probe };\n\nasync fn job() -> nil {\n    let n = await probe(\"k\");\n    let _ = n;\n}\n\nentry fn main() -> nil {\n    launch_future(job());\n}\n",
                )
                .manifest(rut_driver::Manifest::lib("app", &["fixture", "futures"])),
            )
            .entrypoint("app")
            .compile(),
    );
    assert!(out.diags.is_empty(), "diags: {:?}", out.diags);
}
