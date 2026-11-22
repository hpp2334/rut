//! The views — facts derived from `(Doc, AnyHandle)` at read, one
//! file per view. Every fact derives from the node on read: the
//! verbatim text joins over the OWNING file's document, lines ride
//! `doc.line_of`, vis/ambient verdicts ride the modifier chain —
//! nothing stored, nothing stale. The slice helpers below are the
//! views' shared verbatim cuts (signature/head/decl spans, doc lines,
//! parameter names); they are private to this subtree.

mod fn_view;
mod let_view;
mod member_view;
mod mod_view;
mod ty_view;
mod use_view;

pub(crate) use fn_view::FnView;
pub(crate) use let_view::LetView;
pub(crate) use member_view::MemberView;
pub(crate) use mod_view::ModView;
pub(crate) use ty_view::TyView;
pub(crate) use use_view::{GlobUse, UseView};

use rut_ast::{Ast, DocData, Handle, ParamRef};

/// the doc lines a decl node carries, as rendered lines — the parse's
/// own graft (the Go-convention rules). Empty when nothing attaches.
fn doc_lines(src: &str, d: &DocData) -> Vec<String> {
    d.lines(src).map(|l| l.trim_start().to_string()).collect()
}

/// verbatim signature: the source up to (not including) the body; a
/// bodiless decl ends at its own `;` (the exact-span law) and the
/// trailing `;` is trimmed off the render
fn sig_src(
    src: &str,
    ast: &Ast,
    span: rut_lexer::span::Span,
    body: Option<Handle<rut_ast::BlockNode>>,
) -> String {
    let hi = match body {
        Some(b) => b.span(ast).lo,
        None => span.hi, // the item's own terminator — the `;`
    };
    src[span.lo as usize..hi as usize]
        .trim_end()
        .trim_end_matches(';')
        .trim_end()
        .to_string()
}

/// the verbatim decl slice through the item's own `;`, trimmed — the
/// shared cut of the `mod` edges and the alias head
fn decl_src(src: &str, span: rut_lexer::span::Span) -> String {
    src[span.lo as usize..span.hi as usize]
        .trim_end()
        .trim_end_matches(';')
        .trim_end()
        .to_string()
}

/// verbatim head: the source from the item's FIRST token (span law
/// v3.1 — modifiers included, generics and admission bounds as
/// written) up to the body's `{` — the brace pair's left edge, the
/// head slice's right cut
fn head_src(src: &str, span: rut_lexer::span::Span, body: rut_lexer::span::Span) -> String {
    let hi = if body.lo > span.lo { body.lo } else { span.hi };
    src[span.lo as usize..hi as usize]
        .trim_end()
        .trim_end_matches(';')
        .trim_end()
        .to_string()
}

/// declared parameter names in order, `self` excluded — the AST's own
/// data; the inlay hints' call-site parameter names price exact-arity
/// matches against this
fn param_names(ast: &Ast, params: &[ParamRef]) -> Vec<String> {
    params
        .iter()
        .filter_map(|p| match p {
            ParamRef::Param(d) => Some(ast.name(d.get(ast).name.ident).to_string()),
            _ => None, // `self` / `mut self`
        })
        .collect()
}
