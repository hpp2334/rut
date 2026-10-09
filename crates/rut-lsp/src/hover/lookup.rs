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
use super::render::{
    binding_markdown, render_candidates, render_fn_hits, render_member, render_mod,
    render_module_target, render_primitive, render_ty, render_let,
};
use super::types::{DefIndex, FnDef, LetDef, MemberSrc, TyDef, TyForm};
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
    // a field decl, an enum-member decl, a module let, or a `mod NAME;`
    // namespace edge (the decl layer's token-recovered spans; a use
    // site never matches)
    if let Some(md) = decl_site_hover(idxs, t.span, name) {
        return Some(HoverOut { markdown: md, span: t.span });
    }

    // use-path segments — the hovered ident is a path segment of one
    // of the document's `use pkg::a::b::{ .. }` edges: hover the
    // resolved module through the dep tree (a miss is no hover)
    if let Some(md) = use_segment_hover(idxs, t.span) {
        return Some(HoverOut { markdown: md, span: t.span });
    }

    // the binding pass — receivers below AND the plain-identifier hover
    let binds = bindings::collect(ast, toks, idxs);

    // member position: `recv.member`. A resolved receiver never falls
    // through to the name search: a miss there is a use-gated interface
    // member or a genuine miss — either way the member
    // answer is final.
    if let Some(recv) = member_context(toks, t) {
        match member_hover(idxs, ast, &binds, pos, name, recv) {
            MemberText::Found(out) => return Some(HoverOut { markdown: out, span: t.span }),
            MemberText::None => return None,
            MemberText::UnknownReceiver => {} // fall through to the name search
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

/// interface names this document `use`s — the use-both gate's document
/// side: a foreign interface's members resolve only when its name
/// appears here; an interface declared in this document is in scope
/// natively.
pub(crate) fn used_ifaces(ast: &Ast) -> HashSet<String> {
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
/// block, matching what `Color.Red` shows), module lets, and `mod
/// NAME;` namespace edges (the kind, the mounted file, the child
/// count)
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
        for m in &i.mods {
            if m.name == name && m.name_span == Some(span) {
                let child = crate::mods::child_path(&crate::mods::mod_path_of(i), &m.name);
                let mounted = mounted_child(idxs, i, &child);
                return Some(render_mod(
                    i,
                    m,
                    &crate::mods::display(&child),
                    mounted.map(|c| c.mods.len()),
                ));
            }
        }
    }
    None
}

/// the chain index a mod edge's child resolves to — the (module,
/// mod_path) pair when the owner is named; an unnamed owner (an open
/// workspace file) answers any index stamped at the child's mod path
/// (the dep walk's twins — display only: the child count)
fn mounted_child<'a>(idxs: &[&'a DefIndex], owner: &DefIndex, child: &str) -> Option<&'a DefIndex> {
    match crate::mods::find_mod(idxs, owner.module.as_deref(), child) {
        Some(c) => Some(c),
        None if owner.module.is_none() => idxs.iter().copied().find(|i| crate::mods::mod_path_of(i) == child),
        None => None,
    }
}

