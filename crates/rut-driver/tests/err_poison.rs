//! The poison mint (the tykind-err lane): a failing type resolution
//! answers `TY_ERR` — the poison row — instead of the old `i32`
//! placeholder. The connector shape is the regression: an annotation
//! that fails to resolve beside an initializer of a real type. The
//! poison binding must not masquerade as `i32`: the false
//! ``let `resp` is `i32` but ...`` mismatch (the annotation's lie
//! colliding with the honest initializer) is gone — only the
//! resolution failure itself diagnoses. Genuine mismatches still
//! fire: the poison never silences a real disagreement.

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

/// THE CONNECTOR SHAPE (`let resp: Bogus = await r.send();`-shaped):
/// the annotation fails to resolve, the initializer is a real type.
/// The failure mints the poison row — the binding takes `<err>`, and
/// the widens door lets the initializer through (the cause is already
/// on the diag list): the false `is `i32` but` mismatch is GONE, only
/// the unknown-type diag remains.
#[test]
fn failing_annotation_mints_the_poison_not_the_i32_lie() {
    let src = "class Response {\n\
               \x20   code: i32 = 200;\n\
               }\n\
               entry fn main() {\n\
               \x20   let resp: Bogus = Response {};\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert!(
        ms.iter().any(|m| m == "unknown type `Bogus`"),
        "the resolution failure diagnoses: {ms:?}"
    );
    assert!(
        !ms.iter().any(|m| m.contains("but the initializer")),
        "the i32-lie mismatch is gone — no false follow-on: {ms:?}"
    );
    // the honest set at this stage: exactly the resolution failure
    // (a remaining follow-on would be the audit phase's tightening).
    assert_eq!(ds.len(), 1, "one diagnostic, not a cascade: {ms:?}");
    let lo = src.find("Bogus").expect("annotation") as u32;
    assert_eq!(ds[0].span, Span::new(lo, lo + 5), "{ds:?}");
}

/// A genuine mismatch still fires — the poison never silences a real
/// disagreement between a resolving annotation and its initializer.
#[test]
fn genuine_mismatch_still_fires() {
    let src = "entry fn main() {\n\
               \x20   let n: i32 = \"s\";\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(ds.len(), 1, "the mismatch, and nothing else: {ms:?}");
    assert!(
        ms[0].contains("but the initializer"),
        "the honest disagreement diagnoses: {ms:?}"
    );
}
