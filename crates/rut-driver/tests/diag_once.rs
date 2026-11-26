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
