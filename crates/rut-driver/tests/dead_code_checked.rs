//! Dead private code is type-checked: the compile roots dropped the
//! `pub` gate, so a private fn (free or method) nothing calls still
//! reaches the fused FnCompiler walk and its bogus body diagnoses —
//! the same driver lane rut-lsp rides. Generic and async bodies stay
//! call-site shapes (no substitution / weave without a call), so a
//! dead one of those still compiles unchecked, by design.

use rut_parser::Mode;

fn compile(src: &str) -> rut_driver::ProgramOutput {
    // core + pouch bound as the uses; these tests exercise root
    // selection, not use discipline
    let collection = rut_core::binary::Surface::default();
    rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[
            (2, rut_core::binary::Surface::core(), "core".to_string()),
            (3, collection, "collection".to_string()),
        ],
    )
}

fn diags_of(src: &str) -> Vec<String> {
    compile(src).diags.iter().map(|d| d.msg.clone()).collect()
}

#[test]
fn dead_private_free_fn_calling_a_missing_method_diagnoses() {
    // the repro this phase exists for: a private fn whose body calls a
    // nonexistent method never ran through the monomorphization queue —
    // now it is a root, so the bogus call diagnoses (a local class, not
    // std, keeps the test dep-free)
    let ds = diags_of(
        "class Wob { n: i32 }\n\
         impl Wob { pub fn get(self) -> i32 { return self.n; } }\n\
         fn dead(w: Wob) -> i32 { return w.add2(1); }\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`Wob` has no method `add2`")),
        "a dead private fn's body must be checked: {ds:?}"
    );
}

#[test]
fn dead_private_class_method_body_is_checked() {
    // a private method nothing calls: its body now compiles too, so a
    // return-type mismatch inside it diagnoses
    let ds = diags_of(
        "class Counter { n: i32 }\n\
         impl Counter {\n\
         \x20   pub fn new() -> Self { return Counter { n: 0 }; }\n\
         \x20   fn tally(self) -> i32 { return \"nope\"; }\n\
         }\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("return type mismatch")),
        "a dead private method's body must be checked: {ds:?}"
    );
}

#[test]
fn valid_dead_private_code_compiles_clean() {
    // the control: real dead private code must not start diagnosing —
    // only the bogus bodies do
    let out = compile(
        "class Counter { n: i32 }\n\
         impl Counter {\n\
         \x20   pub fn new() -> Self { return Counter { n: 0 }; }\n\
         \x20   fn bump(mut self) -> Self { self.n += 1; return self; }\n\
         }\n\
         enum Dial { Low, High }\n\
         impl Dial {\n\
         \x20   fn flipped(self) -> Dial {\n\
         \x20       return when (self) { Dial::Low -> Dial::High, Dial::High -> Dial::Low };\n\
         \x20   }\n\
         }\n\
         fn helper(c: Counter) -> i32 { return c.bump().n; }\n\
         fn unused_dial() -> Dial { return Dial::Low; }\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        out.diags.is_empty(),
        "valid dead private code must stay clean: {:?}",
        out.diags
    );
}

#[test]
fn dead_generic_and_async_fns_stay_unchecked() {
    // the call-site-shaped law is unchanged: a generic body compiles
    // only under a substitution, an async body only through the weave,
    // and a fn with trait-object parameters only at its call (the call
    // mints its Inst with one concrete origin per box param — an
    // origin-less eager root would be a second instantiation whose
    // boxed lanes refuse), so a dead one of those carries no root and
    // its bogus content diagnoses nothing (checked at its real call
    // sites)
    let ds = diags_of(
        "interface Num { fn m(self) -> i32; }\n\
         class WI(i32);\n\
         impl WI { pub fn m(self) -> i32 { return self.inner; } }\n\
         class Wob { n: i32 }\n\
         impl Wob { pub fn get(self) -> i32 { return self.n; } }\n\
         fn dead_gen<T>(x: T) -> T { let w = Wob { n: 0 }; return w.add2(1); }\n\
         async fn dead_async() -> nil { let w = Wob { n: 0 }; let _ = w.add2(1); }\n\
         fn dead_boxed(r: Num) -> i32 { let w = Wob { n: 0 }; return w.add2(1); }\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.is_empty(),
        "generic/async/box-param bodies stay call-site shapes — no root, no diag: {ds:?}"
    );
}
