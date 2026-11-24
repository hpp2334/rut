//! Signature help's gate — the `signatureHelp` envelope: the verbatim
//! signature, the comma/paren depth's active parameter, the receiver
//! rule (self never surfaces), the never-wrong guards (arity
//! mismatch, ambiguity, chained receivers, f-string commas, struct
//! literals, control parens), the after-closer window (the hint lives
//! through the just-typed `)` and dies on the next keystroke), the
//! keystroke lane (the src rides every call), and the doc lines riding
//! along.

mod common;

use common::*;
use serde_json::Value;

/// the one-signature shape a help answer carries: (label, active
/// slot, the parameter labels as written)
fn slot_of(h: &Value) -> (String, Option<i64>, Vec<String>) {
    let sigs = h["signatures"]
        .as_array()
        .unwrap_or_else(|| panic!("no signatures: {h}"));
    assert_eq!(sigs.len(), 1, "{h}");
    let sig = &sigs[0];
    (
        sig["label"].as_str().unwrap().to_string(),
        h["activeParameter"].as_i64(),
        sig["parameters"]
            .as_array()
            .map(|ps| {
                ps.iter()
                    .map(|p| p["label"].as_str().expect("a simple label").to_string())
                    .collect()
            })
            .unwrap_or_default(),
    )
}

#[test]
fn free_call_active_param_first_and_middle() {
    let mut s = S::boot();
    let src = "fn hex_val(c: str, k: i32) -> i32 {\n    return k;\n}\nentry fn main() -> i32 {\n    return hex_val(\"a\", 2);\n}\n";
    let uri = "file:///t/main.rut";
    // on the first arg (the cursor inside the string literal)
    let (lbl, slot, pieces) = slot_of(&s.sig_at(uri, src, "\"a\"", 1));
    assert_eq!(lbl, "fn hex_val(c: str, k: i32) -> i32");
    assert_eq!(slot, Some(0));
    assert_eq!(pieces, ["c: str", "k: i32"], "params as written");
    // on the second arg
    let (_, slot, _) = slot_of(&s.sig_at(uri, src, "2);", 1));
    assert_eq!(slot, Some(1));
    // mid-typing: one arg in, help shows with the next slot active
    let (_, slot, _) = slot_of(&s.sig_in(uri, src, "\"a\", ", 1, 5));
    assert_eq!(slot, Some(1));
    // on the open paren itself: the first slot
    let (_, slot, _) = slot_of(&s.sig_in(uri, src, "hex_val(", 1, 7));
    assert_eq!(slot, Some(0));
}

#[test]
fn zero_param_call_shows_the_signature_without_an_active_slot() {
    let mut s = S::boot();
    let src = "fn mk() -> i32 {\n    return 1;\n}\nfn main() -> i32 {\n    let v = mk();\n    return v;\n}\n";
    let (lbl, slot, pieces) = slot_of(&s.sig_at("file:///t/main.rut", src, "mk(", 1));
    assert_eq!(lbl, "fn mk() -> i32");
    assert_eq!(slot, None);
    assert!(pieces.is_empty());
}

#[test]
fn nested_calls_answer_at_their_own_depth() {
    let mut s = S::boot();
    let src = "fn inner(a: i32) -> i32 {\n    return a;\n}\nfn outer(b: i32, c: i32) -> i32 {\n    return b + c;\n}\nentry fn main() -> i32 {\n    return outer(inner(1), 2);\n}\n";
    let uri = "file:///t/main.rut";
    // inside the inner call: inner's signature, first slot
    let (lbl, slot, _) = slot_of(&s.sig_in(uri, src, "inner(1)", 1, 6));
    assert_eq!(lbl, "fn inner(a: i32) -> i32");
    assert_eq!(slot, Some(0));
    // after the inner call (on the outer's second arg): outer's
    // signature, second slot — the inner paren netted out
    let (lbl, slot, _) = slot_of(&s.sig_at(uri, src, "2);", 1));
    assert_eq!(lbl, "fn outer(b: i32, c: i32) -> i32");
    assert_eq!(slot, Some(1));
    // on the outer's first arg (the inner call itself): outer, slot 0
    let (lbl, slot, _) = slot_of(&s.sig_at(uri, src, "inner(1)", 1));
    assert_eq!(lbl, "fn outer(b: i32, c: i32) -> i32");
    assert_eq!(slot, Some(0));
}

