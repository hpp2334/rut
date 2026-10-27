use ls_types::{Documentation, ParameterLabel, SignatureHelp};
use rut_ast::ast::Ast;
use rut_lexer::lexer::{lex, normalize};
use rut_lexer::token::Token;
use rut_lsp::hover::DefIndex;
use rut_lsp::signature_help::*;
use rut_parser::{parse, Mode};

const DOC: &str = "file:///t/main.rut";

struct Doc {
    src: String,
    toks: Vec<Token>,
    ast: Ast,
    index: DefIndex,
}

fn doc(src: &str) -> Doc {
    let src = normalize(src);
    let (toks, _) = lex(&src);
    let (ast, _) = parse(&src, Mode::Impl);
    let mut index = rut_lsp::hover::index(&src, &ast, &toks);
    index.origin = DOC.to_string();
    Doc {
        src,
        toks,
        ast,
        index,
    }
}

fn help(d: &Doc, pos: u32, extra: &[DefIndex]) -> Option<SignatureHelp> {
    let idxs: Vec<&DefIndex> = std::iter::once(&d.index).chain(extra.iter()).collect();
    let ctx = Ctx {
        toks: &d.toks,
        ast: &d.ast,
        idxs: &idxs,
    };
    signature_help(&ctx, pos)
}

/// byte offset of the `n`th (1-based) occurrence of `needle`
fn at(src: &str, needle: &str, occurrence: usize) -> u32 {
    let mut from = 0;
    for _ in 0..occurrence {
        match src[from..].find(needle) {
            Some(i) => from += i + needle.len(),
            None => panic!("needle {needle:?} not found (occurrence {occurrence})"),
        }
    }
    (from - needle.len()) as u32
}

