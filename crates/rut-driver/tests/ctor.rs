//! The `[constructor]` designation — the third designated surface beside
//! `[disposal]`/`[iterable]`. The call form `Type(..)` binds to the ONE
//! `[constructor]`-marked impl member of the class: a call-site consumer
//! (the lowering is byte-identical to `Type.ctor(..)`), never an engine
//! row. The pins here:
//!
//! - the sugar: `Point(..)` runs and lowers exactly like
//!   `Point.from_xy(..)` (the IR dumps are byte-identical);
//! - the seal: visibility rides the member's `pub`, the designation
//!   rides the bracket — the call IS the class-method call;
//! - `?Self`: the marked member's try-construction makes `Type(..)`
//!   yield `?Type`;
//! - the gates: one per class (the duplicate names both members), no
//!   receiver, `Self`/`?Self` return, class targets only, newtypes
//!   never carry it (`Name(v)` already IS the construction — the
//!   newtype mint keeps precedence), generics ride the ordinary
//!   unification path;
//! - resolution: a free fn named like the class wins; the no-ctor miss
//!   names the fix;
//! - cross-module: the marker byte crosses the surface row, a used
//!   class's `Point(..)` mints at a distance.

use std::cell::RefCell;
use std::rc::Rc;

use rut_parser::Mode;
use rut_vm::interp::{HostRegistry, Limits, Vm};

/// One source pkg over the auto core — the chain's graph. Closure and
/// shape refusals come back as one span-0 diagnostic.
#[allow(dead_code)] // not every suite in this file needs both lanes
fn compiled(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(rut_driver::RutRun::new().pkg(rut_driver::Pkg::source(spec, src)).entrypoint(spec).compile())
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

fn compile(src: &str) -> rut_driver::ProgramOutput {
    // core bound as the one use: these tests exercise construction,
    // not use discipline
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
        return Err(out.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("; "));
    }
    let prog = rut_core::link::flatten(out.program.expect("linked program"));
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = Vm::builder().program(std::rc::Rc::new(prog)).limits(limits).hooks(rut_vm::interp::HostHooks::default()).hosts(HostRegistry::new()).build()
    .map_err(|e| format!("{e:?}"))?;
    vm.call::<_, i32>("main", ()).map(|v| v as i64).map_err(|e| format!("{e:?}"))
}

/// A multi-pkg graph (the last module is the root).
fn graph(modules: &[(&str, &str)]) -> rut_driver::GraphOutput {
    let mut chain = rut_driver::RutRun::new();
    for (spec, src) in modules {
        chain = chain.pkg(rut_driver::Pkg::source(spec, *src));
    }
    let (root, _) = modules.last().expect("root module");
    graph_of(chain.entrypoint(root).compile())
}

/// run a linked graph's `main` (i64 through the typed boundary).
fn run_graph(g: &rut_driver::GraphOutput) -> Result<i64, String> {
    if !g.diags.is_empty() {
        return Err(g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("; "));
    }
    let prog = g.program.as_ref().expect("linked program").clone();
    let limits = Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = Vm::builder().program(std::rc::Rc::new(prog)).limits(limits).hooks(rut_vm::interp::HostHooks::default()).hosts(HostRegistry::new()).build()
    .map_err(|e| format!("{e:?}"))?;
    vm.call::<_, i32>("main", ()).map(|v| v as i64).map_err(|e| format!("{e:?}"))
}

/// The IR dump of a module's `main` — the lowering-evidence text the
/// byte-identity pin compares.
fn ir_of_main(src: &str) -> String {
    let out = compile(src);
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let prog = out.program.expect("program");
    rut_driver::ir_dump_of(&prog.funcs, &prog.interner)
}

const POINT: &str = "\
class Point {
    x: f32;
    y: f32;
}
impl Point {
    [constructor] pub fn from_xy(x: f32, y: f32) -> Self {
        return Self { x: x, y: y };
    }
    pub fn manhattan(self) -> f32 { return self.x + self.y; }
}
";

// ---- the sugar ---------------------------------------------------------

#[test]
fn the_call_form_runs_like_the_designated_member() {
    // the plan's own example, through both spellings — the sugar and
    // the member call answer identically
    let v = run_returns(&format!(
        "{POINT}\n\
         entry fn main() -> i32 {{\n\
             let p = Point(1.0, 2.0);\n\
             let q = Point.from_xy(1.0, 2.0);\n\
             if (p.manhattan() != q.manhattan()) {{ return 1; }}\n\
             if (p.manhattan() != 3.0) {{ return 2; }}\n\
             return 0;\n\
         }}\n"
    ))
    .expect("runs");
    assert_eq!(v, 0);
}

