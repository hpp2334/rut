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
    // the surface owns the edge (law 3): `mid`'s `pub use util::{ helper }`
    // lays a surface row whose origin is util, so the consumer binds
    // the name like any of mid's exports and the call runs.
    let src = "use mid::{ helper };\n\
               entry fn main() -> i32 { return helper(); }\n";
    let g = reexport_graph_of(true, src);
    assert!(g.diags.is_empty(), "zero diagnostics: {:?}", msgs(&g));
    assert_eq!(run_main_i32(&g), 3, "the re-exported fn runs");
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
    // the qualified lane: `mid`'s `sub` module re-exports `util`'s
    // `helper`, so `use mid::sub::{ helper }` binds the surface row
    // the walked module laid — the call runs.
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
                "use mid::sub::{ helper };\nentry fn main() -> i32 { return helper(); }\n",
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
    assert!(g.diags.is_empty(), "zero diagnostics: {:?}", msgs(&g));
    assert_eq!(run_main_i32(&g), 3, "the walked module's re-export runs");
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
fn every_independent_cycle_is_named_once() {
    // two INDEPENDENT re-export cycles in one unit — a↔b on `f`, c↔d
    // on `g` — both consumed and called: the refusal is one per cycle
    // (the reported set is keyed by the walked spelling, never one
    // latch for the whole unit)
    let root = "mod a;\nmod b;\nmod c;\nmod d;\n\n\
                use a::{ f };\n\
                use c::{ g };\n\n\
                entry fn main() -> i32 { return f() + g(); }\n";
    let g = own_graph_of(
        root,
        &[
            ("a", "pub use b::{ f };\n".to_string()),
            ("b", "pub use a::{ f };\n".to_string()),
            ("c", "pub use d::{ g };\n".to_string()),
            ("d", "pub use c::{ g };\n".to_string()),
        ],
    );
    assert_eq!(g.diags.len(), 2, "one refusal per cycle: {:?}", msgs(&g));
    let all = msgs(&g).join("\n");
    assert!(
        all.contains("cyclic `pub use` — `f` re-exports itself through `a` -> `b` -> `a`"),
        "the f cycle is named: {all}"
    );
    assert!(
        all.contains("cyclic `pub use` — `g` re-exports itself through `c` -> `d` -> `c`"),
        "the g cycle is named: {all}"
    );
}

