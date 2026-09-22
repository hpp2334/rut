//! Signature help — the phase-3 callee machinery, read at a call's
//! argument position (the lsp-features survey §4.4). The call SHAPE is
//! detected token-level (the AST is unreliable exactly when this fires:
//! mid-edit, with the call still unclosed), the CALLEE resolves through
//! the machinery phase 3 landed:
//!
//! * free calls by unique-name match over the recorded index params
//!   (the `call_ty` law — ambiguity is a miss, never a wrong signature);
//! * methods through the receiver's type head (`self`/`Self`,
//!   capitalized, primitive, binding, module let — `recv_type`), then
//!   the inherent impl fn via the shared `impl_method_fn` rule;
//!   own-surface hits (trait bodies, builtin/primitive surfaces) are
//!   FINAL with no recorded params → no help, so a same-named impl fn's
//!   signature can never leak through (the known-receiver-is-final law).
//!
//! The signature renders verbatim (`FnDef.src` — fn name + params as
//! written, the hover render's own slices); the ACTIVE parameter is the
//! comma/paren depth at the request position. F-strings cannot miscount
//! — the lexer hands an f-string over as ONE token, so commas and parens
//! inside its holes are invisible to the scan. Nested calls each answer
//! at their own depth (the innermost unclosed paren wins). The
//! never-wrong guard is phase 3's: ambiguity, unknown receiver,
//! own-surface finality, or MORE arguments than the signature has
//! parameters — no help, never wrong help. A call still being typed
//! (fewer args than parameters) is the help's whole point and shows the
//! signature with the arg under the cursor highlighted.

use ls_types::{
    Documentation, MarkupContent, MarkupKind, ParameterInformation, ParameterLabel, SignatureHelp,
    SignatureInformation,
};

use rut_ast::ast::Ast;
use rut_lexer::token::{Tok, Token};

use crate::hover::lookup::{self, recv_type};
use crate::hover::types::FnDef;
use crate::hover::{bindings, DefIndex};

/// everything one signature-help query needs — the same `doc_ctx`
/// pipeline the other queries ride
pub struct Ctx<'a> {
    pub toks: &'a [Token],
    pub ast: &'a Ast,
    /// the lookup chain: the open document first, then std + workspace
    pub idxs: &'a [&'a DefIndex],
}

/// `textDocument/signatureHelp` at an LSP position (already converted to
/// a byte offset). `None` when there is no call to help with, or when
/// the callee does not resolve to exactly one known signature.
pub fn signature_help(ctx: &Ctx, pos: u32) -> Option<SignatureHelp> {
    let (open_i, commas) = enclosing_call(ctx.toks, pos)?;
    let head = ctx.toks.get(open_i.checked_sub(1)?)?;
    let Tok::Ident(name) = &head.tok else { return None };
    if crate::semantic::is_keyword(name) {
        return None;
    }
    // method call iff a dot sits between the head and what's before it
    let is_method = open_i >= 2 && ctx.toks[open_i - 2].tok == Tok::Dot;

    let (fndef, params) = if is_method {
        // the receiver must be ONE plain identifier (`c`, `Circle`,
        // `self`) — a dot before it means a multi-segment path, anything
        // else (a `)`, a literal) a chained/complex receiver; both stay
        // honest misses mid-typing (the AST the chained rules need is
        // exactly what a half-typed call does not have)
        let recv_tok = ctx.toks.get(open_i.checked_sub(3)?)?;
        let recv = match &recv_tok.tok {
            Tok::Ident(s) if !crate::semantic::is_keyword(s) => s.clone(),
            _ => return None,
        };
        if open_i >= 4 && ctx.toks[open_i - 4].tok == Tok::Dot {
            return None;
        }
        let binds = bindings::collect(ctx.ast, ctx.toks, ctx.idxs);
        let ty_name = recv_type(ctx.idxs, &binds, pos, &recv)?;
        // own-surface finality: `None` here is FINAL — no help, never
        // a same-named impl fn's signature
        let f = lookup::impl_method_fn(ctx.idxs, &ty_name, name)?;
        (f, f.params.clone())
    } else {
        // free fn by unique match (the call_ty law: ambiguity is a miss)
        let hits: Vec<&FnDef> = ctx
            .idxs
            .iter()
            .flat_map(|i| i.fns.iter().filter(|f| f.name == name.as_str() && f.owner.is_none()))
            .collect();
        match hits.as_slice() {
            [one] => (*one, one.params.clone()),
            _ => return None,
        }
    };

    // render: the verbatim signature, params as written
    let (label, pieces) = signature_parts(&fndef.src, &params, is_method)?;

    // the active parameter: depth-0 commas at-or-before the cursor. A
    // call still being typed has fewer args than params — that IS the
    // help's moment; MORE commas than params is a mismatch → no help
    let active_parameter = if params.is_empty() {
        if commas > 0 {
            return None; // arguments where none belong
        }
        None
    } else {
        let c = commas as usize;
        if c >= params.len() {
            return None;
        }
        Some(c as u32)
    };

    Some(SignatureHelp {
        signatures: vec![SignatureInformation {
            label,
            documentation: doc_markdown(&fndef.doc),
            parameters: Some(
                pieces
                    .into_iter()
                    .map(|p| ParameterInformation {
                        label: ParameterLabel::Simple(p),
                        documentation: None,
                    })
                    .collect(),
            ),
            active_parameter,
        }],
        active_signature: Some(0),
        active_parameter,
    })
}

