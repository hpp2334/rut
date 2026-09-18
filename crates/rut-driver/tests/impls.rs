//! Impl blocks and nominal trait semantics (RFC 0012): the two impl
//! forms, coverage/placement checks, the use-both gate, and the two
//! dispatch rules — static bind when the call site names exactly one
//! concrete receiver type, vtable when origins merge.

use rut_parser::Mode;
use rut_core::types::TY_I32;

fn compile(src: &str) -> rut_driver::ProgramOutput {
    // core + pouch bound as the uses (RFC 0028); these
    // tests exercise impl semantics, not use discipline
    let collection = rut_core::binary::Surface::default();
    rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, rut_core::binary::Surface::core()), (3, collection)],
    )
}

fn diags_of(src: &str) -> Vec<String> {
    compile(src)
        .diags
        .iter()
        .map(|d| d.msg.clone())
        .collect()
}

#[test]
fn trait_impl_coverage_is_checked() {
    // a missing trait method diagnoses
    let ds = diags_of(
        "trait Shape { fn area(self) -> f32; }\n\
         struct Circle { r: f32 }\n\
         impl Shape for Circle { }\n\
         fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("impl is missing `area`")),
        "missing method must diagnose: {ds:?}"
    );
    // so does an extra one — inherent methods go in `impl Circle`
    let ds = diags_of(
        "trait Shape { fn area(self) -> f32; }\n\
         struct Circle { r: f32 }\n\
         impl Shape for Circle {\n\
             fn area(self) -> f32 { return self.r; }\n\
             fn extra(self) -> i32 { return 1; }\n\
         }\n\
         fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`extra` is not a member of Shape")),
        "extra method must diagnose: {ds:?}"
    );
    // and a signature that does not match the trait's
    let ds = diags_of(
        "trait Shape { fn area(self) -> f32; }\n\
         struct Circle { r: f32 }\n\
         impl Shape for Circle {\n\
             fn area(self) -> i32 { return 4; }\n\
         }\n\
         fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("does not match the trait's signature")),
        "signature mismatch must diagnose: {ds:?}"
    );
}

#[test]
fn receiver_form_must_match_the_trait() {
    let ds = diags_of(
        "trait T { fn m(mut self) -> nil; }\n\
         struct S { x: i32 }\n\
         impl T for S { fn m(self) -> nil { } }\n\
         fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("must match the trait's receiver")),
        "receiver form mismatch must diagnose: {ds:?}"
    );
}

#[test]
fn duplicate_trait_type_pair_is_an_error() {
    let ds = diags_of(
        "trait I { fn m(self) -> i32; }\n\
         struct S { x: i32 }\n\
         impl I for S { fn m(self) -> i32 { return 1; } }\n\
         impl I for S { fn m(self) -> i32 { return 2; } }\n\
         fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("duplicate impl for the same (trait, type) pair")),
        "duplicate pair must diagnose: {ds:?}"
    );
}

