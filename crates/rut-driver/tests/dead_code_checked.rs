//! Dead private code is type-checked: the compile roots dropped the
//! `pub` gate, so a private fn (free or method) nothing calls still
//! reaches the fused FnCompiler walk and its bogus body diagnoses —
//! the same driver lane rut-lsp rides. Generic and trait-object-param
//! bodies stay call-site shapes (no substitution / no origins), so a
//! dead one of those still compiles unchecked, by design; a dead
//! async body needs no call site and checks like a sync one.

use rut_ast::parser::Mode;

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
         \x20   fn bump(self) -> Self { self.n += 1; return self; }\n\
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
fn a_dead_async_fn_body_is_checked() {
    // law 1: a body is call-site-shaped only when its Inst key needs a
    // call site — a generic substitution, or a trait-object parameter's
    // `iface_origins`. `async` is neither: the dead async free fn roots
    // like any non-generic private fn and its bogus call diagnoses,
    // while the generic fn and the box-param fn around it stay
    // call-site shapes (a box-param fn mints its Inst with one concrete
    // origin per box param at its call — an origin-less eager root
    // would be a second instantiation whose boxed lanes refuse), so a
    // dead one of those carries no root and diagnoses nothing
    let src = "interface Num { fn m(self) -> i32; }\n\
         class WI(i32);\n\
         impl WI { pub fn m(self) -> i32 { return self.inner; } }\n\
         class Wob { n: i32 }\n\
         impl Wob { pub fn get(self) -> i32 { return self.n; } }\n\
         fn dead_gen<T>(x: T) -> T { let w = Wob { n: 0 }; return w.add2(1); }\n\
         async fn dead_async() -> nil { let w = Wob { n: 0 }; let _ = w.add2(1); }\n\
         fn dead_boxed(r: Num) -> i32 { let w = Wob { n: 0 }; return w.add2(1); }\n\
         entry fn main() -> i32 { return 0; }\n";
    let ds = compile(src).diags;
    assert_eq!(
        ds.len(),
        1,
        "exactly one diagnostic — the generic and boxed lines stay silent: {ds:?}"
    );
    assert!(
        ds[0].msg.contains("`Wob` has no method `add2`"),
        "a dead async fn's body must be checked: {ds:?}"
    );
    // the one diagnostic sits at the async body's call
    let needle = "let _ = w.add2(1);";
    let lo = src.find(needle).expect("the async body's call") as u32;
    let hi = lo + needle.len() as u32;
    assert!(
        ds[0].span.lo >= lo && ds[0].span.hi <= hi,
        "the diagnostic must sit at the async body's call: {:?} vs {lo}..{hi}",
        ds[0].span
    );
}

#[test]
fn a_valid_dead_async_fn_compiles() {
    // the control: a well-formed private async fn nothing calls roots
    // and weaves, and stays clean
    let out = compile(
        "async fn idle(n: i32) -> i32 { return n; }\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        out.diags.is_empty(),
        "a valid dead async fn must compile clean: {:?}",
        out.diags
    );
}
