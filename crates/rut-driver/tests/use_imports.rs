//! Flat imports validate at the use line (`use dep::{ A, B }` / `use
//! dep::A;`): every leaf must be a decl of the dep's ROOT module — the
//! surface the flat binding walks — or the dep's namespace head. A
//! typo'd import (`use ink::{ Nope };`) used to compile silently and
//! only fail later at a call site that might never be written; the
//! graph now rejects it where it is written, positioned at the use
//! line (`--> file:line:col`). The gate names only what the dep never
//! declared (plus mod names, which no surface carries); visibility is
//! the validated-must-bind law's (plan10 phase 2) — a leaf that binds
//! nothing errors AT the use line, a private row naming the `pub` fix —
//! while `.d.rut` surfaces and std packages keep loading clean. The
//! binding case runs against a LOCAL dep — no std drift.

use std::collections::BTreeMap;

use rut_ast::ast::Vis;

const DEP: &str = "util";

/// The local dep: a flat source pkg with a pub fn, a private fn, and a
/// mounted `pub mod sub` — the leaf gate's shapes in one package.
fn dep_pkg() -> rut_driver::Mounted {
    let mut p = rut_driver::Mounted::source(
        DEP,
        "pub fn half(x: i32) -> i32 { return x / 2; }\n\
         fn hidden() -> i32 { return 7; }\n\
         pub mod sub;\n",
    );
    p.mods.insert(
        "sub".to_string(),
        rut_driver::ModSource {
            path: "sub".to_string(),
            vis: Vis::Pub,
            text: "pub fn one() -> i32 { return 1; }\n".to_string(),
            file: "mod.rut".to_string(),
        },
    );
    p
}

/// `app` over [`dep_pkg`]: the root DECLARES its dep (the structural
/// manifest); the chain's graph diags come back (a closure refusal as
/// one span-0 diagnostic).
fn graph_of(app_src: &str) -> rut_driver::GraphOutput {
    match rut_driver::RutRun::new()
        .pkg(dep_pkg())
        .pkg(
            rut_driver::Mounted::source("app", app_src)
                .manifest(rut_driver::Manifest::lib("app", &[DEP])),
        )
        .entrypoint("app")
        .compile()
    {
        Ok(c) => c.graph,
        Err(e) => rut_driver::GraphOutput {
            diags: vec![rut_lexer::diag::Diag::new(rut_lexer::span::Span::new(0, 0), e.msg)],
            program: None,
        },
    }
}

fn msgs(g: &rut_driver::GraphOutput) -> Vec<String> {
    g.diags.iter().map(|d| d.msg.clone()).collect()
}

/// The rendered arrow for a root-file diag: the diag's file is the
/// spec, the source under it is the app's own text.
fn rendered(src: &str, g: &rut_driver::GraphOutput) -> String {
    rut_lexer::diag::render_diags_grouped(
        &g.diags,
        &BTreeMap::from([("app".to_string(), src)]),
        None,
    )
}

#[test]
fn unknown_flat_leaf_rejects_at_the_use_line() {
    // the binding case: `Nope` is never referenced — the import alone
    // is the error, diagnosed once, at the line it is written on
    let src = "use util::{ Nope };\n\
               entry fn main() -> i32 { return 0; }\n";
    let g = graph_of(src);
    assert_eq!(
        g.diags.len(),
        1,
        "exactly one diagnostic: {:?}",
        msgs(&g)
    );
    let d = &g.diags[0];
    assert!(
        d.msg.contains("has no `Nope` — unknown import"),
        "got: {}",
        d.msg
    );
    assert!(d.msg.contains("`util`"), "the dep is named: {}", d.msg);
    // the use statement is line 1, column 1 of the app's root file
    assert_eq!(d.file.as_deref(), Some("app"));
    assert!(
        rendered(src, &g).contains("--> app:1:1"),
        "the arrow names the use line:\n{}",
        rendered(src, &g)
    );
}

#[test]
fn the_single_name_flat_form_rejects_too() {
    // `use util::Nope;` degenerates to the same flat leaf — same gate,
    // same position
    let src = "use util::Nope;\n\
               entry fn main() -> i32 { return 0; }\n";
    let g = graph_of(src);
    assert_eq!(g.diags.len(), 1, "exactly one diagnostic: {:?}", msgs(&g));
    assert!(
        g.diags[0].msg.contains("has no `Nope` — unknown import"),
        "got: {}",
        g.diags[0].msg
    );
    assert!(
        rendered(src, &g).contains("--> app:1:1"),
        "the arrow names the use line:\n{}",
        rendered(src, &g)
    );
}