#[test]
fn a_pub_use_of_a_dependency_name_inside_the_own_lane_binds() {
    // `b` re-exports a DEPENDENCY's `f` (`pub use somepkg::{ f }`) and
    // the root imports it through the own lane. Law 3: the row is a
    // surface row of `b`, so the binding is handed to somepkg's surface
    // — the root's call runs, and a `pub use` of it re-exports in turn.
    let root = "mod b;\n\nuse b::{ f };\n\nentry fn main() -> i32 { return f(); }\n";
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
    assert!(g.diags.is_empty(), "zero diagnostics: {:?}", msgs(&g));
    assert_eq!(run_main_i32(&g), 3, "the own-lane consumer runs");
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

// ---- across packages: the surface owns the edge ----

/// The ground truth 2 chain: `utilx` declares, `midx` re-exports (at
/// its root — the flat column — or in its `sub` module — the
/// qualified column), `app` imports the name and calls it. `midx`
/// declares `utilx`; `app` declares `midx` alone.
fn cross_chain_graph(
    utilx: &str,
    midx_root: &str,
    midx_mods: &[(&str, String)],
    app: &str,
) -> rut_driver::GraphOutput {
    let mut midx = rut_driver::Mounted::source("midx", midx_root)
        .manifest(rut_driver::Manifest::lib("midx", &["utilx"]));
    for (sub, text) in midx_mods {
        midx.mods.insert(
            (*sub).to_string(),
            rut_driver::ModSource {
                path: (*sub).to_string(),
                vis: Vis::Pub,
                text: text.clone(),
                file: "mod.rut".to_string(),
            },
        );
    }
    match rut_driver::RutRun::new()
        .mount(rut_driver::Mounted::source("utilx", utilx))
        .mount(midx)
        .mount(
            rut_driver::Mounted::source("app", app)
                .manifest(rut_driver::Manifest::lib("app", &["midx"])),
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
fn a_re_exported_name_binds_across_packages() {
    // THE table: step 1's admitted kinds × {a leaf `use midx::{ x };`,
    // a qualified `use midx::sub::{ x };`}. `midx` re-exports the
    // names on utilx's surface; the surface row carries the declaring
    // unit, so the consumer binds it like any of midx's exports and
    // the program runs. The refused kind (a module let) has its own
    // pin below.
    struct Shape {
        label: &'static str,
        decls: &'static str,
        names: &'static str,
        app: &'static str,
        want: i32,
    }
    let shapes = [
        Shape {
            label: "fn",
            decls: "pub fn f() -> i32 { return 7; }\n",
            names: "f",
            app: "entry fn main() -> i32 { return f(); }\n",
            want: 7,
        },
        Shape {
            label: "generic fn",
            decls: "pub fn ident<T>(v: T) -> T { return v; }\n",
            names: "ident",
            app: "entry fn main() -> i32 { return ident(5); }\n",
            want: 5,
        },
        Shape {
            label: "type",
            decls: "pub struct S { v: i32 }\npub fn make() -> S { return S { v: 7 }; }\n",
            names: "S, make",
            app: "entry fn main() -> i32 { let s = make(); return s.v; }\n",
            want: 7,
        },
        Shape {
            label: "class",
            decls: "pub class C { pub v: i32 = 0; }\npub fn make() -> C { return C { v: 7 }; }\n",
            names: "C, make",
            app: "entry fn main() -> i32 { let c: C = make(); return c.v; }\n",
            want: 7,
        },
        Shape {
            label: "generic type",
            decls: "pub struct Cell<T> { v: T }\npub fn wrap<T>(v: T) -> Cell<T> { return Cell { v: v }; }\n",
            names: "Cell, wrap",
            app: "entry fn main() -> i32 { let c: Cell<i32> = wrap(5); return c.v; }\n",
            want: 5,
        },
        Shape {
            label: "enum",
            decls: "pub enum E { A, B }\n",
            names: "E",
            app: "entry fn main() -> i32 { let e = E::A; if (e == E::A) { return 1; } return 0; }\n",
            want: 1,
        },
        Shape {
            label: "interface",
            decls: "pub interface Shape { fn area(self) -> i32; }\n\
                    pub struct Sq { n: i32 }\n\
                    impl Sq { pub fn area(self) -> i32 { return self.n; } }\n\
                    pub fn mk() -> Sq { return Sq { n: 4 }; }\n",
            names: "Shape, Sq, mk",
            app: "entry fn main() -> i32 { let s: Shape = mk(); return s.area(); }\n",
            want: 4,
        },
    ];
    for shape in shapes {
        for column in ["leaf", "qualified"] {
            let (midx_root, midx_mods, app_src) = if column == "leaf" {
                (
                    format!("pub use utilx::{{ {} }};\n", shape.names),
                    Vec::new(),
                    format!("use midx::{{ {} }};\n{}", shape.names, shape.app),
                )
            } else {
                (
                    "pub mod sub;\n".to_string(),
                    vec![("sub", format!("pub use utilx::{{ {} }};\n", shape.names))],
                    format!("use midx::sub::{{ {} }};\n{}", shape.names, shape.app),
                )
            };
            let g = cross_chain_graph(shape.decls, &midx_root, &midx_mods, &app_src);
            assert!(
                g.diags.is_empty(),
                "[{} / {column}] zero diagnostics: {:?}",
                shape.label,
                msgs(&g)
            );
            let got = run_main_i32(&g);
            assert_eq!(
                got, shape.want,
                "[{} / {column}] the program runs",
                shape.label
            );
        }
    }
    // (d) the pack stays deterministic: the re-export rows are laid in
    // (importing file, leaf) order, so the same source encodes the same
    // bytes twice — never the dep iteration order
    let once = cross_chain_graph(
        "pub fn f() -> i32 { return 7; }\n",
        "pub use utilx::{ f };\n",
        &[],
        "use midx::{ f };\nentry fn main() -> i32 { return f(); }\n",
    );
    let twice = cross_chain_graph(
        "pub fn f() -> i32 { return 7; }\n",
        "pub use utilx::{ f };\n",
        &[],
        "use midx::{ f };\nentry fn main() -> i32 { return f(); }\n",
    );
    assert_eq!(
        rut_core::binary::encode(&once.program.expect("linked")),
        rut_core::binary::encode(&twice.program.expect("linked")),
        "the same source packs byte-identically"
    );
}

#[test]
fn a_re_exported_module_let_stays_refused() {
    // the ONE kind step 1 refuses: a module `let` is not a surface
    // const, so the re-exporter's own `pub use` binds nothing and the
    // use line names the kind honestly.
    let g = cross_chain_graph(
        "pub let MAX: i32 = 7;\n",
        "pub use utilx::{ MAX };\n",
        &[],
        "use midx::{ MAX };\nentry fn main() -> i32 { return MAX; }\n",
    );
    assert_eq!(g.diags.len(), 1, "exactly one diagnostic: {:?}", msgs(&g));
    assert!(
        g.diags[0].msg.contains("`MAX` is a module let")
            && g.diags[0].msg.contains("does not cross package surfaces"),
        "got: {}",
        g.diags[0].msg
    );
}

#[test]
fn a_re_exported_host_const_binds_across_packages() {
    // the const kind crosses from its only lawful source — a host
    // surface const. `host_pkg` folds `PI`, `midx` re-exports it, and
    // `app` reads it through both the flat and the qualified column.
    use rut_driver::MountedBody;
    let host = rut_driver::lower_decl_module(
        "pub let PI: f64 = 3.141592653589793;\n",
        &BTreeMap::new(),
        "host_pkg.d.rut",
    )
    .expect("the surface lowers")
    .named("host_pkg");
    let host_rows = match &host.body {
        MountedBody::Host { consts, .. } => consts.clone(),
        _ => panic!("the host pkg carries a host body"),
    };
    let pi_bits = host_rows
        .iter()
        .find(|(n, _, _)| n == "PI")
        .expect("PI folds")
        .2;
    for column in ["leaf", "qualified"] {
        let (midx_root, midx_mods, app_src) = if column == "leaf" {
            (
                "pub use host_pkg::{ PI };\n".to_string(),
                Vec::new(),
                "use midx::{ PI };\nentry fn main() -> i32 { if (PI == 3.141592653589793f64) { return 1; } return 0; }\n".to_string(),
            )
        } else {
            (
                "pub mod sub;\n".to_string(),
                vec![("sub", "pub use host_pkg::{ PI };\n".to_string())],
                "use midx::sub::{ PI };\nentry fn main() -> i32 { if (PI == 3.141592653589793f64) { return 1; } return 0; }\n".to_string(),
            )
        };
        let mut midx = rut_driver::Mounted::source("midx", &midx_root)
            .manifest(rut_driver::Manifest::lib("midx", &["host_pkg"]));
        for (sub, text) in midx_mods {
            midx.mods.insert(
                sub.to_string(),
                rut_driver::ModSource {
                    path: sub.to_string(),
                    vis: Vis::Pub,
                    text,
                    file: "mod.rut".to_string(),
                },
            );
        }
        let g = match rut_driver::RutRun::new()
            .mount(host.clone())
            .mount(midx)
            .mount(
                rut_driver::Mounted::source("app", app_src)
                    .manifest(rut_driver::Manifest::lib("app", &["midx"])),
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
        assert!(
            g.diags.is_empty(),
            "[host const / {column}] zero diagnostics: {:?}",
            msgs(&g)
        );
        assert_eq!(run_main_i32(&g), 1, "[host const / {column}]");
        let _ = pi_bits;
    }
    // HIGH 2's const twin: the same `pub use` gate lays the const row,
    // so a package-private const re-export crosses nothing either.
    let midx = rut_driver::Mounted::source("midx", "pub(pkg) use host_pkg::{ PI };\n")
        .manifest(rut_driver::Manifest::lib("midx", &["host_pkg"]));
    let g = match rut_driver::RutRun::new()
        .mount(host.clone())
        .mount(midx)
        .mount(
            rut_driver::Mounted::source(
                "app",
                "use midx::{ PI };\nentry fn main() -> i32 { return 0; }\n",
            )
            .manifest(rut_driver::Manifest::lib("app", &["midx"])),
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
    assert_eq!(
        g.diags.len(),
        1,
        "[host const / pub(pkg)] exactly one diagnostic: {:?}",
        msgs(&g)
    );
    assert!(
        g.diags[0].msg.contains("`PI` is not exported by `midx`")
            && g.diags[0].msg.contains("mark it `pub`"),
        "got: {}",
        g.diags[0].msg
    );
}

#[test]
fn a_stale_artifact_refuses() {
    // LAW 4: the re-export wire bump refuses older artifacts with the
    // STANDARD version error — a v26 bundle (this bump's immediate
    // predecessor) never loads back; stale bytes re-pack at source.
    let out = rut_driver::compile_module(
        "entry fn main() -> i32 { return 0; }",
        rut_ast::parser::Mode::Impl,
        "gate",
    );
    let mut bytes = out.binary.expect("a clean compile encodes");
    // MAGIC (4 bytes) then the version u32
    bytes[4..8].copy_from_slice(&26u32.to_le_bytes());
    let err = match rut_core::binary::decode(&bytes) {
        Ok(_) => panic!("a v26 bundle decoded; the wire bump must refuse it"),
        Err(e) => e,
    };
    assert_eq!(
        err,
        format!(
            "unsupported module binary version 26 — this toolchain reads {} only; re-pack the directory",
            rut_core::binary::VERSION
        ),
        "the STANDARD version error"
    );
}

#[test]
fn a_package_private_re_export_stays_inside_the_package() {
    // HIGH 2: law 3 is an ACROSS-package law and the wire carries no
    // vis. Only a full `pub use` lays a surface row; a `pub(pkg)` (or
    // `pub(super)`) re-export is law 2's package-private edge and a
    // foreign consumer must bind nothing.
    let g = cross_chain_graph(
        "pub fn f() -> i32 { return 7; }\n",
        "pub(pkg) use utilx::{ f };\n",
        &[],
        "use midx::{ f };\nentry fn main() -> i32 { return f(); }\n",
    );
    assert_eq!(g.diags.len(), 1, "exactly one diagnostic: {:?}", msgs(&g));
    assert!(
        g.diags[0].msg.contains("`f` is not exported by `midx`")
            && g.diags[0].msg.contains("mark it `pub`"),
        "got: {}",
        g.diags[0].msg
    );
    // the package-private edge still binds INSIDE its package
    let g = cross_chain_graph(
        "pub fn f() -> i32 { return 7; }\n",
        "pub(pkg) use utilx::{ f };\npub fn wrap() -> i32 { return f(); }\n",
        &[],
        "use midx::{ wrap };\nentry fn main() -> i32 { return wrap(); }\n",
    );
    assert!(g.diags.is_empty(), "inside midx it binds: {:?}", msgs(&g));
    assert_eq!(run_main_i32(&g), 7);
}
