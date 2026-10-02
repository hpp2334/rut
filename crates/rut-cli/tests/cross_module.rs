//! Cross-module traits, executed: a trait declared in one module, a type
//! in another, impls wherever they belong — linked into one program and
//! run. Static dispatch (single concrete origin) and vtable dispatch
//! (merged origins) both execute through the merged registry.

use std::cell::RefCell;
use std::rc::Rc;

/// Compile a module graph and run its `main`, returning the exit code.
fn run_graph(modules: &[(&str, &str)]) -> i32 {
    let mut session = rut_driver::Session::new();
    for (spec, src) in modules {
        session
            .register_module(spec, rut_driver::Module { body: rut_driver::ModuleBody::Source { text: src.to_string(), is_decl: false }, ..Default::default() })
            .expect("mount");
    }
    let (root, _) = modules.last().expect("root");
    let g = rut_driver::compile_graph(&session, root);
    assert!(g.diags.is_empty(), "{:?}", g.diags);
    let binary = rut_core::binary::encode(g.program.as_ref().expect("program"));
    let prog = rut_core::binary::decode(&binary).expect("decode");
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), rut_vm::interp::HostRegistry::new())
        .expect("vm");
    let n: i32 = vm.call("main", ()).expect("main runs");
    n
}

const SHAPES: &str = "\
trait Shape { fn area(self) -> f64; }
struct Point { x: f64 }
struct Circle { r: f64 }
impl Shape for Point { fn area(self) -> f64 { return self.x; } }
impl Shape for Circle { fn area(self) -> f64 { return self.r; } }
pub fn make_point() -> Point { return Point { x: 3.0 }; }
pub fn pick(k: bool) -> Shape {
    if (k) { return Point { x: 1.0 }; }
    return Circle { r: 2.0 };
}
";

#[test]
fn cross_module_static_dispatch_runs_the_foreign_impl() {
    let out = run_graph(&[
        ("shapes", SHAPES),
        ("app", "\
use shapes::{Shape, Point, make_point};
entry fn main() -> i32 {
    let p = make_point();
    let a = p.area();
    return a as i32;
}
"),
    ]);
    assert_eq!(out, 3, "Point's impl in shapes answered the call");
}

#[test]
fn cross_module_vtable_dispatch_runs_both_impls() {
    // merged origins: pick() returns either concrete type; each result
    // dispatches through the merged vtable to its own impl
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
    assert_eq!(true_case, 1, "Point's impl via the vtable");
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
    assert_eq!(false_case, 2, "Circle's impl via the vtable");
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
fn type_local_impl_for_a_foreign_trait_runs() {
    // the impl lives in a second module: `Shape` is foreign to it, the
    // type is its own; the consumer uses both names and
    // the call resolves through the module that registered the impl
    let extras = "\
use shapes::Shape;
struct Point { x: f64 }
impl Shape for Point {
    fn area(self) -> f64 { return 42.0; }
}
pub fn make_point() -> Point { return Point { x: 5.0 }; }
";
    let out = run_graph(&[
        ("shapes", "trait Shape { fn area(self) -> f64; }\n"),
        ("extras", extras),
        ("app", "\
use shapes::Shape;
use extras::make_point;
entry fn main() -> i32 {
    let p = make_point();
    let a = p.area();
    return a as i32;
}
"),
    ]);
    assert_eq!(out, 42, "extras' impl answered for extras' Point");
}

// ---- generic foreign traits (the v1 gates lifted) ----

const WRAP: &str = "trait Wrap<T> { fn unwrap_or(self, d: T) -> T; }\n";

#[test]
fn generic_foreign_trait_impl_for_local_type_static_dispatch() {
    // the consumer unit implements a FOREIGN generic trait for its own
    // generic type, and a call site in the SAME unit dispatches
    // statically through the template: `impl<T> Wrap<T> for Box2<T>`
    // registers against the carried `Wrap` descriptor, the concrete
    // `Wrap<i32>` mints per instantiation, and the method body
    // monomorphizes here. A second unit then reaches the SAME impl
    // through the consumer-registered surface row (the mirror lane —
    // the row's mint owner is the impl's home, not the trait's pkg).
    let extras = "\
use traits::Wrap;
struct Box2<T> { v: T }
impl<T> Wrap<T> for Box2<T> {
    fn unwrap_or(self, d: T) -> T { return self.v; }
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
    return try_get(b);
}
"),
    ]);
    assert_eq!(out, 7, "the consumer unit's impl answered both call sites");
}

#[test]
fn generic_foreign_trait_spelled_in_param_type() {
    // the spelling half: a foreign generic trait in type position
    // (`fn describe(w: Wrap<i32>)`) — the carried descriptor mints the
    // instantiation, the argument widens through the registered impl,
    // and the call dispatches by the ordinary law (single concrete
    // origin ⇒ static, else the vtable the fill registered).
    let extras = "\
use traits::Wrap;
struct Box2<T> { v: T }
impl<T> Wrap<T> for Box2<T> {
    fn unwrap_or(self, d: T) -> T { return self.v; }
}
pub fn make() -> Box2<i32> { return Box2 { v: 9 }; }
pub fn describe(w: Wrap<i32>) -> i32 { return w.unwrap_or(0); }
";
    let out = run_graph(&[
        ("traits", WRAP),
        ("extras", extras),
        ("app", "\
use traits::Wrap;
use extras::{Box2, describe, make};
entry fn main() -> i32 {
    let b: Box2<i32> = make();
    return describe(b);
}
"),
    ]);
    assert_eq!(out, 9, "the spelled foreign trait type answered the call");
}

#[test]
fn two_instantiations_of_one_foreign_generic_trait() {
    // the mint cache: `Wrap<i32>` and `Wrap<str>` are two descriptors
    // off one carried decl — each dispatches to its own instantiation,
    // and a widened local of either dispatches statically (the
    // single-origin mint the registry produced)
    let extras = "\
use traits::Wrap;
pub struct Box2<T> { v: T }
impl<T> Wrap<T> for Box2<T> {
    fn unwrap_or(self, d: T) -> T { return self.v; }
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

#[test]
fn orphan_generic_impl_still_rejected() {
    // foreign trait + foreign type: the ONE cross-module impl
    // restriction — the orphan rule — stands unchanged under the lifted
    // genericity gates, with its existing diagnostic.
    let mut session = rut_driver::Session::new();
    for (spec, src) in [
        ("traits", WRAP),
        ("types", "pub struct Cell<T> { v: T }\n"),
        (
            "extras",
            "\
use traits::Wrap;
use types::Cell;
impl<T> Wrap<T> for Cell<T> {
    fn unwrap_or(self, d: T) -> T { return self.v; }
}
",
        ),
    ] {
        session
            .register_module(spec, rut_driver::Module { body: rut_driver::ModuleBody::Source { text: src.to_string(), is_decl: false }, ..Default::default() })
            .expect("mount");
    }
    let g = rut_driver::compile_graph(&session, "extras");
    assert!(
        g.diags.iter().any(|d| d.msg.contains("orphan impl")),
        "the orphan gate rejects the foreign pair: {:?}",
        g.diags
    );
}
