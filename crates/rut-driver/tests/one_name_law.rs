//! THE ONE-NAME LAW — one file scope, one first-spell registry. A
//! file's module-scope names (its own decls of every kind + its `use`
//! leaves) bind at most once per name: the SECOND registration
//! errors, citing the first (kind + file). Sibling modules never meet
//! — except where the MECHANISM is still name-keyed unit-wide (fn↔fn,
//! type↔type), which keeps the conservative cross-module refusals
//! until the compile keys and type rows carry their module.
//!
//! Pins here:
//! - the same-file collision matrix: every kind pair refuses with the
//!   new family text (`duplicate name `X` — the <kind> already
//!   declares it in this file`), pass order deciding the site (types
//!   register before fns, so a fn always cites the type);
//! - decl-vs-use: the use leaf registers in pass 0, so every
//!   decl-vs-use pair keeps the use-citation shape;
//! - the a/b/c illustration: siblings bind freely, the third file's
//!   two use leaves collide at the second leaf;
//! - the LEGAL relaxations: fn↔type and let↔let across siblings —
//!   each file resolves its own, qualified access works;
//! - the conservative boundary: fn↔fn across siblings still refuses
//!   (the compile keys are name-keyed); type↔type rides the same
//!   gate, pinned in `mods.rs`;
//! - the entry gate (conservative on NAME): a name spelled by two
//!   sibling modules' entries is ambiguous — differently-named
//!   entries stay legal (the host calls them by name);
//! - ambient shadowing: a user fn named like an engine name (`panic`)
//!   is legal AND wins the call — ambient names sit below the file
//!   scope (local → file scope → ambient, inner wins).

use rut_ast::parser::Mode;
use rut_ast::Vis;
use rut_driver::{ModSource, Mounted, MountedBody};

// ---- flat single-file harness (the same-file law) ----------------------

fn compile(src: &str) -> rut_driver::ProgramOutput {
    // core bound as the one use: the law tests exercise name binding,
    // and the ambient-shadow test needs the engine name present
    rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, rut_core::binary::Surface::core(), "core".to_string())],
    )
}

fn diags_of(src: &str) -> Vec<String> {
    compile(src).diags.iter().map(|d| d.msg.clone()).collect()
}

/// compile + run: the entry fn hands an i32 back through the typed
/// host boundary.
fn run_returns(src: &str) -> Result<i64, String> {
    let out = compile(src);
    if !out.diags.is_empty() {
        return Err(out
            .diags
            .iter()
            .map(|d| d.msg.clone())
            .collect::<Vec<_>>()
            .join("; "));
    }
    let prog = rut_core::link::flatten(out.program.expect("linked program"));
    rut_vm::verify::verify(&prog).expect("verify");
    let mut vm = rut_vm::interp::Vm::builder()
        .program(std::rc::Rc::new(prog))
        .limits(rut_vm::interp::Limits {
            fuel: Some(1_000_000),
            heap_limit_bytes: Some(4 * 1024 * 1024),
            interrupt_every: 1024,
        })
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .map_err(|e| format!("{e:?}"))?;
    vm.call::<_, i32>("main", ())
        .map(|v| v as i64)
        .map_err(|e| format!("{e:?}"))
}

// ---- multi-pkg harness (the sibling law) -------------------------------

fn modpkg(spec: &str, root: &str, mods: Vec<(&str, Vis, &str)>) -> Mounted {
    Mounted {
        spec: spec.to_string(),
        body: MountedBody::Source {
            text: root.to_string(),
            is_decl: false,
        },
        mods: mods
            .into_iter()
            .map(|(p, v, t)| {
                (
                    p.to_string(),
                    ModSource {
                        path: p.to_string(),
                        vis: v,
                        text: t.to_string(),
                        file: format!("{p}/mod.rut"),
                    },
                )
            })
            .collect(),
        ..Default::default()
    }
}

fn compile_with_diags(pkgs: &[Mounted], entry: &str) -> Vec<String> {
    let mut run = rut_driver::RutRun::new();
    // the hand world declares loosely: every pkg names every other
    for p in pkgs {
        let heads: Vec<&str> = pkgs
            .iter()
            .map(|q| q.spec.as_str())
            .filter(|s| *s != p.spec)
            .collect();
        run = run.mount(
            p.clone()
                .manifest(rut_driver::Manifest::lib(&p.spec, &heads)),
        );
    }
    let c = run
        .entrypoint(entry)
        .compile()
        .expect("compile carries the diags");
    c.graph.diags.iter().map(|d| d.msg.clone()).collect()
}

