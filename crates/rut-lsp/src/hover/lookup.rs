//! Lookup — what's under the cursor: the receiver of a member access,
//! a capitalized type name, or a unique fn name.

use rut_ast::ast::Ast;
use rut_lexer::span::Span;
use rut_lexer::token::{Tok, Token};

use super::infer::infer_local;
use super::render::{render_candidates, render_fn_hits, render_member, render_ty};
use super::types::{DefIndex, FnDef, TyDef, TyForm};
use crate::semantic::is_keyword;

pub(crate) fn contains(sp: Span, pos: u32) -> bool {
    sp.lo <= pos && pos < sp.hi
}

/// The hover result: markdown body + the span it decorates.
pub struct HoverOut {
    pub markdown: String,
    pub span: Span,
}

/// `idxs` in priority order — the open document first, then the std
/// surface, then the rest of the workspace.
pub fn hover(
    idxs: &[&DefIndex],
    src: &str,
    toks: &[Token],
    ast: &Ast,
    pos: u32,
) -> Option<HoverOut> {
    let t = tok_at(toks, pos)?;
    let Tok::Ident(name) = &t.tok else { return None };
    if is_keyword(name) {
        if name == "self" {
            let (i, ty) = enclosing_type(idxs, pos)?;
            return Some(HoverOut { markdown: render_ty(i, ty), span: t.span });
        }
        return None;
    }

    // member position: `recv.member`
    if let Some(recv) = member_context(toks, t) {
        if let Some(out) = member_hover(idxs, src, ast, pos, name, recv) {
            return Some(HoverOut { markdown: out, span: t.span });
        }
        // unresolved receiver falls through to the name search below
    }

    // type position: capitalized or primitive names
    if is_cap(name) || rut_parser::is_primitive_ty(name) {
        if let Some((i, ty)) = find_ty(idxs, name) {
            return Some(HoverOut { markdown: render_ty(i, ty), span: t.span });
        }
    }

    // bare name: unique fn match (free fns, methods); ambiguity lists
    let hits: Vec<(&DefIndex, &FnDef)> = idxs
        .iter()
        .flat_map(|i| i.fns.iter().filter(|f| &f.name == name).map(move |f| (*i, f)))
        .collect();
    match hits.as_slice() {
        [] => None,
        [(_, one)] => Some(HoverOut { markdown: render_fn_hits(&hits, one), span: t.span }),
        _ => Some(HoverOut { markdown: render_candidates(&hits), span: t.span }),
    }
}

/// token under (or immediately before) `pos`
fn tok_at<'t>(toks: &'t [Token], pos: u32) -> Option<&'t Token> {
    toks.iter().rev().find(|t| t.span.lo <= pos && pos <= t.span.hi)
}

/// when the hovered token follows a dot: the receiver's source text —
/// an identifier, or the `str` primitive for string literals
fn member_context<'t>(toks: &'t [Token], t: &Token) -> Option<String> {
    let i = toks.iter().position(|x| x.span.lo == t.span.lo)?;
    if i < 2 || toks[i - 1].tok != Tok::Dot {
        return None;
    }
    match &toks[i - 2].tok {
        Tok::Ident(s) => Some(s.clone()),
        Tok::Str(_) => Some("str".to_string()),
        _ => None,
    }
}

fn is_cap(s: &str) -> bool {
    s.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false)
}

/// resolve the receiver's type name
fn recv_type(idxs: &[&DefIndex], src: &str, ast: &Ast, pos: u32, recv: &str) -> Option<String> {
    match recv {
        "self" | "Self" => enclosing_type(idxs, pos).map(|(_, t)| t.name.clone()),
        _ if is_cap(recv) => Some(recv.to_string()),
        _ if rut_parser::is_primitive_ty(recv) => Some(recv.to_string()),
        _ => infer_local(src, ast, pos, recv),
    }
}

/// the type whose body contains `pos` — a class/dataclass/interface body, or
/// the target of the enclosing impl (via the method's owner)
fn enclosing_type<'a>(idxs: &'a [&'a DefIndex], pos: u32) -> Option<(&'a DefIndex, &'a TyDef)> {
    let mut best: Option<(u32, &'a DefIndex, &'a TyDef)> = None;
    for i in idxs.iter().copied() {
        for t in &i.types {
            if t.form == TyForm::Enum || !contains(t.span, pos) {
                continue;
            }
            if best.map(|(l, _, _)| t.span.hi - t.span.lo < l).unwrap_or(true) {
                best = Some((t.span.hi - t.span.lo, i, t));
            }
        }
        for f in &i.fns {
            if let Some(target) = f.owner.as_deref().and_then(|o| o.rsplit(" for ").next()) {
                if let Some(t) = i.ty(target) {
                    if contains(f.span, pos)
                        && best.map(|(l, _, _)| f.span.hi - f.span.lo < l).unwrap_or(true)
                    {
                        best = Some((f.span.hi - f.span.lo, i, t));
                    }
                }
            }
        }
    }
    best.map(|(_, i, t)| (i, t))
}

fn find_ty<'a>(idxs: &'a [&DefIndex], name: &str) -> Option<(&'a DefIndex, &'a TyDef)> {
    idxs.iter().find_map(|i| i.ty(name).map(|t| (*i, t)))
}

fn member_hover(
    idxs: &[&DefIndex],
    src: &str,
    ast: &Ast,
    pos: u32,
    member: &str,
    recv: String,
) -> Option<String> {
    let ty_name = recv_type(idxs, src, ast, pos, &recv)?;
    let (ti, ty) = find_ty(idxs, &ty_name)?;
    // own surface first
    if let Some(m) = ty.methods.iter().find(|m| m.name == member) {
        return Some(render_member(ti, ty, m, None));
    }
    if let Some(f) = ty.fields.iter().find(|f| f.name == member) {
        return Some(render_member(ti, ty, f, None));
    }
    if ty.form == TyForm::Enum {
        // `Color.Red` — the member IS an enum member; show the enum
        if ty.fields.iter().any(|f| f.name == member) {
            return Some(render_ty(ti, ty));
        }
        return None;
    }
    // interface methods from impls targeting this type (the unified rule)
    for i in idxs {
        for im in &i.impls {
            if im.target_name == ty_name {
                if let Some(t) = i.ty(&im.trait_name) {
                    if let Some(m) = t.methods.iter().find(|m| m.name == member) {
                        return Some(render_member(
                            i,
                            t,
                            m,
                            Some(format!("impl {} for {}", im.trait_name, im.target_name)),
                        ));
                    }
                }
            }
        }
    }
    None
}
