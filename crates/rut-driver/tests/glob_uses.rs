//! The glob expansion (the `use pkg::*` leaf): the walked path's PUB
//! surface binds. The locked laws, each pinned here:
//!
//! - **Precedence — glob < explicit leaf < own decl**, silent
//!   shadowing: a name the file spells on any use line, or declares at
//!   its top level, never glob-binds, and no diagnostic fires.
//! - **Glob–glob conflict** errors AT THE SECOND use line with the
//!   exact text (`` `NAME` arrives from both `a::*` and `b::*` —
//!   import it explicitly ``); the same head re-globbed is idempotent
//!   (silent).
//! - **Must-bind is exempt** for globs (no spelled leaves on the
//!   line): a glob whose surface names nothing the file uses stays
//!   silent; D5 and the own-package head checks are unchanged.
//! - **`pub use pkg::*` re-exports the lot** — the synthesized rows
//!   carry the use's vis, and the const re-export edge lays the rows
//!   onto the unit's surface for downstream consumers.
//! - A glob importing an interface arms the use-both gate: the
//!   descriptor registers and the type-position rung resolves (a
//!   local impl satisfies it, a local signature spells it).

use std::collections::BTreeMap;

/// One util pkg: a pub fn, a pub struct, a private fn, and a mounted
/// `pub mod sub` — the glob surface's shapes in one package.
fn util_pkg() -> rut_driver::Mounted {
    let mut p = rut_driver::Mounted::source(
        "util",
        "pub fn half(x: i32) -> i32 { return x / 2; }\n\
         pub fn shadow() -> i32 { return 1; }\n\
         fn hidden() -> i32 { return 7; }\n\
         pub struct Boxed { n: i32 }\n\
         pub fn make(n: i32) -> Boxed { return Boxed { n: n }; }\n\
         pub mod sub;\n",
    );
    p.mods.insert(
        "sub".to_string(),
        rut_driver::ModSource {
            path: "sub".to_string(),
            vis: rut_ast::Vis::Pub,
            text: "pub fn one() -> i32 { return 1; }\n".to_string(),
            file: "mod.rut".to_string(),
        },
    );
    p
}

/// A second util whose surface overlaps `util`'s (`shadow`) with a
/// different body — the glob–conflict and precedence shapes.
fn other_pkg() -> rut_driver::Mounted {
    rut_driver::Mounted::source(
        "other",
        "pub fn shadow() -> i32 { return 2; }\n\
         pub fn twin() -> i32 { return 9; }\n",
    )
}

