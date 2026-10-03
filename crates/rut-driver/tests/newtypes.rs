//! Newtype classes — `class Name(Wrapped);` — end to end: the call
//! construction manufactures the wrapper (a real cell), the wrapper is
//! nameable and first-class, the boundary crossing is the spelled
//! constructor (no auto-insertion, forever), the generic instantiation
//! law (binder from the wrapped argument, or explicit args —
//! all-or-nothing), and the third-party adapter crosses modules on the
//! surface row's flag.

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

/// [`compiled`] with calc offered (the old `mount_std` shape: core
/// auto-rides, `calc` is an ordinary pkg).
#[allow(dead_code)]
fn compiled_std(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(
        rut_driver::RutRun::new()
            .pkg(rut_driver::Pkg::source(spec, src))
            .pkg(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"))
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

fn compile(src: &str) -> rut_driver::ProgramOutput {
    // core bound as the one use: these tests exercise construction
    // and widening, not use discipline
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
    let limits = Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = Vm::builder().program(std::rc::Rc::new(prog)).limits(limits).hooks(rut_vm::interp::HostHooks::default()).hosts(HostRegistry::new()).build()
    .map_err(|e| format!("{e:?}"))?;
    vm.call::<_, i32>("main", ()).map(|v| v as i64).map_err(|e| format!("{e:?}"))
}

const WRAPPER: &str = "\
interface Serializable { fn encode(self) -> str; }
class JsonI64(i64);
class JsonF64(f64);
impl JsonI64 {
    pub fn get(self) -> i64 { return self.inner; }
    pub fn encode(self) -> str { return f\"{self.inner}\"; }
}
impl JsonF64 {
    pub fn encode(self) -> str { return f\"{self.inner}\"; }
}
fn dump(x: Serializable) -> str { return x.encode(); }
";

#[test]
fn construction_mints_a_usable_wrapper() {
    // the spelled constructor builds the wrapper; its members answer
    let v = run_returns(&format!(
        "{WRAPPER}\n\
         entry fn main() -> i32 {{\n\
             let x = JsonI64(64);\n\
             return x.get() as i32;\n\
         }}\n"
    ))
    .expect("runs");
    assert_eq!(v, 64);
}

#[test]
fn wrapper_crosses_the_boundary_and_plain_value_does_not() {
    // THE law: capability manufactured by a spelled constructor — a
    // fact about the nominal class. The bare value never widens.
    let ds = diags_of(&format!(
        "{WRAPPER}\n\
         entry fn main() {{\n\
             let s = dump(JsonI64(64));\n\
         }}\n"
    ));
    assert!(ds.is_empty(), "the wrapper satisfies at the spelled constructor: {ds:?}");
    let ds = diags_of(&format!(
        "{WRAPPER}\n\
         entry fn main() {{\n\
             let s = dump(64);\n\
         }}\n"
    ));
    assert!(
        ds.iter().any(|d| d.contains("i32") && d.contains("Serializable")),
        "no auto-insertion, forever: {ds:?}"
    );
}

#[test]
fn wrapper_is_first_class() {
    // fields, lets, `is` — the wrapper is an ordinary class value
    let v = run_returns(&format!(
        "{WRAPPER}\n\
         class Holder {{ w: JsonI64; }}\n\
         entry fn main() -> i32 {{\n\
             let h = Holder {{ w: JsonI64(7) }};\n\
             let picked: JsonI64 = h.w;\n\
             if (picked is JsonI64) {{ return picked.get() as i32; }}\n\
             return 0;\n\
         }}\n"
    ))
    .expect("runs");
    assert_eq!(v, 7);
}

#[test]
fn generic_newtype_infers_from_the_wrapped_argument() {
    // the binder IN the wrapped arg → inferred (`Tail<T>([T])`)
    let v = run_returns(
        "class Tail<T>([T]);\n\
         entry fn main() -> i32 {\n\
             let t = Tail([3, 1, 2]);\n\
             return t.inner.len();\n\
         }\n",
    )
    .expect("runs");
    assert_eq!(v, 3);
    // the blanket wrapper claims nothing about T — the arg names it
    let v = run_returns(
        "class DebugWrap<T>(T);\n\
         entry fn main() -> i32 {\n\
             let d = DebugWrap(9);\n\
             return d.inner;\n\
         }\n",
    )
    .expect("runs");
    assert_eq!(v, 9);
}

#[test]
fn binder_not_in_the_wrapped_arg_demands_the_spelling() {
    // `Converter<T>(str)`: the wrapped `str` cannot name T — the
    // constructor demands it, naming the wrapped type
    let ds = diags_of(
        "class Converter<T>(str);\n\
         entry fn main() {\n\
             let c = Converter(\"64\");\n\
         }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`Converter`'s parameter `T` is not determined by `str`")
            && d.contains("spell it: `Converter<")),
        "the undetermined binder names the fix: {ds:?}"
    );
    // ... and the explicit form compiles and runs
    let v = run_returns(
        "class Converter<T>(str);\n\
         entry fn main() -> i32 {\n\
             let c = Converter<i32>(\"64\");\n\
             return c.inner.len();\n\
         }\n",
    )
    .expect("runs");
    assert_eq!(v, 2);
}

#[test]
fn explicit_args_are_all_or_nothing() {
    // two binders, one spelled — the call spells ALL binders or none
    let ds = diags_of(
        "class Pair<A, B>(i32);\n\
         entry fn main() {\n\
             let p = Pair<i32>(7);\n\
         }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("takes 2 type argument(s), 1 given")),
        "partial spelling diagnoses: {ds:?}"
    );
    // the full spelling compiles
    let out = compile(
        "class Pair<A, B>(i32);\n\
         entry fn main() {\n\
             let p = Pair<i32, str>(7);\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
}

#[test]
fn wrapped_value_checks_against_the_instantiated_field() {
    let ds = diags_of(
        "class Tail<T>([T]);\n\
         entry fn main() {\n\
             let t = Tail<i32>([\"no\"]);\n\
         }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("the wrapped value is")),
        "an arg that contradicts the explicit binders diagnoses: {ds:?}"
    );
    let ds = diags_of(
        "class DebugWrap<T>(T);\n\
         entry fn main() {\n\
             let d = DebugWrap(1.5);\n\
             let e: DebugWrap<i32> = d;\n\
         }\n",
    );
    assert!(!ds.is_empty(), "the inferred instance is the arg's: {ds:?}");
}

#[test]
fn braced_one_field_class_has_no_constructor() {
    // the seal holds: the positional spelling is what arms the call
    let ds = diags_of(
        "class Plain { inner: i32; }\n\
         entry fn main() {\n\
             let p = Plain(7);\n\
         }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("construction is a method call")),
        "a braced class keeps the old diagnostic: {ds:?}"
    );
}

#[test]
fn wrapper_family_with_per_type_bodies() {
    // the naming law: wrappers named for what they wrap, one per
    // wrapped type, each with its own body — the JsonI64/JsonF64
    // family from the plan
    let v = run_returns(&format!(
        "{WRAPPER}\n\
         entry fn main() -> i32 {{\n\
             let a = dump(JsonI64(-5));\n\
             let b = dump(JsonF64(2.5));\n\
             return (a.len() + b.len()) as i32;\n\
         }}\n"
    ))
    .expect("runs");
    assert_eq!(v, 5);
}

// ---- the third-party adapter: the orphan case, dissolved ------------

const THEIRS: &str = "\
pub class TheirVal { n: i32 = 0; }
impl TheirVal {
    pub fn new(n: i32) -> Self { return Self { n: n }; }
}
pub class TheirJson(TheirVal);
impl TheirJson {
    pub fn n(self) -> i32 { return self.inner.n; }
}
pub class Wrap<T>(T);
impl<T> Wrap<T> {
    pub fn peek(self) -> ?T { return self.inner; }
}
pub class Blanket<T>(str);
";

fn theirs_surface() -> rut_core::binary::Surface {
    let dep = rut_driver::compile_program(THEIRS, Mode::Impl, "theirjson", 1, &[]);
    assert!(dep.diags.is_empty(), "{:?}", dep.diags);
    dep.program.expect("dep program").surface
}

#[test]
fn adapter_wrapper_constructs_across_modules() {
    // the adapter's package declares the wrapper; the CONSUMER
    // manufactures it — the surface row's flag is the crossing paper
    let surface = theirs_surface();
    let row = surface
        .type_exports
        .iter()
        .find(|t| surface.names.name(t.name) == "TheirJson")
        .expect("TheirJson exported");
    assert!(row.newtype, "the flag crosses on the surface row");
    assert!(row.is_class, "a newtype is a class");
    assert!(
        !surface
            .type_exports
            .iter()
            .find(|t| surface.names.name(t.name) == "TheirVal")
            .expect("TheirVal exported")
            .newtype,
        "a braced class carries no flag"
    );

    let root = rut_driver::compile_program(
        "use theirjson::{ TheirJson, TheirVal };\n\
         entry fn main() -> i32 {\n\
             let w = TheirJson(TheirVal.new(7));\n\
             return w.n();\n\
         }\n",
        Mode::Impl,
        "app",
        2,
        &[(1, surface, "theirjson".to_string()), (3, rut_core::binary::Surface::core(), "core".to_string())],
    );
    assert!(root.diags.is_empty(), "cross-module construction: {:?}", root.diags);
}

#[test]
fn generic_adapter_wrapper_spells_across_modules() {
    // `Wrap(5)` at a distance: the template's placeholder field
    // unifies against the wrapped argument; `Wrap<i32>(5)` spells it
    let surface = theirs_surface();
    for src in [
        "use theirjson::{ Wrap };\n\
         entry fn main() {\n\
             let w = Wrap(5);\n\
         }\n",
        "use theirjson::{ Wrap };\n\
         entry fn main() {\n\
             let w = Wrap<i32>(5);\n\
         }\n",
    ] {
        let root = rut_driver::compile_program(
            src,
            Mode::Impl,
            "app",
            2,
            &[(1, surface.clone(), "theirjson".to_string()), (3, rut_core::binary::Surface::core(), "core".to_string())],
        );
        assert!(root.diags.is_empty(), "{src}: {:?}", root.diags);
    }
}

#[test]
fn undetermined_binder_at_a_distance_names_the_spelling() {
    // `Blanket<T>(str)` — the wrapped `str` names no binder, there or here
    let surface = theirs_surface();
    let root = rut_driver::compile_program(
        "use theirjson::{ Blanket };\n\
         entry fn main() {\n\
             let w = Blanket(\"x\");\n\
         }\n",
        Mode::Impl,
        "app",
        2,
        &[(1, surface, "theirjson".to_string()), (3, rut_core::binary::Surface::core(), "core".to_string())],
    );
    assert!(
        root.diags.iter().any(|d| d.msg.contains("not determined")),
        "{:?}",
        root.diags
    );
}
