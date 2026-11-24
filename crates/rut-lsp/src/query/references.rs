//! References — the definition index read BACKWARDS (the lsp-features
//! survey §4.4). One law, zero duplicated resolution: **a token is a
//! reference to a declaration iff go-to-definition from that token lands
//! on the declaration.** The query first resolves its own position
//! through `definition`'s layers (decl sites, use-graph names, member
//! position, type position, the binding pass, module lets, fns) to a
//! set of targets, then scans the candidate files' identifier tokens
//! through the SAME `definition()` call, keeping the sites whose answer
//! is the target. The two queries cannot disagree about what an ident
//! means — a reference set is exactly the ctrl+click set.
//!
//! * within-file: the binding pass answers locals/params/iter-vars —
//!   shadow-aware by construction (a shadowing decl and its uses resolve
//!   to the shadow, never to the outer binding) and scope-exit correct;
//! * cross-file: for a module-level name (a fn, a type, a module let)
//!   EVERY world file is a candidate — each of its tokens naming the
//!   name runs through the same `definition()` call IN THAT FILE'S OWN
//!   CONTEXT (its own stamp — the own-package lanes key on it,
//!   so a sibling's `use m::{ x }` and its `app::x` both resolve), and
//!   counts iff the answer is the target. No import-map pre-filter: a
//!   qualified spelling, a glob binding, and a pkg-rooted dep path all
//!   name no leaf the pre-filter could see;
//! * `include_declaration` per the LSP spec: the declaring ident joins
//!   the set when the client asks for it.
//!
//! Honest limits (recorded, never wrong): unimported (flat-world) names
//! are scanned in the asking file and the declaring file only — the
//! flat sweep does not pull the whole workspace in; a member reference needs
//! the receiver to resolve exactly where definition resolves it. Field /
//! enum-member / local targets never cross files (locals can't escape;
//! imports carry type and fn names, not members).
//!
//! Every candidate is a world [`File`] — its document and tree are
//! KEPT, so the old re-parse lane (candidate source re-lexed and
//! re-parsed per query) is gone: the scan rides each file's own tokens.

use std::collections::HashSet;

use crate::query::definition::{self, Ctx, DefLocation};
use crate::semantic::Span;
use crate::world::{File, FileId};

/// what the ident at `pos` was resolved to — drives how far the scan
/// reaches (locals stay in the doc; module-level names ride the use
/// graph's reverse edges)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// a fn-local binding (Let/Param/IterVar) — the doc only
    Local,
    /// a module-scope `let`
    ModuleLet,
    /// a type decl (class/struct/interface/enum/…)
    Type,
    /// a free fn or impl method
    Fn,
    /// a field or enum member
    Member,
}

struct Target<'a> {
    file: &'a File,
    /// the target's own id in the world (the candidate list keys on it)
    id: FileId,
    /// the declaring ident's span — the include_declaration location and
    /// the matcher's expected answer
    span: Span,
    kind: Kind,
}