fn slot_of(h: &SignatureHelp) -> (String, Option<u32>, Vec<String>) {
    assert_eq!(h.signatures.len(), 1, "{h:?}");
    let sig = &h.signatures[0];
    (
        sig.label.clone(),
        h.active_parameter,
        sig.parameters
            .as_ref()
            .map(|ps| {
                ps.iter()
                    .map(|p| match &p.label {
                        ParameterLabel::Simple(s) => s.clone(),
                        _ => panic!("label offsets not produced"),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    )
}

// ---- free calls: the active parameter at first/middle positions ----

#[test]
fn free_call_active_param_first_and_middle() {
    let src = [
        "fn hex_val(c: str, k: i32) -> i32 {",
        "    return k;",
        "}",
        "entry fn main() -> i32 {",
        "    return hex_val(\"a\", 2);",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    // on the first arg (the cursor inside the string literal)
    let (label, slot, pieces) = slot_of(&help(&d, at(&d.src, "\"a\"", 1), &[]).expect("help"));
    assert_eq!(label, "fn hex_val(c: str, k: i32) -> i32");
    assert_eq!(slot, Some(0));
    assert_eq!(pieces, ["c: str", "k: i32"], "params as written");
    // on the second arg
    let (_, slot, _) = slot_of(&help(&d, at(&d.src, "2);", 1), &[]).expect("help"));
    assert_eq!(slot, Some(1));
    // mid-typing: one arg in, help shows with the next slot active
    let (_, slot, _) = slot_of(&help(&d, at(&d.src, "\"a\", ", 1) + 5, &[]).expect("help"));
    assert_eq!(slot, Some(1));
    // on the open paren itself: the first slot
    let (_, slot, _) = slot_of(&help(&d, at(&d.src, "hex_val(", 1) + 7, &[]).expect("help"));
    assert_eq!(slot, Some(0));
}

#[test]
fn zero_param_call_shows_the_signature_without_an_active_slot() {
    let src = "fn mk() -> i32 {\n    return 1;\n}\nfn main() -> i32 {\n    let v = mk();\n    return v;\n}\n";
    let d = doc(src);
    let (label, slot, pieces) = slot_of(&help(&d, at(&d.src, "mk(", 1) + 2, &[]).expect("help"));
    assert_eq!(label, "fn mk() -> i32");
    assert_eq!(slot, None);
    assert!(pieces.is_empty());
}

#[test]
fn nested_calls_answer_at_their_own_depth() {
    let src = [
        "fn inner(a: i32) -> i32 {",
        "    return a;",
        "}",
        "fn outer(b: i32, c: i32) -> i32 {",
        "    return b + c;",
        "}",
        "entry fn main() -> i32 {",
        "    return outer(inner(1), 2);",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    // inside the inner call: inner's signature, first slot
    let (label, slot, _) = slot_of(&help(&d, at(&d.src, "inner(1)", 1) + 6, &[]).expect("help"));
    assert_eq!(label, "fn inner(a: i32) -> i32");
    assert_eq!(slot, Some(0));
    // after the inner call (on the outer's second arg): outer's
    // signature, second slot — the inner paren netted out
    let (label, slot, _) = slot_of(&help(&d, at(&d.src, "2);", 1), &[]).expect("help"));
    assert_eq!(label, "fn outer(b: i32, c: i32) -> i32");
    assert_eq!(slot, Some(1));
    // on the outer's first arg (the inner call itself): outer, slot 0
    let (label, slot, _) = slot_of(&help(&d, at(&d.src, "inner(1)", 1), &[]).expect("help"));
    assert_eq!(label, "fn outer(b: i32, c: i32) -> i32");
    assert_eq!(slot, Some(0));
}

// ---- method calls: the receiver rule ----

#[test]
fn method_call_through_a_binding_receiver() {
    let src = [
        "class Circle {",
        "    r: f64;",
        "}",
        "impl Circle {",
        "    fn grown(self, k: f64) -> Circle { return self; }",
        "}",
        "fn go() -> Circle {",
        "    let c = Circle::new(1.0);",
        "    return c.grown(2.0);",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    let (label, slot, pieces) = slot_of(&help(&d, at(&d.src, "2.0)", 1), &[]).expect("help"));
    assert_eq!(label, "fn grown(self, k: f64) -> Circle");
    assert_eq!(slot, Some(0), "self never surfaces as a slot");
    assert_eq!(pieces, ["k: f64"]);
    // `new` has no recorded impl fn here (and no own-surface body to
    // read params from) — the phase-3 law: no help, never a guess
    assert!(help(&d, at(&d.src, "1.0);", 1), &[]).is_none());
}

#[test]
fn own_surface_method_means_no_help() {
    // the interface-body `m` is the receiver's own surface: FINAL
    // with no recorded params — the impl fn's signature must not
    // leak through
    let src = [
        "struct P {",
        "    x: i32;",
        "}",
        "interface Draw {",
        "    fn m(self, k: i32) -> i32;",
        "}",
        "impl P {",
        "    pub fn m(self, k: i32) -> i32 { return k; }",
        "}",
        "fn go(d: Draw) -> i32 {",
        "    return d.m(1);",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    assert!(help(&d, at(&d.src, "d.m(1)", 1) + 5, &[]).is_none());
}

#[test]
fn chained_receiver_is_an_honest_miss_but_the_first_call_helps() {
    let src = [
        "class Circle {",
        "    r: f64;",
        "}",
        "impl Circle {",
        "    fn grown(self, k: f64) -> Circle { return self; }",
        "}",
        "fn go(c: Circle) -> f64 {",
        "    return c.grown(2.0).grown(3.0).r;",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    // the FIRST grown: a plain-ident receiver — help answers
    assert!(help(&d, at(&d.src, "2.0)", 1), &[]).is_some());
    // the SECOND grown: its receiver is a `)` (a chained call) — the
    // AST the chained rules need is not trustworthy mid-typing, so
    // the honest answer is none, never a wrong signature
    assert!(help(&d, at(&d.src, "3.0)", 1), &[]).is_none());
}

// ---- the never-wrong guard ----

#[test]
fn too_many_arguments_means_no_help() {
    let src = [
        "fn f(a: i32, b: i32) -> i32 {",
        "    return a;",
        "}",
        "entry fn main() -> i32 {",
        "    let x: i32 = f(1, 2, 3);",
        "    return x;",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    // three args for two params — on the third arg there is no slot
    // left: no help, never wrong help
    assert!(help(&d, at(&d.src, "3);", 1), &[]).is_none());
    // but the first two slots still answer honestly
    assert!(help(&d, at(&d.src, "1,", 1), &[]).is_some());
}

#[test]
fn ambiguous_callee_stays_silent() {
    let src = "fn f(a: i32) -> i32 {\n    return a;\n}\nfn main() -> i32 {\n    let x = f(1);\n    return x;\n}\n";
    let d = doc(src);
    let mut foreign = doc("fn f(b: str) -> str {\n    return b;\n}\n");
    foreign.index.origin = "file:///ws/other/lib.rut".to_string();
    let extra = [foreign.index];
    assert!(help(&d, at(&d.src, "f(1)", 1) + 2, &extra).is_none());
}

#[test]
fn unknown_callee_stays_silent() {
    let src = "entry fn main() -> nil {\n    mystery(1, 2);\n}\n";
    let d = doc(src);
    assert!(help(&d, at(&d.src, "1, 2", 1), &[]).is_none());
}

// ---- f-strings, struct literals, and control parens cannot miscount ----

#[test]
fn f_string_commas_do_not_count() {
    let src = [
        "fn inner(a: i32, b: i32) -> i32 {",
        "    return a;",
        "}",
        "fn show(c: str, a: i32, b: i32) -> i32 {",
        "    return a;",
        "}",
        "entry fn main() -> i32 {",
        "    return show(f\"{inner(1, 2)}-{inner(3, 4)}\", 5, 6);",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    // the f-string carries four commas inside its holes (nested
    // calls) and its literal — all inside ONE token. If any of them
    // counted, the cursor on `5` would read slot 5 (out of range →
    // no help) instead of slot 1
    let (label, slot, _) = slot_of(&help(&d, at(&d.src, ", 5, 6", 1) + 2, &[]).expect("help"));
    assert_eq!(label, "fn show(c: str, a: i32, b: i32) -> i32");
    assert_eq!(slot, Some(1));
    // the third real arg
    let (_, slot, _) = slot_of(&help(&d, at(&d.src, "5, 6", 1) + 3, &[]).expect("help"));
    assert_eq!(slot, Some(2));
}

#[test]
fn struct_literal_fields_do_not_count_as_arguments() {
    let src = [
        "struct Point {",
        "    x: f64;",
        "    y: f64;",
        "}",
        "fn place(p: Point, k: i32) -> f64 {",
        "    return p.x;",
        "}",
        "entry fn main() -> f64 {",
        "    return place(Point { x: 1.0, y: 2.0 }, 3);",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    // the literal's `y: 2.0` comma is inside `{}` — the cursor on
    // `3` reads slot 1, not slot 2
    let (_, slot, _) = slot_of(&help(&d, at(&d.src, ", 3);", 1) + 2, &[]).expect("help"));
    assert_eq!(slot, Some(1));
    // and the cursor inside the literal itself reads slot 0 (the
    // literal is the first argument)
    let (_, slot, _) = slot_of(&help(&d, at(&d.src, "1.0, y", 1), &[]).expect("help"));
    assert_eq!(slot, Some(0));
}

#[test]
fn a_control_paren_does_not_hide_the_enclosing_call() {
    let src = [
        "fn f(a: i32, b: i32) -> i32 {",
        "    return a;",
        "}",
        "entry fn main() -> i32 {",
        "    let x = f(1, if (true) { 2 } else { 3 });",
        "    return x;",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    // the cursor on the `if` head (before its paren): the innermost
    // unclosed CALL paren is f's — its `(` is skipped as a non-call
    // and help answers with slot 1
    let (label, slot, _) = slot_of(&help(&d, at(&d.src, "if (true)", 1), &[]).expect("help"));
    assert_eq!(label, "fn f(a: i32, b: i32) -> i32");
    assert_eq!(slot, Some(1));
}

// ---- doc comments ride along ----

#[test]
fn doc_lines_render_as_documentation() {
    let src = [
        "// hex digit value",
        "fn hex_val(c: str) -> i32 {",
        "    return 1;",
        "}",
        "entry fn main() -> i32 {",
        "    return hex_val(\"a\");",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    let h = help(&d, at(&d.src, "\"a\"", 1), &[]).expect("help");
    assert_eq!(h.active_parameter, Some(0));
    match &h.signatures[0].documentation {
        Some(Documentation::MarkupContent(m)) => assert_eq!(m.value, "hex digit value"),
        other => panic!("doc markdown expected, got {other:?}"),
    }
}