#[test]
fn satisfaction_is_nominal_not_structural() {
    // S has the method `m` and NO impl — it is not an I, so the widening
    // errors and the probe folds false (RFC 0012 §4)
    let ds = diags_of(
        "trait I { fn m(self) -> i32; }\n\
         struct S { x: i32 }\n\
         impl S { fn m(self) -> i32 { return self.x; } }\n\
         fn main() -> i32 {\n\
             let s = S { x: 1 };\n\
             let i: I = s;\n\
             return i.m();\n\
         }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("the initializer is")),
        "structural satisfaction must not widen: {ds:?}"
    );
    let out = compile(
        "trait I { fn m(self) -> i32; }\n\
         struct S { x: i32 }\n\
         impl S { fn m(self) -> i32 { return self.x; } }\n\
         fn main() -> i32 {\n\
             let s = S { x: 1 };\n\
             if (s is I) { return 1; }\n\
             return 0;\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
}

#[test]
fn marker_impls_are_legal() {
    // an empty body on a method-less trait (RFC 0037 markers)
    let out = compile(
        "trait Mark { }\n\
         struct S { x: i32 }\n\
         impl Mark for S { }\n\
         fn main() -> i32 { return 0; }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
}

#[test]
fn static_dispatch_when_the_origin_is_single() {
    // `w` is trait-typed with ONE concrete origin — the call binds
    // statically to the impl method (no vtable hop, RFC 0012 §5)
    let out = compile(
        "trait Get { fn get(self) -> i32; }\n\
         struct B { v: i32 }\n\
         impl B { fn new(v: i32) -> Self { return Self { v: v }; } }\n\
         impl Get for B { fn get(self) -> i32 { return self.v; } }\n\
         fn main() -> i32 {\n\
             let b = B.new(7);\n\
             let w: Get = b;\n\
             return w.get();\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let p = out.program.expect("program");
    let ir = rut_driver::ir_dump_of(&p.funcs, &p.interner);
    assert!(!ir.contains("calli"), "single origin must bind statically:\n{ir}");
}

#[test]
fn vtable_dispatch_when_origins_merge() {
    // branch-merged origins: the compiler cannot name one concrete
    // receiver, so the call consults the descriptor (CallI, RFC 0012 §5)
    let out = compile(
        "trait Get { fn get(self) -> i32; }\n\
         struct A { v: i32 }\n\
         struct B { v: i32 }\n\
         impl Get for A { fn get(self) -> i32 { return self.v; } }\n\
         impl Get for B { fn get(self) -> i32 { return self.v; } }\n\
         fn pick(k: bool) -> Get {\n\
             if (k) { return A { v: 1 }; }\n\
             return B { v: 2 };\n\
         }\n\
         fn main() -> i32 {\n\
             let g: Get = pick(true);\n\
             return g.get();\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let p = out.program.expect("program");
    let ir = rut_driver::ir_dump_of(&p.funcs, &p.interner);
    assert!(ir.contains("calli"), "merged origins must go through the vtable:\n{ir}");
}

#[test]
fn for_of_without_an_impl_names_the_missing_contract() {
    let ds = diags_of(
        "class Count { n: i32 }\n\
         impl Count { fn new() -> Self { return Self { n: 0 }; } }\n\
         fn main() -> i32 {\n\
             let c = Count.new();\n\
             for (let v of c) { }\n\
             return 0;\n\
         }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("impl Iterator<E> for Count")),
        "missing iterable impl must diagnose: {ds:?}"
    );
}

#[test]
fn impl_target_must_be_a_local_type_or_owned_builtin() {
    let ds = diags_of(
        "trait I { fn m(self) -> i32; }\n\
         impl I for Missing { fn m(self) -> i32 { return 1; } }\n\
         fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("impl target must be a struct or class of this module")),
        "foreign target must diagnose: {ds:?}"
    );
}

#[test]
fn self_spells_the_impl_target() {
    // `-> Self` inside an impl block resolves to the target type
    let out = compile(
        "struct P { x: i32 }\n\
         impl P {\n\
             fn new(x: i32) -> Self { return Self { x: x }; }\n\
             fn bump(self) -> Self { return P { x: self.x + 1 }; }\n\
         }\n\
         fn main() -> i32 {\n\
             let p = P.new(1);\n\
             let q = p.bump();\n\
             return q.x;\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
}

#[test]
fn reassignment_invalidates_a_stale_origin() {
    // `w` starts as A (single origin → static bind) but is re-assigned to
    // B. The compiler must re-derive the origin at the assignment: the
    // later call must dispatch as B — statically re-bound to B's impl
    // (still a single known origin) or through the vtable — never run
    // A's method on a B (RFC 0012 §5: a mis-analysis must not be able
    // to produce a wrong call). Runtime-checked in rut-cli's e2e
    // (`reassigned_trait_binding_dispatches_as_the_new_type`).
    let out = compile(
        "trait Get { fn get(self) -> i32; }\n\
         struct A { v: i32 }\n\
         struct B { v: i32 }\n\
         impl Get for A { fn get(self) -> i32 { return 10; } }\n\
         impl Get for B { fn get(self) -> i32 { return 20; } }\n\
         fn main() -> i32 {\n\
             let mut w: Get = A { v: 1 };\n\
             w = B { v: 2 };\n\
             return w.get();\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    // whatever call form the origin analysis chose, A's method body must
    // not be the statically bound callee (its id would be `callm f0`,
    // the first compiled fn here)
    let p = out.program.expect("program");
    let ir = rut_driver::ir_dump_of(&p.funcs, &p.interner);
    assert!(
        !ir.contains("callm f0"),
        "A's impl must not be the static callee after reassignment:\n{ir}"
    );
}

#[test]
fn builtin_class_inherent_impls_compile() {
    // a module-owned `builtin class` takes an inherent impl (the
    // `LaunchedTask<T>` pattern, RFC 0012 §2): generic through the
    // native-type table, dispatched statically through the shape
    let out = compile(
        "use core::{ Array };\n\
         impl Array<T> {\n\
             fn first(self) -> i32 { return 7; }\n\
         }\n\
         fn main() -> i32 {\n\
             let a = Array<i32>(2);\n\
             return a.first();\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    // and the concrete builtin class
    let out = compile(
        "use core::{ Opaque };\n\
         impl Opaque {\n\
             fn peek(self) -> i32 { return 1; }\n\
         }\n\
         fn main() -> i32 {\n\
             let o = Opaque.new(5);\n\
             return o.peek();\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
}

// ---- primitive trait-impl targets (RFC 0012 §2; the inherent surface
// stays core's `builtin impl`, RFC 0032 §1.1) ----

/// Compile, flatten (RFC 0035 §1), verify, and run a single-module
/// `main` returning i32.
fn run_main(src: &str) -> i32 {
    let out = compile(src);
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let prog = rut_core::link::flatten(out.program.expect("program"));
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(prog),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    vm.call::<_, i32>("main", ()).expect("run")
}

#[test]
fn prim_trait_impl_compiles_and_dispatches_statically() {
    // `impl T for i32` compiles; `let w: T = 5` widens nominally once
    // the impl registers, and the single-origin call binds statically —
    // the scalar receiver crosses as an ordinary argument
    let src = "trait T { fn m(self) -> i32; }\n\
               impl T for i32 { fn m(self) -> i32 { return self + 100; } }\n\
               pub fn main() -> i32 {\n\
                   let w: T = 5;\n\
                   return w.m();\n\
               }\n";
    assert_eq!(run_main(src), 105);
    let out = compile(src);
    let p = out.program.expect("program");
    let ir = rut_driver::ir_dump_of(&p.funcs, &p.interner);
    assert!(!ir.contains("calli"), "single origin must bind statically:\n{ir}");
}

#[test]
fn prim_is_probe_answers_the_registered_impl() {
    // `is` folds at the exact receiver (RFC 0012 §4 nominal): true where
    // the impl is registered, false where it is not
    let src = "trait T { fn m(self) -> i32; }\n\
               trait U { fn n(self) -> i32; }\n\
               impl T for i32 { fn m(self) -> i32 { return 1; } }\n\
               pub fn main() -> i32 {\n\
                   let mut acc = 0;\n\
                   if (5 is T) { acc = acc + 1; }\n\
                   if (5 is U) { acc = acc + 10; }\n\
                   return acc;\n\
               }\n";
    assert_eq!(run_main(src), 1);
}

#[test]
fn merged_origins_prim_and_struct_emit_calli() {
    // i32 and a struct both implement T: the merged-origin call consults
    // the vtable (IR-level — the prim fills the same slot as the record)
    let out = compile(
        "trait T { fn m(self) -> i32; }\n\
         struct S { v: i32 }\n\
         impl T for i32 { fn m(self) -> i32 { return self + 1; } }\n\
         impl T for S { fn m(self) -> i32 { return self.v; } }\n\
         pub fn pick(k: bool) -> T {\n\
             if (k) { return 5; }\n\
             return S { v: 1 };\n\
         }\n\
         fn main() -> i32 {\n\
             let w: T = pick(true);\n\
             let v = w.m();\n\
             return 0;\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let p = out.program.expect("program");
    let ir = rut_driver::ir_dump_of(&p.funcs, &p.interner);
    assert!(ir.contains("calli"), "merged origins must go through the vtable:\n{ir}");
}

#[test]
fn inherent_impl_on_a_primitive_diagnoses() {
    let ds = diags_of(
        "impl i32 { fn f(self) -> i32 { return self; } }\n\
         fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("a primitive takes trait impls only")
            && d.contains("`builtin impl`")),
        "inherent impl on a prim must diagnose: {ds:?}"
    );
}

#[test]
fn prim_target_admission_is_module_agnostic() {
    // `impl ForeignTrait for i32` in a module that declares neither —
    // RFC 0012 §2's admission pattern covers primitives too
    let dep = rut_driver::compile_program(
        "trait T { fn m(self) -> i32; }\n\
         impl T for str { fn m(self) -> i32 { return 7; } }\n",
        Mode::Impl,
        "dep",
        1,
        &[],
    );
    assert!(dep.diags.is_empty(), "{:?}", dep.diags);
    let dep = dep.program.expect("dep program");
    let surface = dep.surface.clone();

    let app = rut_driver::compile_program(
        "use dep::{T};\n\
         impl T for i32 { fn m(self) -> i32 { return self + 100; } }\n\
         pub fn main() -> i32 { return 0; }\n",
        Mode::Impl,
        "app",
        2,
        &[(1, surface)],
    );
    assert!(app.diags.is_empty(), "{:?}", app.diags);
}

#[test]
fn cross_module_duplicate_prim_pair_is_a_link_error() {
    let dep = rut_driver::compile_program(
        "trait T { fn m(self) -> i32; }\n\
         impl T for i32 { fn m(self) -> i32 { return 1; } }\n",
        Mode::Impl,
        "dep",
        1,
        &[],
    );
    assert!(dep.diags.is_empty(), "{:?}", dep.diags);
    let dep = dep.program.expect("dep program");

    let app = rut_driver::compile_program(
        "use dep::{T};\n\
         impl T for i32 { fn m(self) -> i32 { return 2; } }\n\
         fn main() -> i32 { return 0; }\n",
        Mode::Impl,
        "app",
        2,
        &[(1, dep.surface.clone())],
    );
    assert!(app.diags.is_empty(), "admission is per-module; the pair collides at link: {:?}", app.diags);
    let err = rut_core::link::link(vec![dep, app.program.expect("app program")])
        .expect_err("duplicate (T, i32) must refuse to link");
    assert!(
        err.0.contains("duplicate impl") && err.0.contains("(T, i32)"),
        "{err}"
    );
}

#[test]
fn prim_vtable_fill_survives_the_link() {
    // regression (link boot-row merge): the dep module's fill of the
    // GLOBAL i32 row must not be dropped when its module merges —
    // pre-fix, rows below the boot prefix hit `continue` and the fill
    // was silently lost. The cross-module call then runs end-to-end
    // (the app's single-origin call binds statically to the dep's
    // compiled method through the extern registration).
    let dep = rut_driver::compile_program(
        "trait T { fn m(self) -> i32; }\n\
         impl T for i32 { fn m(self) -> i32 { return self + 100; } }\n",
        Mode::Impl,
        "dep",
        1,
        &[],
    );
    assert!(dep.diags.is_empty(), "{:?}", dep.diags);
    let dep = dep.program.expect("dep program");
    let surface = dep.surface.clone();

    let app = rut_driver::compile_program(
        "use dep::{T};\n\
         pub fn main() -> i32 {\n\
             let mut w: T = 5;\n\
             return w.m();\n\
         }\n",
        Mode::Impl,
        "app",
        2,
        &[(1, surface)],
    );
    assert!(app.diags.is_empty(), "{:?}", app.diags);
    let linked = rut_core::link::link(vec![dep, app.program.expect("app program")]).expect("link");

    // the fill survived: the global i32 row carries the dep's impl method
    let gid = linked
        .traits
        .iter()
        .position(|t| linked.name_of(t.name) == "T")
        .expect("one global T") as u32;
    let slot = linked.slot_of(gid, 0).expect("global slot");
    let fill = linked.vtables[TY_I32 as usize][slot as usize];
    assert!(fill.is_some(), "i32's vtable row must carry the dep's impl: {:?}", linked.vtables[TY_I32 as usize]);
    let fname = linked.name_of(linked.funcs[fill.unwrap() as usize].name);
    assert!(fname.contains("m"), "the fill names the impl method: {fname}");

    // and the cross-module call runs end-to-end: the receiver crosses
    // as a scalar argument into the dep's compiled method
    rut_vm::verify::verify(&linked).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(linked),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    assert_eq!(vm.call::<_, i32>("main", ()).expect("run"), 105);
}
