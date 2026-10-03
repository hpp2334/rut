//! Cross-module interfaces & structural satisfaction: an interface is
//! declared in one module, a type in another, and a consumer proves the
//! foreign interface by HAVING the members — on its own type's inherent
//! impl (a used type's members live where the type was declared; a
//! third-party inherent block is refused). Satisfaction is boundary
//! checking (the member-set match), interface-typed values dispatch
//! through per-type itable fills (global iface ids, one slot per
//! member), and the use statement governs which names the call site
//! sees.

use rut_driver::GraphOutput;
use rut_parser::Mode;



/// The shapes library: the interface AND both satisfying types live
/// here (each type's inherent impl carries `area`).
const SHAPES: &str = "\
interface Shape { fn area(self) -> f64; }
struct Point { x: f64 }
struct Circle { r: f64 }
impl Point { pub fn area(self) -> f64 { return self.x; } }
impl Circle { pub fn area(self) -> f64 { return self.r; } }
pub fn make_point() -> Point { return Point { x: 3.0 }; }
pub fn pick(k: bool) -> Shape {
    if (k) { return Point { x: 1.0 }; }
    return Circle { r: 2.0 };
}
";

/// A type library with NO interface and NO members — the interface and
/// the satisfaction live elsewhere (the consumer's own types satisfy).
const SHAPES_LIB: &str = "\
struct Point { x: f64 }
pub fn make_point() -> Point { return Point { x: 3.0 }; }
";

/// The interface + describing module: the interface is extras' own, the
/// satisfying type is the consumer's own — the consumer proves the
/// foreign interface structurally, at the boundary where it passes its
/// value to extras' interface-typed fn.
const EXTRAS: &str = "\
interface Shape { fn area(self) -> f64; }
pub fn describe(s: Shape) -> f64 {
    return s.area();
}
";


/// One source pkg over the auto core — the chain's graph. Closure and
/// shape refusals come back as one span-0 diagnostic.
#[allow(dead_code)] // not every suite in this file needs both lanes
fn compiled(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(rut_driver::RutRun::new().pkg(rut_driver::Pkg::source(spec, src)).entrypoint(spec).compile())
}

/// [`compiled`] with calc offered (the old `mount_std` shape: core
/// auto-rides, `calc` is an ordinary pkg).
#[allow(dead_code)]
fn compiled_std(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(
        rut_driver::RutRun::new()
            .pkg(rut_driver::Pkg::source(spec, src))
            .pkg(rut_driver::calc_pkg())
            .entrypoint(spec)
            .compile(),
    )
}

#[allow(dead_code)]
fn graph_of(c: Result<rut_driver::Compiled, rut_driver::RunError>) -> rut_driver::GraphOutput {
    match c {
        Ok(c) => c.graph,
        Err(e) => rut_driver::GraphOutput {
            diags: vec![rut_lexer::diag::Diag::new(rut_lexer::span::Span::new(0, 0), e.msg)],
            program: None,
        },
    }
}

fn graph(modules: &[(&str, &str)]) -> GraphOutput {
    let mut chain = rut_driver::RutRun::new();
    for (spec, src) in modules {
        chain = chain.pkg(rut_driver::Pkg::source(spec, *src));
    }
    let (root, _) = modules.last().expect("root module");
    graph_of(chain.entrypoint(root).compile())
}

fn linked(modules: &[(&str, &str)]) -> rut_core::binary::Program {
    let g = graph(modules);
    assert!(g.diags.is_empty(), "{:?}", g.diags);
    g.program.expect("linked program")
}

/// Compile a graph's linked program down to a runnable VM and answer
/// `main`'s return.
fn run_main(p: rut_core::binary::Program) -> i32 {
    let flat = rut_core::link::flatten(p);
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp:: Vm::builder().program(std::rc::Rc::new(flat)).limits(limits).hooks(rut_vm::interp::HostHooks::default()).hosts(rut_vm::interp::HostRegistry::new()).build()
    .expect("vm");
    vm.call("main", ()).expect("main runs")
}

fn ir(p: &rut_core::binary::Program) -> String {
    rut_driver::ir_dump_of(&p.funcs, &p.interner)
}

fn iface_of<'p>(p: &'p rut_core::binary::Program, name: &str) -> Option<(u32, &'p rut_core::binary::IfaceDesc)> {
    p.ifaces
        .iter()
        .enumerate()
        .find(|(_, t)| p.name_of(t.name) == name)
        .map(|(i, t)| (i as u32, t))
}

fn type_of(p: &rut_core::binary::Program, name: &str) -> Option<u32> {
    (0..p.types.types.len() as u32).find(|&i| p.type_name(i) == name)
}

