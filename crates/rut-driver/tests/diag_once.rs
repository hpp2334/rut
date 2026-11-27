//! One site, one diagnostic: a fn's signature annotation resolves once
//! per fn, so an unknown parameter type is reported once at the
//! declaration — not once per lane that consults the signature.
//!
//! The lanes: the dead-code roots ask the trait-object question of each
//! parameter type (`program.rs`), the body compile binds the parameter
//! row (`lir/compile.rs`), and a call site reads the callee's
//! annotations as expected types (`lir/call/free.rs`). Every one of
//! them used to re-resolve the same `TyRef` and re-mint the same
//! `unknown type` diag at the same span — twice for a dead fn, three
//! times for a called one.

use rut_ast::parser::Mode;
use rut_lexer::diag::Diag;
use rut_lexer::span::Span;

fn compile(src: &str) -> rut_driver::ProgramOutput {
    rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, rut_core::binary::Surface::core(), "core".to_string())],
    )
}

fn msgs(ds: &[Diag]) -> Vec<String> {
    ds.iter().map(|d| d.msg.clone()).collect()
}

/// THE PLAN'S PIN: a private fn nothing calls, its parameter type
/// unknown. The parameter's type is the site — ONE diagnostic there,
/// however many lanes consult the signature.
#[test]
fn a_dead_fns_unknown_parameter_type_diagnoses_once() {
    let src = "fn task2(x: Nope) {}\n\
               entry fn main() {}\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(
        ds.len(),
        1,
        "the parameter's type resolves once for one fn: {ms:?}"
    );
    assert_eq!(ms[0], "unknown type `Nope`", "{ms:?}");
    let lo = src.find("Nope").expect("the parameter's type") as u32;
    assert_eq!(
        ds[0].span,
        Span::new(lo, lo + 4),
        "the diag sits at the parameter's type: {ds:?}"
    );
}

/// THE CALLED CASE IS THE SAME LAW (a necessary consequent): the call
/// site reads the callee's signature, but its failure belongs to the
/// DECLARATION — the same site, the same message, once. Three lanes
/// (the expected-type read, the unification read, the final param
/// read) all answer from one resolution.
#[test]
fn a_called_fns_unknown_parameter_type_diagnoses_once() {
    let src = "fn task2(x: Nope) {}\n\
               entry fn main() { task2(1); }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(
        ds.len(),
        1,
        "a call site does not re-report the declaration's failure: {ms:?}"
    );
    assert_eq!(ms[0], "unknown type `Nope`", "{ms:?}");
    let lo = src.find("Nope").expect("the parameter's type") as u32;
    assert_eq!(
        ds[0].span,
        Span::new(lo, lo + 4),
        "the diag sits at the parameter's type: {ds:?}"
    );
}

/// THE DOOR ANSWERS ONCE: a generic fn's env-FREE annotation names no
/// generic parameter, so the free-call lane's partial-env read and its
/// completed-env read are ONE question — the leaf is diagnosed once at
/// its declaration, never once per env.
#[test]
fn a_generic_fns_env_free_leaf_diagnoses_once() {
    let src = "fn g<T>(x: Nope, y: T) { }\n\
               entry fn main() { g(1, 2); }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(ds.len(), 1, "an env-free leaf is asked once: {ms:?}");
    assert_eq!(ms[0], "unknown type `Nope`", "{ms:?}");
    let lo = src.find("Nope").expect("the parameter's type") as u32;
    assert_eq!(
        ds[0].span,
        Span::new(lo, lo + 4),
        "the diag sits at the parameter's type: {ds:?}"
    );
}

/// THE CONTROL: an annotation that NAMES a generic parameter is
/// env-DEPENDENT — its key keeps the substitution, so each
/// instantiation is its own question. Two calls with different argument
/// types both type; a mismatch is still caught at the binding, never
/// hidden behind a first-instantiation cache hit.
#[test]
fn env_mentioning_annotations_still_resolve_per_instantiation() {
    let src = "fn id<T>(x: T) -> T { return x; }\n\
               entry fn main() {\n\
               \x20   let a: i32 = id(1);\n\
               \x20   let b: str = id(\"s\");\n\
               }\n";
    let ds = compile(src).diags;
    assert!(ds.is_empty(), "both instantiations type: {:?}", msgs(&ds));

    let bad = "fn id<T>(x: T) -> T { return x; }\n\
               entry fn main() {\n\
               \x20   let a: i32 = id(1);\n\
               \x20   let b: bool = id(\"s\");\n\
               }\n";
    let ds = compile(bad).diags;
    let ms = msgs(&ds);
    assert_eq!(
        ds.len(),
        1,
        "the second instantiation's type is live: {ms:?}"
    );
    assert_eq!(
        ms[0], "let `b` is `bool` but the initializer is `str`",
        "{ms:?}"
    );
}

/// THE ENTRY GATE MASKS ONLY ITSELF: a bad entry signature resolves
/// through the signature door and never suppresses the body pass — the
/// unknown parameter type and the body's own `let` mismatch ride ONE
/// compile. (At the mask: only the unknown type is reported.)
#[test]
fn a_bad_entry_signature_does_not_mask_the_bodys_diagnostics() {
    let src = "entry fn main(x: Nope) -> nil { let y: i32 = \"s\"; }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(
        ds.len(),
        2,
        "the signature's diag rides WITH the body's: {ms:?}"
    );
    assert!(
        ms.iter().any(|m| m == "unknown type `Nope`"),
        "the signature failure is reported: {ms:?}"
    );
    assert!(
        ms.iter()
            .any(|m| m == "let `y` is `i32` but the initializer is `str`"),
        "the body's mismatch is reported in the same compile: {ms:?}"
    );
}

/// THE CONTROL: the same body under a SOUND entry signature reports
/// the mismatch — unchanged by the mask's death.
#[test]
fn a_sound_entry_signature_still_reports_the_bodys_mismatch() {
    let src = "entry fn main(x: i32) -> nil { let y: i32 = \"s\"; }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(ds.len(), 1, "the mismatch, and nothing else: {ms:?}");
    assert_eq!(
        ms[0], "let `y` is `i32` but the initializer is `str`",
        "{ms:?}"
    );
}
