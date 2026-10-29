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
//! * cross-file: the use graph's reverse edges — every indexed file that
//!   `use`s the name from the declaring pkg (`matches_pkg`, the phase-2
//!   rule) is scanned with the same matcher;
//! * `include_declaration` per the LSP spec: the declaring ident joins
//!   the set when the client asks for it.
//!
//! Honest limits (recorded, never wrong): unimported (flat-chain) names
//! are scanned in the asking file and the declaring file only — the flat
//! chain does not pull the whole workspace in; a member reference needs
//! the receiver to resolve exactly where definition resolves it. Field /
//! enum-member / local targets never cross files (locals can't escape;
//! imports carry type and fn names, not members).

use std::collections::HashSet;

use rut_ast::ast::Ast;
use rut_lexer::span::Span;
use rut_lexer::token::{Tok, Token};

use crate::definition::{self, DefLocation};
use crate::hover::lookup;
use crate::hover::{bindings, DefIndex};

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
    idx: &'a DefIndex,
    /// the declaring ident's span — the include_declaration location and
    /// the matcher's expected answer
    span: Span,
    kind: Kind,
}

/// `textDocument/references` — every position whose definition is the
/// declaration under `pos`. Sorted by uri then position; deduplicated.
pub fn references(ctx: &definition::Ctx, pos: u32, include_decl: bool) -> Vec<DefLocation> {
    let Some(t) = lookup::tok_at(ctx.toks, pos) else {
        return Vec::new();
    };
    let Tok::Ident(name) = &t.tok else {
        return Vec::new();
    };
    if crate::semantic::is_keyword(name) {
        return Vec::new();
    }

    // 1. the target(s) — definition's answer at the query position,
    //    matched back to (index, declaring ident, kind) through the decl
    //    layers. Multiple hits = definition's candidate list: every
    //    candidate's references genuinely point at the ambiguous name,
    //    so they all count (never a wrong silent pick).
    let targets = targets(ctx, pos, name);
    if targets.is_empty() {
        return Vec::new();
    }

    // 2. the files worth scanning — the asking document, each target's
    //    own file, and (module-level names only) the reverse use-graph
    //    edges: which indexed files `use` the name from the declaring
    //    pkg. The live doc speaks for its URI; an indexed stale twin of
    //    the open file adds nothing but stale positions.
    let mut cand: Vec<&DefIndex> = vec![ctx.doc()];
    for tg in &targets {
        match tg.kind {
            Kind::Local => {}
            Kind::Member => cand.push(tg.idx),
            Kind::ModuleLet | Kind::Type | Kind::Fn => {
                cand.push(tg.idx);
                for x in ctx.idxs.iter().copied() {
                    let reverse_edge = x
                        .uses
                        .iter()
                        .any(|u| u.name == *name && definition::matches_pkg(tg.idx, &u.pkg));
                    if reverse_edge {
                        cand.push(x);
                    }
                }
            }
        }
    }
    let mut seen: HashSet<&str> = HashSet::new();
    let mut files: Vec<&DefIndex> = Vec::new();
    for f in cand {
        if f.origin == ctx.doc_uri && !std::ptr::eq(f, ctx.doc()) {
            continue; // the doc's stale workspace twin
        }
        if seen.insert(f.origin.as_str()) {
            files.push(f);
        }
    }

    // 3. scan each candidate file: parse once (the doc reuses the query
    //    pipeline's parse), then run definition() on every ident token
    //    carrying the name — a reference iff the answer is the target.
    let mut out: Vec<DefLocation> = Vec::new();
    let mut seen_locs: HashSet<(String, u32, u32, u32, u32)> = HashSet::new();
    scan_files(ctx, &files, &targets, name, &mut out, &mut seen_locs);

    if include_decl {
        for tg in &targets {
            let loc = definition::loc_at(ctx.doc_uri, tg.idx, tg.span);
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
/// never ambiguous, even when the bare name is), so the decl spans are
/// checked first; everything else resolves through `definition()` and
/// keeps its candidate-list honesty.
fn targets<'a>(ctx: &definition::Ctx<'a>, pos: u32, name: &str) -> Vec<Target<'a>> {
    let t = lookup::tok_at(ctx.toks, pos);
    let qspan = t.map(|t| t.span);
    let doc: &'a DefIndex = ctx.idxs[0];
    // 0. decl sites of the doc's own decl layers — the declaring ident
    //    IS the answer (mirrors definition's decl_site + binding-decl
    //    precedence, extended to fn/type name spans)
    if let Some(q) = qspan {
        if let Some(b) = bindings::collect(ctx.ast, ctx.toks, ctx.idxs)
            .iter()
            .find(|b| b.name == name && b.decl_ident_span == q)
        {
            return vec![Target {
                idx: doc,
                span: b.decl_ident_span,
                kind: Kind::Local,
            }];
        }
        if let Some(_l) = doc
            .lets
            .iter()
            .find(|l| l.name == name && l.name_span == Some(q))
        {
            return vec![Target {
                idx: doc,
                span: q,
                kind: Kind::ModuleLet,
            }];
        }
        for ty in doc.types.iter().filter(|t| t.name == name) {
            if definition::ty_span(ty) == q {
                return vec![Target {
                    idx: doc,
                    span: q,
                    kind: Kind::Type,
                }];
            }
        }
        for f in doc.fns.iter().filter(|f| f.name == name) {
            if definition::fn_span(f) == q {
                return vec![Target {
                    idx: doc,
                    span: q,
                    kind: Kind::Fn,
                }];
            }
        }
        for ty in &doc.types {
            for m in ty.fields.iter().chain(ty.methods.iter()) {
                if m.name == name && m.name_span == Some(q) {
                    return vec![Target {
                        idx: doc,
                        span: q,
                        kind: Kind::Member,
                    }];
                }
            }
        }
    }

    let hits = definition::definition(ctx, pos);
    if hits.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (n, idx) in ctx.idxs.iter().copied().enumerate() {
        let is_doc = n == 0;
        // the binding layer — the doc's locals/params/iter-vars
        if is_doc {
            let binds = bindings::collect(ctx.ast, ctx.toks, ctx.idxs);
            for b in binds.iter().filter(|b| b.name == name) {
                if hits.contains(&definition::loc_at(ctx.doc_uri, idx, b.decl_ident_span)) {
                    out.push(Target {
                        idx,
                        span: b.decl_ident_span,
                        kind: Kind::Local,
                    });
                }
            }
        }
        for l in idx.lets.iter().filter(|l| l.name == name) {
            let sp = l.name_span.unwrap_or(l.span);
            if hits.contains(&definition::loc_at(ctx.doc_uri, idx, sp)) {
                out.push(Target {
                    idx,
                    span: sp,
                    kind: Kind::ModuleLet,
                });
            }
        }
        for ty in idx.types.iter().filter(|t| t.name == name) {
            let sp = definition::ty_span(ty);
            if hits.contains(&definition::loc_at(ctx.doc_uri, idx, sp)) {
                out.push(Target {
                    idx,
                    span: sp,
                    kind: Kind::Type,
                });
            }
        }
        for f in idx.fns.iter().filter(|f| f.name == name) {
            let sp = definition::fn_span(f);
            if hits.contains(&definition::loc_at(ctx.doc_uri, idx, sp)) {
                out.push(Target {
                    idx,
                    span: sp,
                    kind: Kind::Fn,
                });
            }
        }
        // fields + enum members + own-surface methods — every recorded
        // member name span of every type
        for ty in &idx.types {
            for m in ty
                .fields
                .iter()
                .chain(ty.methods.iter())
                .filter(|m| m.name == name)
            {
                if let Some(sp) = m.name_span {
                    if hits.contains(&definition::loc_at(ctx.doc_uri, idx, sp)) {
                        out.push(Target {
                            idx,
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

/// one parsed candidate file — the doc reuses the query pipeline's
/// tokens/AST; the other files lex+parse their indexed source once
struct Parsed {
    toks: Vec<Token>,
    ast: rut_ast::ast::Ast,
}

/// a declaring ident never doubles as a reference to a DIFFERENT
/// declaration of the same name — at such a token definition's bare-name
/// layers answer with the ambiguity peek (every same-named fn/type), and
/// counting that as a reference would leak across declarations. The
/// decl layers' recorded name spans, plus the file's binding decl
/// idents, are therefore excluded from the scan (a use-statement name
/// span stays eligible — importing IS a reference).
fn decl_ident_spans(
    idx: &DefIndex,
    toks: &[Token],
    ast: &Ast,
    chain: &[&DefIndex],
) -> HashSet<(u32, u32)> {
    let mut out: HashSet<(u32, u32)> = HashSet::new();
    let put = |out: &mut HashSet<(u32, u32)>, sp: Option<Span>| {
        if let Some(sp) = sp {
            out.insert((sp.lo, sp.hi));
        }
    };
    for l in &idx.lets {
        put(&mut out, l.name_span);
    }
    for t in &idx.types {
        put(&mut out, t.name_span);
        for m in t.fields.iter().chain(t.methods.iter()) {
            put(&mut out, m.name_span);
        }
    }
    for f in &idx.fns {
        out.insert((
            f.name_span.unwrap_or(f.span).lo,
            f.name_span.unwrap_or(f.span).hi,
        ));
    }
    for b in bindings::collect(ast, toks, chain) {
        out.insert((b.decl_ident_span.lo, b.decl_ident_span.hi));
    }
    out
}

fn scan_files(
    ctx: &definition::Ctx,
    files: &[&DefIndex],
    targets: &[Target],
    name: &str,
    out: &mut Vec<DefLocation>,
    seen: &mut HashSet<(String, u32, u32, u32, u32)>,
) {
    // parse the non-doc candidates once (doc-sized files; the per-query
    // re-parse is the house style — hover already rides it)
    let mut parsed: Vec<(&DefIndex, Parsed)> = Vec::new();
    for f in files {
        if std::ptr::eq(*f, ctx.doc()) {
            continue;
        }
        let (toks, ast) = crate::analysis::parse_at(f.origin.as_str(), &f.src);
        parsed.push((f, Parsed { toks, ast }));
    }

    for f in files {
        let is_doc = std::ptr::eq(*f, ctx.doc());
        // the file's own lookup chain: itself first, then the rest — no
        // origin speaks twice in one chain
        let mut chain: Vec<&DefIndex> = vec![*f];
        chain.extend(ctx.idxs.iter().copied().filter(|x| x.origin != f.origin));
        // the doc reuses the query pipeline's tokens/AST; the other
        // files ride their per-file parse
        let (toks, ast): (&[Token], &rut_ast::ast::Ast) =
            match parsed.iter().find(|(i, _)| std::ptr::eq(*i, *f)) {
                Some(p) => (&p.1.toks, &p.1.ast),
                None if is_doc => (ctx.toks, ctx.ast),
                None => unreachable!("every non-doc candidate was parsed above"),
            };
        // the asking doc's own-package context rides only for its own
        // tokens (the two queries cannot disagree about the doc's
        // idents); another file's context is unknown — the default
        // keeps the own lanes off in its scan
        let orphan = crate::mods::DocMods::default();
        let f_ctx = definition::Ctx {
            doc_uri: f.origin.as_str(),
            toks,
            ast,
            idxs: &chain,
            doc_mods: if is_doc { ctx.doc_mods } else { &orphan },
        };
        let decls = decl_ident_spans(f, toks, ast, &chain);

        for t in toks.iter() {
            let Tok::Ident(nm) = &t.tok else { continue };
            if nm != name
                || decls.contains(&(t.span.lo, t.span.hi))
                || targets.iter().any(|tg| tg.span == t.span)
            {
                continue; // not the name, or a declaring ident (include_declaration adds the target's)
            }
            let ans = definition::definition(&f_ctx, t.span.lo);
            if ans.is_empty() {
                continue;
            }
            for tg in targets {
                let expected = definition::loc_at(f.origin.as_str(), tg.idx, tg.span);
                if ans.iter().any(|l| l == &expected) {
                    let loc = definition::loc_at(f.origin.as_str(), f, t.span);
                    if seen.insert(loc_key(&loc)) {
                        out.push(loc);
                    }
                    break; // one entry per site, even under ambiguity
                }
            }
        }
    }
}