// ---- the after-closer window: the hint lives through the just-typed `)` ----
// every query below sits at the END of the in-flight buffer — the
// cursor right after what the user just typed — which is the position
// an editor sends on each keystroke

#[test]
fn one_param_call_answers_right_after_its_closer() {
    let mut s = S::boot();
    let src = "fn g(c: str) -> i32 {\n    return 1;\n}\nentry fn main() -> i32 {\n    let x = g()";
    let uri = "file:///t/main.rut";
    // `g()|`: the cursor right after the just-typed `)` still rides the
    // call — the verbatim signature with the first slot active (zero
    // args in, the call is being filled)
    let (lbl, slot, pieces) = slot_of(&s.sig_in(uri, src, "g()", 1, 3));
    assert_eq!(lbl, "fn g(c: str) -> i32");
    assert_eq!(slot, Some(0));
    assert_eq!(pieces, ["c: str"]);
}

#[test]
fn zero_param_call_answers_right_after_its_closer() {
    let mut s = S::boot();
    let src = "fn mk() -> i32 {\n    return 1;\n}\nentry fn main() -> i32 {\n    let v = mk()";
    // `mk()|`: the empty call answers through its closer — the
    // signature, no active slot (no parameter to ride)
    let (lbl, slot, pieces) = slot_of(&s.sig_in("file:///t/main.rut", src, "mk()", 2, 4));
    assert_eq!(lbl, "fn mk() -> i32");
    assert_eq!(slot, None);
    assert!(pieces.is_empty());
}

#[test]
fn method_call_answers_right_after_its_closer() {
    let mut s = S::boot();
    let src = "class Circle {\n    r: f64;\n}\nimpl Circle {\n    fn grown(self, k: f64) -> Circle { return self; }\n}\nfn go() -> Circle {\n    let c = Circle::new(1.0);\n    c.grown()";
    let uri = "file:///t/main.rut";
    // `c.grown()|`: the method's hint survives its closer — `self`
    // never surfaces, the recorded params put the cursor on `k`
    let (lbl, slot, pieces) = slot_of(&s.sig_in(uri, src, "c.grown()", 1, 9));
    assert_eq!(lbl, "fn grown(self, k: f64) -> Circle");
    assert_eq!(slot, Some(0), "self never surfaces as a slot");
    assert_eq!(pieces, ["k: f64"]);
}

#[test]
fn the_next_keystroke_past_the_closer_dismisses() {
    let mut s = S::boot();
    let uri = "file:///t/main.rut";
    // `g();|`: the `;` typed after the closer is out of the call
    let src = "fn g(c: str) -> i32 {\n    return 1;\n}\nentry fn main() -> i32 {\n    let x = g();";
    let h = s.sig_in(uri, src, "g();", 1, 4);
    assert!(h.is_null(), "{h}");
    // `f() |`: so is the space
    let src = "fn f(a: i32, b: i32) -> i32 {\n    return a;\n}\nentry fn main() -> i32 {\n    f() ";
    let h = s.sig_in(uri, src, "f() ", 1, 4);
    assert!(h.is_null(), "{h}");
    // `f(a, b)|`: a COMPLETE call past its closer is silent too — two
    // args for two params leaves no slot (never wrong, never clamped)
    let src = "fn f(a: i32, b: i32) -> i32 {\n    return a;\n}\nentry fn main() -> i32 {\n    let x = f(1, 2)";
    let h = s.sig_in(uri, src, "f(1, 2)", 1, 7);
    assert!(h.is_null(), "{h}");
}