#[test]
fn the_lowering_is_byte_identical_to_the_member_call() {
    // `Point(..)` IS `Point.from_xy(..)` — the compiled `main` bodies
    // dump identically (same inst, same ops; the spelling is sugar)
    let sugar = ir_of_main(&format!(
        "{POINT}\nentry fn main() -> f32 {{ let p = Point(1.0, 2.0); return p.manhattan(); }}\n"
    ));
    let explicit = ir_of_main(&format!(
        "{POINT}\nentry fn main() -> f32 {{ let p = Point.from_xy(1.0, 2.0); return p.manhattan(); }}\n"
    ));
    assert_eq!(sugar, explicit, "the sugar must lower byte-identically");
}

// ---- the ?Self try-construction ----------------------------------------

#[test]
fn try_ctor_yields_the_nullable() {
    // `?Self` makes `Type(..)` a try-construction: nil on the refusal,
    // the record on the success
    let v = run_returns(
        "use core::{ panic };\
         class Port { n: i32; }\
         impl Port {\
             [constructor] pub fn open(n: i32) -> ?Self {\
                 if (n <= 0) { return nil; }\
                 return Self { n: n };\
             }\
         }\
         entry fn main() -> i32 {\
             let p = Port(41);\
             if (p == nil) { return 1; }\
             let none = Port(-1);\
             if (none != nil) { return 2; }\
             return 0;\
         }\n",
    )
    .expect("runs");
    assert_eq!(v, 0);
}

// ---- the signature gates -----------------------------------------------

#[test]
fn arity_mismatch_names_the_member() {
    // the sugar's arity refusal is the member call's — it names
    // `Point.from_xy` (byte-identical diagnostics, like lowering)
    let ds = diags_of(&format!(
        "{POINT}\nentry fn main() {{ let p = Point(1.0); }}\n"
    ));
    assert!(
        ds.iter().any(|d| d == "call arity: `Point.from_xy` takes 2 parameters, 1 given"),
        "the sugar's arity refusal names the member: {ds:?}"
    );
    let ds = diags_of(&format!(
        "{POINT}\nentry fn main() {{ let p = Point.from_xy(1.0); }}\n"
    ));
    assert!(
        ds.iter().any(|d| d == "call arity: `Point.from_xy` takes 2 parameters, 1 given"),
        "the explicit spelling answers identically: {ds:?}"
    );
}

#[test]
fn duplicate_constructor_names_both_members() {
    let ds = diags_of(
        "class Point { x: f32; y: f32; }\
         impl Point {\
             [constructor] pub fn from_xy(x: f32, y: f32) -> Self { return Self { x: x, y: y }; }\
             [constructor] pub fn origin() -> Self { return Self { x: 0.0, y: 0.0 }; }\
         }\
         entry fn main() { }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`origin` and `from_xy` both designate the construction surface")),
        "the duplicate names both spellings: {ds:?}"
    );
    // an earlier impl block's member counts too (the decl carries it)
    let ds = diags_of(
        "class Point { x: f32; y: f32; }\
         impl Point {\
             [constructor] pub fn from_xy(x: f32, y: f32) -> Self { return Self { x: x, y: y }; }\
         }\
         impl Point {\
             [constructor] pub fn origin() -> Self { return Self { x: 0.0, y: 0.0 }; }\
         }\
         entry fn main() { }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`from_xy` and `origin` both designate the construction surface")),
        "the second impl block's member names the first: {ds:?}"
    );
}

#[test]
fn a_receiver_is_refused() {
    // Law: class method only, NO self — the call spells the class, not
    // a value
    let ds = diags_of(
        "class Point { x: f32; y: f32; }\
         impl Point {\
             [constructor] pub fn from_self(self) -> Self { return Self { x: 0.0, y: 0.0 }; }\
         }\
         entry fn main() { }\n",
    );
    assert!(
        ds.iter().any(|d| d == "a `[constructor]` member takes no receiver — `fn <free>(..) -> Self` (the call `Type(..)` spells the class, not a value)"),
        "the receiver refusal names the shape: {ds:?}"
    );
}

#[test]
fn the_return_must_be_self_shaped() {
    let ds = diags_of(
        "class Point { x: f32; y: f32; }\
         impl Point {\
             [constructor] pub fn from_xy(x: f32, y: f32) -> f32 { return x; }\
         }\
         entry fn main() { }\n",
    );
    assert!(
        ds.iter().any(|d| d == "a `[constructor]` member returns `Self` (or `?Self` — the try-construction: `Type(..)` then yields `?Type`)"),
        "the return refusal names the law: {ds:?}"
    );
}

