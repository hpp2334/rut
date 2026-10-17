//! The OWN-PACKAGE use lane: a use head that is no declared dep may
//! name the consumer's own package — its spec naming the root module
//! (`use app::{ helper };` from a child), or a `mod` edge walked from
//! the importing file's mod path (`use imp::{ TodoList, Task };`
//! beside `mod imp;`, the todolist shape). Binding semantics mirror
//! the qualified positions exactly: edges walk regardless of vis, the
//! LEAF visibility-gates under the crossing predicate (import is no
//! visibility bypass), bindings are per file (`use_sites` — a module
//! needs its own `use`), and a module name is not an import. A head
//! naming BOTH a declared dep and an own mod edge is the loud
//! ambiguity; a true miss keeps today's resolver texts.
//!
//! Both child spellings ride: the directory form (`imp/mod.rut`) and
//! the single-file form (`imp.rut`) — the diagnostics spell the
//! physical name the loader mounted.

use std::collections::BTreeMap;

use rut_ast::ast::Vis;

fn mod_source(path: &str, vis: Vis, text: &str) -> rut_driver::ModSource {
    mod_file_source(path, vis, text, "mod.rut")
}

/// The single-file flavor: the loader mounted `NAME.rut`, and the
/// diagnostics spell that physical name.
fn mod_file_source(path: &str, vis: Vis, text: &str, file: &str) -> rut_driver::ModSource {
    rut_driver::ModSource {
        path: path.to_string(),
        vis,
        text: text.to_string(),
        file: file.to_string(),
    }
}

/// The todolist shape: `use imp::{ TodoList, Task };` + `mod imp;` in
/// the root, `imp/mod.rut` beside it with `pub` members.
fn todo_app() -> rut_driver::Pkg {
    let mut p = rut_driver::Pkg::source(
        "app",
        r#"use imp::{ TodoList, Task };

mod imp;

fn boot() -> TodoList {
    let mut list = TodoList { count: 0, last: Task { id: 0, title: "none", done: false } };
    list.add("first");
    list.add("second");
    return list;
}

entry fn main() -> i64 {
    let list = boot();
    let t: Task = list.last;
    return list.count + t.id;
}
"#,
    );
    p.mods.insert(
        "imp".to_string(),
        mod_source(
            "imp",
            Vis::Self_,
            r#"pub struct Task {
    id: i64;
    title: str;
    done: bool;
}

pub class TodoList {
    pub count: i64;
    pub last: Task;
}

impl TodoList {
    pub fn add(mut self, title: str) {
        self.count += 1;
        self.last = Task { id: self.count, title: title, done: false };
    }
}
"#,
        ),
    );
    p
}

