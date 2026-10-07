//! Cross-module interfaces, executed: an interface declared in one
//! module, types in other modules satisfying it STRUCTURALLY — by
//! having the members on their own PUB inherent impls (placement is
//! irrelevant; there is no registration to place). Linked into one
//! program and run: concrete member calls, producer-side boxing (an
//! interface-typed return in the members' own module), interface-param
//! boundaries carrying an existing box across modules, consumer-side
//! satisfaction of a foreign interface (the consumer's own type), and
//! foreign generic interfaces in type position.

use std::rc::Rc;

/// Compile a module graph and run its `main`, returning the exit code.
fn run_graph(modules: &[(&str, &str)]) -> i32 {
    let mut chain = rut_driver::RutRun::new();
    for (spec, src) in modules {
        chain = chain.pkg(rut_driver::Pkg::source(spec, *src));
    }
    let (root, _) = modules.last().expect("root");
    let g = chain.entrypoint(root).compile().expect("compile the graph");
    assert!(g.graph.diags.is_empty(), "{:?}", g.graph.diags);
    let binary = rut_core::binary::encode(g.graph.program.as_ref().expect("program"));
    let prog = rut_core::binary::decode(&binary).expect("decode");
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp:: Vm::builder().program(Rc::new(prog)).limits(limits).hooks(rut_vm::interp::HostHooks::default()).hosts(rut_vm::interp::HostRegistry::new()).build()
        .expect("vm");
    let n: i32 = vm.call("main", ()).expect("main runs");
    n
}

/// Compile a module graph and return its diagnostics (the refusal lane).
fn compile_expect_err(modules: &[(&str, &str)]) -> Vec<String> {
    let mut chain = rut_driver::RutRun::new();
    for (spec, src) in modules {
        chain = chain.pkg(rut_driver::Pkg::source(spec, *src));
    }
    let (root, _) = modules.last().expect("root");
    let g = chain.entrypoint(root).compile().expect("compile the graph");
    g.graph.diags.iter().map(|d| d.msg.clone()).collect()
}

const SHAPES: &str = "\
pub interface Shape { fn area(self) -> f64; }
pub fn describe(s: Shape) -> f64 { return s.area(); }
pub fn pick(k: bool) -> Shape {
    if (k) { return Point { x: 1.0 }; }
    return Circle { r: 2.0 };
}
struct Point { x: f64 }
struct Circle { r: f64 }
impl Point { pub fn area(self) -> f64 { return self.x; } }
impl Circle { pub fn area(self) -> f64 { return self.r; } }
";