#[test]
fn cross_module_member_call_binds_statically() {
    // the member lives on Point's inherent impl in `shapes`; the call
    // binds to its compiled fn directly — static dispatch, no vtable hop
    let p = linked(&[
        ("shapes", SHAPES),
        ("app", "\
use shapes::{Point, make_point};
entry fn main() -> i32 {
    let p = make_point();
    let a = p.area();
    return 0;
}
"),
    ]);
    let dump = ir(&p);
    assert!(dump.contains("callm"), "static bind through the foreign impl:\n{dump}");
    assert!(!dump.contains("calli"), "no vtable hop for a single concrete origin:\n{dump}");
}

#[test]
fn cross_module_iface_call_uses_the_vtable_when_origins_merge() {
    // `pick` returns the interface: both origins (Point, Circle) flow
    // through one global interface, and the dispatch hops the slot —
    // each concrete type's row carries its own fill
    let p = linked(&[
        ("shapes", SHAPES),
        ("app", "\
use shapes::{Shape, pick};
entry fn main() -> i32 {
    let s: Shape = pick(true);
    let v = s.area();
    return 0;
}
"),
    ]);
    let dump = ir(&p);
    assert!(dump.contains("calli"), "merged origins dispatch through the vtable:\n{dump}");
    let Some((gid, tdesc)) = iface_of(&p, "Shape") else {
        panic!("one global Shape interface: {:?}", p.ifaces.iter().map(|t| p.name_of(t.name)).collect::<Vec<_>>());
    };
    assert_eq!(tdesc.methods.len(), 1);
    let slot = p.slot_of(gid, 0).expect("global slot");
    for ty in ["Point", "Circle"] {
        let t = type_of(&p, ty).unwrap_or_else(|| panic!("{ty} in the global table"));
        let row = &p.vtables[t as usize];
        let f = row.get(slot as usize).and_then(|f| *f);
        assert!(f.is_some(), "{ty} fills Shape's slot: row {row:?}");
    }
    // and the two rows name DIFFERENT impl fns
    let point = p.vtables[type_of(&p, "Point").unwrap() as usize][slot as usize];
    let circle = p.vtables[type_of(&p, "Circle").unwrap() as usize][slot as usize];
    assert_ne!(point, circle, "each type carries its own impl");
}

#[test]
fn inherent_impl_on_a_used_class_is_refused() {
    // the placement law, restated for structural satisfaction: an
    // INHERENT block on a used class lives in the type's module — the
    // class's surface carries its members, and a consumer's block would
    // need the private layout. Satisfaction of a foreign interface is
    // proven on the consumer's OWN types, never by growing members on
    // a foreign one.
    let g = graph(&[
        ("shapes", SHAPES_LIB),
        ("app", "\
use shapes::{Point};
impl<T> Point {
    pub fn probe_hi(self) -> i64 { return 7; }
}
entry fn main() -> i32 { return 0; }
"),
    ]);
    assert!(g.program.is_none(), "the foreign inherent block must refuse");
    let ds = g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n");
    assert!(
        ds.contains("inherent impls live in the type's module"),
        "{ds}"
    );
}

#[test]
fn consumer_satisfies_a_foreign_interface_on_its_own_type() {
    // the structural fork's replacement for the old third-party shape:
    // the interface is extras' (foreign to app), the type is app's own,
    // and app proves `Shape` by spelling `area` on its inherent impl —
    // the boundary check at the interface-typed call admits it.
    let g = graph(&[
        ("extras", EXTRAS),
        ("app", "\
use extras::{Shape, describe};
struct Square { s: f64 }
impl Square {
    pub fn area(self) -> f64 { return self.s * self.s; }
}
entry fn main() -> i32 {
    let d = describe(Square { s: 3.0 });
    return 0;
}
"),
    ]);
    assert!(g.diags.is_empty(), "{:?}", g.diags);
    let p = g.program.expect("the consumer's own type satisfies the foreign interface");
    // the consumer's boxing site filled the GLOBAL row of its own type
    // under extras' interface (link merges used-interface ids)
    let Some((gid, _)) = iface_of(&p, "Shape") else { panic!("Shape in the global interface table: {:?}", p.ifaces.iter().map(|t| p.name_of(t.name)).collect::<Vec<_>>()) };
    let slot = p.slot_of(gid, 0).expect("global slot");
    let square = type_of(&p, "Square").expect("one global Square");
    let f = p.vtables[square as usize][slot as usize];
    assert!(f.is_some(), "the boxing site fills Square's row: {:?}", p.vtables[square as usize]);
    let fname = p.name_of(p.funcs[f.unwrap() as usize].name);
    assert!(fname.contains("area"), "the fill names the area member: {fname}");
}

#[test]
fn member_missing_on_the_receiver_is_the_unknown_member_diag() {
    // the use-gate diagnostic's nearest surviving law: a member call on
    // a type with no such member is the plain unknown-member error —
    // there is no registration for a `use` to complete, so the miss
    // names the type and the member.
    let g = graph(&[
        ("shapes", SHAPES_LIB),
        ("app", "\
use shapes::{Point, make_point};
entry fn main() -> i32 {
    let p = make_point();
    let a = p.area();
    return 0;
}
"),
    ]);
    assert!(g.program.is_none(), "the memberless receiver must refuse");
    let ds = g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n");
    assert!(
        ds.contains("area"),
        "the diag names the missing member: {ds}"
    );
}

#[test]
fn iface_typed_parameter_links_and_dispatches_through_the_vtable() {
    // `describe(s: Shape)` LINKS: an interface-typed parameter is an
    // ordinary value and the call dispatches through the itable slot
    // the consumer's boxing site filled. `main` calls the linked fn
    // through a plain Call, and the fn's body hops the slot (CallI).
    let g = graph(&[
        ("shapes", &format!("{SHAPES}\npub fn describe(s: Shape) -> f64 {{ return s.area(); }}\n")),
        ("app", "\
use shapes::{Shape, Point, make_point, describe};
entry fn main() -> i32 {
    let p = make_point();
    let d = describe(p);
    return 0;
}
"),
    ]);
    assert!(g.diags.is_empty(), "{:?}", g.diags);
    let p = g.program.expect("program");
    // `describe` linked as one fn; `main` calls it directly — never
    // through a spliced clone
    let didx = p.funcs.iter().position(|f| p.name_of(f.name) == "describe");
    let didx = didx.unwrap_or_else(|| panic!("describe linked:\n{}", ir(&p)));
    let main = p.funcs.iter().find(|f| p.name_of(f.name) == "main").unwrap();
    assert!(
        main.code.iter()
            .any(|op| matches!(op, rut_core::ops::Op::Call { func, .. } if *func as usize == didx)),
        "main calls the linked describe:\n{}",
        ir(&p)
    );
    // the linked body dispatches through the interface slot — the
    // per-type fill, not a per-argument clone
    assert!(
        p.funcs[didx].code.iter().any(|op| matches!(op, rut_core::ops::Op::CallI { .. })),
        "describe dispatches through the vtable:\n{}",
        ir(&p)
    );
    // and the dispatch answers at run time
    let got = run_main(p);
    assert_eq!(got, 0, "the linked interface-param call runs");
}

#[test]
fn satisfaction_failure_names_the_missing_member() {
    // the boundary check's diagnostic: passing a concrete where the
    // interface is expected runs the member-set match — the Go-shape
    // error names the type, the interface, and the first missing member
    let g = graph(&[
        ("extras", EXTRAS),
        ("app", "\
use extras::{Shape, describe};
struct TheirType { n: i32 }
entry fn main() -> i32 {
    let d = describe(TheirType { n: 1 });
    return 0;
}
"),
    ]);
    assert!(g.program.is_none(), "an unsatisfied concrete must refuse");
    let ds = g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n");
    assert!(
        ds.contains("`TheirType` does not satisfy `Shape`: no member `area`"),
        "{ds}"
    );
}

#[test]
fn satisfaction_failure_names_the_signature_mismatch() {
    // the member present but wrong-shaped: the check names the first
    // mismatched member instead
    let g = graph(&[
        ("extras", EXTRAS),
        ("app", "\
use extras::{Shape, describe};
struct Odd { n: i32 }
impl Odd {
    pub fn area(self) -> i32 { return self.n; }
}
entry fn main() -> i32 {
    let d = describe(Odd { n: 1 });
    return 0;
}
"),
    ]);
    assert!(g.program.is_none(), "a mismatched member must refuse");
    let ds = g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n");
    assert!(
        ds.contains("`Odd` does not satisfy `Shape`")
            && ds.contains("member `area`'s signature differs from the interface's"),
        "{ds}"
    );
}

#[test]
fn two_phase_surfaces_carry_interfaces() {
    // the lower-level pattern: a library's surface publishes its
    // interface decls; a consumer compiled against it links into one
    // program with one merged interface table (no impl registrations —
    // the fills are demand-recorded at boxing sites)
    let dep = rut_driver::compile_program(SHAPES, Mode::Impl, "shapes", 1, &[]);
    assert!(dep.diags.is_empty(), "{:?}", dep.diags);
    let dep = dep.program.expect("dep");
    let surface = dep.surface.clone();
    assert!(
        surface.ifaces.iter().any(|t| surface.names.name(t.name) == "Shape"),
        "surface exports the Shape decl"
    );
    let app = rut_driver::compile_program(
        "use shapes::{Shape, Point, make_point};\n\
         entry fn main() -> i32 {\n\
             let p = make_point();\n\
             let a = p.area();\n\
             return 0;\n\
         }\n",
        Mode::Impl,
        "app",
        2,
        &[(1, surface, "shapes".to_string())],
    );
    assert!(app.diags.is_empty(), "{:?}", app.diags);
    let out = rut_core::link::link(vec![dep, app.program.expect("app")]).expect("link");
    assert!(iface_of(&out, "Shape").is_some(), "one merged Shape");
    let dump = ir(&out);
    assert!(dump.contains("callm") && !dump.contains("calli"), "\n{dump}");
}
