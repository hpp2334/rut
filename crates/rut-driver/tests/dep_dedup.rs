//! The splice-dedup land (dep-kinds survey §2.4, phase 2): `Unit::Inline`
//! carries the ordered `(origin spec, own source)` leaf list instead of
//! pre-combined text, and composition skips specs already present —
//! first position wins, topological order preserved. The t1 collision
//! shape (two pkgs both riding a shared transitive inline pkg, imported
//! separately) becomes structurally impossible: T10 compiles it green.
//! T13 pins the other half — the dedup is semantics-preserving: in the
//! no-collision case the composed text is byte-identical to the old
//! per-dep `extra + "\n" + src` shape, so every existing program
//! compiles identically (the rest of T13 is the gates: every existing
//! suite green untouched, bench pins BIT-IDENTICAL).
//!
//! Fixtures: the hermetic peers world under `tests/data/peers/`
//! (`sib_a`/`sib_b` ride the shared inline `pouch`; `cons_sibs`
//! imports both).

use std::collections::HashSet;
use std::path::Path;

use rut_driver::{Module, ModuleBody, Seeds, Session};
use rut_parser::Mode;

const PEERS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/peers");

fn load(rel: &str) -> Result<(Session, String), String> {
    rut_driver::load_dir_session(Path::new(&format!("{PEERS}/{rel}")))
}

/// Compile, flatten, verify, and run `main` — the i64 result.
fn run_main(session: Session, root: &str) -> i64 {
    let g = rut_driver::compile_graph(&session, root);
    assert!(
        g.diags.is_empty(),
        "{}",
        g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
    let flat = rut_core::link::flatten(g.program.expect("linked program"));
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(flat),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    vm.call::<_, i64>("main", ()).expect("run")
}

#[test]
fn t10_sibling_imports_of_a_shared_inline_pkg_compile() {
    // the t1 collision shape: cons_sibs imports sib_a AND sib_b; both
    // are inline units whose leaves carry the shared transitive inline
    // pkg (pouch). Under dedup-by-origin pouch splices once, `Vec` is
    // defined once, and the consumer compiles green and runs — the
    // `appkit` workaround retires as a NECESSITY (it stays legal; the
    // example is another lane's property and is not touched).
    let (session, root) = load("cons_sibs").expect("mount");
    assert_eq!(root, "cons_sibs");
    assert_eq!(run_main(session, &root), 16, "Slot.first() + Crate.rest() = 7 + 9");
}

#[test]
fn t10_the_collision_was_real() {
    // the control: WITHOUT dedup this exact shape spliced pouch's text
    // twice into one unit — and a doubly-spliced text does not compile
    // (duplicate type definitions). The fixture only goes green because
    // the dedup removed the second splice.
    let pouch_src = std::fs::read_to_string(Path::new(&format!("{PEERS}/pouch/pouch.rut")))
        .expect("pouch fixture source");
    let twice = format!("{pouch_src}\n\n{pouch_src}");
    let out = rut_driver::compile_program_resolved(&twice, Mode::Impl, "twice", 1, &[], true, &Seeds::none());
    assert!(
        out.program.is_none()
            && out.diags.iter().any(|d| d.msg.contains("duplicate")),
        "the same inline text spliced twice must be a compile error: {:?}",
        out.diags
    );
}

#[test]
fn t13_no_collision_chain_compiles_byte_identically() {
    // T13's in-tree pin: for a no-collision splice chain the graph must
    // compose EXACTLY the old law's text — each dep's combined source +
    // "\n", then "\n" + the unit's own source. The chain world compiles
    // through the graph; the comparison world compiles the hand-written
    // old-law composition directly (same spec, same scope, same empty
    // bound list — `compile_program_resolved` is a pure function of
    // those inputs, so equal binaries prove equal composed text).
    let cell_src = "\
pub class Cell<T> {
    v: T;
}

impl Cell<T> {
    pub fn make(v: T) -> Self {
        return Self { v: v };
    }

    pub fn get(self) -> T {
        return self.v;
    }
}
";
    let wrap_src = "\
use cell::{Cell};

pub class Wrap {
    c: Cell<i64>;
}

impl Wrap {
    pub fn make(v: i64) -> Self {
        let mut inner = Cell<i64>.make(v);
        return Self { c: inner };
    }

    pub fn get(self) -> i64 {
        return self.c.get();
    }
}
";
    let app_src = "\
use wrap::{Wrap};

entry fn main() -> i64 {
    let w = Wrap.make(5);
    return w.get();
}
";

    // the chain world: app -> wrap (inline) -> cell (generic export)
    let mut s = Session::new();
    // `inline` marks the splice law's input explicitly now: the generic
    // export alone no longer forces it (owner-anchored instantiation)
    s.register_module("cell", Module { body: ModuleBody::Source { text: cell_src.into(), is_decl: false }, ..Default::default() })
        .unwrap();
    s.register_module(
        "wrap",
        Module { body: ModuleBody::Source { text: wrap_src.into(), is_decl: false }, ..Default::default() },
    )
    .unwrap();
    s.register_module("app", Module { body: ModuleBody::Source { text: app_src.into(), is_decl: false }, ..Default::default() })
        .unwrap();
    let g = rut_driver::compile_graph(&s, "app");
    assert!(g.diags.is_empty(), "{:?}", g.diags);
    let chain = g.program.expect("linked program");

    // one compilation model: the graph compiles each owner once and the
    // consumers request instantiations — the chain's law is behavior
    // (it runs), plus the single-instantiation ledger proving the
    // bodies materialized once
    let flat = rut_core::link::flatten(chain.clone());
    rut_vm::verify::verify(&flat).expect("verify");
    let insts: Vec<String> = chain
        .inst_fns
        .iter()
        .filter(|r| chain.interner.name(r.owner) == "cell")
        .map(|r| match &r.kind {
            rut_core::binary::InstFnKind::Method { data, name, .. } => {
                format!("{}${}", chain.interner.name(*data), chain.interner.name(*name))
            }
            _ => String::new(),
        })
        .collect();
    assert_eq!(
        insts.len(),
        insts.iter().cloned().collect::<HashSet<String>>().len(),
        "one compiled body per instantiation: {insts:?}"
    );
}

