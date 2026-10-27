//! Completions — member completion after `recv.`, bare-position
//! completion, the type tier at annotation boundaries, the use-path
//! tiers, the `::` tiers (module members by
//! tier, then the type-head lists: enum members, namespace statics, a
//! type's associated fns), and the auto-import tier, over the
//! same definition index hover uses. One rule for
//! the member list (mirrors the compiler's): **members of `T` = T's own
//! surface ∪ inherent `impl T { .. }` methods; on an interface `I` they
//! are exactly `I`'s declared signatures, use-gated when `I` is
//! foreign**. Heuristic, like hover: a
//! miss is an empty list, never wrong text.
//!
//! The use-path tiers: at `use <cursor>` the chain's module names
//! compete (every named index, std surface included — std is not
//! special); at `use mod::<cursor>` that module's exported public
//! names. The bare position rides the compiler's bare-name law: the
//! fn-local bindings ahead of the keyword table, then the document's
//! own decls + the chain's ambient builtin rows + the names the
//! document already `use`s — every other foreign name rides only the
//! auto-import tier (with the `use mod::Name;` insert, sorted after
//! the local items). Annotation boundaries (`let x: ⏐`, `-> T⏐`) get
//! the type tier: primitives + document/imported types, never
//! keywords, fns, or locals.

use std::collections::HashSet;

use ls_types::Range;
use rut_ast::ast::{Ast, ItemKind};
use rut_lexer::token::{Tok, Token};

use crate::hover::lookup::{find_ty, used_ifaces};
use crate::hover::types::{DefIndex, TyDef, TyForm};
use crate::line_index::LineIndex;
use crate::mods::{self, DocMods};

/// One completion item — plain data; the server maps it to LSP types.
#[derive(Debug, Clone)]
pub struct CompletionOut {
    pub label: String,
    /// declaration text shown next to the label (a signature, a field)
    pub detail: String,
    /// markdown documentation lines (may be empty)
    pub doc: Vec<String>,
    pub kind: CompletionKind,
    /// LSP `sortText` — the auto-import tier sorts after every local
    /// item (`~` prefix); `None` sorts by label as before
    pub sort_text: Option<String>,
    /// LSP `additionalTextEdits` — the auto-import insert
    /// (`use mod::Name;`), ranges over the normalized source (the same
    /// coordinate pipeline every other range rides)
    pub additional_text_edits: Vec<(Range, String)>,
}

