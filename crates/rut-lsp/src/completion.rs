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

/// Completions at `pos`: use-path tiers inside a `use` statement,
/// member items after `recv.` (filtered by the typed prefix), otherwise
/// the keyword table plus the visible decls plus the auto-import tier.
/// `src` is the NORMALIZED source — the auto-import edits' coordinates
/// ride the same text the tokens were lexed from.
pub fn complete(idxs: &[&DefIndex], toks: &[Token], ast: &Ast, pos: u32, src: &str) -> Vec<CompletionOut> {
    if let Some((module, prefix)) = use_path_before(toks, pos) {
        return use_path_completions(idxs, module.as_deref(), &prefix);
    }
    let Some((recv, prefix)) = recv_before(toks, pos) else {
        let used = used_ifaces(ast);
        let mut out = bare_completions(idxs, &used);
        out.extend(auto_import_completions(idxs, ast, toks, pos, src));
        return out;
    };
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

/// the use-path context at `pos`: `(module, prefix)` when the tokens
/// before the cursor run back to a `use` keyword — `use ⏐` /
/// `use po⏐` (module `None`, the module-name tier) or
/// `use pouch::⏐` / `use pouch::V⏐` / `use pouch::{V⏐` (module
/// `Some("pouch")`, the names tier). The walk crosses idents, `::`,
/// braces, and commas only — any other token ends the path.
fn use_path_before(toks: &[Token], pos: u32) -> Option<(Option<String>, String)> {
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
    // a `::` anywhere turns the path into the names tier, pkg = first
    // ident; the prefix is a trailing partial ident
    let module = seg.iter().any(|t| t.tok == Tok::Colon).then(|| {
        seg.iter()
            .find_map(|t| match &t.tok {
                Tok::Ident(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_default()
    });
    let prefix = match seg.last() {
        Some(Token { tok: Tok::Ident(s), span }) if !rut_parser::is_reserved_kw(s) => {
            let end = (pos as usize).min(span.hi as usize).max(span.lo as usize);
            s.get(..end - span.lo as usize).unwrap_or(s).to_string()
        }
        _ => String::new(),
    };
    Some((module, prefix))
}

/// `use <cursor>`: the chain's module names — every NAMED index, the
/// std surface included (std is not special; the names come from the
/// uniform index set, no hardcoded table).
fn use_path_completions(idxs: &[&DefIndex], module: Option<&str>, prefix: &str) -> Vec<CompletionOut> {
    let Some(module) = module else {
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
    // `use mod::<cursor>`: the module's exported public names — pub
    // types, free fns, module lets. The import is the gate-opener, so
    // the `pub builtin` gate does not bind here (this IS the use that
    // names them)
    let chain: Vec<&DefIndex> = idxs.iter().copied().filter(|i| crate::definition::matches_pkg(i, module)).collect();
    let mut out: Vec<CompletionOut> = Vec::new();
    for i in &chain {
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
            push_item(
                &mut out,
                CompletionOut::plain(f.name.clone(), f.src.clone(), f.doc.clone(), CompletionKind::Fn),
            );
        }
        for l in &i.lets {
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
        complete(&idxs, &toks, &ast, pos, &s)
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
            complete(&idxs, &toks, &ast, pos, &d2)
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
        assert!(labels(&complete(&idxs, &toks, &ast, pos, &d2)).contains(&"hi"), "own interface completes");
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
            complete(&idxs, &toks, &ast, pos, &d2)
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
        let items = complete(&idxs, &toks, &ast, pos, &d2);
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
        complete(&chain, &toks, &ast, pos, &s)
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
        let items = complete(&idxs, &toks, &ast, s.len() as u32, &s);
        let ls = labels(&items);
        assert!(ls.contains(&"Math"), "the namespace head completes: {ls:?}");
        assert!(ls.contains(&"sqrt"), "the module's fns complete: {ls:?}");
        assert!(ls.contains(&"PI"), "the consts complete: {ls:?}");
        let math = items.iter().find(|c| c.label == "Math").unwrap();
        assert!(matches!(math.kind, CompletionKind::Module), "{:?}", math.kind);
        let pi = items.iter().find(|c| c.label == "PI").unwrap();
        assert!(matches!(pi.kind, CompletionKind::Const), "{:?}", pi.kind);
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
        let items = complete(&chain, &toks, &ast, pos, &s);
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
        let items = complete(&chain, &toks, &ast, pos, &s);
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
        let items = complete(&chain, &toks, &ast, pos, &s);
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
        let items = complete(&chain, &toks, &ast, pos, &s);
        // the name still completes (the bare tier), but NO import edit
        // rides it — the document already imports it
        assert!(
            import_item(&items, "Vec").is_none(),
            "the already-imported name must not re-offer: {items:?}"
        );
    }
}