// ---- the target gates ---------------------------------------------------

#[test]
fn a_struct_impl_marker_is_refused() {
    // structs construct by literal
    let ds = diags_of(
        "struct P { x: i32; }\
         impl P {\
             [constructor] pub fn new(x: i32) -> Self { return Self { x: x }; }\
         }\
         entry fn main() { }\n",
    );
    assert!(
        ds.iter().any(|d| d == "`P` cannot carry `[constructor]` — structs construct by literal (`P { .. }`); the marker designates a class's call form"),
        "the struct refusal names the literal: {ds:?}"
    );
    // and the struct keeps its literal — no call form ever existed
    let ds = diags_of(
        "struct P { x: i32; }\
         entry fn main() { let p = P(1); }\n",
    );
    assert!(
        ds.iter().any(|d| d == "construction is a method call, never a type-call — use a struct literal `P { .. }`"),
        "the struct miss keeps the literal hint: {ds:?}"
    );
}

#[test]
fn an_enum_impl_marker_is_refused() {
    let ds = diags_of(
        "enum Color { Red, Green }\
         impl Color {\
             [constructor] pub fn make() -> Color { return Color.Red; }\
         }\
         entry fn main() { }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`Color` cannot carry `[constructor]` — only a class can")),
        "the enum refusal: {ds:?}"
    );
}

#[test]
fn marker_on_a_newtype_is_refused() {
    // the newtype's `Name(v)` surface already IS the construction —
    // nothing to designate
    let ds = diags_of(
        "class JsonI64(i64);\
         impl JsonI64 {\
             [constructor] pub fn make(v: i64) -> Self { return JsonI64(v); }\
         }\
         entry fn main() { }\n",
    );
    assert!(
        ds.iter().any(|d| d == "`JsonI64` cannot carry `[constructor]` — the newtype's call surface (`JsonI64(v)`) already IS the construction; there is nothing to designate"),
        "the newtype refusal: {ds:?}"
    );
}

#[test]
fn the_newtype_mint_keeps_precedence_beside_a_ctor() {
    // `JsonI64(64)` still mints while an unrelated class carries a
    // constructor — the two call surfaces never collide
    let v = run_returns(
        "use core::{ panic };\
         class Point { x: i32; y: i32; }\
         impl Point {\
             [constructor] pub fn from_xy(x: i32, y: i32) -> Self { return Self { x: x, y: y }; }\
         }\
         class JsonI64(i64);\
         entry fn main() -> i32 {\
             let p = Point(1, 2);\
             if (p.x != 1) { return 1; }\
             let j = JsonI64(64);\
             if (j.inner != 64) { return 2; }\
             return 0;\
         }\n",
    )
    .expect("runs");
    assert_eq!(v, 0);
}

// ---- the closed set -----------------------------------------------------

#[test]
fn unknown_words_name_the_three_designated_surfaces() {
    let ds = diags_of(
        "class P { x: i32; }\
         impl P {\
             [fragile] pub fn m(self) -> i32 { return self.x; }\
         }\
         entry fn main() { }\n",
    );
    assert!(
        ds.iter().any(|d| d == "`[fragile]` is not a designated surface — the closed marker set is `[disposal]`, `[iterable]`, and `[constructor]`"),
        "the closed-set refusal names all three: {ds:?}"
    );
}

#[test]
fn markers_outside_inherent_bodies_stay_parse_rejects() {
    // Law 4's pin: parse_member_marker only runs in the inherent arm —
    // an interface member's marker dies at the parser
    let (_, diags) = rut_parser::parse(
        "interface I { [constructor] fn make() -> i32; }",
        Mode::Impl,
    );
    assert!(
        diags.iter().any(|d| d.msg.contains("marks an inherent impl member")),
        "an interface-body marker must diagnose: {diags:?}"
    );
    // a free fn's marker never parses either (the top level takes no
    // bracket groups)
    let (_, diags) = rut_parser::parse(
        "[constructor] fn make() -> i32 { return 0; }",
        Mode::Impl,
    );
    assert!(
        !diags.is_empty(),
        "a free-fn marker must diagnose: {diags:?}"
    );
    // the trait-impl spelling is gone entirely — the marker never
    // reaches a `for` body
    let (_, diags) = rut_parser::parse(
        "class C { }\
         interface I { fn m(self); }\
         impl I for C { [constructor] fn m(self) { } }",
        Mode::Impl,
    );
    assert!(
        diags.iter().any(|d| d.msg.contains("expected {, found `for`")),
        "the trait-impl spelling dies at the parser: {diags:?}"
    );
}

