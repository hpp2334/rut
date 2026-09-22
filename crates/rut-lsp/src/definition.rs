//! Go-to-definition and go-to-type-definition — the lsp-features survey
//! §3.3 resolution order, one pure query shared by both faces:
//!
//! 1. **within-file binding layer** — locals/params/iter-vars: a LOOKUP
//!    against phase 1's binding pass (`decl_ident_span` is the target),
//!    shadow-aware and scope-exit correct;
//! 2. **within-file decl layer** — fields/enum members at their exact
//!    recovered name spans, module lets, fns/types/impl methods (token-
//!    recovered name idents), all recorded by the phase-0/1 index;
//! 3. **cross-file through the use graph** — a name the document imports
//!    (`use pouch::Vec;`) resolves only inside indexes whose origin
//!    matches the named pkg (path segment or file stem — how `rut.toml`
//!    `name` works, without re-implementing the manifest loader);
//!    un-imported names keep hover's flat-chain semantics;
//! 4. **stdlib** through the embedded surface — the `include_str!`
//!    origins carry the true `rut/...` path (`DefIndex::src_path`), so a
//!    jump lands in the real source file.
//!
//! Layering mirrors `hover` exactly (decl sites, then member position
//! with the known-receiver-is-final law, then type position, then the
//! binding pass, then the bare-name chain) so the two queries can't
//! disagree about what an ident means. Multiple hits come back as
//! multiple locations — the LSP-native candidate list (a peek), never a
//! wrong silent jump. A miss is an empty vector. Honest limits (the
//! survey §3.3): no visibility checking (the engine's `pub(mod)` rules
//! are not modeled); generics resolve at the head.

use std::collections::HashMap;

use ls_types::Range;
use rut_ast::ast::Ast;
use rut_lexer::span::Span;
use rut_lexer::token::{Tok, Token};

use crate::hover::lookup::{self, MemberHit, MemberTarget};
use crate::hover::types::head_of_ty;
use crate::hover::{bindings, DefIndex, FnDef, MemberSrc, TyDef};
use crate::line_index::LineIndex;

/// one resolved jump target — the uri is the doc itself, a workspace
/// path/URI, or (for the embedded std surface) the true repo-relative
/// source path; the client face resolves it against the workspace
#[derive(Debug, Clone, PartialEq)]
pub struct DefLocation {
    pub uri: String,
    pub range: Range,
}

/// everything one query needs — built per request over the normalized
/// source (the same `doc_ctx` pipeline hover rides)
pub struct Ctx<'a> {
    pub doc_uri: &'a str,
    pub toks: &'a [Token],
    pub ast: &'a Ast,
    pub doc: &'a DefIndex,
    pub extra: &'a [DefIndex],
}

impl Ctx<'_> {
    /// the lookup chain: the open document first, then std + workspace
    fn idxs(&self) -> Vec<&DefIndex> {
        std::iter::once(self.doc).chain(self.extra.iter()).collect()
    }
}

/// byte span -> LSP range over the TARGET index's own text
fn loc_at(doc_uri: &str, i: &DefIndex, span: Span) -> DefLocation {
    let uri = if i.origin == doc_uri {
        doc_uri.to_string()
    } else {
        i.src_path
            .clone()
            .unwrap_or_else(|| origin_uri(&i.origin))
    };
    DefLocation {
        uri,
        range: range_of(&i.lines, &i.src, span),
    }
}

/// byte span -> LSP range (the analysis layer's converter, inlined here
/// so the module stays face-agnostic)
fn range_of(index: &LineIndex, src: &str, span: Span) -> Range {
    let (l1, c1) = index.position(src, span.lo);
    let (l2, c2) = index.position(src, span.hi);
    Range {
        start: ls_types::Position { line: l1, character: c1 },
        end: ls_types::Position { line: l2, character: c2 },
    }
}

/// a target index's origin as a URI — workspace origins are already
/// paths or `file://` URIs; the std surface's provenance labels never
/// become jump targets (they carry `src_path` instead)
fn origin_uri(origin: &str) -> String {
    if origin.contains("://") {
        origin.to_string()
    } else if origin.starts_with('/') {
        format!("file://{origin}")
    } else {
        origin.to_string()
    }
}

