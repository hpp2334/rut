//! Parameterized trait impls: phase 1 registered the TEMPLATE (the
//! checker half — the shape guard, the placeholder env, coverage); phase
//! 2 is the DISPATCH half — unification, coercion, vtables. The tests
//! here execute: a registered template's instantiation widens where
//! `Readable<str>` is expected, the call dispatches to the template's
//! method monomorphized at that instantiation, two instantiations get
//! distinct correct vtable rows, the same works inside generic fn bodies
//! (the fat-ref headline), repeated trait parameters
//! (`Writable<T, T>`) resolve positionally, and a concrete impl still
//! shadows the template's instantiation of the same pair (concrete-first).
//! Phase 3 adds the generic INSTANCE-method call: the method's own type
//! arguments join the class instantiation to key the monomorphized
//! frame, and a spelled lambda argument takes its parameter types from
//! the bound part of the expected shape (the placeholder hint).

use rut_parser::Mode;
use rut_driver::{Module, Session};

/// A compile-and-run harness: core + the std surfaces mounted, the test
/// source compiled as the root module, encoded + decoded (RFC 0033) —
/// the same loop the example suites run.
fn boot(src: &str) -> Result<rut_vm::interp::Vm, String> {
    let mut session = Session::new();
    rut_driver::mount_std_core(&mut session);
    let out = compile(src);
    if !out.diags.is_empty() {
        return Err(out
            .diags
            .iter()
            .map(|d| d.msg.clone())
            .collect::<Vec<_>>()
            .join("; "));
    }
    session
        .register_module("test", Module { source: Some(src.to_string()), inline: true, ..Default::default() })
        .map_err(|e| format!("{e:?}"))?;
    let compiled = rut_driver::compile_module_in(&mut session, src, Mode::Impl, "testroot");
    if !compiled.diags.is_empty() {
        return Err(compiled
            .diags
            .iter()
            .map(|d| d.msg.clone())
            .collect::<Vec<_>>()
            .join("; "));
    }
    let bin = compiled.binary.expect("no binary");
    let prog = rut_core::binary::decode(&bin).expect("decode");
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    rut_vm::interp::Vm::new(
        std::rc::Rc::new(prog),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .map_err(|e| format!("{e:?}"))
}

fn compile(src: &str) -> rut_driver::ProgramOutput {
    // the core surface bound as the one use (RFC 0028): these tests
    // exercise impl registration, not use discipline
    rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, rut_core::binary::Surface::core(), "core".to_string())],
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
fn parameterized_trait_impl_registers_as_a_template() {
    // the head's parameter names the target's own parameter — the
    // checker's blocker ("unknown type `T`") is gone, and the concrete
    // form still compiles beside it
    let out = compile(
        "trait Readable<T> { fn atom_id(self) -> u32; }\n\
         class Source<T> { id: u32 = 0; }\n\
         impl Readable<T> for Source<T> {\n\
             fn atom_id(self) -> u32 { return self.id; }\n\
         }\n\
         impl Readable<i64> for Source<i64> {\n\
             fn atom_id(self) -> u32 { return self.id; }\n\
         }\n\
         entry fn main() {}\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
}

#[test]
fn trait_parameter_in_the_impl_methods_signature_resolves() {
    // the coverage check resolves both sides under the placeholder env —
    // a parameter-spelled impl signature must not die as an unknown type
    let ds = diags_of(
        "trait Store<T> {\n\
             fn get(self, k: T) -> u32;\n\
             fn put(self, k: T, v: u32);\n\
         }\n\
         class Source<T> { id: u32 = 0; }\n\
         impl Store<T> for Source<T> {\n\
             fn get(self, k: T) -> u32 { return self.id; }\n\
             fn put(self, k: T, v: u32) { }\n\
         }\n\
         entry fn main() {}\n",
    );
    assert!(ds.is_empty(), "{ds:?}");
}

#[test]
fn repeated_parameter_in_two_trait_slots_is_legal() {
    let ds = diags_of(
        "trait Pair2<A, B> { fn one(self) -> u32; }\n\
         class Source<T> { id: u32 = 0; }\n\
         impl Pair2<T, T> for Source<T> {\n\
             fn one(self) -> u32 { return self.id; }\n\
         }\n\
         entry fn main() {}\n",
    );
    assert!(ds.is_empty(), "{ds:?}");
}

#[test]
fn parameter_nested_in_a_trait_argument_is_a_v1_error() {
    let ds = diags_of(
        "trait Readable<T> { fn atom_id(self) -> u32; }\n\
         class Source<T> { id: u32 = 0; }\n\
         impl Readable<Vec<T>> for Source<T> {\n\
             fn atom_id(self) -> u32 { return self.id; }\n\
         }\n\
         entry fn main() {}\n",
    );
    assert_eq!(ds.len(), 1, "one diagnostic, not a cascade: {ds:?}");
    assert!(
        ds[0].contains("a parameter nested inside a type is not supported yet"),
        "{ds:?}"
    );
}

#[test]
fn trait_argument_naming_no_target_parameter_and_no_type_is_a_v1_error() {
    // `T` is not a parameter of the target (`Source<U>`) nor a type in
    // scope — the guard diagnoses the shape, once
    let ds = diags_of(
        "trait Readable<T> { fn atom_id(self) -> u32; }\n\
         class Source<U> { id: u32 = 0; }\n\
         impl Readable<T> for Source<U> {\n\
             fn atom_id(self) -> u32 { return self.id; }\n\
         }\n\
         entry fn main() {}\n",
    );
    assert_eq!(ds.len(), 1, "one diagnostic, not a cascade: {ds:?}");
    assert!(
        ds[0]
            .contains("`T` is neither a type in scope nor a type parameter of the impl target"),
        "{ds:?}"
    );
}

#[test]
fn exact_template_duplicate_still_errors() {
    // same trait template (same parameters) + same class: the ordinary
    // (trait, type) duplicate check. A template OVERLAPPING a concrete
    // impl for a specific instantiation (`Readable<T>` vs
    // `Readable<i64>`) is deliberately NOT checked at collect time —
    // the phase-2 dispatch half owns that resolution.
    let ds = diags_of(
        "trait Readable<T> { fn atom_id(self) -> u32; }\n\
         class Source<T> { id: u32 = 0; }\n\
         impl Readable<T> for Source<T> {\n\
             fn atom_id(self) -> u32 { return self.id; }\n\
         }\n\
         impl Readable<T> for Source<T> {\n\
             fn atom_id(self) -> u32 { return self.id; }\n\
         }\n\
         entry fn main() {}\n",
    );
    assert_eq!(ds.len(), 1, "one diagnostic: {ds:?}");
    assert!(
        ds[0].contains("duplicate impl for the same (trait, type) pair"),
        "{ds:?}"
    );
}

// ---- phase 2: the dispatch half (these tests EXECUTE) ----------------

/// widening OUTSIDE a generic fn: two instantiations of one template,
/// each boxed into its trait-object slot, each dispatching to its own
/// monomorphized method body
#[test]
fn widening_dispatches_to_the_instantiated_template() {
    let mut vm = boot(
        "trait Readable<T> { fn atom_id(self) -> u32; }\n\
         class Source<T> { id: u32 = 0; }\n\
         impl Readable<T> for Source<T> {\n\
             fn atom_id(self) -> u32 { return self.id; }\n\
         }\n\
         entry fn main() -> u32 {\n\
             let r: Readable<str> = Source<str> { id: 7 };\n\
             let r2: Readable<i64> = Source<i64> { id: 9 };\n\
             return r.atom_id() * 10 + r2.atom_id();\n\
         }\n",
    )
    .expect("compiles");
    let r: u32 = vm.call("main", ()).expect("runs");
    assert_eq!(r, 79, "both instantiations dispatch correctly");
}

/// the same inside a GENERIC fn body: the fat-ref parameter (the
/// phase's headline gate) — one body, two monomorphizations, each
/// statically bound to its origin's method
#[test]
fn generic_fn_body_dispatches_per_monomorphization() {
    let mut vm = boot(
        "trait Readable<T> { fn atom_id(self) -> u32; }\n\
         class Source<T> { id: u32 = 0; }\n\
         class Derived<T> { id: u32 = 0; }\n\
         impl Readable<T> for Source<T> {\n\
             fn atom_id(self) -> u32 { return self.id; }\n\
         }\n\
         impl Readable<T> for Derived<T> {\n\
             fn atom_id(self) -> u32 { return self.id + 1; }\n\
         }\n\
         fn read_id<T>(a: Readable<T>) -> u32 { return a.atom_id(); }\n\
         entry fn main() -> u64 {\n\
             let s = Source<str> { id: 7 };\n\
             let d = Derived<i64> { id: 9 };\n\
             return (read_id(s) + read_id(d)) as u64;\n\
         }\n",
    )
    .expect("compiles");
    let r: u64 = vm.call("main", ()).expect("runs");
    assert_eq!(r, 17, "read_id(s)=7 and read_id(d)=10 — per-instantiation dispatch");
}

/// the vtable: a trait-typed binding with TWO possible origins (a
/// runtime branch) cannot bind statically — the call dispatches through
/// the vtable rows the template filled for each concrete instantiation
#[test]
fn vtable_row_is_filled_for_the_instantiation() {
    let mut vm = boot(
        "trait Readable<T> { fn atom_id(self) -> u32; }\n\
         class Source<T> { id: u32 = 0; }\n\
         class Derived<T> { id: u32 = 0; }\n\
         impl Readable<T> for Source<T> {\n\
             fn atom_id(self) -> u32 { return self.id; }\n\
         }\n\
         impl Readable<T> for Derived<T> {\n\
             fn atom_id(self) -> u32 { return self.id + 100; }\n\
         }\n\
         entry fn main() -> u32 {\n\
             let a: Readable<str> = Source<str> { id: 5 };\n\
             let b: Readable<i64> = Derived<i64> { id: 6 };\n\
             let mut pick: Readable<str> = a;\n\
             let mut pick2: Readable<i64> = b;\n\
             let use_b = 6 == 6;\n\
             if (use_b) {\n\
                 pick2 = Derived<i64> { id: 7 };\n\
             }\n\
             return pick.atom_id() + pick2.atom_id();\n\
         }\n",
    )
    .expect("compiles");
    let r: u32 = vm.call("main", ()).expect("runs");
    assert_eq!(r, 112, "pick via the vtable (5) + pick2 via the vtable (107)");
}

/// repeated parameters (`Writable<T, T>`): the substitution is
/// positional, every trait-argument node re-resolves independently, and
/// the widened value dispatches to the template's method
#[test]
fn repeated_trait_parameters_dispatch() {
    let mut vm = boot(
        "trait Writable<A, R> { fn stamp(self) -> u32; }\n\
         class Source<T> { id: u32 = 0; }\n\
         impl Writable<T, T> for Source<T> {\n\
             fn stamp(self) -> u32 { return self.id; }\n\
         }\n\
         entry fn main() -> u32 {\n\
             let w: Writable<str, str> = Source<str> { id: 21 };\n\
             return w.stamp();\n\
         }\n",
    )
    .expect("compiles");
    let r: u32 = vm.call("main", ()).expect("runs");
    assert_eq!(r, 21, "Writable<T, T> over Source<str> dispatches");
}

/// concrete-first (the v1 law): a hand-written impl for a specific
/// instantiation shadows the template's instantiation of the same pair
#[test]
fn concrete_impl_shadows_the_template_instantiation() {
    let mut vm = boot(
        "trait Readable<T> { fn atom_id(self) -> u32; }\n\
         class Source<T> { id: u32 = 0; }\n\
         impl Readable<T> for Source<T> {\n\
             fn atom_id(self) -> u32 { return self.id; }\n\
         }\n\
         impl Readable<i64> for Source<i64> {\n\
             fn atom_id(self) -> u32 { return 999; }\n\
         }\n\
         entry fn main() -> u32 {\n\
             let t: Readable<str> = Source<str> { id: 7 };\n\
             let c: Readable<i64> = Source<i64> { id: 5 };\n\
             return t.atom_id() * 1000 + c.atom_id();\n\
         }\n",
    )
    .expect("compiles");
    let r: u32 = vm.call("main", ()).expect("runs");
    assert_eq!(r, 7999, "the template serves str; the concrete impl serves i64");
}

/// a template over TWO type parameters with MIXED concrete and
/// parameter trait arguments (`R<Vec<i64>, T>`): only the parameter
/// slots substitute, the concrete slots stay put
#[test]
fn mixed_concrete_and_parameter_trait_arguments() {
    let mut vm = boot(
        "trait Store<A, B> { fn mark(self) -> u32; }\n\
         class Bag<T> { n: u32 = 1; }\n\
         class Box2<T> { id: u32 = 0; }\n\
         impl Store<Bag<i64>, T> for Box2<T> {\n\
             fn mark(self) -> u32 { return self.id; }\n\
         }\n\
         entry fn main() -> u32 {\n\
             let b: Store<Bag<i64>, str> = Box2<str> { id: 33 };\n\
             return b.mark();\n\
         }\n",
    )
    .expect("compiles");
    let r: u32 = vm.call("main", ()).expect("runs");
    assert_eq!(r, 33, "the concrete slot (Bag<i64>) stayed; the parameter slot substituted");
}

// ---- phase 3: generic instance-method frames --------------------------

/// The stale-frame repro, minimized: the same generic body must compute
/// identically as a FREE fn and as an INSTANCE method of a class. The
/// method's own type arguments — spelled (`s.get<i64>(..)`) or inferred
/// through the `Readable<T>` template (`s.get(..)`) — join the class
/// instantiation to key the Inst, so the callee frame's parameters and
/// the trait call inside it type per instantiation (v1 typed method
/// parameters under the CLASS instantiation only, which left the
/// generic-method frames computing stale/wrong values). The spelled
/// lambda argument types its parameter from the BOUND part of the
/// expected fn shape (the placeholder hint, the free-fn door's law,
/// now wired in the method door too).
#[test]
fn generic_instance_method_frames_compute_like_free_fns() {
    let mut vm = boot(
        "trait Readable<T> { fn atom_id(self) -> u32; }\n\
         class Source<T> { id: u32 = 0; }\n\
         impl Readable<T> for Source<T> {\n\
             fn atom_id(self) -> u32 { return self.id; }\n\
         }\n\
         class Ctx2 { x: i32 = 6; }\n\
         class Store { cell: opaque; }\n\
         impl Store {\n\
             pub fn get<T>(self, a: Readable<T>) -> T {\n\
                 let _ = a.atom_id();\n\
                 return opaque.downcast<T>(self.cell);\n\
             }\n\
             pub fn lift<T>(self, f: fn(Ctx2) -> T) -> T {\n\
                 return f(Ctx2 { x: 6 });\n\
             }\n\
         }\n\
         pub fn get_free<T>(st: Store, a: Readable<T>) -> T {\n\
             let _ = a.atom_id();\n\
             return opaque.downcast<T>(st.cell);\n\
         }\n\
         entry fn main() -> i64 {\n\
             let s = Store { cell: opaque(41 as i64) };\n\
             let src = Source<i64> { id: 3 };\n\
             let m_i = s.get<i64>(src);\n\
             let m_infer = s.get(src);\n\
             let f_i = get_free<i64>(s, src);\n\
             let f_infer = get_free(s, src);\n\
             let lifted = s.lift(fn (c) -> i64 { return c.x as i64; });\n\
             return m_i + m_infer + f_i + f_infer + lifted;\n\
         }\n",
    )
    .expect("compiles");
    let r: i64 = vm.call("main", ()).expect("runs");
    assert_eq!(
        r, 170,
        "method and free-fn frames answer 41 each; the spelled lambda lifts 6"
    );
}

/// the method-generic arity guard: a call spelling more type arguments
/// than the method declares diagnoses once, cleanly
#[test]
fn method_generic_arity_mismatch_is_one_clean_diagnostic() {
    let ds = diags_of(
        "class Store { n: u32 = 0; }\n\
         impl Store {\n\
             pub fn get<T>(self, k: u32) -> u32 { return self.n; }\n\
         }\n\
         entry fn main() -> u32 {\n\
             let s = Store { n: 1 };\n\
             return s.get<i64, str>(1);\n\
         }\n",
    );
    assert_eq!(ds.len(), 1, "one diagnostic, not a cascade: {ds:?}");
    assert!(ds[0].contains("takes 1 generic argument(s), 2 given"), "{ds:?}");
}

/// deterministic generic-instance emission (phase 4b): the same source
/// compiled twice — two fresh contexts in this process — must encode
/// byte-identical binaries. `build_vtables` used to walk `inst_data`
/// (a std HashMap) in RandomState order, so the generic-target arm's
/// `mk_trait_inst` calls interned trait ids in a per-context random
/// order and the two binaries diverged (swapped dense receiver-type
/// ids / vtable rows). The walk is canonicalized on the dense type id.
///
/// The harness choice: an in-process double compile is the strongest
/// guard available in this layout — each `Ctx` builds its own HashMaps
/// with independent hasher seeds, so the two compiles here really do
/// iterate `inst_data` in different orders (the same leak the scratch
/// rutdiag repro's `ndet` subcommand caught run-to-run; its
/// `selfcheck`/`lanes` subcommands cover the cross-process and
/// two-lane shapes).
#[test]
fn double_compile_of_one_source_emits_identical_bytes() {
    // four instantiations of the same generic target — the multi-entry
    // `inst_data` shape that makes the (pre-fix) HashMap walk order
    // observable in the emitted trait ids, function ids, and vtable rows
    let src = "trait Readable<T> { fn atom_id(self) -> u32; }\n\
               class Source<T> { id: u32 = 0; }\n\
               impl Readable<T> for Source<T> {\n\
                   fn atom_id(self) -> u32 { return self.id; }\n\
               }\n\
               pub fn read_id<T>(a: Readable<T>) -> u32 {\n\
                   return a.atom_id();\n\
               }\n\
               class W {\n\
                   a$: Source<i64>;\n\
                   b$: Source<str>;\n\
                   c$: Source<f64>;\n\
                   d$: Source<bool>;\n\
               }\n\
               entry fn main() -> u32 {\n\
                   let w = W { a$: Source { id: 1 }, b$: Source { id: 2 }, c$: Source { id: 3 }, d$: Source { id: 4 } };\n\
                   let mut total: u32 = 0;\n\
                   total = total + read_id(w.a$);\n\
                   total = total + read_id(w.b$);\n\
                   total = total + read_id(w.c$);\n\
                   total = total + read_id(w.d$);\n\
                   return total;\n\
               }\n";
    let first = compile(src);
    assert!(first.diags.is_empty(), "{:?}", first.diags);
    let bin1 = rut_core::binary::encode(&first.program.expect("first program"));
    for round in 2..=4 {
        let again = compile(src);
        assert!(again.diags.is_empty(), "round {round}: {:?}", again.diags);
        let bin = rut_core::binary::encode(&again.program.expect("program"));
        assert_eq!(bin1, bin, "compile #{round} diverged from compile #1");
    }
    // canonicalization must not move semantics: the dispatch still runs
    let mut vm = boot(src).expect("compiles");
    let r: u32 = vm.call("main", ()).expect("runs");
    assert_eq!(r, 10, "1 + 2 + 3 + 4 through the vtable rows");
}