#[test]
fn nested_call_answers_the_outer_right_after_its_closer() {
    let mut s = S::boot();
    let src = "fn inner(a: i32) -> i32 {\n    return a;\n}\nfn outer(b: i32, c: i32) -> i32 {\n    return b + c;\n}\nentry fn main() -> i32 {\n    let r = outer(inner(1))";
    let uri = "file:///t/main.rut";
    // `outer(inner(1))|`: the cursor right after the OUTER closer — the
    // outer call answers (one arg in, still being typed) with the slot
    // past its last arg; the inner call does not leak through
    let (lbl, slot, _) = slot_of(&s.sig_in(uri, src, "outer(inner(1))", 1, 15));
    assert_eq!(lbl, "fn outer(b: i32, c: i32) -> i32");
    assert_eq!(slot, Some(1));
}

// ---- method calls: the receiver rule ----

#[test]
fn method_call_through_a_binding_receiver() {
    let mut s = S::boot();
    let src = "class Circle {\n    r: f64;\n}\nimpl Circle {\n    fn grown(self, k: f64) -> Circle { return self; }\n}\nfn go() -> Circle {\n    let c = Circle::new(1.0);\n    return c.grown(2.0);\n}\n";
    let uri = "file:///t/main.rut";
    let (lbl, slot, pieces) = slot_of(&s.sig_at(uri, src, "2.0)", 1));
    assert_eq!(lbl, "fn grown(self, k: f64) -> Circle");
    assert_eq!(slot, Some(0), "self never surfaces as a slot");
    assert_eq!(pieces, ["k: f64"]);
    // `new` has no recorded impl fn here (and no own-surface body to
    // read params from) — no help, never a guess
    let h = s.sig_at(uri, src, "1.0);", 1);
    assert!(h.is_null(), "{h}");
}

#[test]
fn own_surface_method_means_no_help() {
    // the interface-body `m` is the receiver's own surface: FINAL
    // with no recorded params — the impl fn's signature must not
    // leak through
    let mut s = S::boot();
    let src = "struct P {\n    x: i32;\n}\ninterface Draw {\n    fn m(self, k: i32) -> i32;\n}\nimpl P {\n    pub fn m(self, k: i32) -> i32 { return k; }\n}\nfn go(d: Draw) -> i32 {\n    return d.m(1);\n}\n";
    let h = s.sig_in("file:///t/main.rut", src, "d.m(1)", 1, 5);
    assert!(h.is_null(), "{h}");
}

#[test]
fn chained_receiver_is_an_honest_miss_but_the_first_call_helps() {
    let mut s = S::boot();
    let src = "class Circle {\n    r: f64;\n}\nimpl Circle {\n    fn grown(self, k: f64) -> Circle { return self; }\n}\nfn go(c: Circle) -> f64 {\n    return c.grown(2.0).grown(3.0).r;\n}\n";
    let uri = "file:///t/main.rut";
    // the FIRST grown: a plain-ident receiver — help answers
    let h = s.sig_at(uri, src, "2.0)", 1);
    assert!(!h.is_null(), "{h}");
    // the SECOND grown: its receiver is a `)` (a chained call) — the
    // honest answer is none, never a wrong signature
    let h = s.sig_at(uri, src, "3.0)", 1);
    assert!(h.is_null(), "{h}");
}

// ---- the never-wrong guard ----

#[test]
fn too_many_arguments_means_no_help() {
    let mut s = S::boot();
    let src = "fn f(a: i32, b: i32) -> i32 {\n    return a;\n}\nentry fn main() -> i32 {\n    let x: i32 = f(1, 2, 3);\n    return x;\n}\n";
    let uri = "file:///t/main.rut";
    // three args for two params — on the third arg there is no slot
    // left: no help, never wrong help
    let h = s.sig_at(uri, src, "3);", 1);
    assert!(h.is_null(), "{h}");
    // but the first two slots still answer honestly
    let h = s.sig_at(uri, src, "1,", 1);
    assert!(!h.is_null(), "{h}");
}

#[test]
fn ambiguous_callee_stays_silent() {
    let mut s = S::boot();
    s.index_file(
        "file:///ws/other/lib.rut",
        "fn f(b: str) -> str {\n    return b;\n}\n",
    );
    let src = "fn f(a: i32) -> i32 {\n    return a;\n}\nfn main() -> i32 {\n    let x = f(1);\n    return x;\n}\n";
    let h = s.sig_in("file:///t/main.rut", src, "f(1)", 1, 2);
    assert!(h.is_null(), "{h}");
}