/// does this index speak for `pkg`? — exact origin (the std surface's
/// label), a path segment named `pkg` (…/pouch/pouch.rut), or the file
/// stem (…/gadgets/lib.rut for pkg `gadgets`)
fn matches_pkg(i: &DefIndex, pkg: &str) -> bool {
    let hay = i.src_path.as_deref().unwrap_or(&i.origin);
    if hay == pkg {
        return true;
    }
    hay.split(['/', '\\'])
        .any(|seg| seg.strip_suffix(".rut").unwrap_or(seg) == pkg)
}

/// the document's import map: imported name -> its `use` pkg
fn import_map(doc: &DefIndex) -> HashMap<&str, &str> {
    doc.uses
        .iter()
        .map(|u| (u.name.as_str(), u.pkg.as_str()))
        .collect()
}

/// the chain restricted to the pkg that declares `name` — the use
/// graph's gate. An import the chain cannot satisfy falls back to the
/// flat chain (hover's semantics: better a broad answer than none)
fn chain_for<'a>(
    idxs: &[&'a DefIndex],
    imported: &HashMap<&str, &str>,
    name: &str,
) -> Vec<&'a DefIndex> {
    match imported.get(name) {
        Some(pkg) => {
            let filtered: Vec<&DefIndex> =
                idxs.iter().copied().filter(|i| matches_pkg(i, pkg)).collect();
            if filtered.is_empty() {
                idxs.to_vec()
            } else {
                filtered
            }
        }
        None => idxs.to_vec(),
    }
}

/// a type's definition target span — the declaring ident (token
/// recovery at build time); the whole decl is the fallback when
/// recovery failed on a degenerate parse
fn ty_span(t: &TyDef) -> Span {
    t.name_span.unwrap_or(t.span)
}

fn fn_span(f: &FnDef) -> Span {
    f.name_span.unwrap_or(f.span)
}

fn member_span(m: &MemberSrc) -> Option<Span> {
    m.name_span
}

// ---- definition ----