/// `textDocument/references` — every position whose definition is the
/// declaration under `pos`. Sorted by uri then position; deduplicated.
pub fn references(ctx: &Ctx, pos: u32, include_decl: bool) -> Vec<DefLocation> {
    let w = ctx.w;
    let doc = w.file(ctx.file);
    let ast = doc.doc.ast();
    // the cursor's node — the byte under it, else the byte just before
    // (the boundary convention — the one probe)
    let Some(h) = crate::semantic::node_at_caret(&doc.doc, pos) else {
        return Vec::new();
    };
    if h.is::<rut_ast::ErrorNode>() || h.is::<rut_ast::SelfNode>() || h.is::<rut_ast::SelfTyNode>()
    {
        return Vec::new();
    }
    // the name lanes key on the node's own recorded name; a member
    // USE (a method call, a field read) is a reference position too —
    // its member token names the decl through the member door
    let Some((name_id, _)) =
        definition::node_name(ast, h, pos).or_else(|| member_ident(ast, h, pos))
    else {
        return Vec::new();
    };
    let name = ast.name(name_id).to_string();

    // 1. the target(s) — definition's answer at the query position,
    //    matched back to (file, declaring ident, kind) through the decl
    //    layers. Multiple hits = definition's candidate list: every
    //    candidate's references genuinely point at the ambiguous name,
    //    so they all count (never a wrong silent pick).
    let targets = targets(ctx, pos, &name, name_id);
    if targets.is_empty() {
        return Vec::new();
    }

    // 2. the files worth scanning — the asking document first, each
    //    target's own file; a module-level name (a fn, a type, a module
    //    let) reaches every world file (a sibling's bare use binding,
    //    its qualified spelling, a glob, a pkg-rooted dep path — all are
    //    references, none spells a leaf an import map could pre-filter
    //    on); locals stay in the doc and members add only their
    //    declaring file. One entry per file (the store's own law — one
    //    entry per origin, nothing to dedupe beyond the id set).
    let module_level = targets
        .iter()
        .any(|tg| matches!(tg.kind, Kind::ModuleLet | Kind::Type | Kind::Fn));
    let mut cand: Vec<FileId> = vec![ctx.file];
    for tg in &targets {
        cand.push(tg.id);
    }
    if module_level {
        cand.extend(w.files());
    }
    let mut seen: HashSet<FileId> = HashSet::new();
    let mut files: Vec<FileId> = Vec::new();
    for id in cand {
        if seen.insert(id) {
            files.push(id);
        }
    }

    // 3. scan each candidate file: its KEPT document rides (no
    //    re-parse — the mount lane's own law), then definition() runs
    //    on every tree-recorded name position carrying the name — a
    //    reference iff the answer is the target.
    let mut out: Vec<DefLocation> = Vec::new();
    let mut seen_locs: HashSet<(String, u32, u32, u32, u32)> = HashSet::new();
    scan_files(ctx, &files, &targets, &name, &mut out, &mut seen_locs);

    if include_decl {
        for tg in &targets {
            let loc = definition::loc_at(tg.file, tg.span);
            if seen_locs.insert(loc_key(&loc)) {
                out.push(loc);
            }
        }
    }
    out.sort_by(|a, b| {
        (
            &a.uri,
            a.range.start.line,
            a.range.start.character,
            a.range.end.line,
            a.range.end.character,
        )
            .cmp(&(
                &b.uri,
                b.range.start.line,
                b.range.start.character,
                b.range.end.line,
                b.range.end.character,
            ))
    });
    out
}

fn loc_key(l: &DefLocation) -> (String, u32, u32, u32, u32) {
    (
        l.uri.clone(),
        l.range.start.line,
        l.range.start.character,
        l.range.end.line,
        l.range.end.character,
    )
}

