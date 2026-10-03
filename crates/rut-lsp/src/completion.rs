//! Completions — member completion after `recv.` and bare-position
//! completion, over the same definition index hover uses. One rule for
//! the member list (mirrors the compiler's): **members of `T` = T's own
//! surface ∪ inherent `impl T { .. }` methods; on an interface `I` they
//! are exactly `I`'s declared signatures, use-gated when `I` is
//! foreign**. Heuristic, like hover: a
//! miss is an empty list, never wrong text.

use std::collections::HashSet;

use rut_ast::ast::Ast;
use rut_lexer::token::{Tok, Token};

use crate::hover::lookup::used_ifaces;
use crate::hover::types::{DefIndex, TyDef, TyForm};

/// One completion item — plain data; the server maps it to LSP types.
#[derive(Debug, Clone)]
pub struct CompletionOut {
    pub label: String,
    /// declaration text shown next to the label (a signature, a field)
    pub detail: String,
    /// markdown documentation lines (may be empty)
    pub doc: Vec<String>,
    pub kind: CompletionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    Keyword,
    Type,
    Interface,
    Fn,
    Method,
    Field,
}

/// Completions at `pos`: member items after `recv.` (filtered by the
/// typed prefix), otherwise the keyword table plus the visible decls.
pub fn complete(idxs: &[&DefIndex], toks: &[Token], ast: &Ast, pos: u32) -> Vec<CompletionOut> {
    let Some((recv, prefix)) = recv_before(toks, pos) else {
        let used = used_ifaces(ast);
        return bare_completions(idxs, &used);
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
                    CompletionOut {
                        label: m.name.clone(),
                        detail: m.src.clone(),
                        doc: m.doc.clone(),
                        kind: CompletionKind::Method,
                    },
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
            CompletionOut {
                label: f.name.clone(),
                detail: f.src.clone(),
                doc: f.doc.clone(),
                kind: CompletionKind::Field,
            },
        );
    }
    for m in &ty.methods {
        push_item(
            &mut out,
            CompletionOut {
                label: m.name.clone(),
                detail: m.src.clone(),
                doc: m.doc.clone(),
                kind: CompletionKind::Method,
            },
        );
    }
    // inherent impl-block methods — where methods live since type bodies
    // went fields-only; the owner string IS the target type's name
    for i in idxs {
        for f in &i.fns {
            if f.owner.as_deref() == Some(ty_name.as_str()) {
                push_item(
                    &mut out,
                    CompletionOut {
                        label: f.name.clone(),
                        detail: f.src.clone(),
                        doc: f.doc.clone(),
                        kind: CompletionKind::Method,
                    },
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
        out.push(CompletionOut {
            label: (*kw).to_string(),
            detail: String::new(),
            doc: Vec::new(),
            kind: CompletionKind::Keyword,
        });
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
                CompletionOut {
                    label: t.name.clone(),
                    detail: format!("{} {}{}", t.form.keyword(), t.name, gens(t)),
                    doc: t.doc.clone(),
                    kind: match t.form {
                        TyForm::Interface => CompletionKind::Interface,
                        _ => CompletionKind::Type,
                    },
                },
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
                CompletionOut {
                    label: f.name.clone(),
                    detail: f.src.clone(),
                    doc: f.doc.clone(),
                    kind: CompletionKind::Fn,
                },
            );
        }
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
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

use ls_types::{CompletionItem, CompletionItemKind, Documentation, MarkupContent, MarkupKind};

/// plain completion data → the LSP item (kind icons, detail, docs) —
/// shared by the stdio server and the wasm shim
pub fn lsp_item(c: CompletionOut) -> CompletionItem {
    CompletionItem {
        label: c.label,
        kind: Some(match c.kind {
            CompletionKind::Keyword => CompletionItemKind::KEYWORD,
            CompletionKind::Type => CompletionItemKind::CLASS,
            CompletionKind::Interface => CompletionItemKind::INTERFACE,
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
        complete(&idxs, &toks, &ast, pos)
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
            complete(&idxs, &toks, &ast, pos)
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
        assert!(labels(&complete(&idxs, &toks, &ast, pos)).contains(&"hi"), "own interface completes");
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
            complete(&idxs, &toks, &ast, pos)
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
        let items = complete(&idxs, &toks, &ast, pos);
        assert!(labels(&items).contains(&"len"), "{:?}", labels(&items));
    }
}
