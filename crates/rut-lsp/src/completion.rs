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
        CompletionOut { label, detail, doc, kind, sort_text: None, additional_text_edits: Vec::new() }
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
        Tok::Str(_) => "str".to_string(),
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
                    CompletionOut::plain(m.name.clone(), m.src.clone(), m.doc.clone(), CompletionKind::Method),
                );
            }
        }
        if !prefix.is_empty() {
            out.retain(|c| c.label.starts_with(prefix));
        }
        out.sort_by(|a, b| a.label.cmp(&b.label));
        return out;
    }
    let Some((_ti, ty)) = crate::hover::lookup::find_ty(idxs, &ty_name) else { return vec![] };
    let mut out: Vec<CompletionOut> = Vec::new();
    for f in &ty.fields {
        push_item(
            &mut out,
            CompletionOut::plain(f.name.clone(), f.src.clone(), f.doc.clone(), CompletionKind::Field),
        );
    }
    for m in &ty.methods {
        push_item(
            &mut out,
            CompletionOut::plain(m.name.clone(), m.src.clone(), m.doc.clone(), CompletionKind::Method),
        );
    }
    // inherent impl-block methods — where methods live since type bodies
    // went fields-only; the owner string IS the target type's name
    for i in idxs {
        for f in &i.fns {
            if f.owner.as_deref() == Some(ty_name.as_str()) {
                push_item(
                    &mut out,
                    CompletionOut::plain(f.name.clone(), f.src.clone(), f.doc.clone(), CompletionKind::Method),
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
                crate::hover::BindKind::Let | crate::hover::BindKind::IterVar => CompletionKind::Variable,
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
        out.push(CompletionOut::plain((*kw).to_string(), String::new(), Vec::new(), CompletionKind::Keyword));
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
                CompletionOut::plain(f.name.clone(), f.src.clone(), f.doc.clone(), CompletionKind::Fn),
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
    let has_sep = path_seg.iter().any(|t| matches!(t.tok, Tok::Colon | Tok::Colon2));
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
    if let Some(Token { tok: Tok::Ident(s), .. }) = path_seg.last() {
        if !rut_parser::is_reserved_kw(s) {
            segments.pop();
        }
    }
    let prefix = match list_seg.last() {
        Some(Token { tok: Tok::Ident(s), span }) if !rut_parser::is_reserved_kw(s) => {
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
                CompletionOut::plain(m.clone(), i.origin.clone(), Vec::new(), CompletionKind::Module),
            );
        }
        out.retain(|c| prefix.is_empty() || c.label.starts_with(prefix));
        out.sort_by(|a, b| a.label.cmp(&b.label));
        return out;
    };
    // the import is the gate-opener, so the `pub builtin` gate does
    // not bind here (this IS the use that names them)
    let chain: Vec<&DefIndex> = idxs.iter().copied().filter(|i| crate::definition::matches_pkg(i, pkg)).collect();
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
    let Some(path) = walked else { return Vec::new() };
    let scope: Vec<&DefIndex> = if carrying {
        chain.iter().copied().filter(|i| mods::mod_path_of(i) == path).collect()
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
                    format!("file module — {}", mods::display_of(idxs, Some(pkg.as_str()), &child)),
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
                CompletionOut::plain(f.name.clone(), f.src.clone(), f.doc.clone(), CompletionKind::Fn),
            );
        }
        for l in &i.lets {
            if crossed && l.vis != rut_ast::ast::Vis::Pub {
                continue;
            }
            push_item(
                &mut out,
                CompletionOut::plain(l.name.clone(), l.src.clone(), l.doc.clone(), CompletionKind::Const),
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
            CompletionOut::plain((*p).to_string(), "primitive".to_string(), Vec::new(), CompletionKind::Type),
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
                        CompletionOut::plain(m.name.clone(), m.src.clone(), m.doc.clone(), CompletionKind::EnumMember),
                    );
                }
            }
            // `Math::⏐` — the consts as fields, the module's fns as
            // methods (the namespace mint's shape)
            TyForm::Namespace => {
                for f in &ty.fields {
                    push_item(
                        &mut out,
                        CompletionOut::plain(f.name.clone(), f.src.clone(), f.doc.clone(), CompletionKind::Const),
                    );
                }
                for m in &ty.methods {
                    push_item(
                        &mut out,
                        CompletionOut::plain(m.name.clone(), m.src.clone(), m.doc.clone(), CompletionKind::Fn),
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
                            CompletionOut::plain(f.name.clone(), f.src.clone(), f.doc.clone(), CompletionKind::Field),
                        );
                    }
                }
                for i in idxs {
                    for f in &i.fns {
                        if f.owner.as_deref() == Some(head.as_str()) {
                            push_item(
                                &mut out,
                                CompletionOut::plain(f.name.clone(), f.src.clone(), f.doc.clone(), CompletionKind::Fn),
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
    let Some(start) = mods::resolve_position_head(doc, doc_index, idxs, head) else {
        return None;
    };
    let Some(path) = mods::walk_position_mods(idxs, pkg, &start, rest) else {
        return None;
    };
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
            CompletionOut::plain(f.name.clone(), f.src.clone(), f.doc.clone(), CompletionKind::Fn),
        );
    }
    for l in &scope.lets {
        if !mods::vis_allows(from, &path, l.vis) {
            continue;
        }
        push_item(
            &mut out,
            CompletionOut::plain(l.name.clone(), l.src.clone(), l.doc.clone(), CompletionKind::Const),
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
                additional_text_edits: vec![(insert.0.clone(), text)],
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
            let end = (pos as usize).min(t.span.hi as usize).max(t.span.lo as usize);
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
            start: ls_types::Position { line, character: ch },
            end: ls_types::Position { line, character: ch },
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

use ls_types::{CompletionItem, CompletionItemKind, Documentation, MarkupContent, MarkupKind, TextEdit};

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

#[cfg(test)]
mod tests {
    use super::*;

    /// completions with the cursor just after `needle` in `src`
    fn complete_after(src: &str, needle: &str) -> Vec<CompletionOut> {
        let s = rut_lexer::lexer::normalize(src);
        let (toks, _) = rut_lexer::lexer::lex(&s);
        let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
        let idx = crate::hover::index(&s, &ast, &toks);
        let idxs = [&idx];
        let pos = s.rfind(needle).unwrap() as u32 + needle.len() as u32;
        complete(&idxs, &toks, &ast, pos, &s, &DocMods::default())
    }

    fn labels(items: &[CompletionOut]) -> Vec<&str> {
        items.iter().map(|c| c.label.as_str()).collect()
    }

    #[test]
    fn member_completion_lists_impl_methods_and_fields() {
        let src = "\
struct Counter {
n: i32;
}
impl Counter {
fn bump(mut self) -> i32 { return self.n; }
fn reset(mut self) -> nil { }
}
fn use_it(c: Counter) -> i32 {
return c.;
}
";
        let items = complete_after(src, "c.");
        let ls = labels(&items);
        assert!(ls.contains(&"bump"), "{ls:?}");
        assert!(ls.contains(&"reset"), "{ls:?}");
        assert!(ls.contains(&"n"), "fields complete too: {ls:?}");
    }

    #[test]
    fn member_completion_gates_foreign_interface_members() {
        // the foreign interface's members complete only once the doc
        // uses it; the document's own interface needs no gate
        let surf_src = "interface Greeter {\nfn greet(self) -> nil;\n}\n";
        let s2 = rut_lexer::lexer::normalize(surf_src);
        let (stoks, _) = rut_lexer::lexer::lex(&s2);
        let (sast, _) = rut_parser::parse(&s2, rut_parser::Mode::Impl);
        let surf = crate::hover::index(&s2, &sast, &stoks);

        let mk = |doc: &str| {
            let d2 = rut_lexer::lexer::normalize(doc);
            let (toks, _) = rut_lexer::lexer::lex(&d2);
            let (ast, _) = rut_parser::parse(&d2, rut_parser::Mode::Impl);
            let di = crate::hover::index(&d2, &ast, &toks);
            let idxs = [&di, &surf];
            let pos = d2.rfind("g.").unwrap() as u32 + 2;
            complete(&idxs, &toks, &ast, pos, &d2, &DocMods::default())
        };

        let gated = "fn go(g: Greeter) -> nil { g. }\n";
        assert!(!labels(&mk(gated)).contains(&"greet"), "unused foreign interface must not complete");

        let used = "use greets::{ Greeter };\nfn go(g: Greeter) -> nil { g. }\n";
        assert!(labels(&mk(used)).contains(&"greet"), "used interface completes");

        // the document's own interface is in scope natively
        let own = "interface Local {\nfn hi(self) -> nil;\n}\nfn go(l: Local) -> nil { l. }\n";
        let d2 = rut_lexer::lexer::normalize(own);
        let (toks, _) = rut_lexer::lexer::lex(&d2);
        let (ast, _) = rut_parser::parse(&d2, rut_parser::Mode::Impl);
        let di = crate::hover::index(&d2, &ast, &toks);
        let idxs = [&di];
        let pos = d2.rfind("l.").unwrap() as u32 + 2;
        assert!(labels(&complete(&idxs, &toks, &ast, pos, &d2, &DocMods::default())).contains(&"hi"), "own interface completes");
    }

    #[test]
    fn bare_completion_has_the_keyword_table_and_decls() {
        let src = "interface Shape {\nfn area(self) -> f64;\n}\nentry fn main() -> nil { }\n";
        let items = complete_after(src, "entry fn main() -> nil { }");
        let ls = labels(&items);
        // the final keyword table — the current spellings, no retired ones
        for kw in ["interface", "async", "use", "impl", "let", "fn"] {
            assert!(ls.contains(&kw), "keyword `{kw}` completes: {ls:?}");
        }
        // `trait` retired to an ordinary identifier — the keyword table
        // must not offer it
        assert!(!ls.contains(&"trait"), "the retired spelling must not complete: {ls:?}");
        assert!(!ls.iter().any(|l| matches!(*l, "import" | "suspend" | "from" | "dyn")));
        // decls: the document's interface (as an interface) and fn
        let shape = items.iter().find(|c| c.label == "Shape").expect("the decl completes");
        assert!(matches!(shape.kind, CompletionKind::Interface), "{:?}", shape.kind);
        assert!(ls.contains(&"main"), "{ls:?}");
        // methods are reached through a receiver, not named bare
        assert!(!ls.contains(&"area"), "{ls:?}");
    }

    #[test]
    fn bare_completion_gates_pub_builtin_core_names() {
        // the `pub builtin` rows (the disposal pair and the weak
        // reference) complete only when the doc's `use` names them —
        // the compiler's ambient split, mirrored so a completion never
        // offers a name the compile rejects. Ambient core rows
        // (`prelude builtin`) stay ungated.
        let core_src = "pub builtin class Iterable<E> {\nfn next(mut self) -> ?E;\n}\n\
                        pub builtin class Disposal {\nfn dispose(mut self, cx: DisposalContext);\n}\n\
                        pub builtin class DisposalContext { }\n\
                        pub builtin class Weak { }\n\
                        prelude builtin class StackTrace { }\n";
        let c2 = rut_lexer::lexer::normalize(core_src);
        let (ctoks, _) = rut_lexer::lexer::lex(&c2);
        let (cast, _) = rut_parser::parse(&c2, rut_parser::Mode::Decl);
        let mut core = crate::hover::index(&c2, &cast, &ctoks);
        // the manifest module name — the stamp every face lays down
        // (deps::index_dep, std_surface), and what the bare tier's
        // `(name, pkg)` imported gate matches against
        core.module = Some("core".to_string());

        let mk = |doc: &str| {
            let d2 = rut_lexer::lexer::normalize(doc);
            let (toks, _) = rut_lexer::lexer::lex(&d2);
            let (ast, _) = rut_parser::parse(&d2, rut_parser::Mode::Impl);
            let di = crate::hover::index(&d2, &ast, &toks);
            let idxs = [&di, &core];
            let pos = d2.len() as u32;
            complete(&idxs, &toks, &ast, pos, &d2, &DocMods::default())
        };

        let bare = "entry fn main() -> nil { }\n";
        let items = mk(bare);
        let ls = labels(&items);
        assert!(!ls.contains(&"Disposal"), "unused `pub builtin` must not complete: {ls:?}");
        assert!(!ls.contains(&"DisposalContext"), "unused `pub builtin` must not complete: {ls:?}");
        assert!(!ls.contains(&"Iterable"), "the gated builtin name must not complete: {ls:?}");
        assert!(!ls.contains(&"Weak"), "the gated weak reference must not complete: {ls:?}");
        assert!(ls.contains(&"StackTrace"), "the ambient row still completes: {ls:?}");

        let imported = "use core::{ Disposal, DisposalContext, Iterable, Weak };\nfn main() -> nil { }\n";
        let items = mk(imported);
        let ls = labels(&items);
        assert!(ls.contains(&"Disposal"), "the imported builtin class completes: {ls:?}");
        assert!(ls.contains(&"DisposalContext"), "the imported class completes: {ls:?}");
        assert!(ls.contains(&"Iterable"), "the imported generic builtin class completes: {ls:?}");
        assert!(ls.contains(&"Weak"), "the imported weak reference completes: {ls:?}");
    }

    // ---- the honest bare scope (A2): a foreign decl is a PLAIN bare
    // item only when the document's own `use` names it — every other
    // foreign name rides only the auto-import tier (the `use` insert
    // edit rides the item), and a transitive-only dep never offers. ----

    /// a direct dep index — the `gadgets` pkg with one pub fn and one
    /// pub struct, built the way the dep walk does (`index_dep`:
    /// module stamped; the direct-dep marker stamped — the root
    /// manifest's own dep row)
    fn tag_dep() -> DefIndex {
        let mut dep = crate::deps::index_dep(
            "gadgets",
            "file:///deps/gadgets/lib.rut",
            "pub struct Widget {\nid: i32;\n}\npub fn tag() -> i32 { return 1; }\n",
            rut_parser::Mode::Impl,
            None,
            &[],
        );
        dep.direct_dep = true;
        dep
    }

    /// completions over `doc` with `dep` in the chain, the cursor at
    /// the end of the document
    fn complete_with_dep(doc: &str, dep: &DefIndex) -> Vec<CompletionOut> {
        let d2 = rut_lexer::lexer::normalize(doc);
        let (toks, _) = rut_lexer::lexer::lex(&d2);
        let (ast, _) = rut_parser::parse(&d2, rut_parser::Mode::Impl);
        let di = crate::hover::index(&d2, &ast, &toks);
        let idxs = [&di, dep];
        complete(&idxs, &toks, &ast, d2.len() as u32, &d2, &DocMods::default())
    }

    #[test]
    fn a_foreign_dep_fn_rides_only_the_import_tier_until_used() {
        let dep = tag_dep();
        // bare doc: `tag` is NOT a plain bare item — the only offer is
        // the auto-import one (the `use gadgets::tag;` edit rides it)
        let items = complete_with_dep("entry fn main() -> nil { }\n", &dep);
        assert!(
            items.iter().all(|c| c.label != "tag" || !c.additional_text_edits.is_empty()),
            "the dep fn must not complete plain: {:?}",
            labels(&items)
        );
        let imp = import_item(&items, "tag").expect("the direct dep's fn auto-imports");
        assert_eq!(imp.detail, "gadgets::tag — import");

        // after the `use`, `tag` IS a plain bare item — and the
        // auto-import tier never re-offers it
        let items = complete_with_dep("use gadgets::{ tag };\nentry fn main() -> nil { }\n", &dep);
        let plain = items
            .iter()
            .find(|c| c.label == "tag")
            .expect("the imported fn completes bare");
        assert!(
            plain.additional_text_edits.is_empty(),
            "the imported fn completes plain, no insert: {plain:?}"
        );
        assert!(import_item(&items, "tag").is_none(), "never re-offered: {:?}", labels(&items));
    }

    #[test]
    fn a_transitive_only_dep_never_auto_imports() {
        // the compiler's closure check (`Session::resolve_use` →
        // `declares`) refuses `use` of a mounted-but-undeclared
        // package — the auto-import tier mirrors it: an index NOT
        // stamped `direct_dep` never rides the tier
        let mut dep = tag_dep();
        dep.direct_dep = false; // mounted, but the doc's manifest doesn't declare it
        let items = complete_with_dep("entry fn main() -> nil { }\n", &dep);
        assert!(
            import_item(&items, "tag").is_none(),
            "a transitive-only dep's fn must not auto-import: {:?}",
            labels(&items)
        );
        assert!(
            items.iter().all(|c| c.label != "tag"),
            "and no plain offer either: {:?}",
            labels(&items)
        );
    }

    #[test]
    fn a_name_reachable_two_ways_offers_exactly_one_item() {
        // A4: `tag` completes bare (the doc's `use` names it) AND the
        // auto-import tier would offer it — the merged list retains
        // the plain one, exactly one item by the label
        let dep = tag_dep();
        let items = complete_with_dep("use gadgets::{ tag };\nentry fn main() -> nil { }\n", &dep);
        let count = items.iter().filter(|c| c.label == "tag").count();
        assert_eq!(count, 1, "one `tag` item, got {count}: {:?}", labels(&items));
        let the = items.iter().find(|c| c.label == "tag").unwrap();
        assert!(the.additional_text_edits.is_empty(), "the plain (bare) item wins: {the:?}");
    }

    // ---- the brace-form use-path shapes (A6 regression pins — the
    // `{` trigger char's tier shapes) ----

    #[test]
    fn the_brace_form_rides_the_names_tier() {
        // `use gadgets::{⏐` — exports; the comma form too
        let dep = tag_dep();
        for doc in ["use gadgets::{", "use gadgets::{ tag, "] {
            let items = complete_with_dep(doc, &dep);
            let ls = labels(&items);
            assert!(ls.contains(&"Widget"), "exports complete in `{doc}`: {ls:?}");
            assert!(ls.contains(&"tag"), "{ls:?}");
        }
        // `use gadgets::{ Widget, ⏐` — still the full exports
        let items = complete_with_dep("use gadgets::{ Widget, ", &dep);
        assert!(labels(&items).contains(&"tag"), "{:?}", labels(&items));
        // `use gadgets::{ Widget, ta⏐` — the prefix filters
        let items = complete_with_dep("use gadgets::{ Widget, ta", &dep);
        let ls = labels(&items);
        assert!(ls.contains(&"tag"), "{ls:?}");
        assert!(!ls.contains(&"Widget"), "the prefix filters the brace form: {ls:?}");
        // an unmounted head stays an honest empty
        assert!(complete_with_dep("use ghost::{", &dep).is_empty());
    }

    #[test]
    fn string_receiver_completes_str_surface() {
        // std index ahead of the doc: `str`'s builtin contract supplies
        // the member list
        let core_src = "prelude builtin primitive str {\nfn len(self) -> i32;\n}\n";
        let c2 = rut_lexer::lexer::normalize(core_src);
        let (ctoks, _) = rut_lexer::lexer::lex(&c2);
        let (cast, _) = rut_parser::parse(&c2, rut_parser::Mode::Decl);
        let core = crate::hover::index(&c2, &cast, &ctoks);

        let doc = "fn f(s: str) -> i32 { return s. }\n";
        let d2 = rut_lexer::lexer::normalize(doc);
        let (toks, _) = rut_lexer::lexer::lex(&d2);
        let (ast, _) = rut_parser::parse(&d2, rut_parser::Mode::Impl);
        let di = crate::hover::index(&d2, &ast, &toks);
        let idxs = [&di, &core];
        let pos = d2.rfind("s.").unwrap() as u32 + 2;
        let items = complete(&idxs, &toks, &ast, pos, &d2, &DocMods::default());
        assert!(labels(&items).contains(&"len"), "{:?}", labels(&items));
    }

    // ---- the use-path tiers ----

    /// completions with the cursor just after `needle`, over the std
    /// surface (the doc itself carries nothing)
    fn complete_over_std(doc: &str, needle: &str) -> Vec<CompletionOut> {
        let s = rut_lexer::lexer::normalize(doc);
        let (toks, _) = rut_lexer::lexer::lex(&s);
        let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
        let di = crate::hover::index(&s, &ast, &toks);
        let std = crate::std_surface::indexes();
        let mut chain: Vec<&crate::hover::DefIndex> = vec![&di];
        chain.extend(std.iter());
        let pos = s.rfind(needle).unwrap() as u32 + needle.len() as u32;
        complete(&chain, &toks, &ast, pos, &s, &DocMods::default())
    }

    #[test]
    fn use_path_completion_lists_the_chain_module_names() {
        // `use ⏐` — module names from every NAMED index, the std
        // surface included (std is not special; no hardcoded table)
        let items = complete_over_std("use ", "use ");
        let ls = labels(&items);
        for want in ["pouch", "core", "nmapset", "calc", "json"] {
            assert!(ls.contains(&want), "module `{want}` completes: {ls:?}");
        }
        let pouch = items.iter().find(|c| c.label == "pouch").unwrap();
        assert!(matches!(pouch.kind, CompletionKind::Module), "{:?}", pouch.kind);
        // the partial prefix filters
        let items = complete_over_std("use po", "use po");
        let ls = labels(&items);
        assert!(ls.contains(&"pouch"), "{ls:?}");
        assert!(!ls.contains(&"core"), "{ls:?}");
        // and the keyword tier stays OUT of the use path
        assert!(!ls.contains(&"fn"), "{ls:?}");
    }

    #[test]
    fn use_path_completion_lists_the_modules_exports() {
        // `use pouch::V⏐` — the module's public names
        let items = complete_over_std("use pouch::V", "use pouch::V");
        let ls = labels(&items);
        assert!(ls.contains(&"Vec"), "{ls:?}");
        // the brace form rides the same tier
        let items = complete_over_std("use pouch::{V", "use pouch::{V");
        assert!(labels(&items).contains(&"Vec"), "{:?}", labels(&items));
        // a private-less name never offers (nmapset's internals are
        // pub-less and left the surface)
        let items = complete_over_std("use nmapset::H", "use nmapset::H");
        let ls = labels(&items);
        assert!(ls.contains(&"HashMap"), "{ls:?}");
        assert!(ls.contains(&"HashSet"), "{ls:?}");
    }

    #[test]
    fn use_path_completion_lists_the_dep_namespace_shape() {
        // a dep index (the calc shape: namespace Math + consts): the
        // names tier offers the minted head, the fns, and the consts
        let src = "pub host fn sqrt(x: f64) -> f64;\n";
        let consts = vec![("PI".to_string(), 3.141592653589793f64)];
        let calc = crate::deps::index_dep(
            "calc",
            "file:///deps/calc.d.rut",
            src,
            rut_parser::Mode::Decl,
            Some("Math"),
            &consts,
        );
        let doc = "use calc::";
        let s = rut_lexer::lexer::normalize(doc);
        let (toks, _) = rut_lexer::lexer::lex(&s);
        let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
        let di = crate::hover::index(&s, &ast, &toks);
        let idxs = [&di, &calc];
        let items = complete(&idxs, &toks, &ast, s.len() as u32, &s, &DocMods::default());
        let ls = labels(&items);
        assert!(ls.contains(&"Math"), "the namespace head completes: {ls:?}");
        assert!(ls.contains(&"sqrt"), "the module's fns complete: {ls:?}");
        assert!(ls.contains(&"PI"), "the consts complete: {ls:?}");
        let math = items.iter().find(|c| c.label == "Math").unwrap();
        assert!(matches!(math.kind, CompletionKind::Module), "{:?}", math.kind);
        let pi = items.iter().find(|c| c.label == "PI").unwrap();
        assert!(matches!(pi.kind, CompletionKind::Const), "{:?}", pi.kind);
    }

    // ---- the file-module walk (use-path + position tiers) ----

    /// a mod-carrying dep, parsed and STAMPED the way the faces do:
    /// `gadgets` — root (`""`) declaring `pub mod layout;` +
    /// `mod secret;`; `layout` a `pub mod grid;` child with a pub
    /// struct, a private fn, and a `pub(super)` fn; `layout/grid` and
    /// `secret` leaves
    fn gadget_dep() -> Vec<DefIndex> {
        let mk = |mod_path: &str, src: &str| {
            let s = rut_lexer::lexer::normalize(src);
            let (toks, _) = rut_lexer::lexer::lex(&s);
            let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
            let mut idx = crate::hover::index(&s, &ast, &toks);
            idx.module = Some("gadgets".to_string());
            idx.mod_path = Some(mod_path.to_string());
            idx
        };
        vec![
            mk("", "pub mod layout;\nmod secret;\npub fn tag() -> i32 { return 1; }\n"),
            mk(
                "layout",
                "pub mod grid;\npub struct Column {\n    w: i32;\n}\nfn internal() -> i32 { return 2; }\npub(super) fn for_parent() -> i32 { return 3; }\n",
            ),
            mk("layout/grid", "pub struct Cell {\n    x: i32;\n}\npub fn mk() -> Cell { return Cell { x: 1 }; }\n"),
            mk("secret", "pub fn open() -> i32 { return 4; }\nfn hidden() -> i32 { return 5; }\n"),
        ]
    }

    /// completions over the gadget dep with the cursor after `needle`
    fn complete_over_gadgets(doc: &str, needle: &str, doc_mods: DocMods) -> Vec<CompletionOut> {
        let s = rut_lexer::lexer::normalize(doc);
        let (toks, _) = rut_lexer::lexer::lex(&s);
        let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
        let di = crate::hover::index(&s, &ast, &toks);
        let dep = gadget_dep();
        let mut chain: Vec<&DefIndex> = vec![&di];
        chain.extend(dep.iter());
        let pos = s
            .rfind(needle)
            .unwrap_or_else(|| panic!("needle {needle:?} not found in:\n{s}")) as u32
            + needle.len() as u32;
        complete(&chain, &toks, &ast, pos, &s, &doc_mods)
    }

    #[test]
    fn use_path_completion_walks_the_dep_mod_tree() {
        // `use gadgets::⏐` — the root's exports PLUS its pub mod
        // children; the private `secret` edge never offers, and the
        // child's members live under their path (no Column leak)
        let items = complete_over_gadgets("use gadgets::", "use gadgets::", DocMods::default());
        let ls = labels(&items);
        assert!(ls.contains(&"layout"), "the pub mod child completes: {ls:?}");
        assert!(ls.contains(&"tag"), "the root's fn completes: {ls:?}");
        assert!(!ls.contains(&"secret"), "a bare `mod` edge never crosses: {ls:?}");
        assert!(!ls.contains(&"Column"), "child members stay under their path: {ls:?}");
        let layout = ls.iter().find(|l| **l == "layout").unwrap();
        let items = complete_over_gadgets("use gadgets::", "use gadgets::", DocMods::default());
        let item = items.iter().find(|c| c.label == "layout").unwrap();
        assert!(matches!(item.kind, CompletionKind::Module), "{:?}", item.kind);
        assert!(item.detail.contains("layout/mod.rut"), "{:?}", item.detail);
        let _ = layout;

        // the prefix filters the tier
        let items = complete_over_gadgets("use gadgets::la", "use gadgets::la", DocMods::default());
        let ls = labels(&items);
        assert!(ls.contains(&"layout"), "{ls:?}");
        assert!(!ls.contains(&"tag"), "{ls:?}");

        // `use gadgets::layout::⏐` — the walked module's exports, the
        // pub gate on fns now binding (crossed), plus its pub mod
        // children
        let items = complete_over_gadgets(
            "use gadgets::layout::",
            "use gadgets::layout::",
            DocMods::default(),
        );
        let ls = labels(&items);
        assert!(ls.contains(&"Column"), "{ls:?}");
        assert!(ls.contains(&"grid"), "the nested pub mod completes: {ls:?}");
        assert!(!ls.contains(&"for_parent"), "only `pub` leaf names cross packages: {ls:?}");
        assert!(!ls.contains(&"internal"), "a private fn never crosses: {ls:?}");

        // deeper: `use gadgets::layout::grid::⏐`
        let items = complete_over_gadgets(
            "use gadgets::layout::grid::",
            "use gadgets::layout::grid::",
            DocMods::default(),
        );
        let ls = labels(&items);
        assert!(ls.contains(&"Cell"), "{ls:?}");
        assert!(ls.contains(&"mk"), "{ls:?}");

        // the brace form rides the same walk
        let items = complete_over_gadgets(
            "use gadgets::layout::{C",
            "use gadgets::layout::{C",
            DocMods::default(),
        );
        let ls = labels(&items);
        assert!(ls.contains(&"Column"), "{ls:?}");

        // a private edge is a dead end; a ghost is a miss — both empty
        assert!(complete_over_gadgets("use gadgets::secret::", "use gadgets::secret::", DocMods::default()).is_empty());
        assert!(complete_over_gadgets("use gadgets::ghost::", "use gadgets::ghost::", DocMods::default()).is_empty());
    }

    #[test]
    fn position_path_completion_offers_the_module_members_by_tier() {
        // the open document IS the gadgets root: `mod layout;` +
        // `mod secret;` declared in it; the doc mods say so. Positions
        // qualify with `.` (`let c: layout.Column`) — the real spelling
        let doc = DocMods { mod_path: Some(String::new()), pkg: Some("gadgets".to_string()) };
        let doc_src = "mod layout;\nmod secret;\nentry fn main() -> nil {\n    let c: layout.\n}\n";
        let items = complete_over_gadgets(doc_src, "let c: layout.", doc.clone());
        let ls = labels(&items);
        assert!(ls.contains(&"Column"), "the pub type completes: {ls:?}");
        assert!(ls.contains(&"grid"), "any declared edge walks intra-package: {ls:?}");
        assert!(ls.contains(&"for_parent"), "pub(super) reaches the parent (the root): {ls:?}");
        assert!(!ls.contains(&"internal"), "a private fn of a child stays hidden: {ls:?}");

        // the `::` spelling rides the same tier (the plan's letter)
        let doc_src = "mod layout;\nmod secret;\nentry fn main() -> nil {\n    let c: layout::\n}\n";
        let items = complete_over_gadgets(doc_src, "let c: layout::", doc.clone());
        assert!(labels(&items).contains(&"Column"), "{:?}", labels(&items));

        // a child module's own file: `secret`'s members from the root —
        // pub yes, private no
        let doc_src = "mod layout;\nmod secret;\nentry fn main() -> nil {\n    let f: secret.\n}\n";
        let items = complete_over_gadgets(doc_src, "let f: secret.", doc.clone());
        let ls = labels(&items);
        assert!(ls.contains(&"open"), "{ls:?}");
        assert!(!ls.contains(&"hidden"), "a private fn of a sibling is not visible: {ls:?}");

        // the own-pkg head names the root module: `gadgets.` from the
        // root offers the root's own surface + children
        let doc_src = "mod layout;\nmod secret;\npub fn root_fn() -> i32 { return 9; }\nentry fn main() -> nil {\n    let t: gadgets.\n}\n";
        let items = complete_over_gadgets(doc_src, "let t: gadgets.", doc.clone());
        let ls = labels(&items);
        assert!(ls.contains(&"root_fn"), "{ls:?}");
        assert!(ls.contains(&"layout"), "{ls:?}");

        // a deeper doc (layout/grid): a sibling via the parent scope,
        // and the pkg head reaching the root's decls
        let grid_doc = DocMods { mod_path: Some("layout/grid".into()), pkg: Some("gadgets".to_string()) };
        let grid_src = "pub struct Cell {\n    x: i32;\n}\nentry fn main() -> nil {\n    let c: layout.Column\n}\n";
        let items = complete_over_gadgets(grid_src, "let c: layout.", grid_doc.clone());
        let ls = labels(&items);
        assert!(ls.contains(&"Column"), "the sibling-of-parent resolves: {ls:?}");
        // grid is a descendant of layout: layout's PRIVATES are visible
        // (the declaring module + descendants law)
        let grid_src2 = "pub struct Cell {\n    x: i32;\n}\nentry fn main() -> nil {\n    let f: layout.internal\n}\n";
        let items = complete_over_gadgets(grid_src2, "let f: layout.", grid_doc);
        let ls = labels(&items);
        assert!(ls.contains(&"internal"), "a descendant reads the parent's privates: {ls:?}");

        // no doc mods (unknown provenance): the position tier stays off
        // — a plain type receiver falls through to the member tier
        // (empty for an unknown receiver — never wrong names)
        let flat_src = "mod layout;\nmod secret;\nentry fn main() -> nil {\n    let c: layout.\n}\n";
        let items = complete_over_gadgets(flat_src, "let c: layout.", DocMods::default());
        let ls = labels(&items);
        assert!(!ls.contains(&"Column"), "no doc mods, no mod tier: {ls:?}");
    }

    #[test]
    fn the_use_path_tier_answers_an_own_package_head() {
        // the open document IS the gadgets root: `use gadgets::layout::⏐`
        // rides the own lane — the position machinery's visible-members
        // gate, so `pub(super)` reaches the root where the cross-package
        // tier's pub-only crossing law stays shut
        let doc = DocMods { mod_path: Some(String::new()), pkg: Some("gadgets".to_string()) };
        let items = complete_over_gadgets("use gadgets::layout::", "use gadgets::layout::", doc.clone());
        let ls = labels(&items);
        assert!(ls.contains(&"Column"), "{ls:?}");
        assert!(
            ls.contains(&"for_parent"),
            "the own lane gates by visibility, not the pub-crossing law: {ls:?}"
        );
        assert!(!ls.contains(&"internal"), "a private fn stays hidden: {ls:?}");

        // the bare own-pkg head: the doc's own rows are the scope (the
        // freshness law) — the declared children offer
        let doc_src = "mod layout;\nuse gadgets::";
        let items = complete_over_gadgets(doc_src, "use gadgets::", doc);
        let ls = labels(&items);
        assert!(ls.contains(&"layout"), "the own root's edges offer: {ls:?}");

        // an own mod edge head — `use layout::⏐` from the root walks
        // the ancestor spelling to the same tier
        let doc_src = "mod layout;\nuse layout::";
        let items = complete_over_gadgets(
            doc_src,
            "use layout::",
            DocMods { mod_path: Some(String::new()), pkg: Some("gadgets".to_string()) },
        );
        let ls = labels(&items);
        assert!(ls.contains(&"Column"), "the own mod edge head rides the same tier: {ls:?}");
        assert!(ls.contains(&"grid"), "{ls:?}");
    }

    // ---- the type-head tier (after `Type::`) ----

    #[test]
    fn colon2_completion_lists_the_enum_members() {
        // `Color::⏐` — the enum head's member list rides name
        // resolution, not a receiver dot
        let src = "enum Color { Red, Green }\nentry fn main() -> nil {\n    let c = Color::\n}\n";
        let items = complete_after(src, "Color::");
        let ls = labels(&items);
        assert!(ls.contains(&"Red"), "{ls:?}");
        assert!(ls.contains(&"Green"), "{ls:?}");
        let red = items.iter().find(|c| c.label == "Red").unwrap();
        assert!(matches!(red.kind, CompletionKind::EnumMember), "{:?}", red.kind);
        // the typed prefix filters the member list
        let src = "enum Color { Red, Green }\nentry fn main() -> nil {\n    let c = Color::R\n}\n";
        let items = complete_after(src, "Color::R");
        let ls = labels(&items);
        assert!(ls.contains(&"Red"), "{ls:?}");
        assert!(!ls.contains(&"Green"), "{ls:?}");
    }

    #[test]
    fn colon2_completion_lists_the_namespace_statics() {
        // `Math::⏐` over the calc dep — the namespace head's consts
        // and fns (the mint the manifest's namespace/consts ride)
        let src = "pub host fn sqrt(x: f64) -> f64;\n";
        let consts = vec![("PI".to_string(), 3.141592653589793f64)];
        let calc = crate::deps::index_dep(
            "calc",
            "file:///deps/calc.d.rut",
            src,
            rut_parser::Mode::Decl,
            Some("Math"),
            &consts,
        );
        let doc = "entry fn main() -> nil {\n    let x = Math::\n}\n";
        let s = rut_lexer::lexer::normalize(doc);
        let (toks, _) = rut_lexer::lexer::lex(&s);
        let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
        let di = crate::hover::index(&s, &ast, &toks);
        let idxs = [&di, &calc];
        let pos = s.rfind("Math::").unwrap() as u32 + "Math::".len() as u32;
        let items = complete(&idxs, &toks, &ast, pos, &s, &DocMods::default());
        let ls = labels(&items);
        assert!(ls.contains(&"sqrt"), "the namespace's fns complete: {ls:?}");
        assert!(ls.contains(&"PI"), "the consts complete: {ls:?}");
        let pi = items.iter().find(|c| c.label == "PI").unwrap();
        assert!(matches!(pi.kind, CompletionKind::Const), "{:?}", pi.kind);
        let sqrt = items.iter().find(|c| c.label == "sqrt").unwrap();
        assert!(matches!(sqrt.kind, CompletionKind::Fn), "{:?}", sqrt.kind);
    }

    #[test]
    fn colon2_completion_lists_the_type_head_associated_fns() {
        // `Counter::⏐` — the statics + the inherent impl block's fns
        // (the owner string IS the type's name); instance fields stay
        // OFF — they ride a receiver dot, never `::`
        let src = "\
class Counter {
n: i32;
pub static COUNT: i32;
}
impl Counter {
fn bump(mut self) -> i32 { return self.n; }
fn mk() -> Counter { return Counter { n: 0 }; }
}
entry fn main() -> nil {
    let c = Counter::
}
";
        let items = complete_after(src, "Counter::");
        let ls = labels(&items);
        assert!(ls.contains(&"bump"), "{ls:?}");
        assert!(ls.contains(&"mk"), "{ls:?}");
        assert!(ls.contains(&"COUNT"), "the static rides the head: {ls:?}");
        let count = items.iter().find(|c| c.label == "COUNT").unwrap();
        assert!(matches!(count.kind, CompletionKind::Field), "{:?}", count.kind);
        assert!(!ls.contains(&"n"), "instance fields ride a receiver: {ls:?}");
    }

    #[test]
    fn colon2_interface_head_and_deeper_walks_offer_nothing() {
        // an interface head: its signatures ride receivers, never `::`
        // — the known head is FINAL (empty), never the bare tier
        let src = "interface Greeter {\nfn greet(self) -> nil;\n}\nentry fn main() -> nil {\n    let g = Greeter::\n}\n";
        assert!(complete_after(src, "Greeter::").is_empty(), "{:?}", complete_after(src, "Greeter::"));
        // a known head with a non-matching prefix: filtered empty, final
        let src = "enum Color { Red, Green }\nentry fn main() -> nil {\n    let c = Color::X\n}\n";
        assert!(complete_after(src, "Color::X").is_empty());
        // a type head is not a namespace: nothing descends past a member
        let src = "enum Color { Red, Green }\nentry fn main() -> nil {\n    let c = Color::Red::\n}\n";
        assert!(complete_after(src, "Color::Red::").is_empty());
    }

    // ---- the type tier at annotation boundaries (A3) ----

    #[test]
    fn the_type_tier_answers_a_struct_field_annotation() {
        // `id: u⏐` — the primitive table rides the prefix filter; the
        // keyword table, the doc's fns, and the fn-locals stay OFF
        // (values never ride a type position)
        let src = "\
struct Slot {
id: u
}
fn go(ready: bool) -> nil {
    let count = 1;
    count
}
";
        let items = complete_after(src, "id: u");
        let ls = labels(&items);
        for want in ["u8", "u16", "u32", "u64"] {
            assert!(ls.contains(&want), "primitive `{want}` completes at the annotation: {ls:?}");
        }
        assert!(!ls.contains(&"i32"), "the prefix filters the tier: {ls:?}");
        assert!(!ls.contains(&"use"), "no keyword tier at a type position: {ls:?}");
        assert!(!ls.contains(&"fn") && !ls.contains(&"let"), "{ls:?}");
        assert!(!ls.contains(&"go"), "no fns at a type position: {ls:?}");
        assert!(!ls.contains(&"ready") && !ls.contains(&"count"), "no locals at a type position: {ls:?}");
        let u32 = items.iter().find(|c| c.label == "u32").unwrap();
        assert!(matches!(u32.kind, CompletionKind::Type), "{:?}", u32.kind);
    }

    #[test]
    fn the_type_tier_answers_the_bare_colon_boundary() {
        // `let c: ⏐` — the lone colon (the upgrade of the old honest
        // empty): the full tier, the doc's own types included
        let src = "enum Color { Red }\nentry fn main() -> nil {\n    let c: \n}\n";
        let items = complete_after(src, "let c:");
        let ls = labels(&items);
        for want in ["bool", "str", "f64", "i32", "nil"] {
            assert!(ls.contains(&want), "primitive `{want}` completes: {ls:?}");
        }
        let color = items.iter().find(|c| c.label == "Color").expect("the doc's type completes");
        assert!(matches!(color.kind, CompletionKind::Type), "{:?}", color.kind);
        assert!(!ls.contains(&"fn") && !ls.contains(&"let"), "no keywords: {ls:?}");
        assert!(!ls.contains(&"main"), "no fns: {ls:?}");
    }

    #[test]
    fn the_type_tier_answers_the_arrow_boundary() {
        // `fn f() -> ⏐` — the return annotation rides the same tier …
        let src = "struct Thing {\nx: i32;\n}\nfn f() -> \n";
        let items = complete_after(src, "-> ");
        let ls = labels(&items);
        assert!(ls.contains(&"i32") && ls.contains(&"Thing"), "{ls:?}");
        assert!(!ls.contains(&"fn") && !ls.contains(&"f"), "no keywords, no fns: {ls:?}");
        // … with the partial ident prefix-filtering: `-> T⏐`
        let src = "struct Thing {\nx: i32;\n}\nfn f() -> T\n";
        let items = complete_after(src, "-> T");
        let ls = labels(&items);
        assert!(ls.contains(&"Thing"), "{ls:?}");
        assert!(!ls.contains(&"i32"), "the prefix filters the primitives: {ls:?}");
    }

    #[test]
    fn the_type_tier_answers_a_param_annotation() {
        // `fn f(x: u⏐` — parameter annotations are type positions too
        let src = "fn f(x: u) -> nil { }\n";
        let items = complete_after(src, "x: u");
        let ls = labels(&items);
        assert!(ls.contains(&"u32") && ls.contains(&"u64"), "{ls:?}");
        assert!(!ls.contains(&"fn"), "{ls:?}");
    }

    #[test]
    fn the_type_tier_enumerates_exactly_the_primitive_table() {
        // the tier's primitive rows ARE the parser's one PRIMITIVE_TYS
        // table — never a duplicated, drifting list
        let src = "entry fn main() -> nil {\n    let c: \n}\n";
        let items = complete_after(src, "let c:");
        let mut got: Vec<&str> = labels(&items)
            .into_iter()
            .filter(|l| rut_parser::is_primitive_ty(l))
            .collect();
        got.sort_unstable();
        let mut want = rut_parser::PRIMITIVE_TYS.to_vec();
        want.sort_unstable();
        assert_eq!(got, want, "the tier's primitives are exactly PRIMITIVE_TYS");
    }

    #[test]
    fn the_type_tier_offers_the_ambient_and_imported_foreign_types() {
        // a foreign doc: its own types plus the ambient builtin row
        // complete; the `pub builtin` gated row completes only once the
        // document's `use` names it
        let core_src = "pub builtin class Weak { }\nprelude builtin class StackTrace { }\n";
        let c2 = rut_lexer::lexer::normalize(core_src);
        let (ctoks, _) = rut_lexer::lexer::lex(&c2);
        let (cast, _) = rut_parser::parse(&c2, rut_parser::Mode::Decl);
        let mut core = crate::hover::index(&c2, &cast, &ctoks);
        core.module = Some("core".to_string());

        let mk = |doc: &str| {
            let d2 = rut_lexer::lexer::normalize(doc);
            let (toks, _) = rut_lexer::lexer::lex(&d2);
            let (ast, _) = rut_parser::parse(&d2, rut_parser::Mode::Impl);
            let di = crate::hover::index(&d2, &ast, &toks);
            let idxs = [&di, &core];
            let pos = d2.rfind("let e:").unwrap() as u32 + "let e:".len() as u32;
            complete(&idxs, &toks, &ast, pos, &d2, &DocMods::default())
        };

        let items = mk("entry fn main() -> nil {\n    let e: \n}\n");
        let ls = labels(&items);
        assert!(ls.contains(&"StackTrace"), "the ambient builtin type completes: {ls:?}");
        assert!(!ls.contains(&"Weak"), "the gated builtin type stays off: {ls:?}");

        let items = mk("use core::{ Weak };\nentry fn main() -> nil {\n    let e: \n}\n");
        let ls = labels(&items);
        assert!(ls.contains(&"Weak"), "the imported builtin type completes: {ls:?}");
    }

    // ---- the auto-import tier ----

    /// the auto-import item for `label`, if the tier offered one (an
    /// item with additional text edits)
    fn import_item<'a>(items: &'a [CompletionOut], label: &str) -> Option<&'a CompletionOut> {
        items
            .iter()
            .find(|c| c.label == label && !c.additional_text_edits.is_empty())
    }

    #[test]
    fn auto_import_offers_foreign_names_with_the_insert_edit() {
        let doc = "entry fn main() -> nil {\n    let v = Ve\n}\n";
        let s = rut_lexer::lexer::normalize(doc);
        let (toks, _) = rut_lexer::lexer::lex(&s);
        let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
        let di = crate::hover::index(&s, &ast, &toks);
        let std = crate::std_surface::indexes();
        let mut chain: Vec<&crate::hover::DefIndex> = vec![&di];
        chain.extend(std.iter());
        let pos = s.rfind("Ve").unwrap() as u32 + 2;
        let items = complete(&chain, &toks, &ast, pos, &s, &DocMods::default());
        let imp = import_item(&items, "Vec").expect("Vec auto-imports from pouch");
        assert_eq!(imp.detail, "pouch::Vec — import");
        assert_eq!(imp.sort_text.as_deref(), Some("~Vec"), "the tier sorts after the locals");
        // the insert: no uses, no leading comments → top of the body,
        // a fresh line, over the SAME normalized coordinates
        assert_eq!(imp.additional_text_edits.len(), 1);
        let (range, text) = &imp.additional_text_edits[0];
        assert_eq!((range.start.line, range.start.character), (0, 0));
        assert_eq!(range.end, range.start, "an insertion");
        assert_eq!(text, "use pouch::Vec;\n");
    }

    #[test]
    fn auto_import_lands_after_the_last_leading_use() {
        let doc = "use core::{ NAN };\n\nentry fn main() -> nil {\n    let v = Ve\n}\n";
        let s = rut_lexer::lexer::normalize(doc);
        let (toks, _) = rut_lexer::lexer::lex(&s);
        let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
        let di = crate::hover::index(&s, &ast, &toks);
        let std = crate::std_surface::indexes();
        let mut chain: Vec<&crate::hover::DefIndex> = vec![&di];
        chain.extend(std.iter());
        let pos = s.rfind("Ve").unwrap() as u32 + 2;
        let items = complete(&chain, &toks, &ast, pos, &s, &DocMods::default());
        let imp = import_item(&items, "Vec").expect("Vec auto-imports");
        let (range, text) = &imp.additional_text_edits[0];
        // right after the leading use's `;` — column 18 of line 0 —
        // with the fresh use on the next line
        assert_eq!((range.start.line, range.start.character), (0, 18));
        assert_eq!(text, "\nuse pouch::Vec;");
    }

    #[test]
    fn auto_import_skips_leading_comments() {
        let doc = "// the header prose\n\nentry fn main() -> nil {\n    let v = Ve\n}\n";
        let s = rut_lexer::lexer::normalize(doc);
        let (toks, _) = rut_lexer::lexer::lex(&s);
        let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
        let di = crate::hover::index(&s, &ast, &toks);
        let std = crate::std_surface::indexes();
        let mut chain: Vec<&crate::hover::DefIndex> = vec![&di];
        chain.extend(std.iter());
        let pos = s.rfind("Ve").unwrap() as u32 + 2;
        let items = complete(&chain, &toks, &ast, pos, &s, &DocMods::default());
        let imp = import_item(&items, "Vec").expect("Vec auto-imports");
        let (range, text) = &imp.additional_text_edits[0];
        // top of the file BODY — after the comment block and the blank
        assert_eq!((range.start.line, range.start.character), (2, 0));
        assert_eq!(text, "use pouch::Vec;\n");
    }

    #[test]
    fn already_imported_names_are_not_re_offered() {
        let doc = "use pouch::{ Vec };\nentry fn main() -> nil {\n    let v = Ve\n}\n";
        let s = rut_lexer::lexer::normalize(doc);
        let (toks, _) = rut_lexer::lexer::lex(&s);
        let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
        let di = crate::hover::index(&s, &ast, &toks);
        let std = crate::std_surface::indexes();
        let mut chain: Vec<&crate::hover::DefIndex> = vec![&di];
        chain.extend(std.iter());
        let pos = s.rfind("Ve").unwrap() as u32 + 2;
        let items = complete(&chain, &toks, &ast, pos, &s, &DocMods::default());
        // the name still completes (the bare tier), but NO import edit
        // rides it — the document already imports it
        assert!(
            import_item(&items, "Vec").is_none(),
            "the already-imported name must not re-offer: {items:?}"
        );
    }

    // ---- constructor-call inference + the dot guard ----

    #[test]
    fn constructor_call_binding_completes_the_ink_surface() {
        // `let log = Logger("h")` — the constructor-call form types the
        // binding from the indexed class (the embedded ink surface), so
        // `log.` lists the impl methods (new/debug/info/warn/error/log)
        let doc = "fn go() -> nil {\n    let log = Logger(\"h\");\n    log.\n}\n";
        let items = complete_over_std(doc, "log.");
        let ls = labels(&items);
        for want in ["new", "debug", "info", "warn", "error"] {
            assert!(ls.contains(&want), "`{want}` completes through the ctor binding: {ls:?}");
        }
        // the bare tier's host fns never dump into a member position
        assert!(!ls.contains(&"abs"), "no host-fn dump after a dot: {ls:?}");
    }

    #[test]
    fn dot_guard_unknown_receiver_is_empty() {
        // `someUnknown.` — the receiver resolves to nothing: an EMPTY
        // list, never the keyword/host-fn dump (a miss is an empty list)
        let doc = "fn go() -> nil {\n    someUnknown.\n}\n";
        let items = complete_over_std(doc, "someUnknown.");
        assert!(
            items.is_empty(),
            "an unresolved receiver completes nothing: {:?}",
            labels(&items)
        );
    }

    #[test]
    fn dot_guard_non_name_receiver_is_empty() {
        // `probe().` — the token before the dot is not a name the tiers
        // understand; the text-backed dot guard keeps the bare tier out
        let doc = "fn go() -> nil {\n    let x = probe().\n}\n";
        let items = complete_after(doc, "probe().");
        assert!(items.is_empty(), "{:?}", labels(&items));
    }

    // ---- the locals tier (plan3 phase 1, A1) ----

    #[test]
    fn locals_complete_at_a_bare_position_ahead_of_the_keywords() {
        // `che⏐` mid-block: the fn-local `checked` and the param `ready`
        // ride their own kinds, ahead of the keyword table
        let src = "\
fn go(ready: bool) -> nil {
    let mut checked: str = \" \";
    che
}
";
        let items = complete_after(src, "    che\n");
        let ls = labels(&items);
        assert!(ls.contains(&"checked"), "the local completes: {ls:?}");
        assert!(ls.contains(&"ready"), "the param completes: {ls:?}");
        let checked = items.iter().find(|c| c.label == "checked").unwrap();
        assert!(matches!(checked.kind, CompletionKind::Variable), "{:?}", checked.kind);
        assert!(checked.detail.contains("let binding") && checked.detail.contains("str"), "{}", checked.detail);
        let ready = items.iter().find(|c| c.label == "ready").unwrap();
        assert!(matches!(ready.kind, CompletionKind::Parameter), "{:?}", ready.kind);
        // ahead of the keyword table
        let checked_at = items.iter().position(|c| c.label == "checked").unwrap();
        let let_kw_at = items.iter().position(|c| c.label == "let").unwrap();
        assert!(checked_at < let_kw_at, "locals ride ahead of the keywords");
    }

    #[test]
    fn locals_offer_loop_variables_where_their_scope_reaches() {
        // the for-of var inside its body; the for-c counter inside the
        // cond/update too (its scope is the whole loop statement)
        let src = "\
fn go(xs: [i32]) -> nil {
    for (let v of xs) {
        vv
    }
}
";
        let items = complete_after(src, "        vv");
        let v = items.iter().find(|c| c.label == "v").expect("the for-of var completes");
        assert!(matches!(v.kind, CompletionKind::Variable), "{:?}", v.kind);
        assert!(v.detail.contains("loop variable"), "{}", v.detail);

        let src = "\
fn go(n: i32) -> nil {
    let sum = 0;
    for (let i = 0; i < n; i += su) {
        sum += i;
    }
}
";
        // `su` inside the UPDATE — the counter `i` and the outer `sum`
        // both still in scope
        let items = complete_after(src, "i += su");
        let ls = labels(&items);
        assert!(ls.contains(&"i"), "the counter completes in the update: {ls:?}");
        assert!(ls.contains(&"sum"), "{ls:?}");
        assert!(ls.contains(&"n"), "the param reaches the loop: {ls:?}");
        let i = items.iter().find(|c| c.label == "i").unwrap();
        assert!(i.detail.contains("i32"), "{}", i.detail);
    }

    #[test]
    fn locals_shadow_inside_and_exit_their_scope() {
        // inside the if-block the INNER `flag` wins (the i32 detail —
        // one entry per name, the innermost in-scope declaration rides)
        let src = "\
fn go(ready: bool) -> nil {
    let flag: str = \"a\";
    if (ready) {
        let flag: i32 = 1;
        fl
    }
    fl
}
";
        let items = complete_after(src, "        fl");
        let flag = items.iter().find(|c| c.label == "flag").expect("the shadow completes");
        assert!(flag.detail.contains("i32"), "the inner shadow wins inside its scope: {}", flag.detail);
        // scope-exit: a binding declared INSIDE the block is gone past
        // it, while the fn-wide param rides on
        let src = "\
fn go(ready: bool) -> nil {
    if (ready) {
        let tmp: i32 = 1;
        tm
    }
    tm
}
";
        let items = complete_after(src, "        tm");
        assert!(labels(&items).contains(&"tmp"), "inside its block the local completes");
        let items = complete_after(src, "    tm\n");
        let ls = labels(&items);
        assert!(!ls.contains(&"tmp"), "the binding does not leak past its block: {ls:?}");
        assert!(ls.contains(&"ready"), "the fn-wide param rides past the block: {ls:?}");
    }

    #[test]
    fn locals_skip_their_own_declaration_ident() {
        // the cursor inside the declaring ident — `let ch⏐e` — never
        // offers the name being typed; a use after the decl does
        let src = "fn go() -> nil {\n    let che = 1;\n}\n";
        let items = complete_after(src, "let ch");
        assert!(!labels(&items).contains(&"che"), "the declaration's own ident stays off");
        assert!(!labels(&items).contains(&"ch"), "the half-typed name stays off: {:?}", labels(&items));
        let src = "fn go() -> nil {\n    let che = 1;\n    che\n}\n";
        let items = complete_after(src, "    che\n");
        assert!(labels(&items).contains(&"che"), "{:?}", labels(&items));
    }

    // ---- the end-of-buffer scope fix (plan3 phase 1, A7) — the doc
    // ends at/inside the incomplete statement, the parser ends the
    // block's span at the EOF token, the cursor sits ON `hi`; the fn
    // body's scope extends through the source end so resolution keeps
    // working at the most common typing moment. No scope leaks OUT of
    // real (closed) blocks. ----

    /// the List probe doc's head — a type with a field and an impl
    /// method for the member tier to answer with
    const EOF_LIST_HEAD: &str = "\
class List {
    items: i32;
}
impl List {
    fn count(self) -> i32 { return 0; }
}
";

    #[test]
    fn member_completion_survives_the_end_of_buffer() {
        // THE regression: the text ends EXACTLY at the incomplete stmt
        let src = format!("{EOF_LIST_HEAD}fn go() -> nil {{\n    let list = List.new();\n    list.");
        let items = complete_after(&src, "    list.");
        let ls = labels(&items);
        assert!(ls.contains(&"items"), "the field completes at EOF: {ls:?}");
        assert!(ls.contains(&"count"), "the method completes at EOF: {ls:?}");
        // never the bare keyword dump
        assert!(!ls.contains(&"fn"), "no keyword dump after a dot: {ls:?}");

        // one trailing newline — the same hole (the block span still
        // ends at the EOF token)
        let src = format!("{EOF_LIST_HEAD}fn go() -> nil {{\n    let list = List.new();\n    list.\n");
        let items = complete_after(&src, "    list.");
        let ls = labels(&items);
        assert!(ls.contains(&"items") && ls.contains(&"count"), "the member list survives the newline: {ls:?}");

        // last stmt before `}` — the closed-block shape guards today's
        // behavior behind the fix
        let src = format!("{EOF_LIST_HEAD}fn go() -> nil {{\n    let list = List.new();\n    list.\n}}\n");
        let items = complete_after(&src, "    list.");
        let ls = labels(&items);
        assert!(ls.contains(&"items") && ls.contains(&"count"), "{ls:?}");
    }

    #[test]
    fn locals_complete_at_the_end_of_buffer() {
        // A1 at EOF (post-A7): the doc ends mid-statement
        let src = "fn go() -> nil {\n    let mut checked: str = \" \";\n    che";
        let items = complete_after(src, "    che");
        let ls = labels(&items);
        assert!(ls.contains(&"checked"), "the local completes at EOF: {ls:?}");

        // the receiver itself still resolves for member completion when
        // the receiver is the LAST thing before the cursor
        let src = format!("{EOF_LIST_HEAD}fn go() -> nil {{\n    let list = List.new();\n    li");
        let items = complete_after(&src, "    li");
        assert!(labels(&items).contains(&"list"), "the local completes at EOF: {:?}", labels(&items));
    }

    #[test]
    fn the_end_of_buffer_extension_never_leaks_a_closed_block() {
        // a REAL `}` ends the scope exactly as before: the cursor past
        // it (at the source end) sees no fn-local — no leak out of the
        // closed block, with or without a trailing newline
        for (src, needle) in [
            ("fn go() -> nil {\n    let checked: str = \" \";\n}\n", "}\n"),
            ("fn go() -> nil {\n    let checked: str = \" \";\n}", "}"),
        ] {
            let items = complete_after(src, needle);
            let ls = labels(&items);
            assert!(
                !ls.contains(&"checked"),
                "no leak past a closed block ({src:?}): {ls:?}"
            );
        }
        // and a closed first fn never leaks into an unterminated second
        let src = "fn a() -> nil {\n    let alpha: i32 = 1;\n}\nfn b() -> nil {\n    let beta: i32 = 2;\n    be";
        let items = complete_after(src, "    be");
        let ls = labels(&items);
        assert!(ls.contains(&"beta"), "the unterminated fn's local completes: {ls:?}");
        assert!(!ls.contains(&"alpha"), "the closed fn's local stays in: {ls:?}");
    }
}
