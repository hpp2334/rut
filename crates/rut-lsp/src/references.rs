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
    /// a type decl (class/struct/trait/enum/…)
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
    let Some(t) = lookup::tok_at(ctx.toks, pos) else { return Vec::new() };
    let Tok::Ident(name) = &t.tok else { return Vec::new() };
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
        (&a.uri, a.range.start.line, a.range.start.character, a.range.end.line, a.range.end.character).cmp(&(
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
        for b in bindings::collect(ctx.ast, ctx.toks, ctx.idxs)
            .iter()
            .filter(|b| b.name == name && b.decl_ident_span == q)
        {
            return vec![Target { idx: doc, span: b.decl_ident_span, kind: Kind::Local }];
        }
        for _l in doc.lets.iter().filter(|l| l.name == name && l.name_span == Some(q)) {
            return vec![Target { idx: doc, span: q, kind: Kind::ModuleLet }];
        }
        for ty in doc.types.iter().filter(|t| t.name == name) {
            if definition::ty_span(ty) == q {
                return vec![Target { idx: doc, span: q, kind: Kind::Type }];
            }
        }
        for f in doc.fns.iter().filter(|f| f.name == name) {
            if definition::fn_span(f) == q {
                return vec![Target { idx: doc, span: q, kind: Kind::Fn }];
            }
        }
        for ty in &doc.types {
            for m in ty.fields.iter().chain(ty.methods.iter()) {
                if m.name == name && m.name_span == Some(q) {
                    return vec![Target { idx: doc, span: q, kind: Kind::Member }];
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
                    out.push(Target { idx, span: b.decl_ident_span, kind: Kind::Local });
                }
            }
        }
        for l in idx.lets.iter().filter(|l| l.name == name) {
            let sp = l.name_span.unwrap_or(l.span);
            if hits.contains(&definition::loc_at(ctx.doc_uri, idx, sp)) {
                out.push(Target { idx, span: sp, kind: Kind::ModuleLet });
            }
        }
        for ty in idx.types.iter().filter(|t| t.name == name) {
            let sp = definition::ty_span(ty);
            if hits.contains(&definition::loc_at(ctx.doc_uri, idx, sp)) {
                out.push(Target { idx, span: sp, kind: Kind::Type });
            }
        }
        for f in idx.fns.iter().filter(|f| f.name == name) {
            let sp = definition::fn_span(f);
            if hits.contains(&definition::loc_at(ctx.doc_uri, idx, sp)) {
                out.push(Target { idx, span: sp, kind: Kind::Fn });
            }
        }
        // fields + enum members + own-surface methods — every recorded
        // member name span of every type
        for ty in &idx.types {
            for m in ty.fields.iter().chain(ty.methods.iter()).filter(|m| m.name == name) {
                if let Some(sp) = m.name_span {
                    if hits.contains(&definition::loc_at(ctx.doc_uri, idx, sp)) {
                        out.push(Target { idx, span: sp, kind: Kind::Member });
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
fn decl_ident_spans(idx: &DefIndex, toks: &[Token], ast: &Ast, chain: &[&DefIndex]) -> HashSet<(u32, u32)> {
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
        out.insert((f.name_span.unwrap_or(f.span).lo, f.name_span.unwrap_or(f.span).hi));
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
        let (toks, ast): (&[Token], &rut_ast::ast::Ast) = match parsed.iter().find(|(i, _)| std::ptr::eq(*i, *f)) {
            Some(p) => (&p.1.toks, &p.1.ast),
            None if is_doc => (ctx.toks, ctx.ast),
            None => unreachable!("every non-doc candidate was parsed above"),
        };
        let f_ctx = definition::Ctx {
            doc_uri: f.origin.as_str(),
            toks,
            ast,
            idxs: &chain,
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

#[cfg(test)]
mod tests {
    use super::*;
    use rut_lexer::lexer::{lex, normalize};
    use rut_parser::{parse, Mode};

    use crate::hover::DefIndex;
    use crate::line_index::LineIndex;

    const DOC: &str = "file:///t/main.rut";

    struct Doc {
        src: String,
        toks: Vec<Token>,
        ast: rut_ast::ast::Ast,
        index: DefIndex,
    }

    fn doc_at(uri: &str, src: &str) -> Doc {
        let src = normalize(src);
        let (toks, _) = lex(&src);
        let (ast, _) = parse(&src, Mode::Impl);
        let mut index = crate::hover::index(&src, &ast, &toks);
        index.origin = uri.to_string();
        Doc { src, toks, ast, index }
    }

    fn doc(src: &str) -> Doc {
        doc_at(DOC, src)
    }

    /// byte offset of the `n`th (1-based) occurrence of `needle`
    fn at(src: &str, needle: &str, occurrence: usize) -> u32 {
        let mut from = 0;
        for _ in 0..occurrence {
            match src[from..].find(needle) {
                Some(i) => from += i + needle.len(),
                None => panic!("needle {needle:?} not found (occurrence {occurrence})"),
            }
        }
        (from - needle.len()) as u32
    }

    /// every reference's span start over `src` (locations whose uri is
    /// not the doc are listed separately by the cross-file tests)
    fn ref_offsets(src: &str, uri: &str, locs: &[DefLocation]) -> Vec<u32> {
        let ix = LineIndex::new(src);
        locs.iter()
            .filter(|l| l.uri == uri)
            .map(|l| ix.byte(src, l.range.start.line, l.range.start.character))
            .collect()
    }

    fn refs(d: &Doc, pos: u32, extra: &[DefIndex], include_decl: bool) -> Vec<DefLocation> {
        let idxs: Vec<&DefIndex> = std::iter::once(&d.index).chain(extra.iter()).collect();
        let ctx = definition::Ctx { doc_uri: DOC, toks: &d.toks, ast: &d.ast, idxs: &idxs };
        references(&ctx, pos, include_decl)
    }

    /// the query at byte `pos` references EXACTLY the idents at `want`
    /// (byte offsets of the reference idents, order-insensitive)
    fn assert_refs(d: &Doc, pos: u32, want: &[u32], extra: &[DefIndex]) {
        let hits = refs(d, pos, extra, false);
        let mut got = ref_offsets(&d.src, DOC, &hits);
        got.sort();
        got.dedup();
        let mut want_sorted = want.to_vec();
        want_sorted.sort();
        assert_eq!(got, want_sorted, "references at byte {pos}: {hits:?}");
    }

    // ---- the binding pass, reversed: shadow-aware ----

    #[test]
    fn local_binding_refs_are_the_shadow_correct_set() {
        let src = [
            "fn go() -> i32 {",
            "    let v = 1;",
            "    if (v > 0) {",
            "        let v = 2;",
            "        return v;",
            "    }",
            "    return v;",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        // the OUTER v: its use in the if-condition + the final return —
        // the inner shadow's decl and its return do NOT leak in
        assert_refs(
            &d,
            at(&d.src, "v = 1;", 1),
            &[at(&d.src, "v > 0", 1), at(&d.src, "return v;", 2) + 7],
            &[],
        );
        // the INNER v: exactly its own return — the outer's uses don't leak in
        assert_refs(&d, at(&d.src, "v = 2;", 1), &[at(&d.src, "return v;", 1) + 7], &[]);
        // a use site resolves to its own binding (the same set as its decl)
        assert_refs(
            &d,
            at(&d.src, "return v;", 2) + 7,
            &[at(&d.src, "v > 0", 1), at(&d.src, "return v;", 2) + 7],
            &[],
        );
    }

    #[test]
    fn include_declaration_appends_the_declaring_ident() {
        let src = "fn go() -> i32 {\n    let c = 1;\n    return c;\n}\n";
        let d = doc(&src);
        let pos = at(&d.src, "let c", 1) + 4;
        let hits = refs(&d, pos, &[], false);
        assert_eq!(
            ref_offsets(&d.src, DOC, &hits),
            vec![at(&d.src, "return c;", 1) + 7],
            "use only without the toggle"
        );
        let hits = refs(&d, pos, &[], true);
        let mut offs = ref_offsets(&d.src, DOC, &hits);
        offs.sort();
        assert_eq!(
            offs,
            vec![at(&d.src, "let c", 1) + 4, at(&d.src, "return c;", 1) + 7],
            "the decl ident joins with include_declaration"
        );
    }

    #[test]
    fn param_refs_follow_the_landed_shadow_rule() {
        let src = [
            "fn go(k: i32) -> i32 {",
            "    let k = k + 1;",
            "    return k;",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        // the PARAM k: nothing — even the initializer's `k` reads the
        // shadowing local under the landed latest-decl rule (references
        // MUST match what definition answers there — the law of this
        // module — not the compiler's initialize-before-bind reading)
        assert_refs(&d, at(&d.src, "k: i32", 1), &[], &[]);
        // the LOCAL k: the initializer's k + the return
        assert_refs(
            &d,
            at(&d.src, "let k", 1) + 4,
            &[at(&d.src, "k + 1", 1), at(&d.src, "return k;", 1) + 7],
            &[],
        );
    }

    #[test]
    fn scope_exit_never_leaks_a_use() {
        let src = [
            "fn go() -> i32 {",
            "    if (true) {",
            "        let w = 1;",
            "        return w;",
            "    }",
            "    return 0;",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        assert_refs(&d, at(&d.src, "w = 1;", 1), &[at(&d.src, "return w;", 1) + 7], &[]);
    }

    // ---- the decl layer, reversed ----

    #[test]
    fn fn_refs_cover_calls_and_the_decl_toggle() {
        let src = [
            "fn make() -> i32 {",
            "    return 1;",
            "}",
            "fn main() -> i32 {",
            "    let a = make();",
            "    let b = make();",
            "    return a + b;",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        let call1 = at(&d.src, "= make();", 1) + 2;
        let call2 = at(&d.src, "= make();", 2) + 2;
        // from the decl: both calls (the decl itself stays out)
        assert_refs(&d, at(&d.src, "fn make()", 1) + 3, &[call1, call2], &[]);
        // include_declaration puts the declaring ident in
        let hits = refs(&d, at(&d.src, "fn make()", 1) + 3, &[], true);
        assert_eq!(hits.len(), 3, "{hits:?}");
        // from a call site: the same set (references work from any occurrence)
        assert_refs(&d, call2, &[call1, call2], &[]);
    }

    #[test]
    fn type_refs_cover_type_positions_and_ctor_paths() {
        let src = [
            "class Circle {",
            "    r: f64;",
            "}",
            "impl Circle {",
            "    fn area(self) -> f64 { return 3.14; }",
            "}",
            "fn go(c: Circle) -> f64 {",
            "    let w = Circle.new(1.0);",
            "    return c.area() + w.r;",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        // the decl of `Circle` → the param annotation, the impl head,
        // the ctor path (NOT the field read `w.r` / members)
        assert_refs(
            &d,
            at(&d.src, "class Circle", 1) + 6,
            &[
                at(&d.src, "c: Circle", 1) + 3,
                at(&d.src, "impl Circle", 1) + 5,
                at(&d.src, "= Circle.new", 1) + 2,
            ],
            &[],
        );
    }

    #[test]
    fn method_refs_need_the_receiver_type() {
        let src = [
            "class Circle {",
            "    r: f64;",
            "}",
            "struct Box2 {",
            "    s: f64;",
            "}",
            "impl Circle {",
            "    fn area(self) -> f64 { return 3.14; }",
            "}",
            "impl Box2 {",
            "    fn area(self) -> f64 { return 1.0; }",
            "}",
            "fn go(c: Circle, b: Box2) -> f64 {",
            "    let a = c.area();",
            "    let z = b.area();",
            "    return a + z;",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        // references on Circle's `area` decl: the `c.area()` call, whose
        // receiver types to Circle — but NOT `b.area()` (a same-named
        // method on a different receiver type)
        assert_refs(
            &d,
            at(&d.src, "fn area(self) -> f64 { return 3.14;", 1) + 3,
            &[at(&d.src, "c.area();", 1) + 2],
            &[],
        );
        // and Box2's own `area` answers with its own call
        assert_refs(
            &d,
            at(&d.src, "fn area(self) -> f64 { return 1.0;", 1) + 3,
            &[at(&d.src, "b.area();", 1) + 2],
            &[],
        );
    }

    #[test]
    fn field_refs_ride_the_receiver_inference() {
        let src = [
            "class Circle {",
            "    r: f64;",
            "}",
            "fn go(c: Circle, k: f64) -> f64 {",
            "    c.r = k;",
            "    return c.r;",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        // the field decl's references: both dotted reads
        assert_refs(
            &d,
            at(&d.src, "r: f64;", 1),
            &[at(&d.src, "c.r = k;", 1) + 2, at(&d.src, "return c.r;", 1) + 9],
            &[],
        );
    }

    #[test]
    fn module_let_refs_skip_local_shadows() {
        let src = [
            "let TOTAL = 41;",
            "fn bump() -> i32 {",
            "    return TOTAL + 1;",
            "}",
            "fn shadow() -> i32 {",
            "    let TOTAL = 5;",
            "    return TOTAL;",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        // the module let: bump's return only — the shadow's decl and its
        // return belong to the local (and under the landed latest-decl
        // rule, so does an initializer reading: `let TOTAL = TOTAL` is
        // the local's, definition-consistent)
        assert_refs(&d, at(&d.src, "let TOTAL = 41;", 1) + 4, &[at(&d.src, "return TOTAL + 1;", 1) + 7], &[]);
        // the LOCAL TOTAL: its own return only
        assert_refs(&d, at(&d.src, "let TOTAL = 5;", 1) + 8, &[at(&d.src, "return TOTAL;", 1) + 7], &[]);
    }

    // ---- cross-file: the use graph's reverse edges ----

    const WIDGET_URI: &str = "file:///ws/gadgets/lib.rut";

    fn widget_lib() -> Doc {
        doc_at(WIDGET_URI, "class Widget {\n    id: i32;\n}\n")
    }

    fn widget_user() -> (Doc, String) {
        let src = "use gadgets::{ Widget };\nfn main() -> nil {\n    let w = Widget.new();\n}\n";
        (doc(src), normalize(src))
    }

    #[test]
    fn cross_file_refs_find_the_importing_files() {
        let lib = widget_lib();
        let (main, norm) = widget_user();
        let extra = [&lib.index];
        // references on the LIB's type — queried from the importing doc:
        // the use-statement name + the usage site, nothing else
        let idxs: Vec<&DefIndex> = std::iter::once(&main.index).chain(extra.iter().copied()).collect();
        let ctx = definition::Ctx {
            doc_uri: DOC,
            toks: &main.toks,
            ast: &main.ast,
            idxs: &idxs,
        };
        let hits = references(&ctx, at(&norm, "Widget.new", 1), false);
        let mut offs = ref_offsets(&norm, DOC, &hits);
        offs.sort();
        assert_eq!(
            offs,
            vec![at(&norm, "Widget }", 1), at(&norm, "Widget.new", 1)],
            "{hits:?}"
        );
    }

    #[test]
    fn refs_from_the_decl_side_reach_the_importing_doc() {
        let lib = widget_lib();
        let (main, norm) = widget_user();
        let extra = [&main.index];
        // query from the LIB's decl ident, doc = the lib: the scan's
        // reverse edge finds the importing doc's two sites — the only
        // two entries, both carrying the DOC's uri
        let idxs: Vec<&DefIndex> = std::iter::once(&lib.index).chain(extra.iter().copied()).collect();
        let ctx = definition::Ctx {
            doc_uri: WIDGET_URI,
            toks: &lib.toks,
            ast: &lib.ast,
            idxs: &idxs,
        };
        let hits = references(&ctx, at(&lib.src, "class Widget", 1) + 6, false);
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!(hits.iter().all(|l| l.uri == DOC), "{hits:?}");
        let mut offs = ref_offsets(&norm, DOC, &hits);
        offs.sort();
        assert_eq!(offs, vec![at(&norm, "Widget }", 1), at(&norm, "Widget.new", 1)]);
    }

    #[test]
    fn an_unrelated_same_named_type_does_not_leak_in() {
        let lib = widget_lib();
        let foreign = doc_at("file:///ws/other/lib.rut", "class Widget {\n    other: i32;\n}\n");
        let (main, norm) = widget_user();
        let extra = [&lib.index, &foreign.index];
        // references on the gadgets Widget (via the importing doc's use
        // site): the other pkg's same-named type must contribute nothing
        let idxs: Vec<&DefIndex> = std::iter::once(&main.index).chain(extra.iter().copied()).collect();
        let ctx = definition::Ctx {
            doc_uri: DOC,
            toks: &main.toks,
            ast: &main.ast,
            idxs: &idxs,
        };
        let hits = references(&ctx, at(&norm, "Widget.new", 1), false);
        assert_eq!(hits.len(), 2, "use name + usage only: {hits:?}");
    }

    // ---- misses ----

    #[test]
    fn misses_stay_empty() {
        let d = doc("fn main() -> nil {\n}\n");
        assert!(refs(&d, 0, &[], true).is_empty(), "keywords have no references");
        let d2 = doc("fn main() -> nil {\n    let x = mystery();\n}\n");
        assert!(refs(&d2, at(&d2.src, "mystery", 1), &[], true).is_empty(), "unknown names too");
    }
}