#[test]
fn unknown_callee_stays_silent() {
    let mut s = S::boot();
    let src = "entry fn main() -> nil {\n    mystery(1, 2);\n}\n";
    let h = s.sig_at("file:///t/main.rut", src, "1, 2", 1);
    assert!(h.is_null(), "{h}");
}

// ---- f-strings, struct literals, and control parens cannot miscount ----

#[test]
fn f_string_commas_do_not_count() {
    let mut s = S::boot();
    let src = "fn inner(a: i32, b: i32) -> i32 {\n    return a;\n}\nfn show(c: str, a: i32, b: i32) -> i32 {\n    return a;\n}\nentry fn main() -> i32 {\n    return show(f\"{inner(1, 2)}-{inner(3, 4)}\", 5, 6);\n}\n";
    let uri = "file:///t/main.rut";
    // the f-string carries four commas inside its holes (nested
    // calls) and its literal — all inside ONE token. If any of them
    // counted, the cursor on `5` would read slot 5 (out of range →
    // no help) instead of slot 1
    let (lbl, slot, _) = slot_of(&s.sig_in(uri, src, ", 5, 6", 1, 2));
    assert_eq!(lbl, "fn show(c: str, a: i32, b: i32) -> i32");
    assert_eq!(slot, Some(1));
    // the third real arg
    let (_, slot, _) = slot_of(&s.sig_in(uri, src, "5, 6", 1, 3));
    assert_eq!(slot, Some(2));
}

#[test]
fn struct_literal_fields_do_not_count_as_arguments() {
    let mut s = S::boot();
    let src = "struct Point {\n    x: f64;\n    y: f64;\n}\nfn place(p: Point, k: i32) -> f64 {\n    return p.x;\n}\nentry fn main() -> f64 {\n    return place(Point { x: 1.0, y: 2.0 }, 3);\n}\n";
    let uri = "file:///t/main.rut";
    // the literal's `y: 2.0` comma is inside `{}` — the cursor on
    // `3` reads slot 1, not slot 2
    let (_, slot, _) = slot_of(&s.sig_in(uri, src, ", 3);", 1, 2));
    assert_eq!(slot, Some(1));
    // and the cursor inside the literal itself reads slot 0 (the
    // literal is the first argument)
    let (_, slot, _) = slot_of(&s.sig_at(uri, src, "1.0, y", 1));
    assert_eq!(slot, Some(0));
}

#[test]
fn a_control_paren_does_not_hide_the_enclosing_call() {
    let mut s = S::boot();
    let src = "fn f(a: i32, b: i32) -> i32 {\n    return a;\n}\nentry fn main() -> i32 {\n    let x = f(1, if (true) { 2 } else { 3 });\n    return x;\n}\n";
    let uri = "file:///t/main.rut";
    // the cursor on the `if` head (before its paren): the innermost
    // unclosed CALL paren is f's — its `(` is skipped as a non-call
    // and help answers with slot 1
    let (lbl, slot, _) = slot_of(&s.sig_at(uri, src, "if (true)", 1));
    assert_eq!(lbl, "fn f(a: i32, b: i32) -> i32");
    assert_eq!(slot, Some(1));
}

// ---- doc comments ride along ----

#[test]
fn doc_lines_render_as_documentation() {
    let mut s = S::boot();
    let src = "// hex digit value\nfn hex_val(c: str) -> i32 {\n    return 1;\n}\nentry fn main() -> i32 {\n    return hex_val(\"a\");\n}\n";
    let h = s.sig_at("file:///t/main.rut", src, "\"a\"", 1);
    assert_eq!(h["activeParameter"], 0, "the first slot: {h}");
    assert_eq!(
        h["signatures"][0]["documentation"]["value"], "hex digit value",
        "{h}"
    );
}

// ---- the keystroke lane: the src rides every call ----