/// `textDocument/definition` — the ident at `pos` to its declaration.
/// Empty when nothing resolves (keywords, unknown names, primitives
/// with no surface decl).
pub fn definition(ctx: &Ctx, pos: u32) -> Vec<DefLocation> {
    let Some(t) = lookup::tok_at(ctx.toks, pos) else { return Vec::new() };
    let Tok::Ident(name) = &t.tok else { return Vec::new() };
    if crate::semantic::is_keyword(name) {
        return Vec::new();
    }
    let idxs = ctx.idxs();
    let imported = import_map(ctx.doc);

    // 1. decl sites — the hovered span EXACTLY equals a recorded name
    //    span (field / enum member / module let): the declaration IS
    //    the answer (go-to-def on a decl → itself)
    if decl_site(ctx.doc, t.span, name) {
        return vec![DefLocation {
            uri: ctx.doc_uri.to_string(),
            range: range_of(&ctx.doc.lines, &ctx.doc.src, t.span),
        }];
    }

    // 2. use-statement names — the survey's use-graph edge: ctrl+click
    //    on `Vec` inside `use pouch::Vec;` jumps to the exporting
    //    module's decl, strictly inside the named pkg
    if let Some(u) = ctx
        .doc
        .uses
        .iter()
        .find(|u| u.name == *name && u.name_span == Some(t.span))
    {
        return use_graph_targets(&idxs, &u.pkg, name)
            .into_iter()
            .map(|(i, sp)| loc_at(ctx.doc_uri, i, sp))
            .collect();
    }

    let binds = bindings::collect(ctx.ast, ctx.toks, &idxs);

    // decl sites of the binding layer — the hovered ident IS a recorded
    // declaring ident (a param's, or a let the decl_site spans missed):
    // go-to-def on a decl → itself. Params sit before their scope body,
    // so the scope-containment resolve can't see them
    if binds.iter().any(|b| b.name == *name && b.decl_ident_span == t.span) {
        return vec![DefLocation {
            uri: ctx.doc_uri.to_string(),
            range: range_of(&ctx.doc.lines, &ctx.doc.src, t.span),
        }];
    }

    // 3. member position — `recv.member` resolves through phase 1's
    //    receiver inference (params, lets, field reads, chained calls,
    //    for-of vars); a resolved receiver never falls through (the
    //    use-gate law hover established), an unknown one keeps hover's
    //    bare-name fallback
    if let Some(recv) = lookup::member_context(ctx.toks, t) {
        match lookup::member_target(&idxs, ctx.ast, &binds, pos, name, &recv) {
            MemberHit::Found(target) => {
                return member_locations(ctx, &target);
            }
            MemberHit::None => return Vec::new(),
            MemberHit::UnknownReceiver => {} // fall through to the name layers
        }
    }

    // 4. type position — capitalized / primitive names, restricted to
    //    the use graph for imported names
    if crate::hover::infer::is_cap(name) || rut_parser::is_primitive_ty(name) {
        if let Some(locs) = ty_name_locations(ctx, &idxs, &imported, name) {
            return locs;
        }
        // a primitive token with no surface decl (i32, bool …) is a
        // miss — never a wrong jump
        if rut_parser::is_primitive_ty(name) {
            return Vec::new();
        }
    }

    // 5. the binding layer — locals/params/iter-vars, shadow-correct;
    //    the declaring ident span is phase 1's span-first contract
    if let Some(b) = bindings::resolve(&binds, pos, name) {
        return vec![DefLocation {
            uri: ctx.doc_uri.to_string(),
            range: range_of(&ctx.doc.lines, &ctx.doc.src, b.decl_ident_span),
        }];
    }

    // 6. module-let use sites — every hit (the candidate list)
    let chain = chain_for(&idxs, &imported, name);
    let mut lets: Vec<DefLocation> = Vec::new();
    for i in &chain {
        for l in i.lets.iter().filter(|l| l.name == *name) {
            lets.push(loc_at(ctx.doc_uri, i, l.name_span.unwrap_or(l.span)));
        }
    }
    if !lets.is_empty() {
        return lets;
    }

    // 7. fn names — free fns and impl methods by name; ambiguity
    //    returns every hit (a peek, not a guess)
    let mut fns: Vec<DefLocation> = Vec::new();
    for i in &chain {
        for f in i.fns.iter().filter(|f| f.name == *name) {
            fns.push(loc_at(ctx.doc_uri, i, fn_span(f)));
        }
    }
    fns
}

/// `textDocument/typeDefinition` — the survey prices this cheap and it
/// is: resolve the expression's type head (the phase-1 rules — member
/// declared types, binding types, module lets), then jump to that
/// type's declaration. The same layers, one step further.
pub fn type_definition(ctx: &Ctx, pos: u32) -> Vec<DefLocation> {
    let Some(t) = lookup::tok_at(ctx.toks, pos) else { return Vec::new() };
    let Tok::Ident(name) = &t.tok else { return Vec::new() };
    if crate::semantic::is_keyword(name) {
        return Vec::new();
    }
    let idxs = ctx.idxs();
    let imported = import_map(ctx.doc);
    let binds = bindings::collect(ctx.ast, ctx.toks, &idxs);

    // member position: the member's DECLARED type — `w.item` (a
    // `?Circle` field) jumps to the `Circle` decl; a resolved receiver
    // is final (the use-gate law)
    if let Some(recv) = lookup::member_context(ctx.toks, t) {
        match lookup::member_target(&idxs, ctx.ast, &binds, pos, name, &recv) {
            MemberHit::Found(target) => {
                let ty_text = match &target {
                    MemberTarget::Member(_, _, m, _)
                    | MemberTarget::EnumMember(_, _, m)
                    | MemberTarget::TraitMember(_, _, m, _) => m.ty.clone(),
                    MemberTarget::ImplFn(_, f) => f.ret.clone(),
                };
                return ty_text
                    .as_deref()
                    .map(head_of_ty)
                    .and_then(|head| ty_name_locations(ctx, &idxs, &imported, &head))
                    .unwrap_or_default();
            }
            MemberHit::None => return Vec::new(),
            MemberHit::UnknownReceiver => {} // fall through
        }
    }

    // a type token: its own declaration (definition's answer)
    if crate::hover::infer::is_cap(name) || rut_parser::is_primitive_ty(name) {
        if let Some(locs) = ty_name_locations(ctx, &idxs, &imported, name) {
            return locs;
        }
        if rut_parser::is_primitive_ty(name) {
            return Vec::new();
        }
    }

    // locals / module lets — their type head's declaration
    if let Some(b) = bindings::resolve(&binds, pos, name) {
        return b
            .ty_head()
            .and_then(|head| ty_name_locations(ctx, &idxs, &imported, &head))
            .unwrap_or_default();
    }
    match crate::hover::infer::module_let_ty(&idxs, name) {
        Some(head) => ty_name_locations(ctx, &idxs, &imported, &head).unwrap_or_default(),
        None => Vec::new(),
    }
}

