//! Lookup — what's under the cursor: the receiver of a member access,
//! a capitalized type name, a primitive type token, a local binding
//! (the binding pass), a field/enum-member/module-let declaration site
//! (the decl layer's exact spans), or a unique fn name.

use std::collections::HashSet;

use rut_ast::ast::{Ast, ItemKind};
use rut_lexer::span::Span;
use rut_lexer::token::{Tok, Token};

use super::bindings::{self, Binding};
use super::infer::is_cap;
use super::render::{binding_markdown, render_candidates, render_fn_hits, render_member, render_primitive, render_ty, render_let};
use super::types::{DefIndex, FnDef, LetDef, TyDef, TyForm};
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
pub fn hover(idxs: &[&DefIndex], toks: &[Token], ast: &Ast, pos: u32) -> Option<HoverOut> {
    let t = tok_at(toks, pos)?;
    let Tok::Ident(name) = &t.tok else { return None };
    if is_keyword(name) {
        if name == "self" {
            let (i, ty) = enclosing_type(idxs, pos)?;
            return Some(HoverOut { markdown: render_ty(i, ty), span: t.span });
        }
        return None;
    }

    // decl sites — the hovered span EXACTLY equals a recorded name span:
    // a field decl, an enum-member decl, or a module let (the decl
    // layer's token-recovered spans; a use site never matches)
    if let Some(md) = decl_site_hover(idxs, t.span, name) {
        return Some(HoverOut { markdown: md, span: t.span });
    }

    // the binding pass — receivers below AND the plain-identifier hover
    let binds = bindings::collect(ast, toks, idxs);

    // member position: `recv.member`. A resolved receiver never falls
    // through to the name search: a miss there is a use-gated trait
    // method (RFC 0012 §6) or a genuine miss — either way the member
    // answer is final.
    if let Some(recv) = member_context(toks, t) {
        match member_hover(idxs, ast, &binds, pos, name, recv) {
            MemberHit::Found(out) => return Some(HoverOut { markdown: out, span: t.span }),
            MemberHit::None => return None,
            MemberHit::UnknownReceiver => {} // fall through to the name search
        }
    }

    // type position: capitalized or primitive names
    if is_cap(name) || rut_parser::is_primitive_ty(name) {
        if let Some((i, ty)) = find_ty(idxs, name) {
            return Some(HoverOut { markdown: render_ty(i, ty), span: t.span });
        }
        // M7 — the numeric/bool primitives have no surface decl to
        // index; the static blurb is the answer
        if let Some(md) = render_primitive(name) {
            return Some(HoverOut { markdown: md, span: t.span });
        }
    }

    // identifier hover — the binding layer's shadow-correct resolve
    // (params, lets, loop variables; decl and use sites alike)
    if let Some(b) = bindings::resolve(&binds, pos, name) {
        return Some(HoverOut { markdown: binding_markdown(b), span: t.span });
    }

    // module-let use sites — a unique name match across the chain
    if let Some(md) = module_let_hover(idxs, name) {
        return Some(HoverOut { markdown: md, span: t.span });
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
pub(crate) fn tok_at<'t>(toks: &'t [Token], pos: u32) -> Option<&'t Token> {
    toks.iter().rev().find(|t| t.span.lo <= pos && pos <= t.span.hi)
}

