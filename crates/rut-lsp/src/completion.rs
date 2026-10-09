//! Completions — member completion after `recv.`, bare-position
//! completion, the use-path tiers, and the auto-import tier, over the
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
//! names. The auto-import tier: bare position, after the local results
//! (unchanged order), foreign public names matching the prefix ride
//! with an additional text edit inserting `use mod::Name;` after the
//! last leading `use` (none → top of file body, after leading
//! comments) — a fresh line, no merge (`rut fmt` owns tidying). Names
//! already imported are never re-offered.

use std::collections::HashSet;

use ls_types::Range;
use rut_ast::ast::{Ast, ItemKind};
use rut_lexer::token::{Tok, Token};

use crate::hover::lookup::used_ifaces;
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
}

/// Completions at `pos`: use-path tiers inside a `use` statement
/// (walking a package's mod namespaces), qualified-position tiers
/// after `modname::` in the package's own files, member items after
/// `recv.` (filtered by the typed prefix), otherwise the keyword table
/// plus the visible decls plus the auto-import tier. `src` is the
/// NORMALIZED source — the auto-import edits' coordinates ride the
/// same text the tokens were lexed from. `doc` is the open document's
/// file-module context (the face's knowledge; the default keeps
/// position-path completion off).
pub fn complete(
    idxs: &[&DefIndex],
    toks: &[Token],
    ast: &Ast,
    pos: u32,
    src: &str,
    doc: &DocMods,
) -> Vec<CompletionOut> {
    if let Some((segs, prefix)) = use_path_before(toks, pos) {
        return use_path_completions(idxs, &segs, &prefix);
    }
    if let Some((segs, prefix)) = qualified_path_before(toks, pos) {
        if let Some(out) = position_path_completions(idxs, doc, &segs, &prefix) {
            return out;
        }
        // the head did not resolve to a module — `::` in a non-mod
        // context falls through to the tiers below
    }
    let Some((recv, prefix)) = recv_before(toks, pos) else {
        let used = used_ifaces(ast);
        let mut out = bare_completions(idxs, &used);
        out.extend(auto_import_completions(idxs, ast, toks, pos, src));
        return out;
    };
    // the qualified-position spelling: positions qualify with `.` —
    // `helpers.mk(`, `let c: layout.Column` — so a receiver that
    // resolves as a MOD HEAD offers the module's members (the same
    // tier the `::` form rides). A resolved head is final; an unknown
    // one falls through to the member tier
    if let Some(out) = position_path_completions(idxs, doc, std::slice::from_ref(&recv), &prefix) {
        return out;
    }
    // the binding pass — member completion shares hover's receiver
    // inference (field reads, chained calls, loop variables)
    let binds = crate::hover::bindings::collect(ast, toks, idxs);
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

/// Bare position: the keyword table (the grammar's reserved
/// set, one canonical list), then the visible decls — the open document
/// first, then the std surface and the workspace. Free fns and types
/// only; methods are reached through a receiver, not named bare.
/// The import-gated builtin names (`pub builtin` — the core disposal
/// pair) stay off the list unless the document's `use` names them: the
/// same ambient split the compiler binds by (offering a name the
/// compiler then rejects is a trap).
fn bare_completions(idxs: &[&DefIndex], used: &HashSet<String>) -> Vec<CompletionOut> {
    let mut out: Vec<CompletionOut> = Vec::new();
    for kw in rut_parser::RESERVED_KW {
        out.push(CompletionOut::plain((*kw).to_string(), String::new(), Vec::new(), CompletionKind::Keyword));
    }
    let gated: HashSet<&str> = idxs
        .iter()
        .flat_map(|i| i.pub_gated.iter().map(|s| s.as_str()))
        .collect();
    let offered = |name: &str| !gated.contains(name) || used.contains(name);
    for i in idxs {
        for t in &i.types {
            if !offered(&t.name) {
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
            if !offered(&f.name) {
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
            Tok::Ident(_) | Tok::Colon | Tok::LBrace | Tok::Comma => seg.insert(0, t),
            _ => return None,
        }
        if j == 0 {
            return None;
        }
        j -= 1;
    }
    // no `::` typed yet — still the module-NAME tier (the partial
    // ident is the prefix, the segments stay empty); a `::` anywhere
    // switches to the names tier of the spelled path
    let has_sep = seg.iter().any(|t| t.tok == Tok::Colon);
    let segments: Vec<String> = if has_sep {
        seg.iter()
            .filter_map(|t| match &t.tok {
                Tok::Ident(s) if !rut_parser::is_reserved_kw(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    } else {
        Vec::new()
    };
    let prefix = match seg.last() {
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
    // chain; deeper segments walk the pub edges. The typed prefix is a
    // trailing PARTIAL ident — never a segment to walk (`use pkg::V⏐`
    // offers the root; `use pkg::layout::Co⏐` walks `layout` only)
    let carrying = chain.iter().any(|i| i.mod_path.is_some());
    let interior: &[String] = if prefix.is_empty() {
        &segs[1..]
    } else {
        &segs[1..segs.len() - 1]
    };
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
                    format!("file module — {}", mods::display(&child)),
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
            Tok::Colon => run.insert(0, &toks[j]),
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
            Tok::Colon => colons += 1,
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
                format!("file module — {}", mods::display(&child)),
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
/// document already imports are NOT re-offered, and the `pub builtin`
/// gate keeps holding (an import offer for a gated name would complete
/// a name the compile still rejects until the use lands — the gate is
/// the compiler's ambient split, mirrored).
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
        let core = crate::hover::index(&c2, &cast, &ctoks);

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
}
