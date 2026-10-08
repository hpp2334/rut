//! The seed-attach scope-collision law: a reseed's seed rows never move
//! a base a carried block already registered.
//!
//! The downstream shape (the tur kit): TWO surfaces share a dependency —
//! the kit (`kit`) and the app both `use` the row exporter (`fut`), and
//! the app's mirror requests for the kit's generics carry FUT-scope
//! argument rows. The kit's reseed attaches those rows as a seed block
//! under the fut scope — and the old attach MOVED the base: the kit's
//! table already carried fut's FULL surface block (registered when the
//! kit bound fut's surface), and the sparse seed run (only the requested
//! locals, padding shells in the gaps) re-registered the same scope over
//! it — last writer wins. Every fut id the kit spells re-densified
//! through the sparse run: locals past its end panicked
//! (`type_at` index out of bounds, mid-reseed), locals inside it hit a
//! padding shell — a silent wrong-row dispatch. The pinned shape asserts
//! both directions land on the right rows after the reseed, twice (the
//! repeat-load shape: two independent compiles, two VM instances).

use std::rc::Rc;

fn compile_graph(modules: &[(&str, &str)]) -> rut_core::binary::Program {
    let mut chain = rut_driver::RutRun::new();
    for (spec, src) in modules {
        chain = chain.pkg(rut_driver::Pkg::source(spec, *src));
    }
    let (root, _) = modules.last().expect("root");
    let g = chain.entrypoint(root).compile().expect("compile the graph");
    assert!(
        g.graph.diags.is_empty(),
        "{}",
        g.graph
            .diags
            .iter()
            .map(|d| d.msg.clone())
            .collect::<Vec<_>>()
            .join("\n")
    );
    rut_core::link::flatten(g.graph.program.expect("program"))
}

fn run_entry(prog: rut_core::binary::Program, entry: &str) -> i32 {
    rut_vm::verify::verify(&prog).expect("verify");
    let mut vm = rut_vm::interp::Vm::builder()
        .program(Rc::new(prog))
        .hooks(rut_vm::interp::HostHooks::default())
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("vm");
    vm.call(entry, ()).expect("the entry runs")
}

const FUT: &str = "\
// the row exporter both surfaces share: a generic holder class, a
// generic constructor fn (its `#T` placeholder row interns at the TOP of
// this table — past any sparse seed run), and filler rows so the table
// is wide enough for the collision to bite at real locals. `demo`
// instantiates Hold<i32> IN this unit, so the exporter's own surface
// carries the concrete row — every consumer spelling Hold<i32> dedups
// onto the exporter-scoped id, which is what puts fut-scope rows in the
// app's mirror requests.
pub interface Held<T> { fn get(self) -> T; }
pub class Hold<T> {
  v: T;
  tag: i32;
}
impl<T> Hold<T> {
  [constructor] pub fn of(v: T) -> Self { return Self { v: v, tag: 1 }; }
  pub fn get(self) -> T { return self.v; }
  pub fn tag(self) -> i32 { return self.tag; }
}
pub class Pad {
  n: i32;
}
impl Pad {
  [constructor] pub fn of(n: i32) -> Self { return Self { n: n }; }
  pub fn n(self) -> i32 { return self.n; }
}
pub fn wrap<T>(v: T) -> Hold<T> { return Hold.of(v); }
pub fn pad(n: i32) -> Pad { return Pad.of(n); }
pub fn demo() -> i32 { let h: Hold<i32> = wrap(7); return h.get(); }
";

const KIT: &str = "\
// the kit surface: its generics take FUT-typed arguments (the app's
// mirror requests then spell fut-scope rows), and its own generic body
// calls fut's generic fn — the substitution that re-densifies a fut id
// when the reseeded bodies compile.
use fut::{ Hold, wrap };
pub fn passthrough<T>(x: T) -> T { return x; }
pub fn rewrap<T>(v: T) -> Hold<T> { return wrap(v); }
";

const APP: &str = "\
use fut::{ Held, Hold, wrap };
use kit::{ passthrough, rewrap };
entry fn main() -> i32 {
    // the mirror-request shape: the app requests the kit's generic with a
    // FUT-scope argument — the request's descriptor closure seeds fut rows
    // into the kit's reseed
    let h: Hold<i32> = wrap(7);
    let via_kit: Hold<i32> = passthrough<Hold<i32>>(h);
    // the kit's own generic body calls fut's generic fn — compiled from
    // the SEEDED kit program, its substitution reads a fut local
    let r: Hold<i32> = rewrap<i32>(5);
    // fut's own surface, direct: the full block the kit also carries
    let direct: Hold<i32> = wrap(9);
    return via_kit.get() * 100 + r.get() * 10 + direct.get();
}
";

#[test]
fn a_reseeds_seed_rows_never_move_a_carried_blocks_base() {
    let prog = compile_graph(&[("fut", FUT), ("kit", KIT), ("app", APP)]);
    // 7*100 + 5*10 + 9 — every dispatch landed on the row its surface
    // spelled, the kit's own rows and fut's alike
    assert_eq!(
        run_entry(prog, "main"),
        759,
        "both surfaces dispatch correctly after the reseed"
    );
}

#[test]
fn the_shared_scope_survives_a_repeat_load() {
    // the multi-instance shape: an independent compile + VM per load —
    // no state may survive that redirects one load's rows through
    // another's
    for i in 0..2 {
        let prog = compile_graph(&[("fut", FUT), ("kit", KIT), ("app", APP)]);
        assert_eq!(
            run_entry(prog, "main"),
            759,
            "load {i}: both surfaces dispatch correctly after the reseed"
        );
    }
}