/// a use-path SEGMENT hover: the span matches one of the document's
/// path segments — the resolved module (walked through the pub edges;
/// the pkg head answers the root module) renders kind + mounted file
/// + child count. A non-walking path is no hover, never wrong text.
fn use_segment_hover(idxs: &[&DefIndex], span: Span) -> Option<String> {
    let doc = idxs[0];
    for u in &doc.uses {
        for (k, sp) in u.path_spans.iter().enumerate() {
            if *sp != Some(span) {
                continue;
            }
            let pkg = &u.path[0];
            let walked = if k == 0 {
                Some(String::new())
            } else {
                crate::mods::walk_use_mods(idxs, pkg, &u.path[1..=k])
            };
            let path = walked?;
            let target = crate::mods::find_mod(idxs, Some(pkg), &path)?;
            return Some(render_module_target(target, &path));
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

/// the type whose body contains `pos` — a class/struct/interface body,
/// or the target of the enclosing impl (via the method's owner)
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
            // the impl's target — the owner string IS the type's name
            // (inherent impls own their methods)
            let Some(target) = f.owner.as_deref() else { continue };
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

/// The member-lookup verdict: found (a resolved declaration target), a
/// final miss (the receiver's type is known — the gate or a genuine
/// miss), or an unknown receiver (the caller may fall through to the
/// bare-name search).
pub(crate) enum MemberHit<'a> {
    Found(MemberTarget<'a>),
    None,
    UnknownReceiver,
}

/// where a member of `recv` actually declares — the shared resolver
/// behind hover AND definition, one rule for both faces: own surface
/// (methods, then fields), enum members, inherent impl-block methods,
/// and — through an interface-typed receiver — the interface's declared
/// members (use-gated when the interface is foreign)
pub(crate) enum MemberTarget<'a> {
    /// a method/field of the type's own surface (`via` renders a
    /// qualifying member's provenance when it came through a chain)
    Member(&'a DefIndex, &'a TyDef, &'a MemberSrc, Option<String>),
    /// an enum member — hover shows the whole enum's block
    EnumMember(&'a DefIndex, &'a TyDef, &'a MemberSrc),
    /// an impl-block method (`impl T { fn m(self) ... }`)
    ImplFn(&'a DefIndex, &'a FnDef),
    /// an interface's declared member, observed through the interface
    /// (the receiver's static type IS the interface)
    IfaceMember(&'a DefIndex, &'a TyDef, &'a MemberSrc),
}

pub(crate) fn member_target<'a>(
    idxs: &'a [&'a DefIndex],
    ast: &Ast,
    binds: &[Binding],
    pos: u32,
    member: &str,
    recv: &str,
) -> MemberHit<'a> {
    // (body unchanged — returns Found(MemberTarget::..) below)
    let Some(ty_name) = recv_type(idxs, binds, pos, recv) else {
        return MemberHit::UnknownReceiver;
    };
    // an interface receiver: the declared signatures ARE the members —
    // satisfaction is structural (a type qualifies by having the
    // members), so no impl lookup rides this path. The use-both gate:
    // a foreign interface's members resolve only when this document
    // names the interface in a `use`; an interface declared here is in
    // scope natively
    if let Some((home, iface)) = iface_decl(idxs, &ty_name) {
        if !std::ptr::eq(home, idxs[0]) && !used_ifaces(ast).contains(&iface.name) {
            return MemberHit::None;
        }
        if let Some(m) = iface.methods.iter().find(|m| m.name == member) {
            return MemberHit::Found(MemberTarget::IfaceMember(home, iface, m));
        }
        return MemberHit::None;
    }
    let Some((ti, ty)) = find_ty(idxs, &ty_name) else {
        return MemberHit::UnknownReceiver;
    };
    // own surface first
    if let Some(m) = ty.methods.iter().find(|m| m.name == member) {
        return MemberHit::Found(MemberTarget::Member(ti, ty, m, None));
    }
    if let Some(f) = ty.fields.iter().find(|f| f.name == member) {
        return MemberHit::Found(MemberTarget::Member(ti, ty, f, None));
    }
    if ty.form == TyForm::Enum {
        // `Color.Red` — the member IS an enum member; hover shows the enum
        if let Some(m) = ty.fields.iter().find(|f| f.name == member) {
            return MemberHit::Found(MemberTarget::EnumMember(ti, ty, m));
        }
        return MemberHit::None;
    }
    // inherent impl-block methods — where methods live since type bodies
    // went fields-only; the owner string IS the target type's name
    for i in idxs {
        for f in &i.fns {
            if f.name == member && f.owner.as_deref() == Some(ty_name.as_str()) {
                return MemberHit::Found(MemberTarget::ImplFn(i, f));
            }
        }
    }
    MemberHit::None
}

/// hover's half: the resolved target's markdown (definition uses the
/// target's spans instead — see `definition::member_location`)
enum MemberText {
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
) -> MemberText {
    match member_target(idxs, ast, binds, pos, member, &recv) {
        MemberHit::Found(target) => MemberText::Found(render_target(target)),
        MemberHit::None => MemberText::None,
        MemberHit::UnknownReceiver => MemberText::UnknownReceiver,
    }
}

/// the markdown a resolved member target renders to (hover's half of
/// the shared resolver; definition uses the target's spans instead)
pub(crate) fn render_target(t: MemberTarget) -> String {
    match t {
        MemberTarget::Member(i, ty, m, via) => render_member(i, ty, m, via),
        MemberTarget::EnumMember(i, ty, _) => render_ty(i, ty),
        MemberTarget::ImplFn(i, f) => render_fn_hits(&[(i, f)], f),
        MemberTarget::IfaceMember(i, ty, m) => render_member(i, ty, m, None),
    }
}

/// the index declaring interface `name` — its home module; the
/// declaration and the observing receiver may live in different indexes
pub(crate) fn iface_decl<'a>(idxs: &'a [&'a DefIndex], name: &str) -> Option<(&'a DefIndex, &'a TyDef)> {
    find_ty(idxs, name).filter(|(_, t)| matches!(t.form, TyForm::Interface))
}

/// the inherent impl-block fn `impl ty_name { fn member(..) }` — the
/// phase-3 callee rule, shared by the call-site PARAMETER hints and
/// signature help (one rule, three consumers now — the `member_target`
/// precedent). Own-surface methods (interface bodies, builtin/primitive
/// surfaces) have no recorded params: a hit there is FINAL (the
/// known-receiver-is-final law) and yields `None`, never a wrong
/// signature from a same-named impl fn in another index.
pub(crate) fn impl_method_fn<'a>(idxs: &[&'a DefIndex], ty_name: &str, member: &str) -> Option<&'a FnDef> {
    for i in idxs {
        if let Some(t) = i.ty(ty_name) {
            if t.methods.iter().any(|m| m.name == member) {
                return None;
            }
        }
        for f in &i.fns {
            if f.name == member && f.owner.as_deref() == Some(ty_name) {
                return Some(f);
            }
        }
    }
    None
}