// ---- resolution order ---------------------------------------------------

#[test]
fn a_free_fn_named_like_the_class_wins() {
    // Law 6: the ctor fallback is LAST — a local fn-typed/free fn
    // named `Point` answers the call
    let v = run_returns(&format!(
        "{POINT}\n\
         fn Point(tag: i32) -> i32 {{ return tag; }}\n\
         entry fn main() -> i32 {{\n\
             let v = Point(7);\n\
             if (v != 7) {{ return 1; }}\n\
             return 0;\n\
         }}\n"
    ))
    .expect("runs");
    assert_eq!(v, 0);
}

#[test]
fn the_no_ctor_miss_names_the_fix() {
    let ds = diags_of(
        "class Point { x: i32; y: i32; }\
         entry fn main() { let p = Point(1, 2); }\n",
    );
    assert!(
        ds.iter().any(|d| d == "`Point` constructs through its class methods (`Point.new(..)`) — mark one `[constructor]` to call the class itself"),
        "the miss names the fix: {ds:?}"
    );
}

// ---- the generic path ---------------------------------------------------

#[test]
fn generic_class_ctor_rides_the_unification_path() {
    // `Box(3)` lowers exactly like `Box.of(3)` — the type arguments
    // infer from the expected type's instantiation (or refuse to,
    // identically)
    let v = run_returns(
        "use core::{ panic };\
         class Box<T> { v: T; }\
         impl<T> Box<T> {\
             [constructor] pub fn of(v: T) -> Self { return Self { v: v }; }\
             pub fn get(self) -> T { return self.v; }\
         }\
         entry fn main() -> i32 {\
             let b: Box<i32> = Box(3);\
             if (b.get() != 3) { return 1; }\
             let b2 = Box<i32>.of(4);\
             if (b2.get() != 4) { return 2; }\
             return 0;\
         }\n",
    )
    .expect("runs");
    assert_eq!(v, 0);
    // and the inference miss is the member call's own — identical text
    let ds = diags_of(
        "class Box<T> { v: T; }\
         impl<T> Box<T> {\
             [constructor] pub fn of(v: T) -> Self { return Self { v: v }; }\
         }\
         entry fn main() { let b = Box(3); }\n",
    );
    assert!(
        ds.iter().any(|d| d == "cannot infer the type arguments for `Box` — write `Box<..>.of(..)` or annotate the binding"),
        "the sugar rides the member call's inference: {ds:?}"
    );
}

// ---- cross-module -------------------------------------------------------

const POINT_PKG: &str = "\
pub class Point {
    x: f32;
    y: f32;
}
impl Point {
    [constructor] pub fn from_xy(x: f32, y: f32) -> Self {
        return Self { x: x, y: y };
    }
    pub fn manhattan(self) -> f32 { return self.x + self.y; }
}
";

#[test]
fn a_used_class_call_form_mints_at_a_distance() {
    // the marker byte crosses the surface row; the consumer's
    // `Point(..)` resolves through the registry and lowers like the
    // member call
    let v = run_graph(&graph(&[
        ("geometry", POINT_PKG),
        (
            "app",
            "use geometry::{Point};\
             entry fn main() -> i32 {\
                 let p = Point(1.0, 2.0);\
                 let q = Point.from_xy(1.0, 2.0);\
                 if (p.manhattan() != q.manhattan()) { return 1; }\
                 if (p.manhattan() != 3.0) { return 2; }\
                 return 0;\
             }\n",
        ),
    ]))
    .expect("runs");
    assert_eq!(v, 0);
}

#[test]
fn a_private_ctor_does_not_cross() {
    // the seal rides the member's `pub`: a private ctor is invisible
    // across the surface, the consumer's miss names the fix
    let g = graph(&[
        ("geometry",
         "pub class Point { x: f32; y: f32; }\
          impl Point {\
              [constructor] fn from_xy(x: f32, y: f32) -> Self { return Self { x: x, y: y }; }\
          }\n"),
        ("app",
         "use geometry::{Point};\
          entry fn main() -> i32 { let p = Point(1.0, 2.0); return 0; }\n"),
    ]);
    let ds = g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>();
    assert!(
        ds.iter().any(|d| d == "`Point` constructs through its class methods (`Point.new(..)`) — mark one `[constructor]` to call the class itself"),
        "the private ctor never crosses: {ds:?}"
    );
    assert!(g.program.is_none(), "the consumer must refuse");
}