impl CompletionOut {
    /// a plain item — no sort text, no edits (the keyword/local shape)
    fn plain(label: String, detail: String, doc: Vec<String>, kind: CompletionKind) -> Self {
        CompletionOut {
            label,
            detail,
            doc,
            kind,
            sort_text: None,
            additional_text_edits: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    Keyword,
    Module,
    Type,
    Interface,
    Const,
    Fn,
    Method,
    Field,
    EnumMember,
    /// a fn-local `let` binding (or loop variable) at a bare position
    Variable,
    /// a fn-local parameter at a bare position
    Parameter,
}

/// Completions at `pos`: use-path tiers inside a `use` statement
/// (walking a package's mod namespaces), qualified-position tiers
/// after `modname::` in the package's own files, member items after
/// `recv.` (filtered by the typed prefix), the type tier at an
/// annotation boundary, otherwise the visible
/// fn-local bindings ahead of the keyword table plus the visible decls
/// under the bare-name law plus the auto-import tier, deduped by
/// label. `src` is the NORMALIZED source — the
/// auto-import edits' coordinates ride the same text the tokens were
/// lexed from. `doc` is the open document's file-module context (the
/// face's knowledge; the default keeps position-path completion off).
pub fn complete(
    idxs: &[&DefIndex],
    toks: &[Token],
    ast: &Ast,
    pos: u32,
    src: &str,
    doc: &DocMods,
) -> Vec<CompletionOut> {
    // the binding pass, computed once: the member tier keys receivers on
    // it and the bare position's locals tier enumerates it
    let binds = crate::hover::bindings::collect(ast, toks, idxs);
    if let Some((segs, prefix)) = use_path_before(toks, pos) {
        // the OWN lane first — a head that speaks for the doc's own
        // package (its own name, or an own mod edge) rides the
        // position-path machinery: the intra-package walk and the
        // visible-members gate, exactly what the compiler's own-use
        // check accepts (`use imp::⏐` offers the module's visible
        // members). A foreign head keeps the cross-package tier.
        if let Some(out) = position_path_completions(idxs, doc, &segs, &prefix) {
            return out;
        }
        return use_path_completions(idxs, &segs, &prefix);
    }
    // a lone `:` (or the `->` arrow) under the cursor is the
    // annotation boundary — the type tier answers it: the primitive
    // table, the document's types, and the imported foreign types
    // (no keywords, fns, or locals — values never ride a type
    // position). `Vec<⏐` stays ambiguous at token level (`<` vs the
    // comparison) and is not this tier.
    if let Some(prefix) = annotation_boundary_before(toks, pos) {
        return type_tier_completions(idxs, &prefix);
    }
    if let Some((segs, prefix)) = qualified_path_before(toks, pos) {
        if let Some(out) = position_path_completions(idxs, doc, &segs, &prefix) {
            return out;
        }
        // the head did not resolve to a module — the `::` law's other
        // member lists ride the same trigger: enum members, namespace
        // statics, a type head's associated fns
        if let Some(out) = type_head_completions(idxs, &segs, &prefix) {
            return out;
        }
        // the head resolved to nothing known — `::` in an unknown
        // context falls through to the tiers below
    }
    let Some((recv, prefix)) = recv_before(toks, pos) else {
        // the dot guard: the request sits after a `.` the token tiers
        // could not parse a `recv.` pair out of (the receiver's token is
        // not a name — `f().`, `]` — or the text before the cursor ends
        // `.` / `.` + a half-typed member). That is a MEMBER request
        // with an unresolved receiver: a miss is an empty list, never
        // the bare keyword/host-fn dump. Text-backed, so a stale sync
        // (the tail still short the dot) cannot route around it.
        if dot_context_before(src, pos) {
            return Vec::new();
        }
        // the locals tier rides AHEAD of the keyword table — the name
        // being typed is most often a binding just declared
        let mut out = local_completions(&binds, pos);
        out.extend(bare_completions(idxs));
        out.extend(auto_import_completions(idxs, ast, toks, pos, src));
        // one final retain-by-label over the merged list (A4): the
        // extend order IS the precedence — locals > keywords >
        // document decls > imported > auto-import — so the first
        // occurrence of a label wins and a name reachable two ways
        // (e.g. bare via a `use`, and again via the auto-import tier)
        // offers exactly one item.
        let mut seen: HashSet<String> = HashSet::new();
        out.retain(|c| seen.insert(c.label.clone()));
        return out;
    };
    // the qualified-position spelling: positions qualify with `::` —
    // `helpers::mk(`, `let c: layout::Column` — so a receiver that
    // resolves as a MOD HEAD offers the module's members (the same
    // tier the `::` form rides). A resolved head is final; an unknown
    // one falls through to the member tier
    if let Some(out) = position_path_completions(idxs, doc, std::slice::from_ref(&recv), &prefix) {
        return out;
    }
    // the binding pass already ran at the top — member completion shares
    // hover's receiver inference (field reads, chained calls, loop
    // variables)
    member_completions(idxs, ast, &binds, pos, &recv, &prefix)
}

/// The member-completion context at `pos`: the receiver's source text
/// when the tokens before the cursor end with `recv.` — either the dot
/// just typed (`recv.⏐`) or a possibly partial member after it
/// (`recv.na⏐`, prefix `na`). A string receiver is the `str` primitive.
fn recv_before(toks: &[Token], pos: u32) -> Option<(String, String)> {
    let i = toks.iter().rposition(|t| t.span.lo < pos)?;
    let (recv_i, prefix) = match &toks[i].tok {
        Tok::Dot => (i.checked_sub(1)?, String::new()),
        Tok::Ident(s) if i >= 2 && toks[i - 1].tok == Tok::Dot => (i - 2, s.clone()),
        _ => return None,
    };
    let recv = match &toks[recv_i].tok {
        Tok::Ident(s) => s.clone(),
        // every string-literal form is a `str` receiver — `"a".len()`,
        // `r#"b"#.len()`, `f"{x}".len()`
        Tok::Str(_) | Tok::RawStr(_) | Tok::FStr(_) => "str".to_string(),
        _ => return None,
    };
    Some((recv, prefix))
}

/// the dot context, text-backed: the source before `pos` ends with a
/// `.` — right against the cursor (`recv.⏐`) or under one trailing
/// identifier run (`recv.na⏐`). The token tiers' backstop: when the
/// lexed stream carries no parsable `recv.` pair before the cursor (a
/// receiver that is not a name, a snapshot whose tail is short the
/// dot), this still tells a member request from a bare one. `pos` past
/// the source (a stale snapshot's tail) clamps to the end.
fn dot_context_before(src: &str, pos: u32) -> bool {
    let end = (pos as usize).min(src.len());
    let before = src[..end].trim_end();
    if before.ends_with('.') {
        return true;
    }
    let stripped = before.trim_end_matches(|c: char| c.is_ascii_alphanumeric() || c == '_');
    before.len() > stripped.len() && stripped.ends_with('.')
}

/// Members of the receiver's type: on an interface receiver, exactly
/// its declared signatures (the use-both gate keeps foreign interfaces
/// honest); on a type, fields, own methods, and inherent impl-block
/// methods — the mirror of hover's `member_target`.
fn member_completions(
    idxs: &[&DefIndex],
    ast: &Ast,
    binds: &[crate::hover::Binding],
    pos: u32,
    recv: &str,
    prefix: &str,
) -> Vec<CompletionOut> {
    let Some(ty_name) = crate::hover::lookup::recv_type(idxs, binds, pos, recv) else {
        return vec![];
    };
    // an interface receiver: the declared signatures are the whole
    // member list — satisfaction is structural, so no impl lookup
    // rides along. The use-both gate: a foreign interface's members
    // complete only when this document names the interface in a `use`;
    // an interface declared here is in scope natively
    if let Some((home, iface)) = crate::hover::lookup::iface_decl(idxs, &ty_name) {
        let mut out: Vec<CompletionOut> = Vec::new();
        if std::ptr::eq(home, idxs[0]) || used_ifaces(ast).contains(&iface.name) {
            for m in &iface.methods {
                push_item(
                    &mut out,
                    CompletionOut::plain(
                        m.name.clone(),
                        m.src.clone(),
                        m.doc.clone(),
                        CompletionKind::Method,
                    ),
                );
            }
        }
        if !prefix.is_empty() {
            out.retain(|c| c.label.starts_with(prefix));
        }
        out.sort_by(|a, b| a.label.cmp(&b.label));
        return out;
    }
    let Some((_ti, ty)) = crate::hover::lookup::find_ty(idxs, &ty_name) else {
        return vec![];
    };
    let mut out: Vec<CompletionOut> = Vec::new();
    for f in &ty.fields {
        push_item(
            &mut out,
            CompletionOut::plain(
                f.name.clone(),
                f.src.clone(),
                f.doc.clone(),
                CompletionKind::Field,
            ),
        );
    }
    for m in &ty.methods {
        push_item(
            &mut out,
            CompletionOut::plain(
                m.name.clone(),
                m.src.clone(),
                m.doc.clone(),
                CompletionKind::Method,
            ),
        );
    }
    // inherent impl-block methods — where methods live since type bodies
    // went fields-only; the owner string IS the target type's name
    for i in idxs {
        for f in &i.fns {
            if f.owner.as_deref() == Some(ty_name.as_str()) {
                push_item(
                    &mut out,
                    CompletionOut::plain(
                        f.name.clone(),
                        f.src.clone(),
                        f.doc.clone(),
                        CompletionKind::Method,
                    ),
                );
            }
        }
    }
    if !prefix.is_empty() {
        out.retain(|c| c.label.starts_with(prefix));
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

/// The locals tier at a bare position: every fn-local binding visible at
/// `pos`, under the same shadow-correct, scope-exit-correct rule the
/// resolve rides — per NAME the innermost in-scope declaration wins (an
/// inner re-decl replaces the outer inside its scope; past the block the
/// name is gone), and a binding whose own declaring ident holds the
/// cursor stays off (don't offer the name mid-typing at its own
/// declaration). Emitted ahead of the keyword table — the name being
/// typed is most often a binding just declared.
fn local_completions(binds: &[crate::hover::Binding], pos: u32) -> Vec<CompletionOut> {
    let mut best: Vec<&crate::hover::Binding> = Vec::new();
    for b in binds {
        if b.decl_ident_span.lo > pos || !crate::hover::lookup::contains(b.scope_span, pos) {
            continue; // declared later, or its scope ended before the cursor
        }
        if crate::hover::lookup::contains(b.decl_ident_span, pos) {
            continue; // the cursor sits in the declaration's own ident
        }
        match best.iter().position(|x| x.name == b.name) {
            Some(i) => {
                if best[i].decl_ident_span.lo < b.decl_ident_span.lo {
                    best[i] = b; // the inner shadow wins inside its scope
                }
            }
            None => best.push(b),
        }
    }
    let mut out: Vec<CompletionOut> = best
        .into_iter()
        .map(|b| {
            let kind = match b.kind {
                crate::hover::BindKind::Param => CompletionKind::Parameter,
                crate::hover::BindKind::Let | crate::hover::BindKind::IterVar => {
                    CompletionKind::Variable
                }
            };
            CompletionOut::plain(
                b.name.clone(),
                match &b.ty {
                    Some(ty) => format!("{} — {ty}", b.kind.label()),
                    None => b.kind.label().to_string(),
                },
                Vec::new(),
                kind,
            )
        })
        .collect();
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

/// Bare position: the keyword table (the grammar's reserved
/// set, one canonical list), then the visible decls under the
/// compiler's own bare-name law: the open document's decls
/// (`idxs[0]`), the chain's AMBIENT builtin rows (`prelude builtin` —
/// bound in every unit, no `use` needed), and the names the document
/// already `use`s (matched `(name, pkg)` against `idxs[0].uses` — the
/// same set shape the auto-import tier dedups against). Every other
/// foreign name rides ONLY the auto-import tier (with the `use`
/// insert edit): a plain offer the compile would reject is a trap.
/// Free fns and types only; methods are reached through a receiver,
/// not named bare.
fn bare_completions(idxs: &[&DefIndex]) -> Vec<CompletionOut> {
    let mut out: Vec<CompletionOut> = Vec::new();
    for kw in rut_parser::RESERVED_KW {
        out.push(CompletionOut::plain(
            (*kw).to_string(),
            String::new(),
            Vec::new(),
            CompletionKind::Keyword,
        ));
    }
    let doc = idxs[0];
    let imported = crate::definition::import_map(doc);
    for i in idxs {
        let own = std::ptr::eq(*i, doc);
        // the names this document's `use`s bound FROM this index's
        // package — the same (name, pkg) match the definition layer's
        // use-graph walk reads (`matches_pkg`, module name first, the
        // origin/stem derivation on an unnamed index)
        let used_from: HashSet<&str> = imported
            .iter()
            .filter(|(_, pkg)| crate::definition::matches_pkg(i, pkg))
            .map(|(name, _)| *name)
            .collect();
        // the bare gate per row: own decls always; foreign rows only
        // when the ambient linkage bound them or the document's `use`
        // named them from this package
        let offered = |name: &str, ambient: bool| own || ambient || used_from.contains(name);
        for t in &i.types {
            if !offered(&t.name, t.ambient) {
                continue;
            }
            push_item(
                &mut out,
                CompletionOut::plain(
                    t.name.clone(),
                    format!("{} {}{}", t.form.keyword(), t.name, gens(t)),
                    t.doc.clone(),
                    match t.form {
                        TyForm::Interface => CompletionKind::Interface,
                        _ => CompletionKind::Type,
                    },
                ),
            );
        }
        for f in &i.fns {
            if f.owner.is_some() {
                continue;
            }
            if !offered(&f.name, f.ambient) {
                continue;
            }
            push_item(
                &mut out,
                CompletionOut::plain(
                    f.name.clone(),
                    f.src.clone(),
                    f.doc.clone(),
                    CompletionKind::Fn,
                ),
            );
        }
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

// ---- the use-path tiers ----

/// the use-path context at `pos`: the spelled path segments plus the
/// typed prefix when the tokens before the cursor run back to a `use`
/// keyword — `use ⏐` (no segments, the module-name tier), `use
/// pouch::⏐` / `use pouch::V⏐` / `use pouch::layout::V⏐` /
/// `use pouch::{V⏐` (segments `["pouch"]` / `["pouch", "layout"]`, the
/// names tier). The walk crosses idents, `::`, braces, and commas
/// only — any other token ends the path.
fn use_path_before(toks: &[Token], pos: u32) -> Option<(Vec<String>, String)> {
    let i = toks.iter().rposition(|t| t.span.lo < pos)?;
    let mut seg: Vec<&Token> = Vec::new();
    let mut j = i;
    loop {
        let t = &toks[j];
        match &t.tok {
            // the anchor: the reserved `use`
            Tok::Ident(s) if rut_parser::is_reserved_kw(s) => {
                if s != "use" {
                    return None;
                }
                break;
            }
            Tok::Ident(_) | Tok::Colon | Tok::Colon2 | Tok::LBrace | Tok::Comma => seg.insert(0, t),
            _ => return None,
        }
        if j == 0 {
            return None;
        }
        j -= 1;
    }
    // no `::` typed yet — still the module-NAME tier (the partial
    // ident is the prefix, the segments stay empty); a `::` anywhere
    // switches to the names tier of the spelled path. The BRACE form
    // (`use pkg::{ A, B⏐`) splits at the `{`: the path is what runs
    // before it — the names inside the braces are the use-leaf list,
    // never path segments (`use gadgets::{ tag, ⏐` still answers the
    // root exports) — and the typed prefix is the list's trailing
    // ident, if any.
    let brace = seg.iter().rposition(|t| t.tok == Tok::LBrace);
    let (path_seg, list_seg) = match brace {
        Some(b) => (&seg[..b], &seg[b + 1..]),
        None => (&seg[..], &seg[..]),
    };
    let has_sep = path_seg
        .iter()
        .any(|t| matches!(t.tok, Tok::Colon | Tok::Colon2));
    let mut segments: Vec<String> = if has_sep {
        path_seg
            .iter()
            .filter_map(|t| match &t.tok {
                Tok::Ident(s) if !rut_parser::is_reserved_kw(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    } else {
        Vec::new()
    };
    // the flat spelling's trailing ident is the prefix carrier, never
    // a segment to walk (`use pkg::layout::Co⏐` walks `layout` only);
    // the brace form's path never carries one (the list's partial
    // leaf is the prefix alone)
    if let Some(Token {
        tok: Tok::Ident(s), ..
    }) = path_seg.last()
    {
        if !rut_parser::is_reserved_kw(s) {
            segments.pop();
        }
    }
    let prefix = match list_seg.last() {
        Some(Token {
            tok: Tok::Ident(s),
            span,
        }) if !rut_parser::is_reserved_kw(s) => {
            let end = (pos as usize).min(span.hi as usize).max(span.lo as usize);
            s.get(..end - span.lo as usize).unwrap_or(s).to_string()
        }
        _ => String::new(),
    };
    Some((segments, prefix))
}

/// `use <cursor>`: the chain's module names — every NAMED index, the
/// std surface included (std is not special; the names come from the
/// uniform index set, no hardcoded table).
///
/// `use pkg::<cursor>`: the module's exported public names — pub
/// types, free fns, module lets — PLUS its `pub mod` children as
/// walkable path segments. Deeper paths (`use pkg::a::b::<cursor>`)
/// walk the pub edges first (a bare `mod`/`pub(pkg)` child is never
/// offered cross-package) and answer the resolved module's exports —
/// now with the `pub` gate on fns/lets too, the gate the compiler's
/// qualified-use check applies. The std surface is NOT special-cased:
/// std packages carry no mod children, so the walk answers their flat
/// exports exactly as before (the deps-batch law stands — a flat
/// package's chain and gates are today's, byte for byte).
fn use_path_completions(idxs: &[&DefIndex], segs: &[String], prefix: &str) -> Vec<CompletionOut> {
    let Some(pkg) = segs.first() else {
        let mut out: Vec<CompletionOut> = Vec::new();
        for i in idxs {
            let Some(m) = &i.module else { continue };
            push_item(
                &mut out,
                CompletionOut::plain(
                    m.clone(),
                    i.origin.clone(),
                    Vec::new(),
                    CompletionKind::Module,
                ),
            );
        }
        out.retain(|c| prefix.is_empty() || c.label.starts_with(prefix));
        out.sort_by(|a, b| a.label.cmp(&b.label));
        return out;
    };
    // the import is the gate-opener, so the `pub builtin` gate does
    // not bind here (this IS the use that names them)
    let chain: Vec<&DefIndex> = idxs
        .iter()
        .copied()
        .filter(|i| crate::definition::matches_pkg(i, pkg))
        .collect();
    if chain.is_empty() {
        return Vec::new();
    }
    // resolve the walked module: the flat spelling answers the root —
    // for a mod-carrying package that is the ROOT index only (members
    // live under their module path), for a flat package today's whole
    // chain; deeper segments walk the pub edges (`use_path_before`
    // stripped the trailing PARTIAL ident already — `use pkg::V⏐`
    // offers the root; `use pkg::layout::Co⏐` walks `layout` only)
    let carrying = chain.iter().any(|i| i.mod_path.is_some());
    // the walked interior: the segments minus the pkg head —
    // `use_path_before` already stripped the trailing PARTIAL ident
    // (`use pkg::layout::Co⏐` walks `layout` only; the brace form's
    // path never carries one)
    let interior: &[String] = &segs[1..];
    let walked = if interior.is_empty() {
        Some(String::new())
    } else {
        mods::walk_use_mods(idxs, pkg, interior)
    };
    let Some(path) = walked else {
        return Vec::new();
    };
    let scope: Vec<&DefIndex> = if carrying {
        chain
            .iter()
            .copied()
            .filter(|i| mods::mod_path_of(i) == path)
            .collect()
    } else {
        chain
    };
    let crossed = !interior.is_empty();
    let mut out: Vec<CompletionOut> = Vec::new();
    for i in &scope {
        // the walkable children — pub edges only (cross-package law)
        for m in &i.mods {
            if m.vis != rut_ast::ast::Vis::Pub {
                continue;
            }
            let child = mods::child_path(&path, &m.name);
            push_item(
                &mut out,
                CompletionOut::plain(
                    m.name.clone(),
                    format!(
                        "file module — {}",
                        mods::display_of(idxs, Some(pkg.as_str()), &child)
                    ),
                    m.doc.clone(),
                    CompletionKind::Module,
                ),
            );
        }
        for t in &i.types {
            if !t.is_pub {
                continue;
            }
            push_item(
                &mut out,
                CompletionOut::plain(
                    t.name.clone(),
                    format!("{} {}{}", t.form.keyword(), t.name, gens(t)),
                    t.doc.clone(),
                    match t.form {
                        TyForm::Interface => CompletionKind::Interface,
                        TyForm::Namespace => CompletionKind::Module,
                        _ => CompletionKind::Type,
                    },
                ),
            );
        }
        for f in &i.fns {
            if f.owner.is_some() {
                continue;
            }
            // a walked path binds through the compiler's qualified-use
            // gate: only `pub` names cross. The flat spelling keeps
            // today's ungated shape (the deps-batch law)
            if crossed && f.vis != rut_ast::ast::Vis::Pub {
                continue;
            }
            push_item(
                &mut out,
                CompletionOut::plain(
                    f.name.clone(),
                    f.src.clone(),
                    f.doc.clone(),
                    CompletionKind::Fn,
                ),
            );
        }
        for l in &i.lets {
            if crossed && l.vis != rut_ast::ast::Vis::Pub {
                continue;
            }
            push_item(
                &mut out,
                CompletionOut::plain(
                    l.name.clone(),
                    l.src.clone(),
                    l.doc.clone(),
                    CompletionKind::Const,
                ),
            );
        }
    }
    out.retain(|c| prefix.is_empty() || c.label.starts_with(prefix));
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

// ---- the qualified-position tier (intra-package) ----

/// the qualified-position context at `pos`: the path segments plus the
/// typed prefix when the tokens before the cursor end a `a::b::` run
/// OUTSIDE a `use` — type positions spell `::` (`let c: layout::Co⏐`,
/// `Vec<layout::⏐`). The walk-back accepts non-reserved idents and
/// colons only; a `::` separator is exactly TWO colons (a single `:` —
/// an annotation — ends the path, and two idents without a separator
/// are not a path run).
fn qualified_path_before(toks: &[Token], pos: u32) -> Option<(Vec<String>, String)> {
    let i = toks.iter().rposition(|t| t.span.lo < pos)?;
    let mut run: Vec<&Token> = Vec::new();
    let mut j = i;
    loop {
        match &toks[j].tok {
            Tok::Ident(s) if !rut_parser::is_reserved_kw(s) => run.insert(0, &toks[j]),
            Tok::Colon | Tok::Colon2 => run.insert(0, &toks[j]),
            // the anchor — anything else (including reserved keywords)
            _ => break,
        }
        if j == 0 {
            break;
        }
        j -= 1;
    }
    let mut segs: Vec<String> = Vec::new();
    let mut pending: Option<String> = None;
    let mut colons = 0usize;
    let mut ended_on_ident = false;
    for t in &run {
        match &t.tok {
            // `::` lexes as ONE Colon2 token now — the same two-colon
            // count the separator law reads
            Tok::Colon => colons += 1,
            Tok::Colon2 => colons += 2,
            Tok::Ident(s) => {
                match colons {
                    0 => {}
                    // exactly two = the separator; ONE before an ident
                    // is the annotation boundary (`let c: layout::`) —
                    // the path STARTS at that ident, the run before it
                    // is the anchor's side
                    2 => segs.push(pending.take()?), // `::` with no head is not a path
                    _ => {
                        segs.clear();
                    }
                }
                pending = Some(s.clone());
                colons = 0;
                ended_on_ident = true;
            }
            _ => unreachable!("the run collected idents and colons only"),
        }
    }
    // a trailing separator: exactly `::` completes the head (a single
    // trailing `:` is an annotation, not a path)
    if colons > 0 {
        if colons != 2 {
            return None;
        }
        segs.push(pending.take()?);
    }
    if segs.is_empty() {
        return None; // no `::` — the bare tiers answer
    }
    let prefix = if ended_on_ident {
        pending.take().unwrap_or_default()
    } else {
        String::new()
    };
    Some((segs, prefix))
}

/// the annotation boundary: a lone `:` (one colon — `::` lexes as its
/// own Colon2) or the `->` arrow immediately before the cursor, with
/// at most a partial ident riding it (`let c: ⏐`, `id: u⏐`,
/// `fn f() -> T⏐`). Returns the typed partial ident (possibly empty) —
/// the type tier's prefix. Any other shape is not a type position.
fn annotation_boundary_before(toks: &[Token], pos: u32) -> Option<String> {
    let i = toks.iter().rposition(|t| t.span.lo < pos)?;
    match &toks[i].tok {
        Tok::Colon | Tok::Arrow => Some(String::new()),
        Tok::Ident(s) if !rut_parser::is_reserved_kw(s) && i >= 1 => {
            // the ident's predecessor must be the boundary itself
            if !(toks[i - 1].tok == Tok::Colon || toks[i - 1].tok == Tok::Arrow) {
                return None;
            }
            let span = toks[i].span;
            let end = (pos as usize).min(span.hi as usize).max(span.lo as usize);
            Some(s.get(..end - span.lo as usize).unwrap_or(s).to_string())
        }
        _ => None,
    }
}

/// The type tier at an annotation boundary — `let x: ⏐`, `id: u⏐`,
/// `fn f() -> ⏐`: the parser's primitive table (the ONE
/// `PRIMITIVE_TYS` — enumerated, never a duplicated list), the open
/// document's own types, the chain's AMBIENT builtin rows, and the
/// foreign types the document already `use`s. Keywords, fns, and
/// locals stay off: values never ride a type position.
fn type_tier_completions(idxs: &[&DefIndex], prefix: &str) -> Vec<CompletionOut> {
    let mut out: Vec<CompletionOut> = Vec::new();
    for p in rut_parser::PRIMITIVE_TYS {
        push_item(
            &mut out,
            CompletionOut::plain(
                (*p).to_string(),
                "primitive".to_string(),
                Vec::new(),
                CompletionKind::Type,
            ),
        );
    }
    let doc = idxs[0];
    let imported = crate::definition::import_map(doc);
    for i in idxs {
        let own = std::ptr::eq(*i, doc);
        let used_from: HashSet<&str> = imported
            .iter()
            .filter(|(_, pkg)| crate::definition::matches_pkg(i, pkg))
            .map(|(name, _)| *name)
            .collect();
        for t in &i.types {
            // the same bare gate the decl tiers read: own types always,
            // foreign types when ambient or imported from this package
            if !own && !t.ambient && !used_from.contains(t.name.as_str()) {
                continue;
            }
            push_item(
                &mut out,
                CompletionOut::plain(
                    t.name.clone(),
                    format!("{} {}{}", t.form.keyword(), t.name, gens(t)),
                    t.doc.clone(),
                    match t.form {
                        TyForm::Interface => CompletionKind::Interface,
                        TyForm::Namespace => CompletionKind::Module,
                        _ => CompletionKind::Type,
                    },
                ),
            );
        }
    }
    out.retain(|c| prefix.is_empty() || c.label.starts_with(prefix));
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

/// Member completions after `<TypeHead>::` — the `::` law's member
/// lists whose head is a type, reusing the index lookup hover's member
/// lane keys on (`find_ty`): an enum head offers its members, a
/// namespace head (a dep's minted `Math`) its consts + fns, any other
/// type head its statics and associated fns (the inherent `impl T { fn
/// .. }` rows — the owner string IS the type's name). An interface head
/// offers nothing: its signatures ride receivers, never `::`. `None` =
/// the head resolves to no indexed type (the caller falls through); a
/// resolved head is FINAL — the filtered list is the answer even when
/// empty (the known-head law, like the module tier's).
fn type_head_completions(
    idxs: &[&DefIndex],
    segs: &[String],
    prefix: &str,
) -> Option<Vec<CompletionOut>> {
    let (head, rest) = segs.split_first()?;
    let (_ti, ty) = find_ty(idxs, head)?;
    let mut out: Vec<CompletionOut> = Vec::new();
    // a deeper walk under a type head descends nothing — types are not
    // namespaces (`Color::Red::` answers empty, final)
    if rest.is_empty() {
        match ty.form {
            // `Color::⏐` — the declared members
            TyForm::Enum => {
                for m in &ty.fields {
                    push_item(
                        &mut out,
                        CompletionOut::plain(
                            m.name.clone(),
                            m.src.clone(),
                            m.doc.clone(),
                            CompletionKind::EnumMember,
                        ),
                    );
                }
            }
            // `Math::⏐` — the consts as fields, the module's fns as
            // methods (the namespace mint's shape)
            TyForm::Namespace => {
                for f in &ty.fields {
                    push_item(
                        &mut out,
                        CompletionOut::plain(
                            f.name.clone(),
                            f.src.clone(),
                            f.doc.clone(),
                            CompletionKind::Const,
                        ),
                    );
                }
                for m in &ty.methods {
                    push_item(
                        &mut out,
                        CompletionOut::plain(
                            m.name.clone(),
                            m.src.clone(),
                            m.doc.clone(),
                            CompletionKind::Fn,
                        ),
                    );
                }
            }
            // `Greeter::⏐` — interface members ride receivers
            TyForm::Interface => {}
            // `Counter::⏐` — statics first, then the associated fns
            _ => {
                for f in &ty.fields {
                    // a declared static — the rendered slice leads with
                    // its modifiers (`static N: i32`, `pub static N: i32`)
                    let bare = f
                        .src
                        .strip_prefix("pub ")
                        .or_else(|| f.src.strip_prefix("pub(super) "))
                        .or_else(|| f.src.strip_prefix("pub(pkg) "))
                        .unwrap_or(&f.src);
                    if bare.starts_with("static ") {
                        push_item(
                            &mut out,
                            CompletionOut::plain(
                                f.name.clone(),
                                f.src.clone(),
                                f.doc.clone(),
                                CompletionKind::Field,
                            ),
                        );
                    }
                }
                for i in idxs {
                    for f in &i.fns {
                        if f.owner.as_deref() == Some(head.as_str()) {
                            push_item(
                                &mut out,
                                CompletionOut::plain(
                                    f.name.clone(),
                                    f.src.clone(),
                                    f.doc.clone(),
                                    CompletionKind::Fn,
                                ),
                            );
                        }
                    }
                }
            }
        }
    }
    out.retain(|c| prefix.is_empty() || c.label.starts_with(prefix));
    out.sort_by(|a, b| a.label.cmp(&b.label));
    Some(out)
}

/// the leaf scope for a resolved mod path — the open document itself
/// when the path IS its own module (the pkg head resolving to the
/// root from the root), else the chain index stamped for it
fn scope_index<'a>(
    idxs: &[&'a DefIndex],
    doc: &DocMods,
    pkg: Option<&str>,
    path: &str,
) -> Option<&'a DefIndex> {
    if doc.mod_path.as_deref() == Some(path) {
        return Some(idxs[0]);
    }
    mods::find_mod(idxs, pkg, path)
}

/// Position-path completions: after `modname::` in a package's own
/// file, the resolved module's members VISIBLE to the current file —
/// the phase-3 tier predicate (`private` reaches the declaring module
/// and descendants, `pub(super)` the parent's subtree, `pub(pkg)` the
/// whole package, `pub` everywhere) applied to every row. Declared
/// children of any edge vis ride as walkable segments (the
/// intra-package walk gates leaves, not heads). `None` = the head did
/// not resolve to a module (the caller falls through).
fn position_path_completions(
    idxs: &[&DefIndex],
    doc: &DocMods,
    segs: &[String],
    prefix: &str,
) -> Option<Vec<CompletionOut>> {
    let (head, rest) = segs.split_first()?;
    let pkg = doc.pkg.as_deref();
    let doc_index = idxs[0];
    let start = mods::resolve_position_head(doc, doc_index, idxs, head)?;
    let path = mods::walk_position_mods(idxs, pkg, &start, rest)?;
    let from = doc.mod_path.as_deref().unwrap_or("");
    let Some(scope) = scope_index(idxs, doc, pkg, &path) else {
        return Some(Vec::new());
    };
    let mut out: Vec<CompletionOut> = Vec::new();
    // the walkable children — any declared edge (intra-package)
    for m in &scope.mods {
        let child = mods::child_path(&path, &m.name);
        push_item(
            &mut out,
            CompletionOut::plain(
                m.name.clone(),
                format!("file module — {}", mods::display_of(idxs, pkg, &child)),
                m.doc.clone(),
                CompletionKind::Module,
            ),
        );
    }
    for t in &scope.types {
        if !mods::vis_allows(from, &path, t.vis) {
            continue;
        }
        push_item(
            &mut out,
            CompletionOut::plain(
                t.name.clone(),
                format!("{} {}{}", t.form.keyword(), t.name, gens(t)),
                t.doc.clone(),
                match t.form {
                    TyForm::Interface => CompletionKind::Interface,
                    TyForm::Namespace => CompletionKind::Module,
                    _ => CompletionKind::Type,
                },
            ),
        );
    }
    for f in &scope.fns {
        if f.owner.is_some() || !mods::vis_allows(from, &path, f.vis) {
            continue;
        }
        push_item(
            &mut out,
            CompletionOut::plain(
                f.name.clone(),
                f.src.clone(),
                f.doc.clone(),
                CompletionKind::Fn,
            ),
        );
    }
    for l in &scope.lets {
        if !mods::vis_allows(from, &path, l.vis) {
            continue;
        }
        push_item(
            &mut out,
            CompletionOut::plain(
                l.name.clone(),
                l.src.clone(),
                l.doc.clone(),
                CompletionKind::Const,
            ),
        );
    }
    out.retain(|c| prefix.is_empty() || c.label.starts_with(prefix));
    out.sort_by(|a, b| a.label.cmp(&b.label));
    Some(out)
}

// ---- the auto-import tier ----

/// the foreign public names (indexes whose module is named) matching
/// the bare prefix, each carrying the `use mod::Name;` insert — after
/// the local results (the caller extends bare_completions' output), so
/// `sortText` sorts them after everything already offered. Names the
/// document already imports are NOT re-offered, the `pub builtin`
/// gate keeps holding (an import offer for a gated name would complete
/// a name the compile still rejects until the use lands — the gate is
/// the compiler's ambient split, mirrored), and a TRANSITIVE-only dep
/// stays off: the compiler's closure check
/// (`Session::resolve_use` → `declares`) refuses `use` of a
/// mounted-but-undeclared package, so only indexes stamped
/// `direct_dep` (the open document's own manifest rows, plus the std
/// surface) ride this tier.
fn auto_import_completions(
    idxs: &[&DefIndex],
    ast: &Ast,
    toks: &[Token],
    pos: u32,
    src: &str,
) -> Vec<CompletionOut> {
    let prefix = bare_prefix(toks, pos);
    let imported: HashSet<(&str, &str)> = idxs[0]
        .uses
        .iter()
        .map(|u| (u.name.as_str(), u.pkg.as_str()))
        .collect();
    let gated: HashSet<&str> = idxs
        .iter()
        .flat_map(|i| i.pub_gated.iter().map(|s| s.as_str()))
        .collect();
    let insert = import_insert(src, ast, toks);
    let mut out: Vec<CompletionOut> = Vec::new();
    for i in idxs.iter().skip(1) {
        let Some(m) = &i.module else { continue };
        if !i.direct_dep {
            continue; // a transitive-only dep — `use` of it is the D5 error
        }
        let mut foreign = |name: &str, doc: &[String], kind: CompletionKind| {
            if imported.contains(&(name, m.as_str())) {
                return; // already imported — never re-offered
            }
            if gated.contains(name) {
                return; // the ambient split keeps holding
            }
            if !prefix.is_empty() && !name.starts_with(&prefix) {
                return;
            }
            // a fresh line either way: after a use the newline leads,
            // at the body top it trails — `rut fmt` owns any tidying
            let text = if insert.1 {
                format!("\nuse {m}::{name};")
            } else {
                format!("use {m}::{name};\n")
            };
            out.push(CompletionOut {
                label: name.to_string(),
                detail: format!("{m}::{name} — import"),
                doc: doc.to_vec(),
                kind,
                sort_text: Some(format!("~{name}")),
                additional_text_edits: vec![(insert.0, text)],
            });
        };
        for t in &i.types {
            if !t.is_pub {
                continue;
            }
            foreign(
                &t.name,
                &t.doc,
                match t.form {
                    TyForm::Interface => CompletionKind::Interface,
                    TyForm::Namespace => CompletionKind::Module,
                    _ => CompletionKind::Type,
                },
            );
        }
        for f in &i.fns {
            if f.owner.is_some() {
                continue;
            }
            foreign(&f.name, &f.doc, CompletionKind::Fn);
        }
        for l in &i.lets {
            foreign(&l.name, &l.doc, CompletionKind::Const);
        }
    }
    // label dedup across modules: the first module (chain order) wins,
    // so one name offers one import
    let mut seen: HashSet<String> = HashSet::new();
    out.retain(|c| seen.insert(c.label.clone()));
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

/// the bare-position prefix: the partial ident the cursor sits in
/// (`Ve⏐` → `Ve`), else empty
fn bare_prefix(toks: &[Token], pos: u32) -> String {
    let Some(t) = toks.iter().rposition(|t| t.span.lo < pos).map(|i| &toks[i]) else {
        return String::new();
    };
    match &t.tok {
        Tok::Ident(s) if !rut_parser::is_reserved_kw(s) => {
            let end = (pos as usize)
                .min(t.span.hi as usize)
                .max(t.span.lo as usize);
            s.get(..end - t.span.lo as usize).unwrap_or(s).to_string()
        }
        _ => String::new(),
    }
}

/// where an auto-import's `use mod::Name;` inserts: after the last
/// LEADING `use` (the AST's leading Use run — the edit lands right
/// after its `;`, on a fresh line), else at the top of the file body —
/// after the leading comment/blank lines. Returns the insertion range
/// over the normalized source (the same LineIndex every other range
/// rides) plus which side the newline goes on: after a use the text
/// leads with `\n` (the line continues below), at the body top it
/// trails with `\n`.
fn import_insert(src: &str, ast: &Ast, toks: &[Token]) -> (Range, bool) {
    let lines = LineIndex::new(src);
    let after_use = leading_use_end(ast, toks);
    let (offset, newline_first) = match after_use {
        Some(off) => (off, true),
        None => (body_start(src), false),
    };
    let (line, ch) = lines.position(src, offset);
    (
        Range {
            start: ls_types::Position {
                line,
                character: ch,
            },
            end: ls_types::Position {
                line,
                character: ch,
            },
        },
        newline_first,
    )
}

/// the source end of the last `use` in the leading Use run — `None`
/// when the module opens with anything else. Statement spans end at
/// the NEXT token (the parser's convention), so the verbatim end is
/// the item's own `;` (the build pass's cut, mirrored here)
fn leading_use_end(ast: &Ast, toks: &[Token]) -> Option<u32> {
    let mut last: Option<u32> = None;
    for h in ast.module_items(ast.root) {
        match ast.item(*h) {
            ItemKind::Use { .. } => {
                let span = ast.span(h.id());
                let hi = toks
                    .iter()
                    .filter(|t| t.span.lo >= span.lo && t.span.hi <= span.hi && t.tok == Tok::Semi)
                    .map(|t| t.span.hi)
                    .next_back()
                    .unwrap_or(span.hi);
                last = Some(hi);
            }
            // the run ends at the first item that isn't a use
            _ => break,
        }
    }
    last
}

/// the byte offset where the file body starts: past the leading blank
/// and `//` comment lines (a file of only comments → the end)
fn body_start(src: &str) -> u32 {
    let mut off = 0usize;
    for line in src.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            off += line.len();
        } else {
            break;
        }
    }
    off as u32
}

/// first definition wins — the document shadows the std surface
fn push_item(out: &mut Vec<CompletionOut>, c: CompletionOut) {
    if !out.iter().any(|x| x.label == c.label) {
        out.push(c);
    }
}

fn gens(t: &TyDef) -> String {
    if t.generics.is_empty() {
        String::new()
    } else {
        format!("<{}>", t.generics.join(", "))
    }
}

// ---- the LSP mapping ----

use ls_types::{
    CompletionItem, CompletionItemKind, Documentation, MarkupContent, MarkupKind, TextEdit,
};

/// plain completion data → the LSP item (kind icons, detail, docs,
/// the auto-import tier's sort text + additional edits) — shared by
/// the stdio server and the wasm shim
pub fn lsp_item(c: CompletionOut) -> CompletionItem {
    CompletionItem {
        label: c.label,
        kind: Some(match c.kind {
            CompletionKind::Keyword => CompletionItemKind::KEYWORD,
            CompletionKind::Module => CompletionItemKind::MODULE,
            CompletionKind::Type => CompletionItemKind::CLASS,
            CompletionKind::Interface => CompletionItemKind::INTERFACE,
            CompletionKind::Const => CompletionItemKind::CONSTANT,
            CompletionKind::Fn => CompletionItemKind::FUNCTION,
            CompletionKind::Method => CompletionItemKind::METHOD,
            CompletionKind::Field => CompletionItemKind::FIELD,
            CompletionKind::EnumMember => CompletionItemKind::ENUM_MEMBER,
            CompletionKind::Variable => CompletionItemKind::VARIABLE,
            // LSP spells no parameter kind — the variable icon carries it
            CompletionKind::Parameter => CompletionItemKind::VARIABLE,
        }),
        detail: (!c.detail.is_empty()).then_some(c.detail),
        documentation: (!c.doc.is_empty()).then(|| {
            Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: c.doc.join("\n\n"),
            })
        }),
        sort_text: c.sort_text,
        additional_text_edits: (!c.additional_text_edits.is_empty()).then(|| {
            c.additional_text_edits
                .into_iter()
                .map(|(range, new_text)| TextEdit { range, new_text })
                .collect()
        }),
        ..Default::default()
    }
}
