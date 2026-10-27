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
//!   own-surface hits (interface bodies, builtin/primitive surfaces) are
//!   FINAL with no recorded params → no help, so a same-named impl fn's
//!   signature can never leak through (the known-receiver-is-final law).
//!
//! The signature renders verbatim (`FnDef::src` — fn name + params as
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
    let Tok::Ident(name) = &head.tok else {
        return None;
    };
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
            .flat_map(|i| {
                i.fns
                    .iter()
                    .filter(|f| f.name == name.as_str() && f.owner.is_none())
            })
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
/// when-arm commas or a struct literal's field commas can never
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
        if !piece
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|w| w == name)
        {
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
