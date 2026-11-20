//! Lookup — what's under the cursor: the receiver of a member access,
//! a capitalized type name, a primitive type token, a local binding
//! (the binding pass), a field/enum-member/module-let declaration site
//! (the decl layer's exact spans), or a unique fn name.

use super::bindings::{self, Binding};
use super::collection::{Chain, FnView, MemberHit, MemberTarget, MountedFile, TyView};
use super::infer::is_cap;
use super::render::{
    binding_markdown, render_candidates, render_fn_hits, render_let, render_member, render_mod,
    render_module_target, render_primitive, render_ty,
};
use super::types::TyForm;
use crate::mods::DocMods;
use crate::semantic::is_keyword;
use crate::semantic::Span;
use crate::semantic::{Tok, Token};

pub(crate) fn contains(sp: Span, pos: u32) -> bool {
    sp.lo <= pos && pos < sp.hi
}

/// The hover result: markdown body + the span it decorates.
pub struct HoverOut {
    pub markdown: String,
    pub span: Span,
}

/// The chain is the world: the open document first (its fresh entry),
/// then the ambient prelude, then the rest of the workspace. `doc` is
/// the open document's file-module context (the face's knowledge): an
/// own-package use-path segment hover resolves through it, the default
/// keeps the cross-package lanes only.
pub fn hover(chain: &Chain, doc: &DocMods, pos: u32) -> Option<HoverOut> {
    let d = chain.head();
    let toks = d.doc.toks();
    let ast = d.doc.ast();
    let t = tok_at(toks, pos)?;
    let Tok::Ident(name) = &t.tok else {
        return None;
    };
    if is_keyword(name) {
        if name == "self" {
            let ty = enclosing_type(chain.iter(), pos)?;
            return Some(HoverOut {
                markdown: render_ty(&ty),
                span: t.span,
            });
        }
        return None;
    }

    // decl sites — the hovered span EXACTLY equals a recorded name span:
    // a field decl, an enum-member decl, a module let, or a `mod NAME;`
    // namespace edge (the decl layer's token-recovered spans; a use
    // site never matches)
    if let Some(md) = decl_site_hover(chain, t.span, name) {
        return Some(HoverOut {
            markdown: md,
            span: t.span,
        });
    }

    // use-path segments — the hovered ident is a path segment of one
    // of the document's `use pkg::a::b::{ .. }` edges: hover the
    // resolved module through the dep tree (a miss is no hover)
    if let Some(md) = use_segment_hover(chain, doc, t.span) {
        return Some(HoverOut {
            markdown: md,
            span: t.span,
        });
    }

    // the binding pass — receivers below AND the plain-identifier hover
    let binds = bindings::collect(ast, toks, chain);

    // member position: `recv.member`. A resolved receiver never falls
    // through to the name search: a miss there is a use-gated interface
    // member or a genuine miss — either way the member
    // answer is final.
    if let Some(recv) = member_context(toks, t) {
        match chain.member(&binds, pos, name, &recv) {
            MemberHit::Found(target) => {
                return Some(HoverOut {
                    markdown: render_target(target),
                    span: t.span,
                })
            }
            MemberHit::None => return None,
            MemberHit::UnknownReceiver => {} // fall through to the name search
        }
    }

    // type position: capitalized or primitive names
    if is_cap(name) || crate::semantic::is_primitive_ty(name) {
        if let Some(ty) = chain.type_decl(name) {
            return Some(HoverOut {
                markdown: render_ty(&ty),
                span: t.span,
            });
        }
        // M7 — the numeric/bool primitives have no surface decl to
        // index; the static blurb is the answer
        if let Some(md) = render_primitive(name) {
            return Some(HoverOut {
                markdown: md,
                span: t.span,
            });
        }
    }

    // identifier hover — the binding layer's shadow-correct resolve
    // (params, lets, loop variables; decl and use sites alike)
    if let Some(b) = bindings::resolve(&binds, pos, name) {
        return Some(HoverOut {
            markdown: binding_markdown(b),
            span: t.span,
        });
    }

    // module-let use sites — a unique name match across the chain
    if let Some(md) = module_let_hover(chain, name) {
        return Some(HoverOut {
            markdown: md,
            span: t.span,
        });
    }

    // bare name: unique fn match (free fns, methods); ambiguity lists
    let hits: Vec<FnView> = chain.functions(name);
    match hits.as_slice() {
        [] => None,
        [one] => Some(HoverOut {
            markdown: render_fn_hits(one),
            span: t.span,
        }),
        _ => Some(HoverOut {
            markdown: render_candidates(&hits),
            span: t.span,
        }),
    }
}