#[test]
fn the_paren_only_the_request_saw_still_answers() {
    let mut s = S::boot();
    let uri = "file:///t/main.rut";
    // `f(|)`: the provider fires the instant `(` is typed — the caret
    // sits between the parens the stored doc does not have (no analyze
    // ever ran). The src rides the request, so the query syncs FIRST
    // and answers on the CURRENT text: the stored-lane race is dead.
    let src = "fn f(a: i32) -> i32 {\n    return a;\n}\nentry fn main() -> i32 {\n    f()\n}\n";
    let (lbl, slot, _) = slot_of(&s.sig_pos(uri, src, (4, 6)));
    assert_eq!(lbl, "fn f(a: i32) -> i32");
    assert_eq!(slot, Some(0));
}

// ---- the ctor fallback lane ----

both_uri_shapes! {
#[test]
fn a_ctor_call_shows_the_ctor_signature_and_the_active_param() {
    // a capitalized callee the free-fn lane misses names a class: the
    // designated ctor answers with its verbatim impl signature
    // (`is_method = false` — a ctor takes no receiver, its params ARE
    // the call's args)
    let mut s = S::boot();
    let src = "class Logger {\n    msg: str;\n}\nimpl Logger {\n    [constructor] pub fn new(msg: str) -> Self { return Self { msg: msg }; }\n}\nentry fn main() -> nil {\n    let l = Logger(\"hello\");\n}\n";
    let uri = "file:///t/main.rut";
    let (lbl, slot, pieces) = slot_of(&s.sig_at(uri, src, "\"hello\"", 1));
    assert_eq!(lbl, "[constructor] pub fn new(msg: str) -> Self");
    assert_eq!(slot, Some(0));
    assert_eq!(pieces, ["msg: str"], "the params as written");
    // mid-typing: the ctor signature lives through the open paren
    let (_, slot, _) = slot_of(&s.sig_in(uri, src, "Logger(", 1, 7));
    assert_eq!(slot, Some(0));
}
}

both_uri_shapes! {
#[test]
fn a_newtype_call_shows_the_synthesized_mint() {
    // the mint's label is the hover's mint-line spelling (one truth,
    // `TyView::mint_sig`) and mentions the param name — the alignment
    // check holds on the synthesized shape
    let mut s = S::boot();
    let src = "class Meters(f64);\nentry fn main() -> nil {\n    let m = Meters(1.0);\n}\n";
    let uri = "file:///t/main.rut";
    let (lbl, slot, pieces) = slot_of(&s.sig_at(uri, src, "1.0", 1));
    assert_eq!(lbl, "Meters(inner: f64) -> Meters");
    assert_eq!(slot, Some(0));
    assert_eq!(pieces, ["inner: f64"]);
}
}

both_uri_shapes! {
#[test]
fn a_ctor_call_with_more_args_than_params_stays_silent() {
    // the never-wrong guard holds on the ctor lane too
    let mut s = S::boot();
    let src = "class Two {\n    a: i32;\n    b: i32;\n}\nimpl Two {\n    [constructor] pub fn mk(a: i32, b: i32) -> Self { return Self { a: a, b: b }; }\n}\nentry fn main() -> nil {\n    let t = Two(1, 2, 3);\n}\n";
    let uri = "file:///t/main.rut";
    // three args for two params — on the third there is no slot left
    let h = s.sig_at(uri, src, "3);", 1);
    assert!(h.is_null(), "no slot left: {h}");
    // but the first two slots still answer honestly
    let (lbl, slot, pieces) = slot_of(&s.sig_at(uri, src, "1,", 1));
    assert_eq!(lbl, "[constructor] pub fn mk(a: i32, b: i32) -> Self");
    assert_eq!(slot, Some(0));
    assert_eq!(pieces, ["a: i32", "b: i32"]);
}
}

both_uri_shapes! {
#[test]
fn a_no_ctor_class_callee_stays_silent() {
    // a capitalized callee naming a class with neither a designated
    // ctor nor a mint — no help, never wrong help
    let mut s = S::boot();
    let src = "class Plain {\n    n: i32;\n}\nentry fn main() -> nil {\n    let _p = Plain();\n}\n";
    let h = s.sig_in("file:///t/main.rut", src, "Plain(", 1, 5);
    assert!(h.is_null(), "{h}");
}
}
