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

use rut_ast::Vis;

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
        .mount(dep_pkg())
        .mount(
            rut_driver::Mounted::source("app", app_src)
                .manifest(rut_driver::Manifest::lib("app", &[DEP])),
        )
        .entrypoint("app")
        .compile()
    {
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
    assert_eq!(g.diags.len(), 1, "exactly one diagnostic: {:?}", msgs(&g));
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
fn a_flat_leaf_binds_the_module_for_qualified_access() {
    // `sub` is the dep's `pub mod` — the leaf binds the MODULE now
    // (`use util::{ sub }` + `sub::top()`), and a PRIVATE mod leaf
    // still refuses, naming the `pub mod` fix
    let src = "use util::{ sub };\n\
               entry fn main() -> i32 { return sub::one(); }\n";
    let g = graph_of(src);
    assert!(
        !msgs(&g)
            .iter()
            .any(|m| m.contains("`sub` is a module of `util`")),
        "the module leaf binds: {:?}",
        msgs(&g)
    );
    // the fully-qualified form needs no use at all — the path roots
    // at the dep's own name
    let src = "entry fn main() -> i32 { return util::sub::one(); }\n";
    let g = graph_of(src);
    assert!(
        !msgs(&g)
            .iter()
            .any(|m| m.contains("`sub` is a module of `util`")),
        "the pkg-rooted path resolves: {:?}",
        msgs(&g)
    );
}

// ---- `pub use`: the re-export edge joins the module surface ----

/// The re-export chain: `util` (the source), `mid` (re-exports `util`'s
/// `helper` — `pub use` or private, per `reexport`), and `app` (imports
/// the name flat off `mid`). `mid`'s own manifest declares `util`.
fn reexport_graph_of(reexport: bool, app_src: &str) -> rut_driver::GraphOutput {
    let mid_root = if reexport {
        "pub use util::{ helper };\n"
    } else {
        "use util::{ helper };\n"
    };
    match rut_driver::RutRun::new()
        .mount(rut_driver::Mounted::source(
            "util",
            "pub fn helper() -> i32 { return 3; }\n",
        ))
        .mount(
            rut_driver::Mounted::source("mid", mid_root)
                .manifest(rut_driver::Manifest::lib("mid", &["util"])),
        )
        .mount(
            rut_driver::Mounted::source("app", app_src)
                .manifest(rut_driver::Manifest::lib("app", &["mid"])),
        )
        .entrypoint("app")
        .compile()
    {
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

#[test]
fn a_pub_use_re_exported_name_is_the_pkgs_own_surface() {
    // the leaf gate's read: a `pub use`'s imported names ride the
    // module's decl rows, so `use mid::{ helper }` names a row mid
    // DID declare — the `unknown import` refusal is gone. The
    // compiled-surface binding does not chase the edge yet (the
    // must-bind law still speaks, phase 4 threads the const lane) —
    // but the surface question this gate owns answers "declared".
    let src = "use mid::{ helper };\n\
               entry fn main() -> i32 { return 0; }\n";
    let g = reexport_graph_of(true, src);
    let msgs = msgs(&g);
    assert!(
        !msgs.iter().any(|m| m.contains("unknown import")),
        "the re-exported name is part of `mid`'s surface: {msgs:?}"
    );
    assert_eq!(g.diags.len(), 1, "the binding gap alone speaks: {msgs:?}");
    assert!(
        g.diags[0]
            .msg
            .contains("`helper` is not part of `mid`'s exported surface"),
        "got: {}",
        g.diags[0].msg
    );
}

#[test]
fn a_private_use_behind_the_same_import_stays_unknown() {
    // the control: `mid`'s PRIVATE `use util::{ helper }` imports only
    // — no row joins its surface, so the consumer's leaf is the
    // unknown import it always was
    let src = "use mid::{ helper };\n\
               entry fn main() -> i32 { return 0; }\n";
    let g = reexport_graph_of(false, src);
    assert_eq!(g.diags.len(), 1, "exactly one diagnostic: {:?}", msgs(&g));
    assert!(
        g.diags[0]
            .msg
            .contains("`mid` has no `helper` — unknown import"),
        "got: {}",
        g.diags[0].msg
    );
}

#[test]
fn a_qualified_leaf_reads_the_re_exported_row_of_the_walked_module() {
    // the qualified lane's leaf gate: `mid`'s `sub` module re-exports
    // `util`'s `helper`, so `use mid::sub::{ helper }` names a pub row
    // of the walked module — neither the `not declared` nor the `not
    // pub` refusal speaks (the binding gap is the same as the flat
    // shape's, the only diag left)
    let mid = rut_driver::Mounted::source("mid", "pub mod sub;\n")
        .manifest(rut_driver::Manifest::lib("mid", &["util"]));
    let mut mid = mid;
    mid.mods.insert(
        "sub".to_string(),
        rut_driver::ModSource {
            path: "sub".to_string(),
            vis: rut_ast::Vis::Pub,
            text: "pub use util::{ helper };\n".to_string(),
            file: "mod.rut".to_string(),
        },
    );
    let g = match rut_driver::RutRun::new()
        .mount(rut_driver::Mounted::source(
            "util",
            "pub fn helper() -> i32 { return 3; }\n",
        ))
        .mount(mid)
        .mount(
            rut_driver::Mounted::source(
                "app",
                "use mid::sub::{ helper };\nentry fn main() -> i32 { return 0; }\n",
            )
            .manifest(rut_driver::Manifest::lib("app", &["mid"])),
        )
        .entrypoint("app")
        .compile()
    {
        Ok(c) => c.graph,
        Err(e) => rut_driver::GraphOutput {
            diags: vec![rut_lexer::diag::Diag::new(
                rut_lexer::span::Span::new(0, 0),
                e.msg,
            )],
            program: None,
        },
    };
    let msgs = msgs(&g);
    assert!(
        !msgs
            .iter()
            .any(|m| m.contains("is not declared in") || m.contains("is not `pub`")),
        "the walked module's re-export row crosses: {msgs:?}"
    );
    assert_eq!(g.diags.len(), 1, "the binding gap alone speaks: {msgs:?}");
    assert!(
        g.diags[0]
            .msg
            .contains("`helper` is not part of `mid`'s exported surface"),
        "got: {}",
        g.diags[0].msg
    );
}

// ---- the own lane: a `pub use` row names its target ----

/// One own-package program: the root consumer plus its mounted module
/// files (`a` the declarations, `b`/`c` the re-export hops). Every
/// module is a `pub mod` child, single-file form.
fn own_graph_of(root_src: &str, modules: &[(&str, String)]) -> rut_driver::GraphOutput {
    let mut pkg = rut_driver::Mounted::source("app", root_src);
    for (path, text) in modules {
        pkg.mods.insert(
            (*path).to_string(),
            rut_driver::ModSource {
                path: (*path).to_string(),
                vis: Vis::Pub,
                text: text.clone(),
                file: "mod.rut".to_string(),
            },
        );
    }
    match rut_driver::RutRun::new()
        .mount(pkg)
        .entrypoint("app")
        .compile()
    {
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

/// Run a compiled own-package program's `main` and answer its `i32`.
fn run_main_i32(g: &rut_driver::GraphOutput) -> i32 {
    let flat = rut_core::link::flatten(g.program.clone().expect("linked program"));
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::builder()
        .program(std::rc::Rc::new(flat))
        .limits(limits)
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("vm");
    vm.call::<_, i32>("main", ()).expect("run")
}

/// The root that declares `a`, `b`, and (for two hops) `c`, then the
/// consumer.
fn own_root(consumer: &str, hops: usize) -> String {
    let mut root = String::from("mod a;\nmod b;\n");
    if hops == 2 {
        root.push_str("mod c;\n");
    }
    root.push('\n');
    root.push_str(consumer);
    root
}

#[test]
fn a_re_exported_name_binds_inside_the_package() {
    // THE table: {fn, type, module let, enum} × {a leaf `use b::{x}`,
    // a qualified `b::x`} × {one hop, two hops}. A re-export row NAMES
    // its target, so the consumer's leaf import and its qualified
    // spelling both reach the DECLARING module and the program runs.
    struct Shape {
        decls: &'static str,
        name: &'static str,
        leaf: &'static str,
        qualified: &'static str,
        want: i32,
    }
    let shapes = [
        Shape {
            decls: "pub fn f() -> i32 { return 7; }\n",
            name: "f",
            leaf: "use HEAD::{ f };\nentry fn main() -> i32 { return f(); }\n",
            qualified: "entry fn main() -> i32 { return HEAD::f(); }\n",
            want: 7,
        },
        Shape {
            decls: "pub struct S { v: i32 }\npub fn make() -> S { return S { v: 7 }; }\n",
            name: "S",
            leaf: "use HEAD::{ S };\nentry fn main() -> i32 { let s: S = a::make(); return s.v; }\n",
            qualified: "entry fn main() -> i32 { let s: HEAD::S = a::make(); return s.v; }\n",
            want: 7,
        },
        Shape {
            decls: "pub let MAX: i32 = 7;\n",
            name: "MAX",
            leaf: "use HEAD::{ MAX };\nentry fn main() -> i32 { return MAX; }\n",
            qualified: "entry fn main() -> i32 { return HEAD::MAX; }\n",
            want: 7,
        },
        Shape {
            decls: "pub enum E { A, B }\n",
            name: "E",
            leaf: "use HEAD::{ E };\nentry fn main() -> i32 { let e = E::A; if (e == E::A) { return 1; } return 0; }\n",
            qualified: "entry fn main() -> i32 { let e: HEAD::E = a::E::A; if (e == a::E::A) { return 1; } return 0; }\n",
            want: 1,
        },
    ];
    for shape in shapes {
        for hops in [1usize, 2] {
            let head = if hops == 1 { "b" } else { "c" };
            let mut modules = vec![
                ("a", shape.decls.to_string()),
                ("b", format!("pub use a::{{ {} }};\n", shape.name)),
            ];
            if hops == 2 {
                modules.push(("c", format!("pub use b::{{ {} }};\n", shape.name)));
            }
            for (column, body) in [
                ("leaf", shape.leaf.replace("HEAD", head)),
                ("qualified", shape.qualified.replace("HEAD", head)),
            ] {
                let root = own_root(&body, hops);
                let g = own_graph_of(&root, &modules);
                assert!(
                    g.diags.is_empty(),
                    "[{} / {hops} hop(s) / {column}] zero diagnostics: {:?}",
                    shape.name,
                    msgs(&g)
                );
                assert_eq!(
                    run_main_i32(&g),
                    shape.want,
                    "[{} / {hops} hop(s) / {column}] the program runs",
                    shape.name
                );
            }
        }
    }
}

#[test]
fn a_cyclic_re_export_refuses() {
    // `a` and `b` re-export each other's `f`; the consumer's `use
    // a::{ f }` walks a -> b -> a and REFUSES at the use line — one
    // diagnostic, naming the cycle.
    let root = own_root("use a::{ f };\n\nentry fn main() { }\n", 1);
    let g = own_graph_of(
        &root,
        &[
            ("a", "pub use b::{ f };\n".to_string()),
            ("b", "pub use a::{ f };\n".to_string()),
        ],
    );
    assert_eq!(g.diags.len(), 1, "exactly one diagnostic: {:?}", msgs(&g));
    assert_eq!(
        g.diags[0].msg,
        "cyclic `pub use` — `f` re-exports itself through `a` -> `b` -> `a`"
    );
    assert_eq!(g.diags[0].file.as_deref(), Some("app"));
    assert!(
        rendered(&root, &g).contains("--> app:4:1"),
        "the arrow names the use line:\n{}",
        rendered(&root, &g)
    );
}

#[test]
fn a_pub_use_of_a_dependency_name_inside_the_own_lane_refuses() {
    // `b` re-exports a DEPENDENCY's `f` (`pub use somepkg::{ f }`) and
    // the root imports it through the own lane — a cross-package
    // re-export does not bind until the surface owns the edge, so the
    // use line refuses and says why.
    let root = "mod b;\n\nuse b::{ f };\n\nentry fn main() { }\n";
    let mut app = rut_driver::Mounted::source("app", root)
        .manifest(rut_driver::Manifest::lib("app", &["somepkg"]));
    app.mods.insert(
        "b".to_string(),
        rut_driver::ModSource {
            path: "b".to_string(),
            vis: Vis::Pub,
            text: "pub use somepkg::{ f };\n".to_string(),
            file: "mod.rut".to_string(),
        },
    );
    let g = match rut_driver::RutRun::new()
        .mount(rut_driver::Mounted::source(
            "somepkg",
            "pub fn f() -> i32 { return 3; }\n",
        ))
        .mount(app)
        .entrypoint("app")
        .compile()
    {
        Ok(c) => c.graph,
        Err(e) => rut_driver::GraphOutput {
            diags: vec![rut_lexer::diag::Diag::new(
                rut_lexer::span::Span::new(0, 0),
                e.msg,
            )],
            program: None,
        },
    };
    assert_eq!(g.diags.len(), 1, "exactly one diagnostic: {:?}", msgs(&g));
    assert_eq!(
        g.diags[0].msg,
        "`f` is re-exported from `somepkg` — a cross-package re-export does not bind yet"
    );
    assert_eq!(g.diags[0].file.as_deref(), Some("app"));
    assert!(
        rendered(root, &g).contains("--> app:3:1"),
        "the arrow names the use line:\n{}",
        rendered(root, &g)
    );
}

// ---- the std lane: same gate, the toolchain tree's ink pkg ----

/// The toolchain tree's ink walk (`rut/ink`): the pkg plus its
/// `ink_host` dep; only the root's closure compiles.
fn std_graph_of(app_src: &str) -> rut_driver::GraphOutput {
    let tree = rut_native::dir_pkgs(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut/ink"),
    )
    .expect("the ink walk");
    match tree
        .pkgs
        .iter()
        .fold(rut_driver::RutRun::new(), |r, p| r.mount(p.clone()))
        .mount(
            rut_driver::Mounted::source("app", app_src)
                .manifest(rut_driver::Manifest::lib("app", &["ink"])),
        )
        .entrypoint("app")
        .compile()
    {
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