/// The app over the two deps: the root DECLARES its deps (the
/// structural manifest); the chain's graph diags come back (a closure
/// refusal as one span-0 diagnostic).
fn graph_of(app_src: &str) -> rut_driver::GraphOutput {
    match rut_driver::RutRun::new()
        .mount(util_pkg())
        .mount(other_pkg())
        .mount(
            rut_driver::Mounted::source("app", app_src)
                .manifest(rut_driver::Manifest::lib("app", &["util", "other"])),
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

/// Compile [`graph_of`]'s program and run `main`, carrying the i32
/// out — the "the names actually bind" half of every green case.
fn run_main(g: rut_driver::GraphOutput) -> i32 {
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
    vm.call::<_, i32>("main", ()).expect("run")
}

// ------------------------------------------------------------------ //

#[test]
fn glob_binds_every_pub_name_and_the_bodies_run() {
    // one star line, the root surfaces used: the fns and the struct
    // (constructed directly and through its maker). The walked mod's
    // names stay on the pkg-rooted path — a glob binds values, module
    // names keep the qualified spellings.
    let src = "use util::*;\n\
               entry fn main() -> i32 {\n\
                   let b: Boxed = make(20);\n\
                   return half(10) + b.n + util::sub::one();\n\
               }\n";
    assert_eq!(run_main(graph_of(src)), 26);
}

#[test]
fn glob_is_exempt_from_the_must_bind_law() {
    // the star line binds util's whole pub surface but the body uses
    // nothing — no unreferenced-name trip, no must-bind error (the
    // spelled-leaf lane errors exactly here: `use util::{ Nope };`)
    let src = "use util::*;\n\
               entry fn main() -> i32 { return 0; }\n";
    let g = graph_of(src);
    assert!(g.diags.is_empty(), "zero diagnostics: {:?}", msgs(&g));
}

#[test]
fn explicit_leaf_shadows_the_glob_silently() {
    // `shadow` arrives from BOTH heads — but `other`'s is spelled, so
    // the glob line skips the name (precedence: explicit over glob)
    // and no conflict fires. The call resolves to other's body (2).
    let src = "use other::{ shadow };\n\
               use util::*;\n\
               entry fn main() -> i32 { return shadow(); }\n";
    assert_eq!(run_main(graph_of(src)), 2);
}

#[test]
fn own_decl_shadows_the_glob_silently() {
    // the app DECLARES its own `shadow` — the glob line skips the
    // name (precedence: own decl over glob) and the local body wins
    let src = "use util::*;\n\
               fn shadow() -> i32 { return 42; }\n\
               entry fn main() -> i32 { return shadow(); }\n";
    assert_eq!(run_main(graph_of(src)), 42);
}

#[test]
fn the_glob_conflict_errors_at_the_second_use_line() {
    // two heads glob the same name: the SECOND use line errors, with
    // the locked text, positioned at the line it is written on
    let src = "use util::*;\n\
               use other::*;\n\
               entry fn main() -> i32 { return 0; }\n";
    let g = graph_of(src);
    assert_eq!(g.diags.len(), 1, "exactly one diagnostic: {:?}", msgs(&g));
    assert_eq!(
        g.diags[0].msg,
        "`shadow` arrives from both `util::*` and `other::*` — import it explicitly"
    );
    assert_eq!(g.diags[0].file.as_deref(), Some("app"));
    assert!(
        rendered(src, &g).contains("--> app:2:1"),
        "the arrow names the SECOND use line:\n{}",
        rendered(src, &g)
    );
}

#[test]
fn the_same_head_double_glob_is_silent() {
    // util re-globbed: idempotent — no conflict, no diagnostic
    let src = "use util::*;\n\
               use util::*;\n\
               entry fn main() -> i32 { return half(10); }\n";
    assert_eq!(run_main(graph_of(src)), 5);
}

#[test]
fn a_walked_glob_binds_the_mod_path_surface() {
    // `use util::sub::*;` — the prefix walks; sub's pub fn binds in
    // this file under its bare leaf
    let src = "use util::sub::*;\n\
               entry fn main() -> i32 { return one(); }\n";
    assert_eq!(run_main(graph_of(src)), 1);
}

#[test]
fn a_braced_star_rides_beside_the_spelled_leaves() {
    // `{ twin, shadow, * }` — the spelled leaves and the glob share
    // the line; the spelled `shadow` keeps util's glob quiet beside it
    // (precedence), so the two surfaces never conflict
    let src = "use other::{ twin, shadow, * };\n\
               use util::*;\n\
               entry fn main() -> i32 { return twin() + half(10); }\n";
    assert_eq!(run_main(graph_of(src)), 14);
}

#[test]
fn a_glob_of_an_undeclared_head_keeps_d5() {
    // `ghost` is mounted (the host offers it) but `app` does not
    // declare it — the glob head gates exactly like a spelled one
    let c = rut_driver::RutRun::new()
        .mount(rut_driver::Mounted::source(
            "ghost",
            "pub fn haunt() -> i32 { return 1; }\n",
        ))
        .mount(util_pkg())
        .mount(
            rut_driver::Mounted::source(
                "app",
                "use ghost::*;\nentry fn main() -> i32 { return 0; }\n",
            )
            .manifest(rut_driver::Manifest::lib("app", &["util"])),
        )
        .entrypoint("app")
        .compile();
    match c {
        Err(e) => assert!(
            e.msg.contains("`ghost` is mounted in this program")
                && e.msg.contains("does not declare it"),
            "D5 speaks: {}",
            e.msg
        ),
        Ok(c) => {
            let m = msgs(&c.graph);
            assert!(!m.is_empty(), "the undeclared glob head refuses: {m:?}");
            assert!(
                m.iter()
                    .any(|m| m.contains("`ghost` is mounted in this program")),
                "D5 speaks: {m:?}"
            );
        }
    }
}

// ------------------------------------------------------------- own --- //

/// The todolist-ish own shape: the app mounts a `mod helpers;` child
/// whose pub fns glob into the root file.
fn own_app() -> rut_driver::Mounted {
    let mut p = rut_driver::Mounted::source(
        "app",
        "use helpers::*;\n\
         mod helpers;\n\
         entry fn main() -> i32 { return twice(21); }\n",
    );
    p.mods.insert(
        "helpers".to_string(),
        rut_driver::ModSource {
            path: "helpers".to_string(),
            vis: rut_ast::Vis::Self_,
            text: "pub fn twice(x: i32) -> i32 { return x * 2; }\n\
                   fn private_thing() -> i32 { return 0; }\n"
                .to_string(),
            file: "mod.rut".to_string(),
        },
    );
    p
}

#[test]
fn an_own_glob_binds_the_target_module() {
    // `use helpers::*;` beside `mod helpers;` — the own lane: each
    // pub name of the target module claims its binding in this file
    // (the private one never enumerates)
    let g = rut_driver::RutRun::new()
        .mount(own_app())
        .entrypoint("app")
        .compile();
    match g {
        Err(e) => panic!("the own glob compiles: {}", e.msg),
        Ok(c) => {
            assert!(
                c.graph.diags.is_empty(),
                "zero diagnostics: {:?}",
                msgs(&c.graph)
            );
            let flat = rut_core::link::flatten(c.graph.program.expect("linked program"));
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
            assert_eq!(vm.call::<_, i32>("main", ()).expect("run"), 42);
        }
    }
}

// ----------------------------------------------------- round 1 ------ //

/// [`run_main`] over a compile: a closure refusal is a loud panic (the
/// green lanes' shape).
fn run_compiled(c: Result<rut_driver::Compiled, rut_driver::RunError>) -> i32 {
    match c {
        Ok(compiled) => run_main(compiled.graph),
        Err(e) => panic!("the compile refuses: {}", e.msg),
    }
}

/// The review round-1 shape: the plan's literal own-glob spelling —
/// the package's OWN spec as the head with an interior segment. The
/// star enumerates the WALKED module's surface (`helpers`), never the
/// head's root: `twice` binds in the root file, the root's own decls
/// were never the glob's surface to begin with.
fn walked_own_app() -> rut_driver::Mounted {
    let mut p = rut_driver::Mounted::source(
        "app",
        "use app::helpers::*;\n\
         mod helpers;\n\
         fn rootfn() -> i32 { return 100; }\n\
         entry fn main() -> i32 { return twice(20) + rootfn(); }\n",
    );
    p.mods.insert(
        "helpers".to_string(),
        rut_driver::ModSource {
            path: "helpers".to_string(),
            vis: rut_ast::Vis::Self_,
            text: "pub fn twice(x: i32) -> i32 { return x * 2; }\n".to_string(),
            file: "mod.rut".to_string(),
        },
    );
    p
}

#[test]
fn a_walked_own_glob_binds_the_interior_module() {
    // `use app::helpers::*;` — the interior segment walks: helpers'
    // pub surface glob-binds (twice(20) = 40 + rootfn() = 100)
    assert_eq!(
        run_compiled(
            rut_driver::RutRun::new()
                .mount(walked_own_app())
                .entrypoint("app")
                .compile()
        ),
        140
    );
}

/// The sibling-head shape: the head names a `mod` child, the interior
/// segment walks DEEPER (`helpers/deep`). Only deep's surface
/// enumerates — helpers' own root fn stays unbound in this file.
fn deep_own_app() -> rut_driver::Mounted {
    deep_own_app_with(
        "use helpers::deep::*;\n\
         mod helpers;\n\
         entry fn main() -> i32 { return triple(3); }\n",
    )
}

#[test]
fn a_sibling_head_glob_walks_its_interior_segments() {
    // `use helpers::deep::*;` — the head resolves the sibling, the
    // segment walks deeper: deep's pub fn binds (triple(3) = 9)
    assert_eq!(
        run_compiled(
            rut_driver::RutRun::new()
                .mount(deep_own_app())
                .entrypoint("app")
                .compile()
        ),
        9
    );
}

#[test]
fn a_walked_own_glob_enumerates_only_the_walked_module() {
    // the negative pin: helpers' ROOT fn is not deep's surface — the
    // walked glob binds `triple`, never `twice`
    let src = "use helpers::deep::*;\n\
               mod helpers;\n\
               entry fn main() -> i32 { return twice(3); }\n";
    let g = match rut_driver::RutRun::new()
        .mount(deep_own_app_with(src))
        .entrypoint("app")
        .compile()
    {
        Ok(c) => c.graph,
        Err(e) => panic!("the compile refuses: {}", e.msg),
    };
    assert_eq!(g.diags.len(), 1, "exactly one diagnostic: {:?}", msgs(&g));
    // the elsewhere hint, not a binding: the glob never claimed
    // helpers' root fn — the name stays module-resident
    assert!(
        g.diags[0]
            .msg
            .contains("`twice` is declared in module `helpers`"),
        "got: {}",
        g.diags[0].msg
    );
}

/// [`deep_own_app`] over a different root text (the same mounted tree).
fn deep_own_app_with(src: &str) -> rut_driver::Mounted {
    let mut p = rut_driver::Mounted::source("app", src);
    p.mods.insert(
        "helpers".to_string(),
        rut_driver::ModSource {
            path: "helpers".to_string(),
            vis: rut_ast::Vis::Self_,
            text: "mod deep;\n\
                   pub fn twice(x: i32) -> i32 { return x * 2; }\n"
                .to_string(),
            file: "mod.rut".to_string(),
        },
    );
    p.mods.insert(
        "helpers/deep".to_string(),
        rut_driver::ModSource {
            path: "helpers/deep".to_string(),
            vis: rut_ast::Vis::Pub,
            text: "pub fn triple(x: i32) -> i32 { return x * 3; }\n".to_string(),
            file: "mod.rut".to_string(),
        },
    );
    p
}

#[test]
fn cross_file_glob_overlap_does_not_conflict() {
    // the T1 per-file law: root globbing `util` and helpers.rut
    // globbing `other` share the name `shadow` across their surfaces —
    // no conflict (each file's ledger is its own), exactly like the
    // identical shape with spelled leaves
    let root = "use util::*;\n\
                mod helpers;\n\
                entry fn main() -> i32 { return half(10) + helpers::bridge(); }\n";
    let mut p = rut_driver::Mounted::source("app", root)
        .manifest(rut_driver::Manifest::lib("app", &["util", "other"]));
    p.mods.insert(
        "helpers".to_string(),
        rut_driver::ModSource {
            path: "helpers".to_string(),
            vis: rut_ast::Vis::Self_,
            text: "use other::*;\n\
                   pub fn bridge() -> i32 { return twin(); }\n"
                .to_string(),
            file: "mod.rut".to_string(),
        },
    );
    assert_eq!(
        run_compiled(
            rut_driver::RutRun::new()
                .mount(util_pkg())
                .mount(other_pkg())
                .mount(p)
                .entrypoint("app")
                .compile()
        ),
        14
    );
}

// ------------------------------------------------------------- pub --- //

#[test]
fn a_pub_glob_reexports_for_downstream_consumers() {
    // `m` re-exports core's const surface with a star (`pub use
    // core::*;`); the consumer's glob of `m` sees NAN — a name that
    // only crossed BECAUSE the re-export edge laid it on m's surface
    let m = rut_driver::Mounted::source(
        "m",
        "pub use core::*;\n\
         pub fn half(x: i32) -> i32 { return x / 2; }\n",
    );
    let cons = "use m::*;\n\
                entry fn main() -> i32 {\n\
                    let n: f64 = NAN;\n\
                    if (n != n) { return half(20); }\n\
                    return 0;\n\
                }\n";
    let g = match rut_driver::RutRun::new()
        .mount(m)
        .mount(
            rut_driver::Mounted::source("cons", cons)
                .manifest(rut_driver::Manifest::lib("cons", &["m"])),
        )
        .entrypoint("cons")
        .compile()
    {
        Ok(c) => c.graph,
        Err(e) => panic!("the re-export chain compiles: {}", e.msg),
    };
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
    assert_eq!(vm.call::<_, i32>("main", ()).expect("run"), 10);
}

// ----------------------------------------------------------- iface --- //

/// The extras pkg: the interface AND the interface-typed fn. The
/// consumer proves a LOCAL type structurally and spells the trait in a
/// local signature — both need the descriptor registered.
fn extras_pkg() -> rut_driver::Mounted {
    rut_driver::Mounted::source(
        "extras",
        "pub interface Shape { fn area(self) -> f64; }\n\
         pub fn describe(s: Shape) -> f64 { return s.area(); }\n",
    )
}

#[test]
fn a_glob_imported_interface_registers_the_descriptor() {
    // `use extras::*;` + a LOCAL impl satisfying Shape + a local
    // signature spelling Shape: the descriptor registers (the use-both
    // gate — glob names join the binding gate) and the type-position
    // rung resolves in this file
    let src = "use extras::*;\n\
               struct Sq { r: f64 }\n\
               impl Sq { pub fn area(self) -> f64 { return self.r; } }\n\
               fn double(s: Shape) -> f64 { return s.area() * 2.0; }\n\
               entry fn main() -> i32 {\n\
                   let a: f64 = describe(Sq { r: 2.5 });\n\
                   let b: f64 = double(Sq { r: 1.5 });\n\
                   return (a + b) as i32;\n\
               }\n";
    let g = match rut_driver::RutRun::new()
        .mount(extras_pkg())
        .mount(
            rut_driver::Mounted::source("app", src)
                .manifest(rut_driver::Manifest::lib("app", &["extras"])),
        )
        .entrypoint("app")
        .compile()
    {
        Ok(c) => c.graph,
        Err(e) => panic!("the glob iface compiles: {}", e.msg),
    };
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