#[test]
fn a_valid_flat_import_compiles_and_the_name_runs() {
    // the control: a real leaf compiles with ZERO diagnostics and the
    // imported name stays usable — main calls it, the program runs
    let src = "use util::{ half };\n\
               entry fn main() -> i32 { return half(10); }\n";
    let g = graph_of(src);
    assert!(g.diags.is_empty(), "zero diagnostics: {:?}", msgs(&g));
    let flat = rut_core::link::flatten(g.program.expect("linked program"));
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::builder()
        .program(std::rc::Rc::new(flat))
        .limits(limits)
        .hooks(rut_vm::interp::HostHooks::default())
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("vm");
    assert_eq!(vm.call::<_, i32>("main", ()).expect("run"), 5);
}

#[test]
fn a_private_leaf_errors_at_the_use_line() {
    // `hidden` IS declared (private) — the validated-must-bind law
    // (plan10 phase 2): the leaf binds nothing (the surface carries
    // `pub` rows only), and the use line says so, naming the `pub`
    // fix. The old silence is gone: the error is the import's own,
    // positioned at the line it is written on.
    let src = "use util::{ hidden };\n\
               entry fn main() -> i32 { return 0; }\n";
    let g = graph_of(src);
    assert_eq!(g.diags.len(), 1, "exactly one diagnostic: {:?}", msgs(&g));
    assert!(
        g.diags[0]
            .msg
            .contains("`hidden` is not exported by `util`")
            && g.diags[0].msg.contains("mark it `pub`"),
        "got: {}",
        g.diags[0].msg
    );
    assert_eq!(g.diags[0].file.as_deref(), Some("app"));
    assert!(
        rendered(src, &g).contains("--> app:1:1"),
        "the arrow names the use line:\n{}",
        rendered(src, &g)
    );
}

#[test]
fn a_flat_leaf_naming_a_module_diagnoses_with_the_walk_hint() {
    // `sub` is the dep's `pub mod` — no surface ever carries a module,
    // so the leaf would bind nothing; the diag says what to spell
    // instead instead of the flat `has no` shape
    let src = "use util::{ sub };\n\
               entry fn main() -> i32 { return 0; }\n";
    let g = graph_of(src);
    assert_eq!(g.diags.len(), 1, "exactly one diagnostic: {:?}", msgs(&g));
    assert!(
        g.diags[0]
            .msg
            .contains("`sub` is a module of `util`")
            && g.diags[0].msg.contains("use util::sub::{ .. }"),
        "got: {}",
        g.diags[0].msg
    );
}

// ---- the std lane: same gate, the toolchain tree's ink pkg ----

/// The toolchain tree's ink walk (`rut/ink`): the pkg plus its
/// `ink_host` dep; only the root's closure compiles.
fn std_graph_of(app_src: &str) -> rut_driver::GraphOutput {
    let tree = rut_native::dir_pkgs(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut/ink"))
        .expect("the ink walk");
    match tree
        .pkgs
        .iter()
        .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()))
        .pkg(
            rut_driver::Mounted::source("app", app_src)
                .manifest(rut_driver::Manifest::lib("app", &["ink"])),
        )
        .entrypoint("app")
        .compile()
    {
        Ok(c) => c.graph,
        Err(e) => rut_driver::GraphOutput {
            diags: vec![rut_lexer::diag::Diag::new(rut_lexer::span::Span::new(0, 0), e.msg)],
            program: None,
        },
    }
}

#[test]
fn an_unknown_std_leaf_rejects_at_the_use_line() {
    // the probed shape, against a real std pkg: `use ink::{ Nope };`
    // compiled silently before; now it names the miss, at the line
    let src = "use ink::{ Nope };\n\
               entry fn main() -> i32 { return 0; }\n";
    let g = std_graph_of(src);
    assert_eq!(g.diags.len(), 1, "exactly one diagnostic: {:?}", msgs(&g));
    assert!(
        g.diags[0].msg.contains("has no `Nope` — unknown import"),
        "got: {}",
        g.diags[0].msg
    );
    assert_eq!(g.diags[0].file.as_deref(), Some("app"));
    assert!(
        rendered(src, &g).contains("--> app:1:1"),
        "the arrow names the use line:\n{}",
        rendered(src, &g)
    );
}

#[test]
fn a_valid_std_leaf_compiles_clean() {
    // and the legitimate std import stays clean: `Logger` is ink's pub
    // class at its root — zero diagnostics, compile only (running it
    // would want ink_host's registered bodies)
    let src = "use ink::{ Logger };\n\
               entry fn main() -> i32 { return 0; }\n";
    let g = std_graph_of(src);
    assert!(g.diags.is_empty(), "zero diagnostics: {:?}", msgs(&g));
}
