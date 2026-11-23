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
//!
//! The Step-4 closer completes the plan's family list: the unknown fn
//! RET annotation (no "tail type mismatch" follow-on), the tuple's
//! no-collapse regression (one poison elem — the tuple keeps its
//! shape and still destructures), the unknown container head (the
//! failure cascades nowhere — args and uses stay quiet), and the
//! pack gate (a diag-carrying unit refuses the pack — no `Err` ever
//! encodes into bundle bytes).

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
/// once, the param binds the poison row, and the body, the lambda's
/// own shape checks, AND the call with args (the callable verdict and
/// the argument unification against the poison param) stay silent.
#[test]
fn poisoned_closure_param_is_quiet() {
    let src = "entry fn main() {\n\
               \x20   let f = fn (x: Bogus) { x };\n\
               \x20   let r = f(1);\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(
        ds.len(),
        1,
        "one diag, no follow-on — the call with args included: {ms:?}"
    );
    assert_eq!(ms[0], "unknown type `Bogus`", "{ms:?}");
}

/// A poison `as`-cast: the operand (a poison-typed binding) rides the
/// numeric cast silently — the numeric verdicts against `<err>` never
/// fire. (The `as` grammar accepts only the ten numeric primitive
/// names as its want — `crates/rut-ast/src/parser/expr.rs`'s
/// `is_numeric_prim` — so a cast WANT can never itself be an unknown
/// spelling; the unknown-want quietness is the `is`-test's pin below.)
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

/// A poison `is`-test, both sides of the family: a poison receiver
/// rides the runtime identity test (quiet); an unknown WANT spelling
/// on a real receiver folds to the quiet miss. Neither the by-value
/// verdict nor the per-member dispatch verdict fires — the want's own
/// resolution failure is the one diag.
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
    // the unknown WANT: the receiver is real, the spelling fails —
    // the test folds to the quiet miss, the failure itself is the one
    // diag
    let src = "entry fn main() {\n\
               \x20   let y = 5;\n\
               \x20   if (y is Bogus) {\n\
               \x20       let z = 1;\n\
               \x20   }\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(
        ds.len(),
        1,
        "the unknown want folds to the quiet miss: {ms:?}"
    );
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

// ---- review round 1: the struct-literal dispatch tail ----

/// The bare-generic literal: `Wrap { .. }` spells a generic head
/// WITHOUT its arguments — the spelling fails honestly (one diag),
/// the poison row it mints must not fall through the struct-literal
/// dispatch into `` `<err>` is not a struct/class of this module ``
/// (the extern-literal entry verdict quoting the poison). Exactly ONE
/// diag, no `<err>` text anywhere.
#[test]
fn bare_generic_literal_mints_no_err_leak() {
    let src = "class Wrap<T> { v: ?T = nil; }\n\
               entry fn main() {\n\
               \x20   let w = Wrap { v: 5 };\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(
        ds.len(),
        1,
        "the spelling failure, and nothing else: {ms:?}"
    );
    assert!(
        ms[0].contains("type argument"),
        "the generic-spelling failure diagnoses: {ms:?}"
    );
    assert!(
        !ms.iter().any(|m| m.contains("<err>")),
        "no poison text: {ms:?}"
    );
}

/// The composite: the connector shape followed by the bare-generic
/// literal — each site's own honest failure, still no `<err>` leak
/// from the dispatch tail.
#[test]
fn connector_composite_stays_quiet() {
    let src = "class Wrap<T> { v: ?T = nil; }\n\
               entry fn main() {\n\
               \x20   let x: Bogus = 5;\n\
               \x20   let w = Wrap { v: 5 };\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(
        ds.len(),
        2,
        "two honest failures (the annotation + the spelling): {ms:?}"
    );
    assert!(
        !ms.iter().any(|m| m.contains("<err>")),
        "no poison text: {ms:?}"
    );
}

/// The bounded-binder pin (review round 1's minor, now the guarded
/// law): a poison argument bound to a `requires`-bounded generic
/// defers admission — exactly ONE diag, no false
/// "`<err>` does not satisfy".
#[test]
fn bounded_binder_defers_admission_on_a_poison_arg() {
    let src = "interface Hash { fn hash(self) -> u64; }\n\
               struct Token { v: i32 }\n\
               impl Token { pub fn hash(self) -> u64 { return 9; } }\n\
               fn p<T requires Hash>(x: T) -> u64 { return x.hash(); }\n\
               entry fn main() {\n\
               \x20   let x: Bogus = 5;\n\
               \x20   let q = p(x);\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(ds.len(), 1, "admission defers on `<err>`: {ms:?}");
    assert_eq!(ms[0], "unknown type `Bogus`", "{ms:?}");
}

