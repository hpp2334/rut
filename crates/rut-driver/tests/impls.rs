//! Inherent impl blocks and structural interface satisfaction: the
//! impl forms (struct/class/enum targets, coverage/placement checks),
//! the boundary satisfaction check (missing member, signature differs,
//! receiver form), and the two dispatch rules — static bind when the
//! call site names exactly one concrete origin, vtable when origins
//! merge. Primitives carry no members: capability on a value type is
//! manufactured by a spelled wrapper (a newtype class) whose inherent
//! impl carries the members.

use rut_parser::Mode;


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
    // core + pouch bound as the uses; these
    // tests exercise impl semantics, not use discipline
    let collection = rut_core::binary::Surface::default();
    rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, rut_core::binary::Surface::core(), "core".to_string()), (3, collection, "collection".to_string())],
    )
}

fn diags_of(src: &str) -> Vec<String> {
    compile(src)
        .diags
        .iter()
        .map(|d| d.msg.clone())
        .collect()
}

// The old nominal laws — `impl I for T` coverage/extra-member checks,
// the duplicate (trait, type) pair error, and the nominal (not
// structural) satisfaction — died with the trait registry. Their
// nearest surviving laws live at the BOUNDARY (the member-set match:
// `satisfaction_fails_names_the_missing_member` /
// `receiver_form_must_match_the_interface` below and in
// cross_traits.rs) and in the duplicate-inherent-member check
// (`duplicate_inherent_member_diagnoses`).

#[test]
fn duplicate_inherent_member_diagnoses() {
    // two inherent blocks spelling the same member: the second is a
    // duplicate-method error (the registry's old duplicate-pair law,
    // restated for the members that replaced it)
    let ds = diags_of(
        "struct S { x: i32 }\n\
         impl S { pub fn m(self) -> i32 { return 1; } }\n\
         impl S { pub fn m(self) -> i32 { return 2; } }\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("duplicate method `m` on `S`")),
        "duplicate member must diagnose: {ds:?}"
    );
}

#[test]
fn satisfaction_is_structural() {
    // S HAS the member `m` on its inherent impl — it satisfies I at the
    // boundary (the fork's inversion of the old nominal law): the
    // widening compiles, dispatches, and the probe folds true
    let out = compile(
        "interface I { fn m(self) -> i32; }\n\
         struct S { x: i32 }\n\
         impl S { pub fn m(self) -> i32 { return self.x; } }\n\
         entry fn main() -> i32 {\n\
             let s = S { x: 1 };\n\
             let i: I = s;\n\
             return i.m();\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let out = compile(
        "interface I { fn m(self) -> i32; }\n\
         struct S { x: i32 }\n\
         impl S { pub fn m(self) -> i32 { return self.x; } }\n\
         entry fn main() -> i32 {\n\
             let s = S { x: 1 };\n\
             if (s is I) { return 1; }\n\
             return 0;\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
}

#[test]
fn satisfaction_fails_names_the_missing_member() {
    // S has NO member `m` — the boundary check names the type, the
    // interface, and the member
    let ds = diags_of(
        "interface I { fn m(self) -> i32; }\n\
         struct S { x: i32 }\n\
         entry fn main() -> i32 {\n\
             let s = S { x: 1 };\n\
             let i: I = s;\n\
             return i.m();\n\
         }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`S` does not satisfy `I`: no member `m`")),
        "structural satisfaction must diagnose at the boundary: {ds:?}"
    );
}

#[test]
fn receiver_form_must_match_the_interface() {
    // the member is there but spelled `self` where the interface asks
    // `mut self` — the signature differs, at the boundary
    let ds = diags_of(
        "interface I { fn m(mut self) -> nil; }\n\
         struct S { x: i32 }\n\
         impl S { pub fn m(self) -> nil { } }\n\
         entry fn main() -> i32 {\n\
             let s = S { x: 1 };\n\
             let i: I = s;\n\
             return 0;\n\
         }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`S` does not satisfy `I`")
            && d.contains("signature differs")),
        "receiver form mismatch must diagnose: {ds:?}"
    );
}