fn run_main_pkgs(pkgs: &[Mounted], entry: &str) -> i64 {
    let mut run = rut_driver::RutRun::new();
    for p in pkgs {
        let heads: Vec<&str> = pkgs
            .iter()
            .map(|q| q.spec.as_str())
            .filter(|s| *s != p.spec)
            .collect();
        run = run.mount(
            p.clone()
                .manifest(rut_driver::Manifest::lib(&p.spec, &heads)),
        );
    }
    let c = run.entrypoint(entry).compile().expect("compile the walk");
    assert!(c.graph.diags.is_empty(), "{:?}", c.graph.diags);
    let flat = rut_core::link::flatten(c.graph.program.expect("linked program"));
    rut_vm::verify::verify(&flat).expect("verify");
    let mut vm = rut_vm::interp::Vm::builder()
        .program(std::rc::Rc::new(flat))
        .limits(rut_vm::interp::Limits {
            fuel: Some(2_000_000),
            heap_limit_bytes: Some(16 * 1024 * 1024),
            interrupt_every: 1024,
        })
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("vm");
    vm.call::<_, i32>("main", ()).expect("run main").into()
}

// ---- the same-file collision matrix ------------------------------------

#[test]
fn same_file_kind_pairs_refuse_citing_the_first() {
    // fn is the SECOND registration against every type kind (types
    // register in pass 1a, fns in pass 2 — the site is the fn
    // regardless of source order, the citation the type's kind)
    for (decl, kind) in [
        ("class X { }", "class"),
        ("struct X { v: i32 }", "struct"),
        ("enum X { M }", "enum"),
        ("interface X { fn m(self); }", "interface"),
        ("type X = i64;", "alias"),
    ] {
        // fn spelled FIRST still errors AT the fn (pass order)
        let ds = diags_of(&format!("fn X() {{ }}\n{decl}\nentry fn main() {{ }}\n"));
        assert!(
            ds.iter().any(|d| d.contains("duplicate name `X`")
                && d.contains(&format!("the {kind} already declares it in this file"))),
            "fn x {kind}: {ds:?}"
        );
        // and with the type spelled first, the fn is simply second
        let ds = diags_of(&format!("{decl}\nfn X() {{ }}\nentry fn main() {{ }}\n"));
        assert!(
            ds.iter().any(|d| d.contains("duplicate name `X`")
                && d.contains(&format!("the {kind} already declares it in this file"))),
            "{kind} x fn: {ds:?}"
        );
    }
    // let↔let and let↔fn/let↔type (module lets ride pass 2 in source
    // order, so the citation follows the source order)
    let ds = diags_of("let V = 1;\nlet V = 2;\nentry fn main() { }\n");
    assert!(
        ds.iter().any(|d| d.contains("duplicate name `V`")
            && d.contains("the module let already declares it in this file")),
        "let x let: {ds:?}"
    );
    let ds = diags_of("fn V() { }\nlet V = 1;\nentry fn main() { }\n");
    assert!(
        ds.iter().any(|d| d.contains("duplicate name `V`")
            && d.contains("the fn already declares it in this file")),
        "fn x let: {ds:?}"
    );
    let ds = diags_of("let V = 1;\nfn V() { }\nentry fn main() { }\n");
    assert!(
        ds.iter().any(|d| d.contains("duplicate name `V`")
            && d.contains("the module let already declares it in this file")),
        "let x fn: {ds:?}"
    );
    let ds = diags_of("struct V { v: i32 }\nlet V = 1;\nentry fn main() { }\n");
    assert!(
        ds.iter().any(|d| d.contains("duplicate name `V`")
            && d.contains("the struct already declares it in this file")),
        "type x let (the let always cites the type): {ds:?}"
    );
    // mod edge ↔ every kind: `mod m;` binds a file-scope name
    // like any decl (the edge registers in pass 1a, so fns/lets cite
    // it)
    for decl in [
        "fn m() { }",
        "class m { }",
        "struct m { v: i32 }",
        "enum m { M }",
        "let m = 1;",
    ] {
        let ds = diags_of(&format!("mod m;\n{decl}\nentry fn main() {{ }}\n"));
        assert!(
            ds.iter().any(|d| d.contains("duplicate name `m`")
                && d.contains("the `mod` edge already declares it in this file")),
            "mod x `{decl}`: {ds:?}"
        );
    }
    // and the edge second-spelled against a type (both pass 1a — the
    // SOURCE order decides the citation)
    let ds = diags_of("class m { }\nmod m;\nentry fn main() { }\n");
    assert!(
        ds.iter().any(|d| d.contains("duplicate name `m`")
            && d.contains("the class already declares it in this file")),
        "class x mod edge: {ds:?}"
    );
    let ds = diags_of("mod m;\nclass m { }\nentry fn main() { }\n");
    assert!(
        ds.iter().any(|d| d.contains("duplicate name `m`")
            && d.contains("the `mod` edge already declares it in this file")),
        "mod edge x class: {ds:?}"
    );
}

