//! Cross-module traits & the impl registry (RFC 0012 §2/§5/§6): trait
//! impls may live in ANY module — a trait declared in one, a type in
//! another, the impl in a third. Per-module compiles cannot see each
//! other's registrations, so module surfaces export traits + impls, the
//! link merges them (duplicate `(trait, type)` pairs are a link error,
//! global trait ids, cross-scope vtable fill), and the use-both gate
//! holds at the call site.

use rut_driver::{GraphOutput, Module, Session};
use rut_parser::Mode;

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

/// A trait+type library with NO impls — the impl lives elsewhere.
const SHAPES_LIB: &str = "\
trait Shape { fn area(self) -> f64; }
struct Point { x: f64 }
pub fn make_point() -> Point { return Point { x: 3.0 }; }
";

/// An impl module: `impl ForeignTrait for ForeignType` — both foreign
/// (RFC 0012 §2), plus a helper its consumers can use.
const EXTRAS: &str = "\
use shapes::{Shape, Point};
impl Shape for Point {
    fn area(self) -> f64 { return 42.0; }
}
pub fn describe() -> f64 {
    let p = Point { x: 1.0 };
    return p.area();
}
pub fn make_point() -> Point { return Point { x: 5.0 }; }
";

fn graph(modules: &[(&str, &str)]) -> GraphOutput {
    let mut s = Session::new();
    for (spec, src) in modules {
        let _ = s.register_module(spec, Module { source: Some(src.to_string()), ..Default::default() });
    }
    let (root, _) = modules.last().expect("root module");
    rut_driver::compile_graph(&s, root)
}

fn linked(modules: &[(&str, &str)]) -> rut_core::binary::Program {
    let g = graph(modules);
    assert!(g.diags.is_empty(), "{:?}", g.diags);
    g.program.expect("linked program")
}

fn ir(p: &rut_core::binary::Program) -> String {
    rut_driver::ir_dump_of(&p.funcs, &p.interner)
}

fn trait_of<'p>(p: &'p rut_core::binary::Program, name: &str) -> Option<(u32, &'p rut_core::binary::TraitDesc)> {
    p.traits
        .iter()
        .enumerate()
        .find(|(_, t)| p.name_of(t.name) == name)
        .map(|(i, t)| (i as u32, t))
}

fn type_of(p: &rut_core::binary::Program, name: &str) -> Option<u32> {
    (0..p.types.types.len() as u32).find(|&i| p.type_name(i) == name)
}