/// a type head's declaration location(s) — the use graph restricts the
/// search; `None` when the name has no indexed decl anywhere (a bare
/// miss, kept distinct from an empty hit list)
fn ty_name_locations(
    ctx: &Ctx,
    idxs: &[&DefIndex],
    imported: &HashMap<&str, &str>,
    name: &str,
) -> Option<Vec<DefLocation>> {
    let chain = chain_for(idxs, imported, name);
    let hits: Vec<(&DefIndex, &TyDef)> = chain
        .iter()
        .filter_map(|i| i.ty(name).map(|t| (*i, t)))
        .collect();
    match hits.as_slice() {
        [] => None,
        _ => Some(
            hits.iter()
                .map(|(i, t)| loc_at(ctx.doc_uri, i, ty_span(t)))
                .collect(),
        ),
    }
}

/// where an imported name declares — strictly inside the named pkg's
/// indexes: types, then module lets, then free fns
fn use_graph_targets<'a>(
    idxs: &[&'a DefIndex],
    pkg: &str,
    name: &str,
) -> Vec<(&'a DefIndex, Span)> {
    let mut out: Vec<(&DefIndex, Span)> = Vec::new();
    for i in idxs.iter().filter(|i| matches_pkg(i, pkg)) {
        if let Some(t) = i.ty(name) {
            out.push((i, ty_span(t)));
        }
        if let Some(l) = i.lets.iter().find(|l| l.name == name) {
            out.push((i, l.name_span.unwrap_or(l.span)));
        }
        for f in i.fns.iter().filter(|f| f.name == name && f.owner.is_none()) {
            out.push((i, fn_span(f)));
        }
    }
    out
}

fn decl_site(doc: &DefIndex, span: Span, name: &str) -> bool {
    doc.types.iter().any(|t| {
        t.fields
            .iter()
            .any(|f| f.name == name && f.name_span == Some(span))
    }) || doc
        .lets
        .iter()
        .any(|l| l.name == name && l.name_span == Some(span))
}