/// token under (or immediately before) `pos`
pub(crate) fn tok_at(toks: &[Token], pos: u32) -> Option<&Token> {
    toks.iter()
        .rev()
        .find(|t| t.span.lo <= pos && pos <= t.span.hi)
}

/// when the hovered token follows a `.` (value access) or a `::`
/// (name resolution — enum members, namespace consts): the head's
/// source text — an identifier, or the `str` primitive for string
/// literals
pub(crate) fn member_context(toks: &[Token], t: &Token) -> Option<String> {
    let i = toks.iter().position(|x| x.span.lo == t.span.lo)?;
    if i < 2 || !(toks[i - 1].tok == Tok::Dot || toks[i - 1].tok == Tok::Colon2) {
        return None;
    }
    match &toks[i - 2].tok {
        Tok::Ident(s) => Some(s.clone()),
        // every string-literal form is a `str` receiver
        Tok::Str(_) | Tok::RawStr(_) | Tok::FStr(_) => Some("str".to_string()),
        _ => None,
    }
}

/// decl-site hover: the exact name-span match in the decl layer —
/// fields (the owning type's member render), enum members (the enum's
/// block, matching what `Color::Red` shows), module lets, and `mod
/// NAME;` namespace edges (the kind, the mounted file, the child
/// count)
fn decl_site_hover(chain: &Chain, span: Span, name: &str) -> Option<String> {
    for f in chain.iter() {
        for t in f.types() {
            for m in t.fields().into_iter().chain(t.methods()) {
                if m.name() == name && m.name_span() == Some(span) {
                    return Some(match t.form() {
                        TyForm::Enum => render_ty(&t),
                        _ => render_member(&m, None),
                    });
                }
            }
        }
        for l in f.lets() {
            if l.name() == name && l.name_span() == Some(span) {
                return Some(render_let(&l));
            }
        }
        for m in f.mods() {
            if m.name() == name && m.name_span() == Some(span) {
                let child = crate::mods::child_path(crate::mods::mod_path_of(f), m.name());
                let mounted = mounted_child(chain, f, &child);
                return Some(render_mod(
                    f,
                    &m,
                    &crate::mods::display_of(chain, f.mount.module.as_deref(), &child),
                    mounted.map(|c| c.mods().len()),
                ));
            }
        }
    }
    None
}

/// the chain file a mod edge's child resolves to — the (module,
/// mod_path) pair when the owner is named; an unnamed owner (an open
/// workspace file) answers any file stamped at the child's mod path
/// (the dep walk's twins — display only: the child count)
fn mounted_child<'a>(
    chain: &'a Chain<'a>,
    owner: &MountedFile,
    child: &str,
) -> Option<&'a MountedFile> {
    match crate::mods::find_mod(chain, owner.mount.module.as_deref(), child) {
        Some(c) => Some(c),
        None if owner.mount.module.is_none() => {
            chain.iter().find(|i| crate::mods::mod_path_of(i) == child)
        }
        None => None,
    }
}

/// a use-path SEGMENT hover: the span matches one of the document's
/// path segments — the resolved module (walked through the pub edges;
/// the pkg head answers the root module) renders kind + mounted file
/// + child count.
///
/// An OWN package head — the doc's own pkg name, or one of its own
/// mod edges (the ancestor-walk spelling, keyed on the doc context) —
/// commits to the intra-package walk (any declared edge; the doc's
/// own rows read for its own scope). A non-walking path is no hover,
/// never wrong text.
fn use_segment_hover(chain: &Chain, doc: &DocMods, span: Span) -> Option<String> {
    let di = chain.head();
    for u in di.uses() {
        for (k, sp) in u.path_spans().iter().enumerate() {
            if *sp != Some(span) {
                continue;
            }
            let pkg = u.pkg().to_string();
            // the OWN lane — committed once the head speaks for the
            // doc's own package: `k == 0` answers the resolved start
            // module, deeper segments walk intra-package
            if let Some(start) = crate::mods::resolve_position_head(doc, di, chain, &pkg) {
                let path = if k == 0 {
                    start
                } else {
                    crate::mods::walk_position_mods(
                        chain,
                        doc.pkg.as_deref(),
                        &start,
                        &u.path()[1..=k],
                    )?
                };
                let target = if doc.mod_path.as_deref() == Some(path.as_str()) {
                    di
                } else {
                    crate::mods::find_mod(chain, doc.pkg.as_deref(), &path)?
                };
                return Some(render_module_target(target, &path));
            }
            let walked = if k == 0 {
                Some(String::new())
            } else {
                crate::mods::walk_use_mods(chain, &pkg, &u.path()[1..=k])
            };
            let path = walked?;
            let target = crate::mods::find_mod(chain, Some(&pkg), &path)?;
            return Some(render_module_target(target, &path));
        }
    }
    None
}