#[test]
fn empty_interface_admits_every_class() {
    // an interface with no members: the member-set match is vacuous, so
    // any class satisfies it at the boundary
    let out = compile(
        "interface Mark { }\n\
         struct S { x: i32 }\n\
         entry fn main() -> i32 {\n\
             let s = S { x: 1 };\n\
             let m: Mark = s;\n\
             return 0;\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
}

#[test]
fn static_dispatch_when_the_origin_is_single() {
    // `w` is interface-typed with ONE concrete origin — the call binds
    // statically to the member (no vtable hop)
    let out = compile(
        "interface Get { fn get(self) -> i32; }\n\
         struct B { v: i32 }\n\
         impl B {\n\
             pub fn new(v: i32) -> Self { return Self { v: v }; }\n\
             pub fn get(self) -> i32 { return self.v; }\n\
         }\n\
         entry fn main() -> i32 {\n\
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
    // receiver, so the call consults the descriptor (CallI)
    let out = compile(
        "interface Get { fn get(self) -> i32; }\n\
         struct A { v: i32 }\n\
         struct B { v: i32 }\n\
         impl A { pub fn get(self) -> i32 { return self.v; } }\n\
         impl B { pub fn get(self) -> i32 { return self.v; } }\n\
         fn pick(k: bool) -> Get {\n\
             if (k) { return A { v: 1 }; }\n\
             return B { v: 2 };\n\
         }\n\
         entry fn main() -> i32 {\n\
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
         entry fn main() -> i32 {\n\
             let c = Count.new();\n\
             for (let v of c) { }\n\
             return 0;\n\
         }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`for (let .. of ..)` needs a sequence")
            && d.contains("`Count` is not one")
            && d.contains("marks no `[iterable]` member")),
        "missing iterable member must diagnose: {ds:?}"
    );
}

#[test]
fn impl_target_must_be_a_local_type() {
    // an unknown name keeps the target diagnosis...
    let ds = diags_of(
        "impl Missing { fn m(self) -> i32 { return 1; } }\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("impl target must be a struct, class, or enum of this module")),
        "foreign target must diagnose: {ds:?}"
    );
    // ...and a USED class refuses the block outright: its members live
    // where the type was declared (the structural fork's placement law)
    let dep = rut_driver::compile_program(
        "pub struct Used { v: i32 }\n",
        Mode::Impl,
        "dep",
        1,
        &[],
    );
    assert!(dep.diags.is_empty(), "{:?}", dep.diags);
    let dep = dep.program.expect("dep program");
    let ds: Vec<String> = rut_driver::compile_program(
        "use dep::{Used};\n\
         impl Used { pub fn probe(self) -> i32 { return 7; } }\n\
         entry fn main() -> i32 { return 0; }\n",
        Mode::Impl,
        "app",
        2,
        &[(1, dep.surface.clone(), "dep".to_string())],
    )
    .diags
    .iter()
    .map(|d| d.msg.clone())
    .collect();
    assert!(
        ds.iter().any(|d| d.contains("inherent impls live in the type's module")),
        "a used type takes no consumer impl blocks: {ds:?}"
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
         entry fn main() -> i32 {\n\
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
    // later call must dispatch as B — statically re-bound to B's member
    // (still a single known origin) or through the vtable — never run
    // A's member on a B (a mis-analysis must not be able
    // to produce a wrong call). Runtime-checked in rut-cli's e2e
    // (`reassigned_trait_binding_dispatches_as_the_new_type`).
    let out = compile(
        "interface Get { fn get(self) -> i32; }\n\
         struct A { v: i32 }\n\
         struct B { v: i32 }\n\
         impl A { pub fn get(self) -> i32 { return 10; } }\n\
         impl B { pub fn get(self) -> i32 { return 20; } }\n\
         entry fn main() -> i32 {\n\
             let mut w: Get = A { v: 1 };\n\
             w = B { v: 2 };\n\
             return w.get();\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    // whatever call form the origin analysis chose, A's member body must
    // not be the statically bound callee (its id would be `callm f0`,
    // the first compiled fn here)
    let p = out.program.expect("program");
    let ir = rut_driver::ir_dump_of(&p.funcs, &p.interner);
    assert!(
        !ir.contains("callm f0"),
        "A's member must not be the static callee after reassignment:\n{ir}"
    );
}

#[test]
fn builtin_composites_take_no_impl_blocks() {
    // the closed-contract law: a builtin class's members are the
    // engine's closed surface — the fork withdrew the `[T]`/`opaque`
    // impl-block escape entirely (the old `LaunchedTask<T>` pattern
    // dies with it; capability on a composite is a wrapper's job)
    let ds = diags_of(
        "impl<T> [T] {\n\
             fn first(self) -> i32 { return 7; }\n\
         }\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("impl target must be a struct, class, or enum")),
        "a builtin composite takes no impl block: {ds:?}"
    );
    // and the concrete builtin class
    let ds = diags_of(
        "impl opaque {\n\
             fn peek(self) -> i32 { return 1; }\n\
         }\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("a builtin class takes no impl blocks")
            && d.contains("closed contract")),
        "a builtin class takes no impl block: {ds:?}"
    );
}

// ---- primitive capability is wrapper-manufactured --------------------
// (the prim trait-impl rows died with the registry: a primitive takes no
// impl blocks and never satisfies an interface — a newtype wrapper
// class is the spelled manufacture)

/// Compile, flatten, verify, and run a single-module
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
    let mut vm = rut_vm::interp:: Vm::builder().program(std::rc::Rc::new(prog)).limits(limits).hooks(rut_vm::interp::HostHooks::default()).hosts(rut_vm::interp::HostRegistry::new()).build()
    .expect("vm");
    vm.call::<_, i32>("main", ()).expect("run")
}

const WI: &str = "interface Num { fn m(self) -> i32; }\n\
                  class WI(i32);\n\
                  impl WI { pub fn m(self) -> i32 { return self.inner + 100; } }\n";

#[test]
fn prim_capability_rides_a_wrapper_and_dispatches_statically() {
    // the wrapper satisfies; `let w: Num = WI(5)` widens at the spelled
    // constructor, and the single-origin call binds statically —
    // the wrapped scalar rides inside the ref-repr wrapper
    let src = &format!("{WI}\
                entry fn main() -> i32 {{\n\
                    let w: Num = WI(5);\n\
                    return w.m();\n\
                }}\n");
    assert_eq!(run_main(src), 105);
    let out = compile(src);
    let p = out.program.expect("program");
    let ir = rut_driver::ir_dump_of(&p.funcs, &p.interner);
    assert!(!ir.contains("calli"), "single origin must bind statically:\n{ir}");
}

#[test]
fn primitive_takes_no_impl_blocks() {
    // the wrapper law's gate: `impl i32 { .. }` diagnoses, naming the
    // wrapper manufacture
    let ds = diags_of(
        "impl i32 { fn f(self) -> i32 { return self; } }\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("a primitive takes no impl blocks")
            && d.contains("wrapper")),
        "inherent impl on a prim must diagnose: {ds:?}"
    );
}

#[test]
fn probe_answers_the_boxed_satisfaction() {
    // `is` folds at the exact receiver: true where the type's itable
    // carries a fill for the interface (it was satisfaction-boxed
    // somewhere in the program), false where it did not
    let src = "interface T { fn m(self) -> i32; }\n\
               class WI(i32);\n\
               impl WI { pub fn m(self) -> i32 { return 1; } }\n\
               class Plain(i32);\n\
               entry fn main() -> i32 {\n\
                   let mut acc = 0;\n\
                   let w: T = WI(5);\n\
                   if (w is T) { acc = acc + 1; }\n\
                   let p = Plain(7);\n\
                   if (p is T) { acc = acc + 10; }\n\
                   return acc;\n\
               }\n";
    assert_eq!(run_main(src), 1);
}

#[test]
fn merged_origins_wrapper_and_struct_emit_calli() {
    // a wrapper class and a struct both satisfy Num: the merged-origin
    // call consults the vtable (IR-level — the wrapper fills the same
    // slot as the record)
    let out = compile(
        "interface Num { fn m(self) -> i32; }\n\
         class WI(i32);\n\
         impl WI { pub fn m(self) -> i32 { return self.inner + 1; } }\n\
         struct S { v: i32 }\n\
         impl S { pub fn m(self) -> i32 { return self.v; } }\n\
         fn pick(k: bool) -> Num {\n\
             if (k) { return WI(5); }\n\
             return S { v: 1 };\n\
         }\n\
         entry fn main() -> i32 {\n\
             let w: Num = pick(true);\n\
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
fn dep_manufactures_capability_via_a_wrapper() {
    // the old `impl ForeignTrait for str` shape is gone with the
    // registry; the surviving cross-module shape: the dep exports a
    // WRAPPER over the value type, its inherent impl carries the
    // members, and the consumer wraps at the spelled constructor
    let dep = rut_driver::compile_program(
        "pub class Tag(str);\n\
         impl Tag {\n\
             pub fn m(self) -> i32 { return 7; }\n\
         }\n",
        Mode::Impl,
        "dep",
        1,
        &[],
    );
    assert!(dep.diags.is_empty(), "{:?}", dep.diags);
    let dep = dep.program.expect("dep program");
    let surface = dep.surface.clone();

    let app = rut_driver::compile_program(
        "use dep::{Tag};\n\
         entry fn main() -> i32 { return Tag(\"s\").m(); }\n",
        Mode::Impl,
        "app",
        2,
        &[(1, surface, "dep".to_string())],
    );
    assert!(app.diags.is_empty(), "{:?}", app.diags);
}

#[test]
fn class_vtable_fill_survives_the_link() {
    // regression (link boot-row merge), restated for the itable: the
    // dep module's fill of its class's GLOBAL row must not be dropped
    // when the module merges — pre-fix, rows below the boot prefix hit
    // `continue` and the fill was silently lost. The cross-module call
    // then runs end-to-end.
    let dep = rut_driver::compile_program(
        "interface Num { fn m(self) -> i32; }\n\
         class WI(i32);\n\
         impl WI { pub fn m(self) -> i32 { return self.inner + 100; } }\n\
         pub fn boxed() -> Num { return WI(5); }\n",
        Mode::Impl,
        "dep",
        1,
        &[],
    );
    assert!(dep.diags.is_empty(), "{:?}", dep.diags);
    let dep = dep.program.expect("dep program");
    let surface = dep.surface.clone();

    let app = rut_driver::compile_program(
        "use dep::{Num, boxed};\n\
         entry fn main() -> i32 {\n\
             let w: Num = boxed();\n\
             return w.m();\n\
         }\n",
        Mode::Impl,
        "app",
        2,
        &[(1, surface, "dep".to_string())],
    );
    assert!(app.diags.is_empty(), "{:?}", app.diags);
    let linked = rut_core::link::link(vec![dep, app.program.expect("app program")]).expect("link");

    // the fill survived: the global WI row carries the dep's member
    let gid = linked
        .ifaces
        .iter()
        .position(|t| linked.name_of(t.name) == "Num")
        .expect("one global Num") as u32;
    let slot = linked.slot_of(gid, 0).expect("global slot");
    let wi_ty = (0..linked.types.types.len() as u32)
        .find(|&i| linked.type_name(i) == "WI")
        .expect("WI in the global table") as usize;
    let fill = linked.vtables[wi_ty][slot as usize];
    assert!(fill.is_some(), "WI's vtable row must carry the dep's member: {:?}", linked.vtables[wi_ty]);
    let fname = linked.name_of(linked.funcs[fill.unwrap() as usize].name);
    assert!(fname.contains("m"), "the fill names the member: {fname}");

    // and the cross-module call runs end-to-end
    rut_vm::verify::verify(&linked).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp:: Vm::builder().program(std::rc::Rc::new(linked)).limits(limits).hooks(rut_vm::interp::HostHooks::default()).hosts(rut_vm::interp::HostRegistry::new()).build()
    .expect("vm");
    assert_eq!(vm.call::<_, i32>("main", ()).expect("run"), 105);
}

#[test]
fn wrapper_slots_survive_calls_and_vtable_dispatch() {
    // the ref-repr slot ABI, over wrappers: interface-typed values
    // survive ref copies and call boundaries, a `Self`-spelled member
    // parameter binds per instantiation (the satisfaction check reads
    // the member under the substitution), and a merged-origin calli
    // dispatches on the carried type. (The prim-slot shapes this test
    // once pinned died with prim satisfaction — wrappers are the
    // surviving manufacture.)
    let src = "interface K { fn key(self) -> u64; fn same(self, other: Self) -> bool; }\n\
               class KI(i32);\n\
               impl KI {\n\
               \x20   pub fn key(self) -> u64 { return (self.inner as u64).wrapping_mul(7); }\n\
               \x20   pub fn same(self, other: KI) -> bool { return self.inner == other.inner; }\n\
               }\n\
               class KU(u8);\n\
               impl KU {\n\
               \x20   pub fn key(self) -> u64 { return self.inner as u64; }\n\
               \x20   pub fn same(self, other: KU) -> bool { return self.inner == other.inner; }\n\
               }\n\
               fn probe(k: K, h: u64) -> bool {\n\
               \x20   return k.key() == h && k.same(k);\n\
               }\n\
               fn pick(k: bool) -> K {\n\
               \x20   if (k) { return KI(5); }\n\
               \x20   return KU(9u8);\n\
               }\n\
               entry fn main() -> i32 {\n\
               \x20   let w: K = KI(5);\n\
               \x20   if (!probe(w, 35)) { return -1; }\n\
               \x20   let v: K = pick(true);\n\
               \x20   if (!v.same(v)) { return -2; }\n\
               \x20   if (v.key() != 35) { return -3; }\n\
               \x20   return 5;\n\
               }\n";
    assert_eq!(run_main(src), 5);
}

// ---- impl-method visibility (the pub law) ---------------------------
// `pub fn` on a struct's inherent impl exports cross-module; plain
// `fn` is module-private (the class rule). Struct FIELDS keep the
// all-pub law — this is about methods only.

const VIS_DEP: &str = "\
pub struct Counter { n: i32 }\n\
impl Counter {\n\
\x20   pub fn new() -> Self { return Counter { n: 0 }; }\n\
\x20   pub fn bump(mut self) -> Self { self.n += 1; return self; }\n\
\x20   fn secret(self) -> i32 { return self.n; }\n\
}\n\
// same-module: the plain fn is right there — the ordinary law\n\
pub fn peek(c: Counter) -> i32 { return c.secret(); }\n\
pub fn fresh() -> Counter { return Counter.new(); }\n";

#[test]
fn pub_struct_methods_cross_private_ones_stay_home() {
    let dep = rut_driver::compile_program(VIS_DEP, Mode::Impl, "counter", 1, &[]);
    assert!(dep.diags.is_empty(), "the plain fn is fine same-module: {:?}", dep.diags);
    let dep = dep.program.expect("dep");
    let surface = dep.surface.clone();

    // the consumer calls the pub methods; the private one is a
    // standard no-method error (visibility enforcement is structural)
    let pub_only = rut_driver::compile_program(
        "use counter::{Counter, fresh, peek};\n\
         entry fn main() -> i32 {\n\
         \x20   let c = fresh().bump();\n\
         \x20   if (peek(c) != 1) { return -1; }\n\
         \x20   return 0;\n\
         }\n",
        Mode::Impl,
        "app",
        2,
        &[(1, surface.clone(), "counter".to_string())],
    );
    assert!(pub_only.diags.is_empty(), "pub methods cross: {:?}", pub_only.diags);
    let out = rut_core::link::link(vec![dep.clone(), pub_only.program.expect("app")]).expect("link");
    assert_eq!(run_main_src(out), 0, "the linked pub-method call runs");

    let private = rut_driver::compile_program(
        "use counter::{fresh};\n\
         entry fn main() -> i32 {\n\
         \x20   let c = fresh();\n\
         \x20   return c.secret();\n\
         }\n",
        Mode::Impl,
        "app",
        2,
        &[(1, surface, "counter".to_string())],
    );
    let ds: Vec<String> = private.diags.iter().map(|d| d.msg.clone()).collect();
    assert!(
        ds.iter().any(|d| d.contains("`Counter` has no method `secret`")),
        "a private method is a no-method error at the consumer: {ds:?}"
    );
}

fn run_main_src(p: rut_core::binary::Program) -> i32 {
    let prog = rut_core::link::flatten(p);
    rut_vm::verify::verify(&prog).expect("verify");
    let mut vm = rut_vm::interp:: Vm::builder().program(std::rc::Rc::new(prog)).limits(rut_vm::interp::Limits::default()).hooks(rut_vm::interp::HostHooks::default()).hosts(rut_vm::interp::HostRegistry::new()).build()
    .expect("vm");
    vm.call::<_, i32>("main", ()).expect("run")
}

// ---- enums as impl targets ------------------------------------------
// `impl Color { .. }` attaches to the enum's decl slot: non-self
// statics (`Color.default()`), self methods (`c.label()`), and the
// `[iterable]` member (`for (let v of c)` rides the same desugar as a
// class's).

#[test]
fn enum_inherent_statics_and_self_calls_compile() {
    let src = "enum Color { Red, Green, Blue }\n\
               impl Color {\n\
               \x20   fn default() -> Self { return Color.Green; }\n\
               \x20   fn label(self) -> str {\n\
               \x20       return when (self) {\n\
               \x20           Color.Red -> \"red\",\n\
               \x20           Color.Green -> \"green\",\n\
               \x20           Color.Blue -> \"blue\",\n\
               \x20       };\n\
               \x20   }\n\
               }\n\
               entry fn main() -> i32 {\n\
               \x20   let d: Color = Color.default();\n\
               \x20   if (d.label() != \"green\") { return -1; }\n\
               \x20   if (Color.Red.label() != \"red\") { return -2; }\n\
               \x20   return 7;\n\
               }\n";
    assert_eq!(run_main(src), 7);
}

#[test]
fn iterable_member_drives_for_break_continue() {
    // the for-of desugar is the marked member: `for (let v of c)` calls
    // the impl's `[iterable] fn iterate` with a synthetic emit closure —
    // `break` returns false, `continue` returns true. (The marker lives
    // on struct and class impls — an enum's members are immortal
    // singletons and carry no engine contract, pinned beside the
    // disposal twin below.)
    let src = "enum Light { Green, Yellow, Red }\n\
               class Lights { }\n\
               struct Acc { hits: i32 = 0; }\n\
               impl Lights {\n\
               \x20   fn new() -> Self { return Self { }; }\n\
               \x20   [iterable] fn iterate(self, emit: fn(Light) -> bool) {\n\
               \x20       if (!emit(Light.Green)) { return; }\n\
               \x20       if (!emit(Light.Yellow)) { return; }\n\
               \x20       emit(Light.Red);\n\
               \x20   }\n\
               }\n\
               entry fn main() -> i32 {\n\
               \x20   // the capture law: scalars copy into the emit closure,\n\
               \x20   // so the accumulator is a shared cell\n\
               \x20   let mut acc: ?Acc = Acc { };\n\
               \x20   for (let v of Lights.new()) {\n\
               \x20       acc.hits += 1;\n\
               \x20       if (acc.hits == 2) { break; }\n\
               \x20   }\n\
               \x20   if (acc.hits != 2) { return -1; }\n\
               \x20   let mut all: ?Acc = Acc { };\n\
               \x20   for (let v of Lights.new()) {\n\
               \x20       all.hits += 1;\n\
               \x20       continue;\n\
               \x20   }\n\
               \x20   if (all.hits != 3) { return -2; }\n\
               \x20   return 9;\n\
               }\n";
    assert_eq!(run_main(src), 9);
}

#[test]
fn enum_target_rejects_generic_arguments_and_foreign_names() {
    // enums are concrete: `Light<E>` is a spelled-arity error, and a
    // foreign name keeps the fallthrough diagnosis (now naming enums)
    let ds = diags_of(
        "enum Light { Green, Red }\n\
         impl Light<i32> { [iterable] fn iterate(self, emit: fn(i32) -> bool) { } }\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`Light` takes no generic arguments")),
        "an enum target is concrete: {ds:?}"
    );
}

#[test]
fn disposal_stays_data_only_for_enums() {
    // `[disposal]` refuses an enum target: the engine disposes record
    // cells only, and an enum's members are immortal singletons.
    // `[iterable]` is LEGAL on an enum (a value iterates like a
    // class's — the docs pin it), so the second half drives one.
    let ds = diags_of(
        "use core::{ DisposalContext };\n\
         enum Light { Green, Red }\n\
         impl Light { [disposal] fn dispose_light(mut self, cx: DisposalContext) { } }\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("cannot carry `[disposal]`")
            && d.contains("immortal singletons")),
        "the engine disposes record cells only: {ds:?}"
    );
    // the capture law: the emit closure copies scalars, so the loop
    // accumulates through a ref-headed cell (the docs' own law)
    let v = run_main(
        "class Acc { n: i32 = 0; }\n\
         enum Light { Green, Red }\n\
         impl Light {\n\
             [iterable] fn iterate(self, emit: fn(Light) -> bool) { emit(Light.Green); emit(Light.Red); }\n\
         }\n\
         entry fn main() -> i32 {\n\
             let mut acc: ?Acc = Acc { };\n\
             for (let l of Light.Red) {\n\
                 let x = when (l) { Light.Green -> 1, Light.Red -> 2 };\n\
                 acc.n = acc.n + x;\n\
             }\n\
             return acc.n;\n\
         }\n",
    );
    assert_eq!(v, 3, "the enum's marked member drives the for-of");
}