fn graph_of(pkgs: Vec<rut_driver::Pkg>, root: &str) -> rut_driver::GraphOutput {
    let mut run = rut_driver::RutRun::new();
    for p in pkgs {
        run = run.pkg(p);
    }
    match run.entrypoint(root).compile() {
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

/// Compile, link, verify, and run `main` — the answer back.
fn run_main(g: rut_driver::GraphOutput, expected: i64) {
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
    assert_eq!(vm.call::<_, i64>("main", ()).expect("run"), expected);
}

#[test]
fn the_todolist_shape_imports_and_runs() {
    // `use imp::{ TodoList, Task };` beside `mod imp;` — the root
    // imports the child's pub class and struct, constructs both,
    // calls the child's methods
    let g = graph_of(vec![todo_app()], "app");
    run_main(g, 4); // count 2 + last.id 2
}

#[test]
fn the_own_pkg_name_head_imports_from_the_root_module() {
    // a CHILD file imports through the package's own name: the root
    // module's `pub` names come to it
    let mut app = rut_driver::Pkg::source(
        "app",
        r#"mod imp;
mod sub;

pub fn helper() -> i64 { return 4; }

entry fn main() -> i64 { return sub::use_helper(); }
"#,
    );
    app.mods.insert(
        "sub".to_string(),
        mod_source(
            "sub",
            Vis::Self_,
            r#"use app::{ helper };

pub fn use_helper() -> i64 { return helper() + 1; }
"#,
        ),
    );
    app.mods.insert("imp".to_string(), mod_source("imp", Vis::Self_, "pub fn ignored() -> i64 { return 0; }\n"));
    let g = graph_of(vec![app], "app");
    run_main(g, 5);
}

#[test]
fn interior_segments_walk_the_own_tree() {
    // `use imp::grid::{ Cell };` — the interior segment walks the
    // child mods from the head's module (edges walk regardless of
    // vis — `mod grid;` is plain — the LEAF visibility-gates)
    let mut app = rut_driver::Pkg::source(
        "app",
        r#"use imp::grid::{ Cell };

mod imp;

entry fn main() -> i64 {
    let c = Cell { x: 3 };
    return c.x;
}
"#,
    );
    app.mods.insert(
        "imp".to_string(),
        mod_source("imp", Vis::Self_, "mod grid;\npub fn ignored() -> i64 { return 0; }\n"),
    );
    app.mods.insert(
        "imp/grid".to_string(),
        mod_source("imp/grid", Vis::Self_, "pub struct Cell { x: i64; }\n"),
    );
    let g = graph_of(vec![app], "app");
    run_main(g, 3);
}

#[test]
fn the_pkg_name_head_walks_interior_segments_too() {
    // `use app::sub::{ go };` from the root — the own pkg name names
    // the root module, then `sub` walks
    let mut app = rut_driver::Pkg::source(
        "app",
        r#"use app::sub::{ go };

mod sub;

entry fn main() -> i64 { return go(); }
"#,
    );
    app.mods.insert(
        "sub".to_string(),
        mod_source("sub", Vis::Self_, "pub fn go() -> i64 { return 7; }\n"),
    );
    let g = graph_of(vec![app], "app");
    run_main(g, 7);
}

#[test]
fn a_private_leaf_is_not_importable() {
    // import is no visibility bypass: a bare (private) member of imp
    // is refused at the use line — the text names the importing file
    // and the declaring file (the physical name the loader recorded)
    let mut app = rut_driver::Pkg::source(
        "app",
        "use imp::{ Task };\n\
         mod imp;\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    app.mods.insert(
        "imp".to_string(),
        mod_source(
            "imp",
            Vis::Self_,
            "struct Task { id: i64; }\npub fn ignored() -> i64 { return 0; }\n",
        ),
    );
    let src = "use imp::{ Task };\n\
               mod imp;\n\
               entry fn main() -> i32 { return 0; }\n";
    let g = graph_of(vec![app], "app");
    assert_eq!(g.diags.len(), 1, "exactly one diagnostic: {:?}", msgs(&g));
    let d = &g.diags[0];
    assert_eq!(
        d.msg,
        "`Task` is not visible from `mod.rut` — mark it `pub` (or `pub(pkg)`) in `imp/mod.rut`"
    );
    assert_eq!(d.file.as_deref(), Some("app"));
    assert!(
        rendered(src, &g).contains("--> app:1:1"),
        "the arrow names the use line:\n{}",
        rendered(src, &g)
    );
}

#[test]
fn a_private_leaf_spells_the_single_file_mount() {
    // the physical-name law: `imp` mounted as `imp.rut` (not the
    // directory-form guess) — the declaring file is spelled `imp.rut`
    let mut app = rut_driver::Pkg::source(
        "app",
        "use imp::{ Task };\n\
         mod imp;\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    app.mods.insert(
        "imp".to_string(),
        mod_file_source(
            "imp",
            Vis::Self_,
            "struct Task { id: i64; }\npub fn ignored() -> i64 { return 0; }\n",
            "imp.rut",
        ),
    );
    let g = graph_of(vec![app], "app");
    assert_eq!(g.diags.len(), 1, "exactly one diagnostic: {:?}", msgs(&g));
    assert_eq!(
        g.diags[0].msg,
        "`Task` is not visible from `mod.rut` — mark it `pub` (or `pub(pkg)`) in `imp.rut`"
    );
}

#[test]
fn a_missing_leaf_is_the_unknown_import() {
    // a name the walked module never declared reads the walked module
    // the way the use spelled it
    let mut app = rut_driver::Pkg::source(
        "app",
        "use imp::{ Ghost };\n\
         mod imp;\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    app.mods.insert(
        "imp".to_string(),
        mod_source("imp", Vis::Self_, "pub fn ignored() -> i64 { return 0; }\n"),
    );
    let g = graph_of(vec![app], "app");
    assert_eq!(g.diags.len(), 1, "exactly one diagnostic: {:?}", msgs(&g));
    assert_eq!(g.diags[0].msg, "`imp` has no `Ghost` — unknown import");
}

#[test]
fn a_missing_leaf_hint_names_the_module_it_lives_in() {
    // the name exists in the package but in a DIFFERENT module — the
    // hint names it (`TodoList` lives in `imp::grid`, the use spelled
    // `imp::{TodoList}`)
    let mut app = rut_driver::Pkg::source(
        "app",
        "use imp::{ TodoList };\n\
         mod imp;\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    app.mods.insert(
        "imp".to_string(),
        mod_source("imp", Vis::Self_, "mod grid;\npub fn ignored() -> i64 { return 0; }\n"),
    );
    app.mods.insert(
        "imp/grid".to_string(),
        mod_source("imp/grid", Vis::Self_, "pub class TodoList { n: i64; }\n"),
    );
    let g = graph_of(vec![app], "app");
    assert_eq!(g.diags.len(), 1, "exactly one diagnostic: {:?}", msgs(&g));
    assert_eq!(
        g.diags[0].msg,
        "`imp` has no `TodoList` — unknown import (declared in module `imp::grid`)"
    );
}

#[test]
fn a_deep_walk_miss_reads_the_walked_module() {
    // `use imp::grid::{ Tool };` — the walked module spells `::`-joined
    let mut app = rut_driver::Pkg::source(
        "app",
        "use imp::grid::{ Tool };\n\
         mod imp;\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    app.mods.insert(
        "imp".to_string(),
        mod_source("imp", Vis::Self_, "mod grid;\npub fn ignored() -> i64 { return 0; }\n"),
    );
    app.mods.insert(
        "imp/grid".to_string(),
        mod_source("imp/grid", Vis::Self_, "pub fn ignored() -> i64 { return 0; }\n"),
    );
    let g = graph_of(vec![app], "app");
    assert_eq!(g.diags.len(), 1, "exactly one diagnostic: {:?}", msgs(&g));
    assert_eq!(g.diags[0].msg, "`imp::grid` has no `Tool` — unknown import");
}

#[test]
fn a_pub_pkg_leaf_imports_within_the_package() {
    // `pub(pkg)` crosses inside the package — the import included
    let mut app = rut_driver::Pkg::source(
        "app",
        r#"use imp::{ Counter };

mod imp;

entry fn main() -> i64 {
    let mut c = Counter { n: 40 };
    c.bump();
    return c.n;
}
"#,
    );
    app.mods.insert(
        "imp".to_string(),
        mod_source(
            "imp",
            Vis::Self_,
            r#"pub(pkg) class Counter {
    pub(pkg) n: i64;
}

impl Counter {
    pub fn bump(mut self) {
        self.n += 1;
    }
}
"#,
        ),
    );
    let g = graph_of(vec![app], "app");
    run_main(g, 41);
}

#[test]
fn a_head_naming_both_dep_and_mod_edge_is_ambiguous() {
    // mounts and mod edges are different namespaces: a head that
    // answers as BOTH the declared dep and the own module is the loud
    // ambiguity naming both — never a silent pick
    let mut app = rut_driver::Pkg::source(
        "app",
        "use util::{ half };\n\
         mod util;\n\
         entry fn main() -> i32 { return 0; }\n",
    )
    .manifest(rut_driver::Manifest::lib("app", &["util"]));
    app.mods.insert(
        "util".to_string(),
        mod_source("util", Vis::Self_, "pub fn ignored() -> i64 { return 0; }\n"),
    );
    let dep = rut_driver::Pkg::source("util", "pub fn half(x: i32) -> i32 { return x / 2; }\n");
    let g = graph_of(vec![dep, app], "app");
    assert_eq!(g.diags.len(), 1, "exactly one diagnostic: {:?}", msgs(&g));
    assert!(
        g.diags[0]
            .msg
            .contains("`util` names both the declared dependency `util` and this package's own module `util` (util/mod.rut)")
            && g.diags[0].msg.contains("the use path is ambiguous; rename one of the two"),
        "got: {}",
        g.diags[0].msg
    );
    assert_eq!(g.diags[0].file.as_deref(), Some("app"));
    let src = "use util::{ half };\n\
               mod util;\n\
               entry fn main() -> i32 { return 0; }\n";
    assert!(
        rendered(src, &g).contains("--> app:1:1"),
        "the arrow names the use line:\n{}",
        rendered(src, &g)
    );
}

#[test]
fn bindings_do_not_leak_across_files() {
    // the per-file law: the root's `use imp::{ helper }` binds in the
    // ROOT only — a sibling module needs its own `use`, and the miss
    // says where the name is bound and what it was imported from
    let mut app = rut_driver::Pkg::source(
        "app",
        r#"use imp::{ helper };

mod imp;
mod sub;

entry fn main() -> i64 { return sub::go(); }
"#,
    );
    app.mods.insert(
        "imp".to_string(),
        mod_source("imp", Vis::Self_, "pub fn helper() -> i64 { return 4; }\n"),
    );
    app.mods.insert(
        "sub".to_string(),
        mod_source("sub", Vis::Self_, "pub fn go() -> i64 { return helper(); }\n"),
    );
    let g = graph_of(vec![app], "app");
    assert_eq!(g.diags.len(), 1, "exactly one diagnostic: {:?}", msgs(&g));
    assert!(
        g.diags[0].msg.contains(
            "`helper` is bound by a `use` in the root module (imported there from module `imp`) — write the `use` in this file"
        ),
        "got: {}",
        g.diags[0].msg
    );
}

#[test]
fn a_child_module_binds_with_its_own_use() {
    // the per-file law's fix: the child spells its own import and the
    // name binds there
    let mut app = rut_driver::Pkg::source(
        "app",
        r#"mod imp;
mod sub;

entry fn main() -> i64 { return sub::go(); }
"#,
    );
    app.mods.insert(
        "imp".to_string(),
        mod_source("imp", Vis::Self_, "pub fn helper() -> i64 { return 4; }\n"),
    );
    app.mods.insert(
        "sub".to_string(),
        mod_source(
            "sub",
            Vis::Self_,
            r#"use imp::{ helper };

pub fn go() -> i64 { return helper() + 1; }
"#,
        ),
    );
    let g = graph_of(vec![app], "app");
    run_main(g, 5);
}

#[test]
fn a_module_name_is_not_an_import() {
    // modules are namespaces, never values: the leaf gate refuses a
    // module name with the walk fix
    let mut app = rut_driver::Pkg::source(
        "app",
        "use imp::{ grid };\n\
         mod imp;\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    app.mods.insert(
        "imp".to_string(),
        mod_source(
            "imp",
            Vis::Self_,
            "mod grid;\npub fn ignored() -> i64 { return 0; }\n",
        ),
    );
    app.mods.insert(
        "imp/grid".to_string(),
        mod_source("imp/grid", Vis::Self_, "pub fn ignored() -> i64 { return 0; }\n"),
    );
    let g = graph_of(vec![app], "app");
    assert_eq!(g.diags.len(), 1, "exactly one diagnostic: {:?}", msgs(&g));
    assert_eq!(
        g.diags[0].msg,
        "`grid` is a module of this package — a module is not imported; walk it: `use imp::grid::{ .. }`"
    );
}

#[test]
fn a_true_miss_keeps_todays_text() {
    // a head that is neither a declared dep nor an own lane keeps the
    // resolver's bare NoModule text
    let mut app = rut_driver::Pkg::source(
        "app",
        "use nope::{ x };\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    app.mods.insert(
        "imp".to_string(),
        mod_source("imp", Vis::Self_, "pub fn ignored() -> i64 { return 0; }\n"),
    );
    let g = graph_of(vec![app], "app");
    assert_eq!(g.diags.len(), 1, "exactly one diagnostic: {:?}", msgs(&g));
    assert!(
        g.diags[0]
            .msg
            .contains("cannot resolve `nope` — no module with that name is mounted; declare it in your `rut.jsonc` `deps`"),
        "got: {}",
        g.diags[0].msg
    );
}

#[test]
fn the_own_lanes_bare_names_resolve_everywhere() {
    // the ladder: an own-imported enum (member path + when pattern),
    // module let, fn (call + value), and struct all resolve through
    // the target module
    let mut app = rut_driver::Pkg::source(
        "app",
        r#"use imp::{ Color, MAX, bump, Boxed };

mod imp;

entry fn main() -> i64 {
    let c = Color::Green;
    let n: i64 = when (c) {
        Color::Red -> 1,
        Color::Green -> 2,
    };
    let f: fn(i64) -> i64 = bump;
    let b = Boxed { v: MAX };
    return f(b.v) + n;
}
"#,
    );
    app.mods.insert(
        "imp".to_string(),
        mod_source(
            "imp",
            Vis::Self_,
            r#"pub enum Color { Red, Green }

pub let MAX: i64 = 10;

pub fn bump(x: i64) -> i64 { return x + 1; }

pub struct Boxed { v: i64; }
"#,
        ),
    );
    let g = graph_of(vec![app], "app");
    run_main(g, 13); // bump(10) = 11, Green = 2
}

#[test]
fn a_host_dep_keeps_the_d_rut_spelling() {
    // the DEP lanes keep their flavor: a host surface's walk misses
    // name `mod.d.rut` — the file that exists
    let root = "pub mod net;\npub host fn legacy_ping() -> bool;\n";
    let mut children = BTreeMap::new();
    children.insert(
        "net".to_string(),
        mod_source(
            "net",
            Vis::Pub,
            "pub host fn connect(url: str) -> i64;\n",
        ),
    );
    let host = rut_driver::lower_decl_module(root, &children, "myhost.d.rut")
        .expect("lowers")
        .named("myhost");
    let app = rut_driver::Pkg::source(
        "app",
        "use myhost::nope::{ x };\n\
         entry fn main() -> i32 { return 0; }\n",
    )
    .manifest(rut_driver::Manifest::lib("app", &["myhost"]));
    let g = graph_of(vec![host, app], "app");
    assert_eq!(g.diags.len(), 1, "exactly one diagnostic: {:?}", msgs(&g));
    assert!(
        g.diags[0]
            .msg
            .contains("`nope` is not a module of `myhost` — no `mod nope;` under mod.d.rut"),
        "got: {}",
        g.diags[0].msg
    );
}