#[test]
fn cross_module_member_call_runs_the_foreign_members() {
    let out = run_graph(&[
        ("shapes", SHAPES),
        ("app", "\
use shapes::{Point, make_point};
entry fn main() -> i32 {
    let p = Point { x: 3.0 };
    let a = p.area();
    return a as i32;
}
"),
    ]);
    assert_eq!(out, 3, "Point's inherent members in shapes answered the call");
}

#[test]
fn cross_module_boxed_dispatch_runs_both_types() {
    // merged origins: pick() boxes either concrete type at its own
    // `Shape` return (the members' module proves both satisfactions);
    // each result dispatches through the itable to its own members
    let true_case = run_graph(&[
        ("shapes", SHAPES),
        ("app", "\
use shapes::{Shape, pick};
entry fn main() -> i32 {
    let s: Shape = pick(true);
    let a = s.area();
    return a as i32;
}
"),
    ]);
    assert_eq!(true_case, 1, "Point's members via the itable");
    let false_case = run_graph(&[
        ("shapes", SHAPES),
        ("app", "\
use shapes::{Shape, pick};
entry fn main() -> i32 {
    let s: Shape = pick(false);
    let a = s.area();
    return a as i32;
}
"),
    ]);
    assert_eq!(false_case, 2, "Circle's members via the itable");
}

#[test]
fn cross_module_interface_param_boundary_carries_the_box() {
    // the param boundary: an interface-typed parameter crosses modules —
    // the value arrives ALREADY BOXED (its satisfaction proven in the
    // members' unit, here by `pick`), and the callee's `s.area()`
    // dispatches through the box's slot
    let out = run_graph(&[
        ("shapes", SHAPES),
        ("app", "\
use shapes::{Shape, describe, pick};
entry fn main() -> i32 {
    let s: Shape = pick(true);
    let a = describe(s);
    return a as i32;
}
"),
    ]);
    assert_eq!(out, 1, "the boxed value crossed the param boundary and dispatched");
}

#[test]
fn cross_module_fn_value_position() {
    // the bare fn-path lane crosses `use`: an imported fn's value binds
    // the exporter's scope-qualified fn (the link relocates closure
    // targets exactly like call targets)
    let out = run_graph(&[
        ("helpers", "pub fn triple(x: i32) -> i32 { return x * 3; }"),
        ("app", "\
use helpers::triple;
fn apply(f: fn(i32) -> i32, v: i32) -> i32 { return f(v); }
entry fn main() -> i32 {
    return apply(triple, 4);
}
"),
    ]);
    assert_eq!(out, 12, "the imported fn's value answered the indirect call");
}

#[test]
fn consumer_side_satisfaction_of_a_foreign_interface() {
    // consumer-side satisfaction: the CONSUMER's own type qualifies for
    // a FOREIGN interface by having the member (pub, on its own
    // inherent impl) — the once-orphan shape, now registration-free:
    // there is no placement to refuse, the member-set match is all
    // there is. The box crosses into the interface's home module as a
    // plain interface value.
    let extras = "\
use shapes::Shape;
pub struct Wrap2 { x: f64 }
impl Wrap2 {
    pub fn area(self) -> f64 { return 7.0; }
}
pub fn log_area(w: Wrap2) -> f64 {
    let s: Shape = w;
    return s.area();
}
";
    let out = run_graph(&[
        ("shapes", "pub interface Shape { fn area(self) -> f64; }\npub fn describe(s: Shape) -> f64 { return s.area(); }\n"),
        ("extras", extras),
        ("app", "\
use shapes::{Shape, describe};
use extras::{Wrap2, log_area};
struct Point { x: f64 }
impl Point {
    pub fn area(self) -> f64 { return 42.0; }
}
entry fn main() -> i32 {
    let p = Point { x: 5.0 };
    let a = p.area();            // the concrete member, statically
    let s: Shape = p;            // consumer-side satisfaction, boxed here
    let b = s.area();            // ...dispatches through the itable
    let c = describe(s);         // the box crosses the foreign boundary
    let d = log_area(Wrap2 { x: 1.0 });  // extras proves its own pair
    return (a + b + c + d) as i32;
}
"),
    ]);
    assert_eq!(out, 133, "42 + 42 + 42 + 7 — the foreign pair answered everywhere");
}

// ---- generic foreign interfaces (the v1 gates stay lifted) ----

const WRAP: &str = "pub interface Wrap<T> { fn unwrap_or(self, d: T) -> T; }\n";

#[test]
fn generic_foreign_interface_satisfied_by_local_type_static_dispatch() {
    // the consumer unit's own generic type satisfies a FOREIGN generic
    // interface by having the member; a call site in the SAME unit
    // dispatches statically through the monomorphized member, and the
    // foreign generic interface spells in TYPE POSITION (`Wrap<i32>`)
    // exactly like a local one.
    let extras = "\
use traits::Wrap;
struct Box2<T> { v: T }
impl<T> Box2<T> {
    pub fn unwrap_or(self, d: T) -> T { return self.v; }
}
pub fn make() -> Box2<i32> { return Box2 { v: 7 }; }
pub fn try_get(b: Box2<i32>) -> i32 { return b.unwrap_or(-1); }
";
    let out = run_graph(&[
        ("traits", WRAP),
        ("extras", extras),
        ("app", "\
use traits::Wrap;
use extras::{Box2, make, try_get};
entry fn main() -> i32 {
    let b: Box2<i32> = make();
    if (b.unwrap_or(0) != 7) { return 1; }
    let w: Wrap<i32> = b;
    if (w.unwrap_or(1) != 7) { return 2; }
    return try_get(make());
}
"),
    ]);
    assert_eq!(out, 7, "the consumer unit's members answered every call site");
}

#[test]
fn foreign_generic_interface_param_boundary_carries_the_box() {
    // the generic spelling of the boundary: a foreign generic interface
    // in param-type position (`fn describe(w: Wrap<i32>)`), the value
    // boxed in the members' unit (its own `Wrap<i32>`-typed local), the
    // box crossing and dispatching in the callee
    let extras = "\
use traits::Wrap;
pub struct Box2<T> { v: T }
impl<T> Box2<T> {
    pub fn unwrap_or(self, d: T) -> T { return self.v; }
}
pub fn make() -> Box2<i32> { return Box2 { v: 9 }; }
pub fn boxed() -> Wrap<i32> { let w: Wrap<i32> = make(); return w; }
";
    let out = run_graph(&[
        ("traits", WRAP),
        ("extras", extras),
        ("app", "\
use traits::Wrap;
use extras::boxed;
fn describe(w: Wrap<i32>) -> i32 { return w.unwrap_or(0); }
entry fn main() -> i32 {
    return describe(boxed());
}
"),
    ]);
    assert_eq!(out, 9, "the spelled foreign interface type answered the call");
}

#[test]
fn two_instantiations_of_one_foreign_generic_interface() {
    // the mint cache: `Wrap<i32>` and `Wrap<str>` are two descriptors
    // off one carried decl — each dispatches to its own instantiation,
    // and a widened local of either dispatches statically (the
    // single-origin mint the registry produced)
    let extras = "\
use traits::Wrap;
pub struct Box2<T> { v: T }
impl<T> Box2<T> {
    pub fn unwrap_or(self, d: T) -> T { return self.v; }
}
pub fn make_i() -> Box2<i32> { return Box2 { v: 7 }; }
pub fn make_s() -> Box2<str> { return Box2 { v: \"hi\" }; }
";
    let out = run_graph(&[
        ("traits", WRAP),
        ("extras", extras),
        ("app", "\
use traits::Wrap;
use extras::{Box2, make_i, make_s};
entry fn main() -> i32 {
    let a: Box2<i32> = make_i();
    let b: Box2<str> = make_s();
    let x: i32 = a.unwrap_or(0);
    let y: str = b.unwrap_or(\"d\");
    let w: Wrap<i32> = a;
    let z: i32 = w.unwrap_or(1);
    return x + z + y.len() as i32;
}
"),
    ]);
    assert_eq!(out, 16, "both instantiations answered (7 + 7 + 2)");
}

// NOTE on the one shape this suite does NOT yet pin: boxing a
// FOREIGN-OWNED concrete type in the consumer's unit (`let s: Shape =
// foreign_point;`). The fill's member is extern there, and the
// extern-fill mirror is still landing compiler-side (a nil-stub verify
// failure, or a runtime "no impl for interface slot N"). Every crossing
// above proves satisfaction in the members' own unit; when the mirror
// lands, a consumer-side box of a foreign type belongs here as a test.

// ---- the fill/dispatch identity law ---------------------------------------
//
// The reactor shape (downstream tur): a kit defines per-kind source
// classes over generic interfaces; a consumer boxes the concrete
// sources at its own bindings and passes the BOXES back into the kit's
// generic ctx methods (`ctx.set<f64>(src, v)` — the param is
// `Writable<T>`, the value's static type is `Readable<T>`). The fill
// the callee's dynamic dispatch reads must be keyed on the value's
// carried ORIGIN row — the row the payload cell carries — never on the
// box's own spelling. A fill recorded against the box row while the
// dispatch reads the payload row is the trap shape: `no impl for
// interface slot N on SourceF64`.

const REACTOR_KIT: &str = "\
pub interface Readable<T> { fn get(self) -> T; fn atom_id(self) -> u64; }
pub interface Writable<T> { fn put(self, v: T) -> nil; }
pub class SourceF64 {
  v: f64;
  atom: u64;
}
impl SourceF64 {
  [constructor] pub fn of(v: f64) -> Self { return Self { v: v, atom: 7 }; }
  pub fn get(self) -> f64 { return self.v; }
  pub fn put(mut self, v: f64) -> nil { self.v = v; }
  pub fn atom_id(self) -> u64 { return self.atom; }
}
pub class SourceStr {
  v: str;
  atom: u64;
}
impl SourceStr {
  [constructor] pub fn of(v: str) -> Self { return Self { v: v, atom: 8 }; }
  pub fn get(self) -> str { return self.v; }
  pub fn put(mut self, v: str) -> nil { self.v = v; }
  pub fn atom_id(self) -> u64 { return self.atom; }
}
pub class SourceBool {
  v: bool;
  atom: u64;
}
impl SourceBool {
  [constructor] pub fn of(v: bool) -> Self { return Self { v: v, atom: 9 }; }
  pub fn get(self) -> bool { return self.v; }
  pub fn put(mut self, v: bool) -> nil { self.v = v; }
  pub fn atom_id(self) -> u64 { return self.atom; }
}
// the read-only lane: satisfies Readable only — the write face refuses it
pub class StaticValue {
  v: f64;
}
impl StaticValue {
  [constructor] pub fn of(v: f64) -> Self { return Self { v: v }; }
  pub fn get(self) -> f64 { return self.v; }
}
pub class MutationCtx {
  n: i32;
}
impl MutationCtx {
  [constructor] pub fn over(n: i32) -> Self { return Self { n: n }; }
  pub fn get<T>(self, r: Readable<T>) -> T { return r.get(); }
  pub fn set<T>(self, s: Writable<T>, v: T) -> nil { s.put(v); }
}
pub fn source_f64(v: f64) -> SourceF64 { return SourceF64.of(v); }
pub fn source_str(v: str) -> SourceStr { return SourceStr.of(v); }
pub fn source_bool(v: bool) -> SourceBool { return SourceBool.of(v); }
pub fn static_value(v: f64) -> StaticValue { return StaticValue.of(v); }
pub fn mutate_ctx() -> MutationCtx { return MutationCtx.over(0); }
";

#[test]
fn boxed_foreign_sources_dispatch_through_the_generic_ctx() {
    // the boxed spell: the consumer widens at its OWN binding
    // (`let src: Readable<f64> = source_f64(..)`) and passes the BOX
    // back into the kit's generic set/get. The dispatch lands — and
    // the write is observable through a fresh read (tick mutations
    // land).
    let out = run_graph(&[
        ("kit", REACTOR_KIT),
        ("app", "\
use kit::{ Readable, source_f64, mutate_ctx };
entry fn main() -> i32 {
    let src: Readable<f64> = source_f64(60.0);
    let ctx = mutate_ctx();
    ctx.set<f64>(src, 2.5);
    let after: f64 = ctx.get<f64>(src);
    return after as i32;
}
"),
    ]);
    assert_eq!(out, 2, "the write through the boxed handle landed (60.0 -> 2.5)");
}

#[test]
fn a_consumer_fn_over_boxed_params_rides_the_param_origins() {
    // the sync_chrome shape: a consumer fn spells its params as the
    // interfaces; inside, the boxed params flow into the generic ctx —
    // the call's origins pin the params' concrete rows, the fills land
    // there, and both the write and the read answer
    let out = run_graph(&[
        ("kit", REACTOR_KIT),
        ("app", "\
use kit::{ MutationCtx, Readable, source_f64, source_str, mutate_ctx };
fn sync(ctx: MutationCtx, remaining: Readable<f64>, pill: Readable<str>) -> f64 {
    ctx.set<f64>(remaining, 41.0);
    ctx.set<str>(pill, \"Running\");
    return ctx.get<f64>(remaining);
}
entry fn main() -> i32 {
    let remaining: Readable<f64> = source_f64(60.0);
    let pill: Readable<str> = source_str(\"Ready\");
    let out = sync(mutate_ctx(), remaining, pill);
    let probe = mutate_ctx();
    let after: f64 = probe.get<f64>(remaining);
    return (out + after) as i32;
}
"),
    ]);
    assert_eq!(out, 82, "the write landed inside the callee AND through the caller's box");
}

#[test]
fn a_mutation_lambda_writes_through_its_captured_sources() {
    // the b_start shape: a lambda captures the CONCRETE sources; the
    // body's boxed passes re-bind through the captures' carried
    // origins
    let out = run_graph(&[
        ("kit", REACTOR_KIT),
        ("app", "\
use kit::{ MutationCtx, source_f64, source_bool, mutate_ctx };
entry fn main() -> i32 {
    let remaining = source_f64(60.0);
    let running = source_bool(false);
    let f = fn (ctx: MutationCtx) {
        ctx.set<bool>(running, true);
        ctx.set<f64>(remaining, 5.0);
    };
    f(mutate_ctx());
    let probe = mutate_ctx();
    let r: f64 = probe.get<f64>(remaining);
    let b: bool = probe.get<bool>(running);
    return r as i32 + b as i32;
}
"),
    ]);
    assert_eq!(out, 6, "the captures' writes landed (5.0 + true)");
}

#[test]
fn an_untracked_box_refuses_the_cross_interface_pass() {
    // the origin law's edge: a box whose concrete origin is untracked
    // (a fn RETURN — no binding pins a row) proves nothing; the pass
    // refuses at compile time instead of recording a fill the
    // dispatch can never find
    let out = compile_expect_err(&[
        ("kit", REACTOR_KIT),
        ("app", "\
use kit::{ MutationCtx, Readable, source_f64, mutate_ctx };
fn fresh() -> Readable<f64> { return source_f64(1.0); }
entry fn main() -> i32 {
    let ctx = mutate_ctx();
    ctx.set<f64>(fresh(), 2.0);
    return 0;
}
"),
    ]);
    assert!(
        out.iter().any(|d| d.contains("does not satisfy") && d.contains("no member set is visible")),
        "the untracked pass diagnoses: {out:?}"
    );
}

#[test]
fn the_write_face_refuses_a_read_only_source() {
    // the write-side law through the origin: StaticValue satisfies
    // Readable only — spelling it into a write slot diagnoses with the
    // missing member, never a silent dispatch
    let out = compile_expect_err(&[
        ("kit", REACTOR_KIT),
        ("app", "\
use kit::{ MutationCtx, Readable, static_value, mutate_ctx };
entry fn main() -> i32 {
    let lit: Readable<f64> = static_value(9.0);
    let ctx = mutate_ctx();
    ctx.set<f64>(lit, 1.0);
    return 0;
}
"),
    ]);
    assert!(
        out.iter().any(|d| d.contains("does not satisfy") && d.contains("no member `put`")),
        "the read-only source refuses the write slot: {out:?}"
    );
}

// the orphan-placement law is DEAD: `impl I for T` was the only thing
// placement could gate, and structural satisfaction has no registration
// — a foreign interface + a type qualifies wherever the members are
// (consumer_side_satisfaction_of_a_foreign_interface pins the
// once-orphan shape running).