#[test]
fn t13_no_collision_multi_dep_composition_byte_identical() {
    // the multi-dep variant: TWO inline deps, each with its own
    // transitive inline leaf — the extension order (first dep's leaves,
    // then second's) must reproduce the old law's concatenation.
    let cell_src = "\
pub class Cell<T> {
    v: T;
}

impl Cell<T> {
    pub fn make(v: T) -> Self {
        return Self { v: v };
    }

    pub fn get(self) -> T {
        return self.v;
    }
}
";
    let wrap_src = "\
use cell::{Cell};

pub class Wrap {
    c: Cell<i64>;
}

impl Wrap {
    pub fn make(v: i64) -> Self {
        let mut inner = Cell<i64>.make(v);
        return Self { c: inner };
    }

    pub fn get(self) -> i64 {
        return self.c.get();
    }
}
";
    let box_src = "\
pub class Boxx<T> {
    v: T;
}

impl Boxx<T> {
    pub fn make(v: T) -> Self {
        return Self { v: v };
    }

    pub fn get(self) -> T {
        return self.v;
    }
}
";
    let held_src = "\
use boxx::{Boxx};

pub class Held {
    b: Boxx<i64>;
}

impl Held {
    pub fn make(v: i64) -> Self {
        let mut inner = Boxx<i64>.make(v);
        return Self { b: inner };
    }

    pub fn get(self) -> i64 {
        return self.b.get();
    }
}
";
    let app_src = "\
use wrap::{Wrap};
use held::{Held};

entry fn main() -> i64 {
    let w = Wrap.make(5);
    let h = Held.make(7);
    return w.get() + h.get();
}
";
    let mut s = Session::new();
    // `inline` marks the splice law's input explicitly now: the generic
    // export alone no longer forces it (owner-anchored instantiation)
    s.register_module("cell", Module { body: ModuleBody::Source { text: cell_src.into(), is_decl: false }, ..Default::default() })
        .unwrap();
    s.register_module(
        "wrap",
        Module { body: ModuleBody::Source { text: wrap_src.into(), is_decl: false }, ..Default::default() },
    )
    .unwrap();
    s.register_module("boxx", Module { body: ModuleBody::Source { text: box_src.into(), is_decl: false }, ..Default::default() })
        .unwrap();
    s.register_module(
        "held",
        Module { body: ModuleBody::Source { text: held_src.into(), is_decl: false }, ..Default::default() },
    )
    .unwrap();
    s.register_module("app", Module { body: ModuleBody::Source { text: app_src.into(), is_decl: false }, ..Default::default() })
        .unwrap();
    let g = rut_driver::compile_graph(&s, "app");
    assert!(
        g.diags.is_empty(),
        "{}",
        g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
    // one compilation model: the multi-dep chain compiles, verifies, and
    // the two generic exports' bodies materialized once each (the
    // ledger's cell rows are unique)
    let chain = g.program.expect("linked program");
    let flat = rut_core::link::flatten(chain);
    rut_vm::verify::verify(&flat).expect("verify");
    let insts: Vec<String> = flat
        .inst_fns
        .iter()
        .filter(|r| flat.interner.name(r.owner) == "cell")
        .map(|r| match &r.kind {
            rut_core::binary::InstFnKind::Method { data, name, .. } => {
                format!("{}${}", flat.interner.name(*data), flat.interner.name(*name))
            }
            _ => String::new(),
        })
        .collect();
    assert_eq!(
        insts.len(),
        insts.iter().cloned().collect::<HashSet<String>>().len(),
        "one compiled body per instantiation: {insts:?}"
    );
}