#[test]
fn cross_module_trait_call_binds_statically() {
    let p = linked(&[
        ("shapes", SHAPES),
        ("app", "\
use shapes::{Shape, Point, make_point};
fn main() -> i32 {
    let p = make_point();
    let a = p.area();
    return 0;
}
"),
    ]);
    let dump = ir(&p);
    // the impl lives in `shapes`; the call binds to its compiled fn
    // directly — static dispatch, no vtable hop (RFC 0012 §5)
    assert!(dump.contains("callm"), "static bind through the foreign impl:\n{dump}");
    assert!(!dump.contains("calli"), "no vtable hop for a single concrete origin:\n{dump}");
}

#[test]
fn cross_module_trait_call_uses_the_vtable_when_origins_merge() {
    let p = linked(&[
        ("shapes", SHAPES),
        ("app", "\
use shapes::{Shape, pick};
fn main() -> i32 {
    let s: Shape = pick(true);
    let v = s.area();
    return 0;
}
"),
    ]);
    let dump = ir(&p);
    assert!(dump.contains("calli"), "merged origins dispatch through the vtable:\n{dump}");
    // both impls filled the merged (global) slot — pick()'s runtime
    // result finds its method whichever concrete type it carries
    let Some((gid, tdesc)) = trait_of(&p, "Shape") else {
        panic!("one global Shape trait: {:?}", p.traits.iter().map(|t| p.name_of(t.name)).collect::<Vec<_>>());
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
fn foreign_trait_impl_for_a_foreign_type_dispatches() {
    // `Shape` is declared in `shapes`, `Point` too, but the impl lives
    // in `extras` — the third-party-module shape of RFC 0012 §2: a
    // consumer uses both names and calls through whichever module
    // registered the impl.
    let p = linked(&[
        ("shapes", SHAPES_LIB),
        ("extras", EXTRAS),
        ("app", "\
use shapes::{Shape, Point};
use extras::{describe, make_point};
fn main() -> i32 {
    let p = make_point();
    let a = p.area();
    let d = describe();
    return 0;
}
"),
    ]);
    let dump = ir(&p);
    assert!(dump.contains("callm"), "static bind through extras' impl:\n{dump}");
    assert!(!dump.contains("calli"), ":\n{dump}");
    // cross-scope vtable fill: extras' registration fills the GLOBAL row
    // of shapes' Point (link merges used-block rows)
    let Some((gid, _)) = trait_of(&p, "Shape") else { panic!("Shape in the global trait table") };
    let slot = p.slot_of(gid, 0).expect("global slot");
    let point = type_of(&p, "Point").expect("one global Point");
    let f = p.vtables[point as usize][slot as usize];
    assert!(f.is_some(), "extras' impl fills Point's row: {:?}", p.vtables[point as usize]);
    let fname = p.name_of(p.funcs[f.unwrap() as usize].name);
    assert!(fname.contains("area"), "the fill names the area impl: {fname}");
}

#[test]
fn duplicate_impl_pair_across_modules_is_a_link_error() {
    // `shapes` and `extras` BOTH register (Shape, Point). Neither
    // compile can see the other — the collision surfaces only at link.
    let g = graph(&[
        ("shapes", SHAPES),
        ("extras", EXTRAS),
        ("app", "\
use extras::make_point;
fn main() -> i32 { return 0; }
"),
    ]);
    assert!(
        g.program.is_none(),
        "the duplicate pair must refuse to link"
    );
    assert!(
        g.diags.iter().any(|d| d.msg.contains("duplicate impl")
            && d.msg.contains("(Shape, Point)")),
        "{:?}",
        g.diags
    );
}

#[test]
fn unused_but_implemented_trait_gives_the_use_gate_diagnostic() {
    // `Point` is used, `Shape` is not in any `use` — the impl exists,
    // but the use-both gate keeps its methods uncallable and says so
    // (RFC 0012 §6).
    let g = graph(&[
        ("shapes", SHAPES),
        ("app", "\
use shapes::{Point, make_point};
fn main() -> i32 {
    let p = make_point();
    let a = p.area();
    return 0;
}
"),
    ]);
    assert!(
        g.diags.iter().any(|d| {
            d.msg.contains("use `Shape` to call its methods on `Point`")
        }),
        "{:?}",
        g.diags
    );
}

#[test]
fn trait_typed_parameter_specializes_per_concrete_argument() {
    // `describe(s: Shape)` cannot be linked: its trait-typed parameter
    // specializes per concrete argument where the arguments are (RFC
    // 0012 §5 — one clone per argument type), so its source splices
    // into the consumer and `s.area()` binds statically against the
    // argument's type.
    let g = graph(&[
        ("shapes", &format!("{SHAPES}\npub fn describe(s: Shape) -> f64 {{ return s.area(); }}\n")),
        ("app", "\
use shapes::{Shape, Point, make_point, describe};
fn main() -> i32 {
    let p = make_point();
    let d = describe(p);
    return 0;
}
"),
    ]);
    assert!(g.diags.is_empty(), "{:?}", g.diags);
    let p = g.program.expect("program");
    // the clone of `describe` for the `Point` argument binds `area`
    // statically (one clone per concrete argument type, RFC 0012 §5),
    // and `main` calls it through a plain Call — never the vtable
    let sidx = p.funcs.iter().position(|f| {
        p.name_of(f.name) == "describe"
            && f.code.iter().any(|op| matches!(op, rut_core::ops::Op::CallM { .. }))
    });
    let sidx = sidx.unwrap_or_else(|| panic!("describe specialized per argument:\n{}", ir(&p)));
    let main = p.funcs.iter().find(|f| p.name_of(f.name) == "main").unwrap();
    assert!(
        main.code.iter()
            .any(|op| matches!(op, rut_core::ops::Op::Call { func, .. } if *func as usize == sidx)),
        "main calls the specialized clone:\n{}",
        ir(&p)
    );
    assert!(
        !main.code.iter().any(|op| matches!(op, rut_core::ops::Op::CallI { .. })),
        "main never hops the vtable:\n{}",
        ir(&p)
    );
}

#[test]
fn two_phase_surfaces_carry_traits_and_impls() {
    // the lower-level pattern: a library's surface publishes its trait
    // decls and impl registrations; a consumer compiled against it
    // links into one program with one merged trait
    let dep = rut_driver::compile_program(SHAPES, Mode::Impl, "shapes", 1, &[]);
    assert!(dep.diags.is_empty(), "{:?}", dep.diags);
    let dep = dep.program.expect("dep");
    let surface = dep.surface.clone();
    assert!(
        surface.traits.iter().any(|t| surface.names.name(t.name) == "Shape"),
        "surface exports the Shape decl"
    );
    assert_eq!(surface.impls.len(), 2, "surface carries both impl registrations");
    let app = rut_driver::compile_program(
        "use shapes::{Shape, Point, make_point};\n\
         fn main() -> i32 {\n\
             let p = make_point();\n\
             let a = p.area();\n\
             return 0;\n\
         }\n",
        Mode::Impl,
        "app",
        2,
        &[(1, surface)],
    );
    assert!(app.diags.is_empty(), "{:?}", app.diags);
    let out = rut_core::link::link(vec![dep, app.program.expect("app")]).expect("link");
    assert!(trait_of(&out, "Shape").is_some(), "one merged Shape");
    let dump = ir(&out);
    assert!(dump.contains("callm") && !dump.contains("calli"), "\n{dump}");
}

