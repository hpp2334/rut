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
            .register_module(spec, rut_driver::Module { source: Some(src.to_string()), ..Default::default() })
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
    let mut vm = rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default())
        .expect("vm");
    match vm.call("main", &[]).expect("main runs") {
        rut_vm::heap::Value::I64(n) => n as i32,
        other => panic!("expected an i32 result, got {other:?}"),
    }
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
pub fn main() -> i32 {
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
pub fn main() -> i32 {
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
pub fn main() -> i32 {
    let s: Shape = pick(false);
    let a = s.area();
    return a as i32;
}
"),
    ]);
    assert_eq!(false_case, 2, "Circle's impl via the vtable");
}

#[test]
fn foreign_trait_impl_for_a_foreign_type_runs() {
    // the impl lives in a third module: `Shape` and `Point` are both
    // foreign to it (RFC 0012 §2); the consumer uses both names and the
    // call resolves through the module that registered the impl
    let extras = "\
use shapes::{Shape, Point};
impl Shape for Point {
    fn area(self) -> f64 { return 42.0; }
}
pub fn make_point() -> Point { return Point { x: 5.0 }; }
";
    let out = run_graph(&[
        ("shapes", "trait Shape { fn area(self) -> f64; }\nstruct Point { x: f64 }\n"),
        ("extras", extras),
        ("app", "\
use shapes::{Shape, Point};
use extras::make_point;
pub fn main() -> i32 {
    let p = make_point();
    let a = p.area();
    return a as i32;
}
"),
    ]);
    assert_eq!(out, 42, "extras' impl answered for shapes' Point");
}