/// when the hovered token follows a dot: the receiver's source text —
/// an identifier, or the `str` primitive for string literals
pub(crate) fn member_context<'t>(toks: &'t [Token], t: &Token) -> Option<String> {
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

/// trait names this document `use`s — the use-both gate's document side
/// (RFC 0012 §6). A foreign trait's methods dispatch only when its name
/// appears here; a trait declared in this document is in scope natively.
pub(crate) fn used_traits(ast: &Ast) -> HashSet<String> {
    let mut out = HashSet::new();
    for h in ast.module_items(ast.root) {
        if let ItemKind::Use { names, .. } = ast.item(*h) {
            out.extend(names.iter().map(|n| ast.name(*n).to_string()));
        }
    }
    out
}

/// decl-site hover: the exact name-span match in the decl layer —
/// fields (the owning type's member render), enum members (the enum's
/// block, matching what `Color.Red` shows), and module lets
fn decl_site_hover(idxs: &[&DefIndex], span: Span, name: &str) -> Option<String> {
    for i in idxs {
        for t in &i.types {
            for f in &t.fields {
                if f.name == name && f.name_span == Some(span) {
                    return Some(match t.form {
                        TyForm::Enum => render_ty(i, t),
                        _ => render_member(i, t, f, None),
                    });
                }
            }
        }
        for l in &i.lets {
            if l.name == name && l.name_span == Some(span) {
                return Some(render_let(i, l));
            }
        }
    }
    None
}

/// module-let use sites: a unique name match across the chain renders
/// the decl; ambiguity stays a miss (the flat chain's honesty rule)
fn module_let_hover(idxs: &[&DefIndex], name: &str) -> Option<String> {
    let hits: Vec<(&DefIndex, &LetDef)> = idxs
        .iter()
        .flat_map(|i| i.lets.iter().filter(|l| l.name == name).map(move |l| (*i, l)))
        .collect();
    match hits.as_slice() {
        [(i, l)] => Some(render_let(i, l)),
        _ => None,
    }
}

/// resolve the receiver's type name — `self`, capitalized and primitive
/// names spell themselves, anything else goes through the binding pass
/// (params, lets — including field-read / chained-call / loop-var
/// initializers — via `expr_ty`), then module lets
pub(crate) fn recv_type(
    idxs: &[&DefIndex],
    binds: &[Binding],
    pos: u32,
    recv: &str,
) -> Option<String> {
    match recv {
        "self" | "Self" => enclosing_type(idxs, pos).map(|(_, t)| t.name.clone()),
        _ if is_cap(recv) => Some(recv.to_string()),
        _ if rut_parser::is_primitive_ty(recv) => Some(recv.to_string()),
        _ => bindings::resolve(binds, pos, recv)
            .and_then(|b| b.ty_head())
            .or_else(|| super::infer::module_let_ty(idxs, recv)),
    }
}

/// the type whose body contains `pos` — a class/struct/trait body, or
/// the target of the enclosing impl (via the method's owner)
pub(crate) fn enclosing_type<'a>(idxs: &'a [&'a DefIndex], pos: u32) -> Option<(&'a DefIndex, &'a TyDef)> {
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
            // the impl target: `impl Circle` (inherent — RFC 0012 §4)
            // or the tail of `impl Drawable for Circle`
            let Some(owner) = f.owner.as_deref() else { continue };
            let target = match owner.strip_prefix("impl ") {
                Some(rest) if !rest.contains(" for ") => rest,
                _ => owner.rsplit(" for ").next().unwrap_or(owner),
            };
            if let Some(t) = i.ty(target) {
                if contains(f.span, pos)
                    && best.map(|(l, _, _)| f.span.hi - f.span.lo < l).unwrap_or(true)
                {
                    best = Some((f.span.hi - f.span.lo, i, t));
                }
            }
        }
    }
    best.map(|(_, i, t)| (i, t))
}

pub(crate) fn find_ty<'a>(idxs: &'a [&DefIndex], name: &str) -> Option<(&'a DefIndex, &'a TyDef)> {
    idxs.iter().find_map(|i| i.ty(name).map(|t| (*i, t)))
}

/// The member-lookup verdict: found text, a final miss (the receiver's
/// type is known — the gate or a genuine miss), or an unknown receiver
/// (the caller may fall through to the bare-name search).
enum MemberHit {
    Found(String),
    None,
    UnknownReceiver,
}

fn member_hover(
    idxs: &[&DefIndex],
    ast: &Ast,
    binds: &[Binding],
    pos: u32,
    member: &str,
    recv: String,
) -> MemberHit {
    let Some(ty_name) = recv_type(idxs, binds, pos, &recv) else {
        return MemberHit::UnknownReceiver;
    };
    let Some((ti, ty)) = find_ty(idxs, &ty_name) else {
        return MemberHit::UnknownReceiver;
    };
    // own surface first
    if let Some(m) = ty.methods.iter().find(|m| m.name == member) {
        return MemberHit::Found(render_member(ti, ty, m, None));
    }
    if let Some(f) = ty.fields.iter().find(|f| f.name == member) {
        return MemberHit::Found(render_member(ti, ty, f, None));
    }
    if ty.form == TyForm::Enum {
        // `Color.Red` — the member IS an enum member; show the enum
        if ty.fields.iter().any(|f| f.name == member) {
            return MemberHit::Found(render_ty(ti, ty));
        }
        return MemberHit::None;
    }
    // inherent impl-block methods — where methods live since type bodies
    // went fields-only (RFC 0012 §4); one fn per `impl T { .. }` member
    let owner = format!("impl {ty_name}");
    for i in idxs {
        for f in &i.fns {
            if f.name == member && f.owner.as_deref() == Some(owner.as_str()) {
                return MemberHit::Found(render_fn_hits(&[(*i, f)], f));
            }
        }
    }
    // trait methods from impls targeting this type (the unified rule).
    // The use-both gate rides the trait's HOME module, wherever the impl
    // block lives: a trait declared in another module dispatches only
    // when this document names it in a `use` (RFC 0012 §6)
    let used = used_traits(ast);
    for i in idxs {
        for im in &i.impls {
            if im.target_name != ty_name || im.trait_name.is_empty() {
                continue;
            }
            let Some((home, t)) = trait_decl(idxs, &im.trait_name) else { continue };
            if !std::ptr::eq(home, idxs[0]) && !used.contains(&im.trait_name) {
                continue;
            }
            if let Some(m) = t.methods.iter().find(|m| m.name == member) {
                return MemberHit::Found(render_member(
                    home,
                    t,
                    m,
                    Some(format!("impl {} for {}", im.trait_name, im.target_name)),
                ));
            }
        }
    }
    MemberHit::None
}

/// the index declaring trait `name` — its home module; the declaration
/// and the impl block may live in different indexes
pub(crate) fn trait_decl<'a>(idxs: &'a [&'a DefIndex], name: &str) -> Option<(&'a DefIndex, &'a TyDef)> {
    find_ty(idxs, name).filter(|(_, t)| matches!(t.form, TyForm::Trait | TyForm::BuiltinTrait))
}
