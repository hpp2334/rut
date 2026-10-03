//! Structural interfaces: the dispatch laws that survived the
//! trait/impl fork, spelled in the new surface. An `interface Name<..>`
//! is an observed capability, satisfied STRUCTURALLY by pub inherent
//! members; `requires` bounds gate generic instantiations and prove the
//! widening; wrapper newtype classes (`class W<T>(inner);`) manufacture
//! capability. The tests here execute: a bound-spelled generic fn whose
//! bound-parameter signature resolves over two satisfying classes, the
//! wrapper-construction inference (`let w = JsonW(t);` binds T from the
//! wrapped arg) dispatching per instantiation, per-monomorphization
//! dispatch inside generic fn bodies, the itable fill for a
//! generic-interface instantiation (branch-merged origins), generic
//! INSTANCE-method frames computing like free fns, and deterministic
//! generic-instance emission (two compiles, byte-identical binaries).

use rut_parser::Mode;


/// A compile-and-run harness: core + the std surfaces mounted, the test
/// source compiled as the root module, encoded + decoded —
/// the same loop the example suites run.
fn boot(src: &str) -> Result<rut_vm::interp::Vm, String> {
    let compiled = rut_driver::RutRun::new()
        .pkg(rut_driver::Pkg::source("testroot", src))
        .entrypoint("testroot")
        .compile()
        .map_err(|e| e.to_string())?;
    if !compiled.graph.diags.is_empty() {
        return Err(compiled
            .graph
            .diags
            .iter()
            .map(|d| d.msg.clone())
            .collect::<Vec<_>>()
            .join("; "));
    }
    let bin = rut_core::binary::encode(compiled.graph.program.as_ref().expect("no binary"));
    let prog = rut_core::binary::decode(&bin).expect("decode");
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    rut_vm::interp:: Vm::builder().program(std::rc::Rc::new(prog)).limits(limits).hooks(rut_vm::interp::HostHooks::default()).hosts(rut_vm::interp::HostRegistry::new()).build()
    .map_err(|e| format!("{e:?}"))
}