/// definition's answer at the query position, matched back to concrete
/// decl records — the targets the scan will match against. A query ON a
/// recorded declaring ident is that ONE declaration (a decl site is
/// never ambiguous, even when the bare name is), so the decl positions
/// are checked first; everything else resolves through `definition()`
/// and keeps its candidate-list honesty.
fn targets<'a>(ctx: &Ctx<'a>, pos: u32, name: &str, name_id: rut_core::IdentId) -> Vec<Target<'a>> {
    let w = ctx.w;
    // 0. decl positions of the doc's own decl layers — the declaring
    //    ident IS the answer (the tree's own name-position downcasts,
    //    mirroring definition's decl-site + binding-decl precedence,
    //    extended to fn/type name spans)
    if let Some(sp) = decl_pos(w.file(ctx.file), ctx.file, pos, name, name_id) {
        return vec![sp];
    }

    let hits = definition::get_definition(ctx, pos);
    if hits.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for id in w.chain(ctx.file) {
        let f = w.file(id);
        let is_doc = id == ctx.file;
        // the binding layer — the doc's locals/params/iter-vars
        if is_doc {
            for b in f.doc.bindings().iter().filter(|b| b.name == name_id) {
                if hits.contains(&definition::loc_at(f, b.decl_ident_span)) {
                    out.push(Target {
                        file: f,
                        id,
                        span: b.decl_ident_span,
                        kind: Kind::Local,
                    });
                }
            }
        }
        for l in f.lets().into_iter().filter(|l| l.name() == name) {
            let sp = l.name_span().unwrap_or(l.span());
            if hits.contains(&definition::loc_at(f, sp)) {
                out.push(Target {
                    file: f,
                    id,
                    span: sp,
                    kind: Kind::ModuleLet,
                });
            }
        }
        for ty in f.types().iter().filter(|t| t.name() == name) {
            let sp = definition::ty_span(ty);
            if hits.contains(&definition::loc_at(f, sp)) {
                out.push(Target {
                    file: f,
                    id,
                    span: sp,
                    kind: Kind::Type,
                });
            }
        }
        for fv in f.fns().into_iter().filter(|f| f.name() == name) {
            let sp = definition::fn_span(&fv);
            if hits.contains(&definition::loc_at(f, sp)) {
                out.push(Target {
                    file: f,
                    id,
                    span: sp,
                    kind: Kind::Fn,
                });
            }
        }
        // fields + enum members + own-surface methods — every recorded
        // member name span of every type
        for ty in f.types().iter() {
            for m in ty.fields().into_iter().chain(ty.methods()) {
                if m.name() != name {
                    continue;
                }
                if let Some(sp) = m.name_span() {
                    if hits.contains(&definition::loc_at(f, sp)) {
                        out.push(Target {
                            file: f,
                            id,
                            span: sp,
                            kind: Kind::Member,
                        });
                    }
                }
            }
        }
    }
    out
}

/// the doc's own decl position under the cursor, as a target — the
/// name-position downcasts (binding rows, module lets, types, fns,
/// members)
fn decl_pos<'a>(
    doc: &'a File,
    id: FileId,
    pos: u32,
    name: &str,
    name_id: rut_core::IdentId,
) -> Option<Target<'a>> {
    let on = |sp: rut_lexer::span::Span| sp.lo <= pos && pos <= sp.hi;
    // the binding layer's rows first (a param/let/loop-var decl)
    for b in doc.doc.bindings().iter() {
        if b.name == name_id && on(b.decl_ident_span) {
            return Some(Target {
                file: doc,
                id,
                span: b.decl_ident_span,
                kind: Kind::Local,
            });
        }
    }
    for l in doc.lets() {
        if let Some(sp) = l.name_span() {
            if l.name() == name && on(sp) {
                return Some(Target {
                    file: doc,
                    id,
                    span: sp,
                    kind: Kind::ModuleLet,
                });
            }
        }
    }
    for ty in doc.types() {
        if ty.name() == name && on(definition::ty_span(&ty)) {
            return Some(Target {
                file: doc,
                id,
                span: definition::ty_span(&ty),
                kind: Kind::Type,
            });
        }
    }
    for f in doc.fns() {
        if f.name() == name && on(definition::fn_span(&f)) {
            return Some(Target {
                file: doc,
                id,
                span: definition::fn_span(&f),
                kind: Kind::Fn,
            });
        }
    }
    for ty in doc.types() {
        for m in ty.fields().into_iter().chain(ty.methods()) {
            if m.name() == name && m.name_span().is_some_and(on) {
                return Some(Target {
                    file: doc,
                    id,
                    span: m.name_span()?,
                    kind: Kind::Member,
                });
            }
        }
    }
    None
}