#[test]
fn enum_pub_methods_cross_private_ones_stay_home() {
    // the surface lane is kind-blind now: an enum's pub inherent
    // methods cross exactly like a class's, via the inherent rows
    let dep = rut_driver::compile_program(
        "pub enum Dial { Low, High }\n\
         impl Dial {\n\
         \x20   pub fn default() -> Self { return Dial.Low; }\n\
         \x20   pub fn flipped(self) -> Self {\n\
         \x20       return when (self) { Dial.Low -> Dial.High, Dial.High -> Dial.Low };\n\
         \x20   }\n\
         \x20   fn hidden(self) -> i32 { return 5; }\n\
         }\n",
        Mode::Impl,
        "dial",
        1,
        &[],
    );
    assert!(dep.diags.is_empty(), "{:?}", dep.diags);
    let dep = dep.program.expect("dep");
    let surface = dep.surface.clone();

    let pub_only = rut_driver::compile_program(
        "use dial::{Dial};\n\
         entry fn main() -> i32 {\n\
         \x20   let d = Dial.default().flipped();\n\
         \x20   return when (d) { Dial.Low -> 1, Dial.High -> 2 };\n\
         }\n",
        Mode::Impl,
        "app",
        2,
        &[(1, surface.clone(), "dial".to_string())],
    );
    assert!(pub_only.diags.is_empty(), "pub enum methods cross: {:?}", pub_only.diags);
    let out = rut_core::link::link(vec![dep.clone(), pub_only.program.expect("app")]).expect("link");
    assert_eq!(run_main_src(out), 2, "the static chains through the flipped self method");

    let private = rut_driver::compile_program(
        "use dial::{Dial};\n\
         entry fn main() -> i32 {\n\
         \x20   return Dial.High.hidden();\n\
         }\n",
        Mode::Impl,
        "app",
        2,
        &[(1, surface, "dial".to_string())],
    );
    let ds: Vec<String> = private.diags.iter().map(|d| d.msg.clone()).collect();
    assert!(
        ds.iter().any(|d| d.contains("no method `hidden`") || d.contains("unknown name `Dial.hidden`")),
        "a private enum method is invisible at the consumer: {ds:?}"
    );
}