/// the innermost call whose argument list contains `pos`, token-level:
/// scan backwards counting unclosed parens. Returns the open paren's
/// token index and the commas at-or-before `pos` that sit at the call's
/// own depth — paren depth 0 AND outside any `{}` / `[]` block, so a
/// when-select's arm commas or a struct literal's field commas can never
/// pose as argument separators. An f-string is ONE token (its holes live
/// inside it), so it can never miscount either. A non-call paren (`if
/// (`/`while (`/`when (` — the head is a keyword) is skipped so an
/// enclosing call still answers.
fn enclosing_call(toks: &[Token], pos: u32) -> Option<(usize, u32)> {
    let mut depth = 0i32;
    let mut braces = 0i32;
    let mut brackets = 0i32;
    let mut commas = 0u32;
    for (i, t) in toks.iter().enumerate().rev() {
        if t.span.lo > pos {
            continue; // strictly after the request position
        }
        match &t.tok {
            Tok::LParen => {
                if depth == 0 {
                    match toks.get(i.checked_sub(1)?).map(|p| &p.tok) {
                        Some(Tok::Ident(s)) if !crate::semantic::is_keyword(s) => {
                            return Some((i, commas));
                        }
                        _ => {} // not a call — keep scanning for an enclosing one
                    }
                } else {
                    depth -= 1;
                }
            }
            Tok::RParen => depth += 1,
            Tok::LBrace => braces = (braces - 1).max(0),
            Tok::RBrace => braces += 1,
            Tok::LBracket => brackets = (brackets - 1).max(0),
            Tok::RBracket => brackets += 1,
            Tok::Comma if depth == 0 && braces == 0 && brackets == 0 && t.span.hi <= pos => {
                commas += 1;
            }
            _ => {}
        }
    }
    None
}

/// the verbatim signature as the label, plus each parameter's as-written
/// piece (a substring of the label, per the LSP contract). `self` (or
/// `mut self`) never surfaces — the recorded params exclude it, and the
/// surviving pieces must line up with them or there is no help.
fn signature_parts(src: &str, params: &[String], is_method: bool) -> Option<(String, Vec<String>)> {
    let open = src.find('(')?;
    let mut depth = 0i32;
    let mut close = None;
    for (off, c) in src[open..].char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + off);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    let section = &src[open + 1..close];
    // top-level commas only — a tuple type inside a param (`-> (a, b)`
    // callers, `HashMap<i32, str>` generics) must not split
    let mut pieces: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut paren = 0i32;
    let mut angle = 0i32;
    for c in section.chars() {
        match c {
            '(' | '[' => {
                paren += 1;
                cur.push(c);
            }
            ')' | ']' => {
                paren -= 1;
                cur.push(c);
            }
            '<' => {
                angle += 1;
                cur.push(c);
            }
            '>' => {
                angle -= 1;
                cur.push(c);
            }
            ',' if paren == 0 && angle == 0 => {
                pieces.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        pieces.push(cur.trim().to_string());
    }
    if is_method {
        let first = pieces.first()?;
        if first != "self" && first != "mut self" {
            return None; // the recorded params exclude self — a mismatch is degenerate
        }
        pieces.remove(0);
    }
    if pieces.len() != params.len() {
        return None;
    }
    // each piece must mention its recorded param name — the recorded
    // names come from the AST, the pieces from the verbatim slice; if
    // they disagree the parse is degenerate and the honest answer is none
    for (name, piece) in params.iter().zip(&pieces) {
        if !piece.split(|c: char| !c.is_alphanumeric() && c != '_').any(|w| w == name) {
            return None;
        }
    }
    Some((src.to_string(), pieces))
}

/// the fn's doc lines as the signature documentation (markdown), only
/// when there are any
fn doc_markdown(doc: &[String]) -> Option<Documentation> {
    if doc.is_empty() {
        return None;
    }
    Some(Documentation::MarkupContent(MarkupContent {
        kind: MarkupKind::Markdown,
        value: doc.join("\n"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rut_lexer::lexer::{lex, normalize};
    use rut_parser::{parse, Mode};

    use crate::hover::DefIndex;

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
        let mut index = crate::hover::index(&src, &ast, &toks);
        index.origin = DOC.to_string();
        Doc { src, toks, ast, index }
    }

    fn help(d: &Doc, pos: u32, extra: &[DefIndex]) -> Option<SignatureHelp> {
        let idxs: Vec<&DefIndex> = std::iter::once(&d.index).chain(extra.iter()).collect();
        let ctx = Ctx { toks: &d.toks, ast: &d.ast, idxs: &idxs };
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
                .map(|ps| ps.iter().map(|p| match &p.label {
                    ParameterLabel::Simple(s) => s.clone(),
                    _ => panic!("label offsets not produced"),
                }).collect())
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
            "fn main() -> i32 {",
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
        let d = doc(&src);
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
            "fn main() -> i32 {",
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
            "    let c = Circle.new(1.0);",
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
        // the trait-body `m` is the receiver's own surface: FINAL with no
        // recorded params — the impl fn's signature must not leak through
        let src = [
            "struct P {",
            "    x: i32;",
            "}",
            "trait Draw {",
            "    fn m(self, k: i32) -> i32;",
            "}",
            "impl Draw for P {",
            "    fn m(self, k: i32) -> i32 { return k; }",
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
            "fn main() -> i32 {",
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
        let src = "fn main() -> nil {\n    mystery(1, 2);\n}\n";
        let d = doc(&src);
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
            "fn main() -> i32 {",
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
            "fn main() -> f64 {",
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
            "fn main() -> i32 {",
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
            "fn main() -> i32 {",
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
}