/// a declaring ident never doubles as a reference to a DIFFERENT
/// declaration of the same name — at such a token definition's bare-name
/// layers answer with the ambiguity peek (every same-named fn/type), and
/// counting that as a reference would leak across declarations. The
/// decl layers' recorded name spans, plus the file's binding decl
/// idents, are therefore excluded from the scan (a use-statement name
/// span stays eligible — importing IS a reference).
fn decl_ident_spans(f: &File) -> HashSet<(u32, u32)> {
    let mut out: HashSet<(u32, u32)> = HashSet::new();
    let put = |out: &mut HashSet<(u32, u32)>, sp: Option<Span>| {
        if let Some(sp) = sp {
            out.insert((sp.lo, sp.hi));
        }
    };
    for l in f.lets() {
        put(&mut out, l.name_span());
    }
    for t in f.types() {
        put(&mut out, t.name_span());
        for m in t.fields().into_iter().chain(t.methods()) {
            put(&mut out, m.name_span());
        }
    }
    for fv in f.fns() {
        let sp = definition::fn_span(&fv);
        out.insert((sp.lo, sp.hi));
    }
    for b in f.doc.bindings() {
        out.insert((b.decl_ident_span.lo, b.decl_ident_span.hi));
    }
    out
}

/// the scan over every candidate file — each candidate's KEPT document
/// rides (the mount lane's law: the tree is kept, nothing re-parses).
/// The positions enumerated are the tree's own name inventory
/// (`name_positions` — every recorded name span); no token scan.
/// Each candidate resolves under ITS OWN STAMP (`Ctx { w, file: cand }`
/// — one world, one store: a file's own-package lanes key on the pair
/// its loader lane laid down, exactly as when that file is the open
/// document; the asking doc keeps its own for its idents).
fn scan_files(
    ctx: &Ctx,
    files: &[FileId],
    targets: &[Target],
    name: &str,
    out: &mut Vec<DefLocation>,
    seen: &mut HashSet<(String, u32, u32, u32, u32)>,
) {
    for id in files {
        let f = ctx.w.file(*id);
        let f_ctx = Ctx {
            w: ctx.w,
            file: *id,
        };
        let decls = decl_ident_spans(f);
        let ast = f.doc.ast();

        for (sp, ident) in rut_ast::name_positions(ast) {
            if ast.name(ident) != name
                || decls.contains(&(sp.lo, sp.hi))
                || targets
                    .iter()
                    .any(|tg| tg.span == sp && std::ptr::eq(tg.file, f))
            {
                continue; // not the name, or a declaring ident (include_declaration adds the target's)
            }
            let ans = definition::get_definition(&f_ctx, sp.lo);
            if ans.is_empty() {
                continue;
            }
            for tg in targets {
                let expected = definition::loc_at(tg.file, tg.span);
                if ans.iter().any(|l| l == &expected) {
                    let loc = definition::loc_at(f, sp);
                    if seen.insert(loc_key(&loc)) {
                        out.push(loc);
                    }
                    break; // one entry per site, even under ambiguity
                }
            }
        }
    }
}

/// the token under the cursor that names a decl without a node of
/// its own — a method call's member, a field read's name (the member
/// door's position test), a `use` statement's leaf
fn member_ident(
    ast: &rut_ast::Ast,
    h: rut_ast::AnyHandle,
    pos: u32,
) -> Option<(rut_core::IdentId, Span)> {
    let on = |sp: Span| sp.lo <= pos && pos <= sp.hi;
    if let Some(m) = h.downcast::<rut_ast::MethodNode>() {
        let d = m.get(ast);
        return d.member.filter(|n| on(n.span)).map(|n| (n.ident, n.span));
    }
    if let Some(f) = h.downcast::<rut_ast::FieldNode>() {
        let d = f.get(ast);
        return on(d.name.span).then_some((d.name.ident, d.name.span));
    }
    // a `use` statement's LEAF name (its path segments are module
    // names — the walk's business, never a decl's references)
    if let Some(u) = h.downcast::<rut_ast::UseNode>() {
        let d = u.get(ast);
        return d
            .names
            .iter()
            .find(|n| on(n.span))
            .map(|n| (n.ident, n.span));
    }
    None
}
