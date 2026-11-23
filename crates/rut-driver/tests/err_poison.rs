//! The poison mint (the tykind-err lane): a failing type resolution
//! answers `TY_ERR` — the poison row — instead of the old `i32`
//! placeholder. The connector shape is the regression: an annotation
//! that fails to resolve beside an initializer of a real type. The
//! poison binding must not masquerade as `i32`: the false
//! ``let `resp` is `i32` but ...`` mismatch (the annotation's lie
//! colliding with the honest initializer) is gone — only the
//! resolution failure itself diagnoses. Genuine mismatches still
//! fire: the poison never silences a real disagreement.
//!
//! The quiet audit (this file's second half): Err is SILENT in every
//! lane. The cause is always already on the diag list when a poison
//! row exists, so every later lane that consults the type stays quiet
//! — a second diagnostic touching an Err would read as a false error
//! (the poisoned-names law mirrored for types). Uses of a poison
//! binding (member calls, field reads), poison annotations in closure
//! params / cast wants / `is` tests / entry signatures — each is
//! quiet beyond the one unknown-type diag.

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

// ---- the quiet audit: Err is silent in every lane ----

/// The CONNECTOR'S RESIDUAL (phase 27's known follow-on, now silent):
/// a USE of the poison binding. `resp.code` on the `<err>`-typed
/// binding used to diagnose `` `<err>` has no field `code` `` —
/// receiver silence (the audit): a field read on the poison row
/// resolves nothing, diagnoses nothing. The connector answers EXACTLY
/// ONE diag: the resolution failure.
#[test]
fn poisoned_binding_field_read_is_silent() {
    let src = "class Response {\n\
               \x20   code: i32 = 200;\n\
               }\n\
               entry fn main() {\n\
               \x20   let resp: Bogus = Response {};\n\
               \x20   let c = resp.code;\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(ds.len(), 1, "the residual is gone — one diag: {ms:?}");
    assert_eq!(ms[0], "unknown type `Bogus`", "{ms:?}");
}