/// module-let use sites: a unique name match across the chain renders
/// the decl; ambiguity stays a miss (the flat chain's honesty rule)
fn module_let_hover(chain: &Chain, name: &str) -> Option<String> {
    chain.module_let(name).map(|l| render_let(&l))
}

/// resolve the receiver's type name — `self`, capitalized and primitive
/// names spell themselves, anything else goes through the binding pass
/// (params, lets — including field-read / chained-call / loop-var
/// initializers — via `expr_ty`), then module lets
pub(crate) fn recv_type(
    chain: &Chain<'_>,
    binds: &[Binding],
    pos: u32,
    recv: &str,
) -> Option<String> {
    match recv {
        "self" | "Self" => enclosing_type(chain.iter(), pos).map(|t| t.name().to_string()),
        _ if is_cap(recv) => Some(recv.to_string()),
        _ if crate::semantic::is_primitive_ty(recv) => Some(recv.to_string()),
        _ => bindings::resolve(binds, pos, recv)
            .and_then(|b| b.ty_head())
            .or_else(|| super::infer::module_let_ty(chain.iter(), recv)),
    }
}

/// the type whose body contains `pos` — a class/struct/interface body,
/// or the target of the enclosing impl (via the method's owner). Over
/// any file window (the chain's iteration or the inference pass's
/// slice — one law, two shapes).
pub(crate) fn enclosing_type<'a, I>(files: I, pos: u32) -> Option<TyView<'a>>
where
    I: IntoIterator<Item = &'a MountedFile>,
{
    let mut best: Option<(u32, TyView<'a>)> = None;
    for f in files {
        for t in f.types() {
            if t.form() == TyForm::Enum || !contains(t.span(), pos) {
                continue;
            }
            if best
                .map(|(l, _)| t.span().hi - t.span().lo < l)
                .unwrap_or(true)
            {
                best = Some((t.span().hi - t.span().lo, t));
            }
        }
        for fv in f.fns() {
            // the impl's target — the owner string IS the type's name
            // (inherent impls own their methods)
            let Some(target) = fv.owner() else {
                continue;
            };
            if let Some(t) = f.ty(&target) {
                if contains(fv.span(), pos)
                    && best
                        .map(|(l, _)| fv.span().hi - fv.span().lo < l)
                        .unwrap_or(true)
                {
                    best = Some((fv.span().hi - fv.span().lo, t));
                }
            }
        }
    }
    best.map(|(_, t)| t)
}

/// `find_ty` over a plain file window — the inference pass rides
/// slices (a one-file window for a module-let's initializer, the
/// chain's pointer slice inside the binding pass); the law is the
/// [`crate::hover::collection::Chain::type_decl`] door's
/// mounted-over-embed rule, spelled once here for the window shape
pub(crate) fn find_ty<'a>(idxs: &[&'a MountedFile], name: &str) -> Option<TyView<'a>> {
    let mut fallback = None;
    for f in idxs.iter().copied() {
        if let Some(t) = f.ty(name) {
            if f.mount.src_path.is_none() {
                return Some(t);
            }
            if fallback.is_none() {
                fallback = Some(t);
            }
        }
    }
    fallback
}

/// the inherent impl-block fn `impl ty_name { fn member(..) }` — the
/// phase-3 callee rule, shared by the call-site PARAMETER hints and
/// signature help (one rule, three consumers now — the `member_target`
/// precedent). Own-surface methods (interface bodies, builtin/primitive
/// surfaces) have no recorded params: a hit there is FINAL (the
/// known-receiver-is-final law) and yields `None`, never a wrong
/// signature from a same-named impl fn in another file.
pub(crate) fn impl_method_fn<'a>(
    chain: &'a Chain<'a>,
    ty_name: &str,
    member: &str,
) -> Option<FnView<'a>> {
    for i in chain.iter() {
        if let Some(t) = i.ty(ty_name) {
            if t.methods().iter().any(|m| m.name() == member) {
                return None;
            }
        }
        for f in i.fns() {
            if f.name() == member && f.owner().as_deref() == Some(ty_name) {
                return Some(f);
            }
        }
    }
    None
}

/// the markdown a resolved member target renders to (hover's half of
/// the shared resolver; definition uses the target's spans instead)
pub(crate) fn render_target(t: MemberTarget) -> String {
    match t {
        MemberTarget::Member(_, m, via) => render_member(&m, via),
        MemberTarget::EnumMember(ty, _) => render_ty(&ty),
        MemberTarget::ImplFn(f) => render_fn_hits(&f),
        MemberTarget::IfaceMember(_, m) => render_member(&m, None),
    }
}