fn member_locations(ctx: &Ctx, target: &MemberTarget) -> Vec<DefLocation> {
    match target {
        MemberTarget::Member(i, _, m, _)
        | MemberTarget::EnumMember(i, _, m)
        | MemberTarget::TraitMember(i, _, m, _) => match member_span(m) {
            Some(sp) => vec![loc_at(ctx.doc_uri, i, sp)],
            // a member whose name recovery failed has no span to jump
            // to — an empty answer, never a wrong jump
            None => Vec::new(),
        },
        MemberTarget::ImplFn(i, f) => vec![loc_at(ctx.doc_uri, i, fn_span(f))],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rut_lexer::lexer::{lex, normalize};
    use rut_parser::{parse, Mode};

    const DOC: &str = "file:///t/main.rut";

    struct Doc {
        src: String,
        toks: Vec<Token>,
        ast: Ast,
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

    fn ctx<'a>(d: &'a Doc, extra: &'a [DefIndex]) -> Ctx<'a> {
        Ctx { doc_uri: DOC, toks: &d.toks, ast: &d.ast, doc: &d.index, extra }
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

    /// the byte span a DefLocation points at, over `src`
    fn span_of(src: &str, ix: &LineIndex, l: &DefLocation) -> (u32, u32) {
        (
            ix.byte(src, l.range.start.line, l.range.start.character),
            ix.byte(src, l.range.end.line, l.range.end.character),
        )
    }

    fn def(d: &Doc, pos: u32, extra: &[DefIndex]) -> Vec<DefLocation> {
        definition(&ctx(d, extra), pos)
    }

    fn typedef(d: &Doc, pos: u32, extra: &[DefIndex]) -> Vec<DefLocation> {
        type_definition(&ctx(d, extra), pos)
    }

    const WIDGET_DOC: &str = "class Widget {\n    id: i32;\n}\n";
    const WIDGET_URI: &str = "file:///ws/gadgets/lib.rut";

    fn widget_index() -> DefIndex {
        doc_at(WIDGET_URI, WIDGET_DOC).index
    }

    // ---- layer 1: the binding pass ----

    #[test]
    fn local_binding_jumps_to_declaring_ident() {
        let src = "fn go() -> i32 {\n    let c = 1;\n    return c;\n}\n";
        let d = doc(src);
        let hits = def(&d, at(&d.src, "return c;", 1) + 7, &[]);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].uri, DOC);
        let ix = LineIndex::new(&d.src);
        let want = at(&d.src, "let c", 1) + 4; // the `c` inside `let c`
        assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 1));
    }

    #[test]
    fn shadowing_picks_the_latest_decl() {
        // rut has no bare-block statements — if blocks are the scope form
        let src = [
            "fn go() -> i32 {",
            "    let v = 1;",
            "    if (v > 0) {",
            "        let v = 2;",
            "        return v;",
            "    }",
            "    return 0;",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        let hits = def(&d, at(&d.src, "return v;", 1) + 7, &[]);
        assert_eq!(hits.len(), 1, "{hits:?}");
        let ix = LineIndex::new(&d.src);
        let want = at(&d.src, "let v = 2;", 1) + 4; // the INNER `v`
        assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 1));
    }

    #[test]
    fn scope_exit_keeps_the_outer_binding() {
        let src = [
            "fn go() -> i32 {",
            "    let v = 1;",
            "    if (v > 0) {",
            "        let v = 2;",
            "    }",
            "    return v;",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        let hits = def(&d, at(&d.src, "return v;", 1) + 7, &[]);
        assert_eq!(hits.len(), 1, "{hits:?}");
        let ix = LineIndex::new(&d.src);
        let want = at(&d.src, "let v = 1;", 1) + 4; // the OUTER `v` — the inner one is out of scope
        assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 1));
    }

    #[test]
    fn param_jumps_to_its_decl() {
        let src = "fn go(p: Point) -> f64 {\n    return p.x;\n}\nstruct Point {\n    x: f64;\n}\n";
        let d = doc(&src);
        let hits = def(&d, at(&d.src, "p: Point", 1), &[]); // the param decl site → itself
        assert_eq!(hits.len(), 1);
        let hits = def(&d, at(&d.src, "return p", 1) + 7, &[]); // the use site
        let ix = LineIndex::new(&d.src);
        let want = at(&d.src, "go(p: Point)", 1) + 3;
        assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 1));
    }

    // ---- layer 2: the decl layer ----

    #[test]
    fn field_read_jumps_to_the_field_decl() {
        let src = [
            "class Circle {",
            "    r: f64;",
            "}",
            "struct Wrap {",
            "    c: Circle;",
            "}",
            "fn go(wrap: Wrap) -> f64 {",
            "    let inner = wrap.c;",
            "    return inner.r;",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        // `wrap.c` — the `c` after the dot → the field decl ident
        let hits = def(&d, at(&d.src, "wrap.c;", 1) + 5, &[]);
        assert_eq!(hits.len(), 1, "{hits:?}");
        let ix = LineIndex::new(&d.src);
        let want = at(&d.src, "c: Circle;", 1); // the declaring `c`
        assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 1));
        // the field DECL site answers itself
        let hits = def(&d, at(&d.src, "c: Circle;", 1), &[]);
        assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 1));
    }

    #[test]
    fn method_call_jumps_to_the_impl_fn_name() {
        let src = [
            "class Circle {",
            "    r: f64;",
            "}",
            "impl Circle {",
            "    fn area(self) -> f64 { return 3.14; }",
            "}",
            "fn go() -> f64 {",
            "    let c = Circle.new(1.0);",
            "    return c.area();",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        let hits = def(&d, at(&d.src, "c.area();", 1) + 2, &[]);
        assert_eq!(hits.len(), 1, "{hits:?}");
        let ix = LineIndex::new(&d.src);
        let want = at(&d.src, "fn area(self)", 1) + 3;
        assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 4));
    }

    #[test]
    fn enum_member_jumps_to_its_decl_ident() {
        let src = "enum Color { Red, Blue }\nfn pick(c: Color) -> Color {\n    return c;\n}\nfn go() -> Color {\n    return Color.Red;\n}\n";
        let d = doc(&src);
        let hits = def(&d, at(&d.src, "Color.Red;", 1) + 6, &[]);
        assert_eq!(hits.len(), 1, "{hits:?}");
        let ix = LineIndex::new(&d.src);
        let want = at(&d.src, "Red,", 1);
        assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 3));
    }

    #[test]
    fn type_and_fn_names_jump_to_their_decls() {
        let src = [
            "class Circle {",
            "    r: f64;",
            "}",
            "fn make() -> Circle {",
            "    return Circle.new(1.0);",
            "}",
            "fn main() -> nil {",
            "    let c = make();",
            "    let d = make();",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        // the type token in `Circle.new` → the class decl ident
        let hits = def(&d, at(&d.src, "Circle.new(1.0);", 1), &[]);
        assert_eq!(hits.len(), 1, "{hits:?}");
        let ix = LineIndex::new(&d.src);
        let want = at(&d.src, "class Circle", 1) + 6;
        assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 6));
        // a free-fn call site → the fn decl ident
        let hits = def(&d, at(&d.src, "= make();", 2) + 2, &[]);
        assert_eq!(hits.len(), 1, "{hits:?}");
        let want = at(&d.src, "fn make()", 1) + 3;
        assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 4));
    }

    #[test]
    fn module_let_use_site_jumps_to_the_decl() {
        let src = "let TOTAL = 41;\nfn bump() -> i32 {\n    return TOTAL + 1;\n}\n";
        let d = doc(&src);
        let hits = def(&d, at(&d.src, "return TOTAL", 1) + 7, &[]);
        assert_eq!(hits.len(), 1, "{hits:?}");
        let ix = LineIndex::new(&d.src);
        let want = at(&d.src, "let TOTAL", 1) + 4;
        assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 5));
    }

    // ---- layer 3: the use graph ----

    #[test]
    fn use_name_jumps_to_the_exporting_module() {
        let src = "use gadgets::{ Widget };\nfn main() -> nil {\n    let w = Widget.new();\n}\n";
        let d = doc(&src);
        let gadgets = widget_index();
        let extra = [gadgets];
        // the use-statement name itself
        let hits = def(&d, at(&d.src, "use gadgets::{ Widget }", 1) + 15, &extra);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].uri, WIDGET_URI);
        let ix = LineIndex::new(&normalize(WIDGET_DOC));
        let want = at(&normalize(WIDGET_DOC), "class Widget", 1) + 6;
        assert_eq!(span_of(&normalize(WIDGET_DOC), &ix, &hits[0]), (want, want + 6));
        // and a usage site resolves through the same edge
        let hits = def(&d, at(&d.src, "= Widget.new();", 1) + 2, &extra);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].uri, WIDGET_URI);
    }

    #[test]
    fn the_use_graph_restricts_the_search_to_the_imported_pkg() {
        let src = "use gadgets::{ Widget };\nfn main() -> nil {\n    let w = Widget.new();\n}\n";
        let d = doc(&src);
        let mut foreign = doc("class Widget {\n    other: i32;\n}\n").index;
        foreign.origin = "file:///ws/other/lib.rut".to_string();
        let extra = [widget_index(), foreign];
        let hits = def(&d, at(&d.src, "= Widget.new();", 1) + 2, &extra);
        assert_eq!(hits.len(), 1, "ambiguity from the OTHER pkg must not leak: {hits:?}");
        assert_eq!(hits[0].uri, WIDGET_URI);
    }

    #[test]
    fn unimported_names_keep_the_flat_chain() {
        let src = "fn main() -> nil {\n    let w = Widget.new();\n}\n";
        let d = doc(&src);
        let extra = [widget_index()];
        let hits = def(&d, at(&d.src, "= Widget.new();", 1) + 2, &extra);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].uri, WIDGET_URI);
    }

    // ---- layer 4: the stdlib surface ----

    #[test]
    fn std_use_jumps_to_the_true_source_path() {
        let src = "use pouch::{ Vec };\nfn main() -> nil {\n    let v = Vec.new();\n}\n";
        let d = doc(&src);
        let extra = crate::std_surface::indexes();
        let hits = def(&d, at(&d.src, "= Vec.new();", 1) + 2, &extra);
        assert_eq!(hits.len(), 1, "{hits:?}");
        // the TRUE rut/... path, not a label
        assert_eq!(hits[0].uri, "rut/pouch/pouch.rut");
        // the reported range is the declaring `Vec` ident in the REAL
        // source file (normalize the embedded const like the index did)
        let real = normalize(crate::std_surface::POUCH);
        let ix = LineIndex::new(&real);
        let want = at(&real, "class Vec", 1) + 6;
        assert_eq!(span_of(&real, &ix, &hits[0]), (want, want + 3));
        // the use-statement name resolves to the same target
        let hits = def(&d, at(&d.src, "use pouch::{ Vec }", 1) + 13, &extra);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].uri, "rut/pouch/pouch.rut");
        assert_eq!(span_of(&real, &ix, &hits[0]), (want, want + 3));
    }

    // ---- typeDefinition ----

    #[test]
    fn type_definition_from_a_binding() {
        let src = [
            "class Circle {",
            "    r: f64;",
            "}",
            "fn go() -> f64 {",
            "    let c = Circle.new(1.0);",
            "    return c.r;",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        let hits = typedef(&d, at(&d.src, "return c.r;", 1) + 7, &[]);
        assert_eq!(hits.len(), 1, "{hits:?}");
        let ix = LineIndex::new(&d.src);
        let want = at(&d.src, "class Circle", 1) + 6;
        assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 6));
    }

    #[test]
    fn type_definition_from_a_nullable_field_member() {
        let src = [
            "class Circle {",
            "    r: f64;",
            "}",
            "struct Slot {",
            "    item: ?Circle;",
            "}",
            "fn go(s: Slot) -> f64 {",
            "    return s.item.r;",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        // `s.item` — the `?Circle` field's type head is `Circle`
        let hits = typedef(&d, at(&d.src, "s.item.r;", 1) + 2, &[]);
        assert_eq!(hits.len(), 1, "{hits:?}");
        let ix = LineIndex::new(&d.src);
        let want = at(&d.src, "class Circle", 1) + 6;
        assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 6));
    }

    #[test]
    fn type_definition_on_a_type_token_is_its_own_decl() {
        let src = "class Circle {\n    r: f64;\n}\nfn go() -> f64 {\n    let c = Circle.new(1.0);\n    return c.r;\n}\n";
        let d = doc(&src);
        let hits = typedef(&d, at(&d.src, "Circle.new(1.0);", 1), &[]);
        assert_eq!(hits.len(), 1, "{hits:?}");
        let ix = LineIndex::new(&d.src);
        let want = at(&d.src, "class Circle", 1) + 6;
        assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 6));
    }

    #[test]
    fn misses_stay_misses() {
        let src = "fn main() -> nil {\n}\n";
        let d = doc(src);
        // keywords, unknown names, and decl-less primitives are empty
        assert!(def(&d, 0, &[]).is_empty()); // `fn` keyword
        let src2 = "fn main() -> nil {\n    let x = i32.max(1);\n}\n";
        let d2 = doc(src2);
        assert!(def(&d2, at(&d2.src, "i32.max", 1), &[]).is_empty(), "i32 has no decl to jump to");
        let src3 = "fn main() -> nil {\n    let x = mystery();\n}\n";
        let d3 = doc(src3);
        assert!(def(&d3, at(&d3.src, "mystery", 1), &[]).is_empty());
    }
}
