//! The one-block-per-scope law — the third duplicate-boot-scope
//! type-interning defect, pinned beside the two in
//! `cross_scope_identity.rs`.
//!
//! A unit's surface is its whole table above the boot prefix, so every
//! package that depends on `common` RE-SHIPS common's block. A consumer
//! that binds three such surfaces (the dependency itself and two
//! packages that both depend on it — tur's playground root: `tur_kit`
//! plus two kits built on it) used to register common's block three
//! times: each `TypeTable::use_block` moved the scope's base onto the
//! new copy and left the earlier copies' rows in the table. The middle
//! copy then sat directly above another package's own block — still
//! "registered" through THAT scope's base — so a structural spelling of
//! one of its rows (`TypeTable::intern`'s name+kind dedup: a closure
//! whose type common's block already carries) hit the stale copy, and
//! `id_for` spelled it through the other scope. Link maps such an id
//! past that scope's real block, and the VM dies at boot on an
//! out-of-range type id (`Interp::new`'s ref-register scan).
//!
//! The law: a scope an earlier surface already registered keeps its
//! block and its base; a later surface's re-shipped copy is not
//! appended again.

/// The dependency: its own block carries the `fn(u64) -> u64` row —
/// late in the block, behind a dozen filler classes, so a row spelled
/// through the NEXT scope's block lands past that scope's real rows
/// (and, after link, past the table).
const COMMON: &str = "\
pub class C0 { a: u64; }
pub class C1 { a: u64; }
pub class C2 { a: u64; }
pub class C3 { a: u64; }
pub class C4 { a: u64; }
pub class C5 { a: u64; }
pub class C6 { a: u64; }
pub class C7 { a: u64; }
pub class C8 { a: u64; }
pub class C9 { a: u64; }
pub class C10 { a: u64; }
pub class C11 { a: u64; }

pub fn apply(f: fn(u64) -> u64, v: u64) -> u64 {
    return f(v);
}

pub fn bump(v: u64) -> u64 {
    return v + 1u64;
}
";

/// A package on `common`: its surface re-ships common's block and
/// carries own rows after it.
const ALPHA: &str = "\
use common::{ apply, bump };

pub class Wrap {
    kid: u64;
}

impl Wrap {
    [constructor] pub fn of(kid: u64) -> Self {
        return Self { kid: kid };
    }
    pub fn get(self) -> u64 {
        return self.kid;
    }
}

pub fn wrapped(v: u64) -> Wrap {
    return Wrap::of(apply(bump, v));
}
";

/// A second package on `common` — the third registration of the block.
const BETA: &str = "\
use common::{ bump };

pub fn beta_bump(v: u64) -> u64 {
    return bump(v);
}
";

/// The consumer: alpha's surface, common's own, beta's — then a closure
/// of its own whose type is a row common's block already carries.
const ROOT: &str = "\
use alpha::{ wrapped };
use common::{ bump };
use beta::{ beta_bump };

entry fn main() -> i32 {
    let mine = fn (x: u64) -> u64 { return x + 5u64; };
    if (mine(10u64) != 15u64) { return 1; }
    let w = wrapped(10u64);
    if (w.get() != 11u64) { return 2; }
    if (bump(10u64) != 11u64) { return 3; }
    if (beta_bump(10u64) != 11u64) { return 4; }
    return 0;
}
";

fn graph(modules: &[(&str, &str)]) -> rut_driver::GraphOutput {
    let mut chain = rut_driver::RutRun::new();
    for (spec, src) in modules {
        let heads: Vec<&str> = modules
            .iter()
            .map(|(s, _)| *s)
            .filter(|s| *s != *spec)
            .collect();
        chain = chain.mount(
            rut_driver::Mounted::source(spec, *src)
                .manifest(rut_driver::Manifest::lib(spec, &heads)),
        );
    }
    let (root, _) = modules.last().expect("root");
    match chain.entrypoint(root).compile() {
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

fn diags_of(g: &rut_driver::GraphOutput) -> String {
    g.diags
        .iter()
        .map(|d| d.msg.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

fn run_main(modules: &[(&str, &str)]) -> i32 {
    let g = graph(modules);
    assert!(g.diags.is_empty(), "{}", diags_of(&g));
    let prog = rut_core::link::flatten(g.program.expect("program"));
    rut_vm::verify::verify(&prog).expect("verify");
    let mut vm = rut_vm::interp::Vm::builder()
        .program(std::rc::Rc::new(prog))
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("vm");
    vm.call("main", ()).expect("the entry runs")
}

const WORLD: &[(&str, &str)] = &[
    ("common", COMMON),
    ("alpha", ALPHA),
    ("beta", BETA),
    ("app", ROOT),
];

#[test]
fn a_re_shipped_block_registers_once_per_scope() {
    let g = graph(WORLD);
    assert!(g.diags.is_empty(), "{}", diags_of(&g));
    let prog = rut_core::link::flatten(g.program.expect("program"));
    rut_vm::verify::verify(&prog).expect("verify");
    // the entry's own closure: a function-typed register of its own
    let main = prog
        .funcs
        .iter()
        .find(|f| prog.interner.name(f.name) == "main")
        .expect("main");
    assert!(
        main.regs.iter().any(|&r| {
            (r as usize) < prog.types.types.len()
                && matches!(
                    prog.types.types[r as usize].kind,
                    rut_core::types::TyKind::Fn { .. }
                )
        }),
        "`main` creates a closure — one of its registers is function-typed"
    );
    // the closure's type IS common's row — the one `apply` names as its
    // first parameter — not merely some row inside the table
    let apply = prog
        .funcs
        .iter()
        .find(|f| prog.interner.name(f.name) == "apply")
        .expect("`apply` links: `wrapped` calls it");
    for &r in &main.regs {
        if (r as usize) < prog.types.types.len()
            && matches!(
                prog.types.types[r as usize].kind,
                rut_core::types::TyKind::Fn { .. }
            )
        {
            assert_eq!(r, apply.params[0], "the closure's type is common's row");
        }
    }
}

#[test]
fn a_closure_type_a_carried_block_already_holds_keeps_its_scope() {
    assert_eq!(
        run_main(WORLD),
        0,
        "the consumer's closure type is common's row, never a row lent by another scope"
    );
}