fn compile(src: &str) -> rut_driver::ProgramOutput {
    // the core surface bound as the one use: these tests
    // exercise interface satisfaction, not use discipline
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

// DEAD with parameterized trait impls: the template registry —
// `impl<T> I for T` no longer parses, so nothing registers as a
// dispatch template.

// DEAD with parameterized trait impls: the (trait, type) impl registry —
// structural satisfaction registers nothing, so there are no duplicates
// to diagnose.

// DEAD with parameterized trait impls: concrete-first shadowing of the
// template's instantiation — with no registry there is no template
// instantiation for a concrete impl to shadow.

// DEAD with parameterized trait impls: impl-head trait-argument
// substitution (parameter slots substituting beside concrete ones) —
// the impl head is gone.

// DEAD with parameterized trait impls: the v1 guard diagnosing a
// parameter nested inside an impl-head trait argument — no impl heads,
// no trait arguments.

// DEAD with parameterized trait impls: the impl-head guard diagnosing a
// parameter naming neither the target's parameters nor a type in scope —
// the head it guarded is gone.

// DEAD with parameterized trait impls: repeated impl-head parameters
// (`I<T, T>`) — the impl-head grammar that spelled them is gone.

// DEAD with parameterized trait impls: positional dispatch of repeated
// impl-head parameter spellings — died with the impl-head grammar.

// ---- the surviving dispatch laws (these tests EXECUTE) ----------------

/// a bound-parameter signature resolves: `fn use_it<C requires Calc>`
/// admits two satisfying classes, and the bound proves the widening —
/// the call dispatches to each origin's own member
#[test]
fn trait_parameter_in_the_impl_methods_signature_resolves() {
    let mut vm = boot(
        "interface Calc { fn calc(self, v: i32) -> i32; }\n\
         class Adder { n: i32 }\n\
         impl Adder { pub fn calc(self, v: i32) -> i32 { return self.n + v; } }\n\
         class Multer { n: i32 }\n\
         impl Multer { pub fn calc(self, v: i32) -> i32 { return self.n * v; } }\n\
         fn use_it<C requires Calc>(c: C) -> i32 {\n\
             let w: Calc = c;\n\
             return w.calc(3);\n\
         }\n\
         entry fn main() -> i32 {\n\
             let a = Adder { n: 4 };\n\
             let m = Multer { n: 5 };\n\
             return use_it(a) * 100 + use_it(m);\n\
         }\n",
    )
    .expect("compiles");
    let r: i32 = vm.call("main", ()).expect("runs");
    assert_eq!(r, 715, "use_it(Adder)=7 and use_it(Multer)=15 — both dispatch");
}

/// the wrapper-construction inference: `let w = JsonW(t);` binds the
/// wrapper's T from the wrapped arg (the unified-wrapper law), and each
/// instantiation widens to its own `Src<T>` slot and dispatches there
#[test]
fn widening_dispatches_to_the_instantiated_template() {
    let mut vm = boot(
        "interface Src<T> { fn get(self) -> T; }\n\
         class JsonW<T>(T);\n\
         impl<T> JsonW<T> {\n\
             pub fn get(self) -> T { return self.inner; }\n\
         }\n\
         fn grab_i(w: Src<i64>) -> i64 { return w.get(); }\n\
         fn grab_s(w: Src<str>) -> i64 { return w.get().len() as i64; }\n\
         entry fn main() -> i64 {\n\
             let t = 41 as i64;\n\
             let w = JsonW(t);\n\
             let s = JsonW(\"hi\");\n\
             return grab_i(w) + grab_s(s);\n\
         }\n",
    )
    .expect("compiles");
    let r: i64 = vm.call("main", ()).expect("runs");
    assert_eq!(r, 43, "JsonW(t) is JsonW<i64> (41); JsonW(\"hi\") is JsonW<str> (len 2)");
}

/// the same inside a GENERIC fn body: one body, two monomorphizations,
/// each dispatching to its origin's own member (the fat-ref headline,
/// spelled with interface bounds)
#[test]
fn generic_fn_body_dispatches_per_monomorphization() {
    let mut vm = boot(
        "interface Read { fn read_id(self) -> u32; }\n\
         class SA { id: u32 }\n\
         impl SA { pub fn read_id(self) -> u32 { return self.id; } }\n\
         class SB { id: u32 }\n\
         impl SB { pub fn read_id(self) -> u32 { return self.id + 1; } }\n\
         fn read_of<R requires Read>(a: R) -> u32 {\n\
             let w: Read = a;\n\
             return w.read_id();\n\
         }\n\
         entry fn main() -> u32 {\n\
             let s = SA { id: 7 };\n\
             let d = SB { id: 9 };\n\
             return read_of(s) * 100 + read_of(d);\n\
         }\n",
    )
    .expect("compiles");
    let r: u32 = vm.call("main", ()).expect("runs");
    assert_eq!(r, 710, "read_of(s)=7 and read_of(d)=10 — per-instantiation dispatch");
}

/// the itable: an interface-typed binding with TWO possible origins (a
/// runtime branch) cannot bind statically — the call dispatches through
/// the itable rows the boxing sites filled for each concrete origin
#[test]
fn vtable_row_is_filled_for_the_instantiation() {
    let mut vm = boot(
        "interface Src<T> { fn get(self) -> T; }\n\
         class IntW(i64);\n\
         impl IntW { pub fn get(self) -> i64 { return self.inner; } }\n\
         class AltW(i64);\n\
         impl AltW { pub fn get(self) -> i64 { return self.inner + 100; } }\n\
         entry fn main() -> i64 {\n\
             let a: Src<i64> = IntW(5);\n\
             let b: Src<i64> = AltW(6);\n\
             let mut pick: Src<i64> = a;\n\
             let mut pick2: Src<i64> = b;\n\
             let use_b = 6 == 6;\n\
             if (use_b) {\n\
                 pick2 = AltW(7);\n\
             }\n\
             return pick.get() + pick2.get();\n\
         }\n",
    )
    .expect("compiles");
    let r: i64 = vm.call("main", ()).expect("runs");
    assert_eq!(r, 112, "pick via the static bind (5) + pick2 via the itable (107)");
}

/// The stale-frame repro, minimized: the same generic body must compute
/// identically as a FREE fn and as an INSTANCE method of a class. The
/// method's own type arguments — spelled (`s.get<i64>(..)`) or inferred
/// from an interface-typed argument's instantiation (`s.get(boxed)`) —
/// join the class instantiation to key the frame, so the callee frame's
/// parameters and the interface call inside it type per instantiation.
/// The spelled lambda argument types its parameter from the BOUND part
/// of the expected fn shape (the placeholder hint).
#[test]
fn generic_instance_method_frames_compute_like_free_fns() {
    let mut vm = boot(
        "interface Read<T> { fn peek(self) -> u32; }\n\
         class Src2<T> { id: u32 }\n\
         impl<T> Src2<T> {\n\
             pub fn peek(self) -> u32 { return self.id; }\n\
         }\n\
         class Ctx2 { x: i32 }\n\
         class Store { cell: opaque; }\n\
         impl Store {\n\
             pub fn get<T>(self, a: Read<T>) -> T {\n\
                 let _ = a.peek();\n\
                 return opaque.downcast<T>(self.cell);\n\
             }\n\
             pub fn lift<T>(self, f: fn(Ctx2) -> T) -> T {\n\
                 return f(Ctx2 { x: 6 });\n\
             }\n\
         }\n\
         pub fn get_free<T>(st: Store, a: Read<T>) -> T {\n\
             let _ = a.peek();\n\
             return opaque.downcast<T>(st.cell);\n\
         }\n\
         entry fn main() -> i64 {\n\
             let s = Store { cell: opaque(41 as i64) };\n\
             let src = Src2<i64> { id: 3 };\n\
             let boxed: Read<i64> = src;\n\
             let m_i = s.get<i64>(src);\n\
             let m_infer = s.get(boxed);\n\
             let f_i = get_free<i64>(s, src);\n\
             let f_infer = get_free(s, boxed);\n\
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

/// deterministic generic-instance emission: the same source compiled
/// twice — two fresh contexts in this process — must encode
/// byte-identical binaries. `build_vtables` used to walk `inst_data`
/// (a std HashMap) in RandomState order, so the generic-target arm's
/// `mk_iface_inst` calls interned interface ids in a per-context random
/// order and the two binaries diverged (swapped dense receiver-type
/// ids / itable rows). The walk is canonicalized on the dense type id.
///
/// The harness choice: an in-process double compile is the strongest
/// guard available in this layout — each `Ctx` builds its own HashMaps
/// with independent hasher seeds, so the two compiles here really do
/// iterate `inst_data` in different orders.
#[test]
fn double_compile_of_one_source_emits_identical_bytes() {
    // four instantiations of one generic interface — the multi-entry
    // `inst_data` shape that makes the (pre-fix) HashMap walk order
    // observable in the emitted interface ids, function ids, and itable
    // rows
    let src = "interface Read<T> { fn peek(self) -> u32; }\n\
               class Src2<T> { id: u32 }\n\
               impl<T> Src2<T> {\n\
                   pub fn peek(self) -> u32 { return self.id; }\n\
               }\n\
               pub fn read_id<T>(a: Read<T>) -> u32 {\n\
                   return a.peek();\n\
               }\n\
               class W {\n\
                   a$: Src2<i64>;\n\
                   b$: Src2<str>;\n\
                   c$: Src2<f64>;\n\
                   d$: Src2<bool>;\n\
               }\n\
               entry fn main() -> u32 {\n\
                   let w = W { a$: Src2 { id: 1 }, b$: Src2 { id: 2 }, c$: Src2 { id: 3 }, d$: Src2 { id: 4 } };\n\
                   let a: Read<i64> = w.a$;\n\
                   let b: Read<str> = w.b$;\n\
                   let c: Read<f64> = w.c$;\n\
                   let d: Read<bool> = w.d$;\n\
                   let mut total: u32 = 0;\n\
                   total = total + read_id(a);\n\
                   total = total + read_id(b);\n\
                   total = total + read_id(c);\n\
                   total = total + read_id(d);\n\
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
    assert_eq!(r, 10, "1 + 2 + 3 + 4 through the itable rows");
}