/// A member CALL on the poison binding: the same receiver silence —
/// the dispatch lanes (union capability, builtin impls, the prim
/// contracts, the no-method residue) never speak on `<err>`.
#[test]
fn poisoned_binding_method_call_is_silent() {
    let src = "class Response {\n\
               \x20   code: i32 = 200;\n\
               }\n\
               entry fn main() {\n\
               \x20   let resp: Bogus = Response {};\n\
               \x20   resp.ping();\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(ds.len(), 1, "no `<err>` has no method` false error: {ms:?}");
    assert_eq!(ms[0], "unknown type `Bogus`", "{ms:?}");
}

/// A poison annotation in a closure param: the annotation diagnoses
/// once, the param binds the poison row, the body and the lambda's
/// own shape checks stay silent.
#[test]
fn poisoned_closure_param_is_quiet() {
    let src = "entry fn main() {\n\
               \x20   let f = fn (x: Bogus) { x };\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(ds.len(), 1, "one diag, no follow-on: {ms:?}");
    assert_eq!(ms[0], "unknown type `Bogus`", "{ms:?}");
}

/// A poison `as`-cast: both the operand (a poison-typed binding) and
/// the want (a failing annotation spelling) stay silent — the numeric
/// verdicts against `<err>` never fire.
#[test]
fn poisoned_cast_is_quiet() {
    let src = "entry fn main() {\n\
               \x20   let x: Bogus = 5;\n\
               \x20   let n = x as i32;\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(ds.len(), 1, "the cast on `<err>` is silent: {ms:?}");
    assert_eq!(ms[0], "unknown type `Bogus`", "{ms:?}");
}

/// A poison `is`-test: a poison receiver rides the runtime identity
/// test (quiet), a failing RHS spelling folds to the quiet miss —
/// neither the by-value verdict nor the per-member dispatch verdict
/// fires on `<err>`.
#[test]
fn poisoned_is_test_is_quiet() {
    let src = "entry fn main() {\n\
               \x20   let x: Bogus = 5;\n\
               \x20   if (x is i32) {\n\
               \x20       let y = 1;\n\
               \x20   }\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(ds.len(), 1, "the `is` on `<err>` is silent: {ms:?}");
    assert_eq!(ms[0], "unknown type `Bogus`", "{ms:?}");
}

/// A poison entry-fn signature: the failing annotation diagnoses once;
/// the crossing check (params and return) stays silent — no
/// "only primitives ... cross the host boundary" verdict on `<err>`.
#[test]
fn poisoned_entry_signature_skips_the_crossing_verdict() {
    let src = "entry fn main(q: Bogus) {\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(ds.len(), 1, "no crossing follow-on on `<err>`: {ms:?}");
    assert_eq!(ms[0], "unknown type `Bogus`", "{ms:?}");
}

/// A REAL signature still crosses the crossing verdict (the same
/// guard never silences an honest refusal).
#[test]
fn real_entry_signature_still_refuses_the_crossing() {
    let src = "class Wob { n: i32 }\n\
               entry fn main(w: Wob) {\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert!(
        ms.iter()
            .any(|m| m.contains("cross the host boundary") && m.contains("`w` is `Wob`")),
        "the honest crossing refusal fires: {ms:?}"
    );
}

/// The regression pins: REAL types keep their honest misses — the
/// quiet audit never over-silences. No `<err>` text anywhere.
#[test]
fn real_type_field_and_method_misses_still_fire() {
    let src = "class Wob { n: i32 }\n\
               impl Wob { pub fn get(self) -> i32 { return self.n; } }\n\
               entry fn main() {\n\
               \x20   let w = Wob { n: 1 };\n\
               \x20   let a = w.nope;\n\
               \x20   let b = w.miss();\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert!(
        ms.iter().any(|m| m.contains("`Wob` has no field `nope`")),
        "the real no-field pin fires: {ms:?}"
    );
    assert!(
        ms.iter().any(|m| m.contains("`Wob` has no method `miss`")),
        "the real no-method pin fires: {ms:?}"
    );
    assert!(
        !ms.iter().any(|m| m.contains("<err>")),
        "no poison text leaks into real-type diags: {ms:?}"
    );
}

// ---- more audited lanes, one pin each ----

/// The numeric/bool verdicts of `-`/`!` stay silent on a poison
/// operand.
#[test]
fn poisoned_unary_is_quiet() {
    let src = "entry fn main() {\n\
               \x20   let x: Bogus = 5;\n\
               \x20   let a = -x;\n\
               \x20   let b = !x;\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(ds.len(), 1, "unary on `<err>` is silent: {ms:?}");
}

/// A fn-value call on a poison-typed callee: the callable verdict on
/// `<err>` stays silent.
#[test]
fn poisoned_fn_value_call_is_silent() {
    let src = "entry fn main() {\n\
               \x20   let f: Bogus = 5;\n\
               \x20   f(1);\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(ds.len(), 1, "the callee verdict is silent: {ms:?}");
}

/// An index read on a poison indexee: the sequence verdict stays
/// silent (the write form is the same guard in the assign lane).
#[test]
fn poisoned_index_read_is_silent() {
    let src = "entry fn main() {\n\
               \x20   let v: Bogus = 5;\n\
               \x20   let w = v[0];\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(ds.len(), 1, "indexing on `<err>` is silent: {ms:?}");
}

/// A poison argument unifying a generic: the binder takes the poison
/// row (no conflict, no false cannot-infer); the call's answer flows
/// out as poison, universally compatible.
#[test]
fn poisoned_generic_unification_binds_quietly() {
    let src = "fn id<T>(x: T) -> T { return x; }\n\
               entry fn main() {\n\
               \x20   let x: Bogus = 5;\n\
               \x20   let y = id(x);\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(ds.len(), 1, "unification with `<err>` is silent: {ms:?}");
}

/// A poison annotation on a destructuring let degrades to the
/// INITIALIZER's own tuple shape — the bindings stay real, the uses
/// downstream compile honestly.
#[test]
fn poisoned_destructuring_takes_the_initializer_shape() {
    let src = "entry fn main() {\n\
               \x20   let (a, b): Bogus = (1, 2);\n\
               \x20   let c = a + b;\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(ds.len(), 1, "one diag, the tuple is real: {ms:?}");
    assert_eq!(ms[0], "unknown type `Bogus`", "{ms:?}");
}

/// A REAL method fed a poison argument: the receiver's dispatch stays
/// honest, the argument's poison widens quietly (no false mismatch).
#[test]
fn real_method_with_poison_arg_stays_honest() {
    let src = "class Wob { n: i32 }\n\
               impl Wob { pub fn get(self, k: i32) -> i32 { return self.n + k; } }\n\
               entry fn main() {\n\
               \x20   let x: Bogus = 5;\n\
               \x20   let w = Wob { n: 1 };\n\
               \x20   let a = w.get(x);\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(ds.len(), 1, "the arg widens quietly: {ms:?}");
}