/// The same bound still rejects a REAL non-satisfying type — the
/// deferral never over-silences.
#[test]
fn bounded_binder_still_rejects_a_real_non_satisfier() {
    let src = "interface Hash { fn hash(self) -> u64; }\n\
               struct Token { v: i32 }\n\
               impl Token { pub fn hash(self) -> u64 { return 9; } }\n\
               fn p<T requires Hash>(x: T) -> u64 { return x.hash(); }\n\
               entry fn main() {\n\
               \x20   let q = p(\"s\");\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert!(
        ms.iter()
            .any(|m| m.contains("`str` does not satisfy `Hash`")),
        "the honest bound rejection fires: {ms:?}"
    );
}

// ---- the Step-4 closer: the family's remaining shapes ----

/// The unknown fn RET annotation (the plan's tail lane): the return
/// spelling fails once per site, and BOTH tail lanes stay silent —
/// the nil-tail verdict is guarded, the value tail rides `widens` on
/// `<err>`. No "tail type mismatch" follow-on exists.
#[test]
fn unknown_fn_ret_annotation_mints_no_tail_follow_on() {
    let src = "fn f() -> Bogus { return 1; }\n\
               fn g() -> Bogus { }\n\
               entry fn main() {\n\
               \x20   let x = f();\n\
               \x20   let y = g();\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(
        ds.len(),
        2,
        "one resolution failure per spelling, nothing from the tails: {ms:?}"
    );
    assert!(
        ms.iter().all(|m| m == "unknown type `Bogus`"),
        "only the annotations diagnose: {ms:?}"
    );
    assert!(
        !ms.iter().any(|m| m.contains("tail type mismatch")),
        "no tail follow-on: {ms:?}"
    );
}

/// The no-collapse law's regression (the plan's tuple shape): a tuple
/// EXPRESSION with one poison element keeps its structure — the type
/// is `(err, i32)`, poison at the leaf — and STILL destructures: the
/// bindings are real, the uses compile honestly. Exactly the one
/// unknown-type diag, no poison text anywhere.
#[test]
fn poisoned_tuple_expr_still_destructures() {
    let src = "entry fn main() -> i64 {\n\
               \x20   let x: Bogus = 5;\n\
               \x20   let t = (x, 2);\n\
               \x20   let (a, b) = t;\n\
               \x20   return a + b;\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(
        ds.len(),
        1,
        "the tuple keeps its shape and destructures: {ms:?}"
    );
    assert_eq!(ms[0], "unknown type `Bogus`", "{ms:?}");
    assert!(
        !ms.iter().any(|m| m.contains("<err>")),
        "no poison text: {ms:?}"
    );
}

/// The plan's `Vec<Bogus>` spelling, in THIS harness (no std `Vec`
/// mounts here): the HEAD is the unresolvable name — one diag names
/// it, and the failure cascades NOWHERE. The generic args stay
/// undiagnosed (no second `unknown type` for the elem spelling), the
/// binding is poison, and its uses stay silent.
#[test]
fn unknown_container_head_cascades_nowhere() {
    let src = "entry fn main() {\n\
               \x20   let v: Vec<Bogus> = nil;\n\
               \x20   let a = v;\n\
               }\n";
    let ds = compile(src).diags;
    let ms = msgs(&ds);
    assert_eq!(ds.len(), 1, "the head's failure, and nothing else: {ms:?}");
    assert_eq!(ms[0], "unknown type `Vec`", "{ms:?}");
    assert!(
        !ms.iter().any(|m| m.contains("Bogus")),
        "the args stay quiet — no cascade into the elem: {ms:?}"
    );
}

/// The pack/export gate (the no-`Err`-encodes law): a unit carrying
/// any diagnostic refuses the pack — the poison row can never reach
/// bundle bytes. The gate is the compile arm in the pack lane
/// (`crates/rut-driver/src/pack/mod.rs`):
/// `!units.diags.is_empty()` answers the refusal before any binary
/// is emitted.
#[test]
fn pack_refuses_a_diag_carrying_unit() {
    let d = std::env::temp_dir().join(format!("rut-err-poison-pack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("rut.jsonc"), r#"{"name": "poison"}"#).unwrap();
    std::fs::write(
        d.join("src/mod.rut"),
        "entry fn main() {\n    let n: Bogus = 10;\n}\n",
    )
    .unwrap();
    let err = rut_native::pack_dir(&d).expect_err("a diag-carrying unit refuses the pack");
    let text = err.to_string();
    assert!(
        text.contains("pack: unknown type `Bogus`"),
        "the refusal names the unit's diag: {text}"
    );
}