#[test]
fn decl_vs_use_refuses_with_the_use_citation() {
    // the use leaf registers in pass 0 — EVERY decl-vs-use pair keeps
    // the use-law shape, fired at the decl's site, regardless of
    // source order. The exporter only needs the NAME on its surface
    // (a pub fn Foo); the colliding decl kind varies.
    for decl in [
        "class Foo { }",
        "struct Foo { v: i32 }",
        "enum Foo { M }",
        "interface Foo { }",
        "type Foo = i64;",
        "fn Foo() { }",
        "let Foo = 1;",
    ] {
        let pkgs = vec![
            modpkg("pkga", "pub fn Foo() -> i32 { return 1; }\n", vec![]),
            modpkg(
                "app",
                &format!("use pkga::{{ Foo }};\n{decl}\nentry fn main() {{ }}\n"),
                vec![],
            ),
        ];
        let ds = compile_with_diags(&pkgs, "app");
        assert!(
            ds.iter().any(|d| d.contains("duplicate name `Foo`")
                && d.contains("`use pkga::{ Foo }`")
                && d.contains("already binds it in this file")),
            "use x `{decl}`: {ds:?}"
        );
        // and the decl spelled first — the use still registered first
        // (pass order), so the citation is identical
        let pkgs = vec![
            modpkg("pkga", "pub fn Foo() -> i32 { return 1; }\n", vec![]),
            modpkg(
                "app",
                &format!("{decl}\nuse pkga::{{ Foo }};\nentry fn main() {{ }}\n"),
                vec![],
            ),
        ];
        let ds = compile_with_diags(&pkgs, "app");
        assert!(
            ds.iter().any(|d| d.contains("duplicate name `Foo`")
                && d.contains("`use pkga::{ Foo }`")
                && d.contains("already binds it in this file")),
            "`{decl}` x use: {ds:?}"
        );
    }
}

// ---- the a/b/c illustration --------------------------------------------

#[test]
fn the_abc_illustration_siblings_bind_freely_the_third_file_collides() {
    // a.rut: fn Foo() — fine; b.rut: fn Foo() — fine (siblings never
    // meet); c's two use leaves collide AT THE SECOND LEAF, citing
    // the first (`use pkga::{ Foo }` already binds it in this file)
    let pkgs = vec![
        modpkg("pkga", "pub fn Foo() -> i32 { return 1; }\n", vec![]),
        modpkg("pkgb", "pub fn Foo() -> i32 { return 2; }\n", vec![]),
        modpkg(
            "app",
            "use pkga::{ Foo };\nuse pkgb::{ Foo };\nentry fn main() -> i32 { return Foo(); }\n",
            vec![],
        ),
    ];
    let diags = compile_with_diags(&pkgs, "app");
    assert!(
        diags.iter().any(|d| d.contains("duplicate name `Foo`")
            && d.contains("`use pkga::{ Foo }`")
            && d.contains("already binds it in this file")),
        "{diags:?}"
    );
    // a use leaf against a SIBLING's decl is no collision at all:
    // app's `use pkga::{ Foo }` binds one Foo; app's own decl of a
    // DIFFERENT name is untouched
    let ok = vec![
        modpkg("pkga", "pub fn Foo() -> i32 { return 1; }\n", vec![]),
        modpkg(
            "app",
            "use pkga::{ Foo };\nentry fn main() -> i32 { return Foo(); }\n",
            vec![],
        ),
    ];
    assert_eq!(run_main_pkgs(&ok, "app"), 1);
}

#[test]
fn use_in_a_third_file_binds_one_of_the_siblings() {
    // a and b both spell `mk`; the third file's `use a::{ mk }` binds
    // A's — the bare call resolves through the file's own binding
    let pkgs = vec![
        modpkg("a", "pub fn mk() -> i32 { return 3; }\n", vec![]),
        modpkg("b", "pub fn mk() -> i32 { return 4; }\n", vec![]),
        modpkg(
            "app",
            "use a::{ mk };\nentry fn main() -> i32 { return mk(); }\n",
            vec![],
        ),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "app"), 3);
}

// ---- the LEGAL relaxations (siblings, different kinds) ------------------

#[test]
fn fn_vs_type_in_sibling_modules_is_legal_each_file_resolves_its_own() {
    // a's `Config` is the struct, b's `Config` is the fn — the
    // file scopes never meet, and every file resolves its own
    let pkgs = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg(
            "pkg",
            "mod a;\nmod b;\nentry fn main() -> i32 {\n    let c: a::Config = a::mk(20);\n    return c.n + b::Config(1);\n}\n",
            vec![
                (
                    "a",
                    Vis::Self_,
                    "pub struct Config { n: i32 }\npub fn mk(n: i32) -> Config { return Config { n: n }; }\n",
                ),
                (
                    "b",
                    Vis::Self_,
                    "pub fn Config(tag: i32) -> i32 { return tag; }\n",
                ),
            ],
        ),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 21);
}

#[test]
fn lets_share_names_across_siblings() {
    // a's `LIMIT` is 3, b's is 5 — the value lane resolves per file
    // (let_in), the qualified spellings never blur
    let pkgs = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg(
            "pkg",
            "mod a;\nmod b;\nentry fn main() -> i32 { return a::LIMIT + b::LIMIT; }\n",
            vec![
                ("a", Vis::Self_, "pub let LIMIT = 3;\n"),
                ("b", Vis::Self_, "pub let LIMIT = 5;\n"),
            ],
        ),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 8);
}

// ---- the conservative boundary ------------------------------------------

#[test]
fn fn_fn_same_file_refuses_in_the_registry() {
    // two fns of one name in ONE file: the registry's decl-vs-decl
    // refusal citing the first — and never the program-wide
    // "already declared in module" text (that shape belongs to the
    // conservative CROSS-module gate only)
    let ds = diags_of("fn dup() { }\nfn dup() { }\nentry fn main() { }\n");
    assert!(
        ds.iter().any(|d| d.contains("duplicate name `dup`")
            && d.contains("the fn already declares it in this file")),
        "{ds:?}"
    );
    assert!(
        ds.iter().all(|d| !d.contains("already declared in module")),
        "same-file fns must not take the cross-module text: {ds:?}"
    );
}

#[test]
fn fn_fn_across_siblings_still_refuses_conservatively() {
    // THE MECHANISM-BOUND HALF of the law: the compile keys
    // (`FnKey::Free`, the instantiation map, the link ledger's
    // `Free { name }` row, the export surface) are name-keyed within
    // the owner package — a same-named fn pair would share one
    // compiled body. The gate stays program-wide until the keys
    // carry their module; the same-file pair is the registry's
    // (the matrix above), and the type↔type twin is pinned in
    // `mods.rs` (`duplicate_names_across_modules_are_loud`).
    let pkgs = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg(
            "pkg",
            "mod a;\nmod b;\nentry fn main() -> i32 { return 0; }\n",
            vec![
                ("a", Vis::Self_, "pub fn dup() -> i32 { return 1; }\n"),
                ("b", Vis::Self_, "pub fn dup() -> i32 { return 2; }\n"),
            ],
        ),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    assert!(
        diags
            .iter()
            .any(|d| d.contains("duplicate fn `dup`")
                && d.contains("already declared in module `a`")),
        "{diags:?}"
    );
}

// ---- the entry gate ------------------------------------------------------

#[test]
fn entry_fn_twice_in_sibling_modules_refuses() {
    // the entry gate (conservative on NAME): the host calls entry fns
    // by name, so the AMBIGUITY is a name spelled by two modules'
    // entries — the shape the old program-wide fn law used to catch.
    // Differently-named entries stay legal (the examples' dispatcher
    // surface spells many).
    let pkgs = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg(
            "pkg",
            "mod a;\nmod b;\nentry fn main() -> i32 { return 0; }\n",
            vec![
                ("a", Vis::Self_, "entry fn main() -> i32 { return 1; }\n"),
                ("b", Vis::Self_, "entry fn main() -> i32 { return 2; }\n"),
            ],
        ),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    assert!(
        diags
            .iter()
            .any(|d| d.contains("duplicate `entry fn` `main`")
                && d.contains("already spells it")
                && d.contains("the host calls entry fns by name")),
        "{diags:?}"
    );
    // the control: differently-named entries in sibling modules are
    // the host-callable surface, one fn per name
    let ok = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg(
            "pkg",
            "mod a;\nmod b;\nentry fn main() -> i32 { return 0; }\n",
            vec![
                ("a", Vis::Self_, "entry fn aux() -> i32 { return 1; }\n"),
                ("b", Vis::Self_, "entry fn zip() -> i32 { return 2; }\n"),
            ],
        ),
    ];
    assert_eq!(run_main_pkgs(&ok, "pkg"), 0);
}

// ---- ambient shadowing ---------------------------------------------------

#[test]
fn a_user_fn_named_like_an_engine_name_is_legal_and_wins() {
    // ambient (engine/prelude) names sit BELOW the file scope —
    // shadowable, never colliding. The file's `panic` binds its own;
    // the call resolves through the file scope BEFORE the ambient
    // layer is ever consulted (the local rung comes first).
    assert_eq!(
        run_returns(
            "fn panic(code: i32) -> i32 { return code + 1; }\n\
             entry fn main() -> i32 { return panic(41); }\n",
        )
        .expect("the file's own panic answers"),
        42
    );
}
