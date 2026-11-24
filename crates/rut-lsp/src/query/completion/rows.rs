//! The completion row vocabulary — the tier rows ([`Row`],
//! [`CompletionRow`]) and the one wire conversion
//! ([`CompletionOut::of`]), plus the LSP mapping block every host
//! shares. One vocabulary, one allocation point.

use ls_types::{
    CompletionItem, CompletionItemKind, Documentation, MarkupContent, MarkupKind, Range, TextEdit,
};

use super::CompletionKind;
use crate::world::collection::{FnView, LetView, MemberView, TyView};
use crate::world::ty::{Ty, TyForm};

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
    pub(super) fn plain(
        label: String,
        detail: String,
        doc: Vec<String>,
        kind: CompletionKind,
    ) -> Self {
        CompletionOut {
            label,
            detail,
            doc,
            kind,
            sort_text: None,
            additional_text_edits: Vec::new(),
        }
    }
    /// THE conversion — a tier row into the wire item. Every tier
    /// assembles rows; this is the one place a label allocates.
    pub(super) fn of(row: &Row) -> Self {
        CompletionOut::plain(row.name().to_string(), row.detail(), row.docs(), row.kind())
    }
}

/// The wire's question set, asked of every tier's row — the name and
/// kind borrowed from the view's own node, the detail/docs rendered
/// verbatim from the spans. The tiers produce ROWS (views over the
/// decls they found); the ONLY conversion to the owned wire item is
/// [`CompletionOut::of`], at the ABI edge — one vocabulary, one
/// allocation point.
pub(super) trait CompletionRow {
    fn name(&self) -> &str;
    fn detail(&self) -> String;
    fn docs(&self) -> Vec<String>;
    fn kind(&self) -> CompletionKind;
}

/// A binding row — the rows carry interner ids, the wire spells them;
/// the detail rides the doc's own text join
pub(super) struct BindRow<'a> {
    pub b: &'a crate::world::Binding,
    pub doc: &'a crate::world::File,
    /// the resolver's declared-or-inferred answer (the tier's one
    /// resolver read per row)
    pub ty: Option<Ty>,
}

/// One tier row — the view the tier door found it from. The variants
/// are the tier views themselves; the wire item is one
/// [`CompletionOut::of`] away.
pub(super) enum Row<'a> {
    Ty(TyView<'a>),
    Fn(FnView<'a>),
    Member(MemberView<'a>),
    Let(LetView<'a>),
    Bind(BindRow<'a>),
    Kw(&'static str),
}

impl CompletionRow for Row<'_> {
    fn name(&self) -> &str {
        match self {
            Row::Ty(t) => t.name(),
            Row::Fn(f) => f.name(),
            Row::Member(m) => m.name(),
            Row::Let(l) => l.name(),
            Row::Bind(r) => r.doc.doc.ast().name(r.b.name),
            Row::Kw(k) => k,
        }
    }
    fn detail(&self) -> String {
        match self {
            Row::Ty(t) => t.head(),
            Row::Fn(f) => f.src(),
            Row::Member(m) => m.src(),
            Row::Let(l) => l.src(),
            Row::Bind(r) => match r.ty.clone() {
                Some(ty) => format!("{} — {ty}", r.b.kind.label()),
                None => r.b.kind.label().to_string(),
            },
            Row::Kw(_) => String::new(),
        }
    }
    fn docs(&self) -> Vec<String> {
        match self {
            Row::Ty(t) => t.doc_lines(),
            Row::Fn(f) => f.doc_lines(),
            Row::Member(m) => m.doc_lines(),
            Row::Let(l) => l.doc_lines(),
            Row::Bind(_) => Vec::new(),
            Row::Kw(_) => Vec::new(),
        }
    }
    fn kind(&self) -> CompletionKind {
        match self {
            Row::Ty(t) => match t.form() {
                TyForm::Interface => CompletionKind::Interface,
                _ => CompletionKind::Type,
            },
            Row::Fn(f) => {
                if f.owner().is_some() {
                    CompletionKind::Method
                } else {
                    CompletionKind::Fn
                }
            }
            Row::Member(m) => {
                if m.h.is::<rut_ast::MethodDeclNode>() {
                    CompletionKind::Method
                } else {
                    CompletionKind::Field
                }
            }
            Row::Let(_) => CompletionKind::Const,
            Row::Bind(r) => match r.b.kind {
                crate::world::BindKind::Param => CompletionKind::Parameter,
                crate::world::BindKind::Let | crate::world::BindKind::IterVar => {
                    CompletionKind::Variable
                }
            },
            Row::Kw(_) => CompletionKind::Keyword,
        }
    }
}

/// first definition wins — the document shadows the chain
/// the call site's prefix filter + the label sort — presentation over
/// a tier's rows (the doors enumerate unfiltered)
pub(super) fn filtered(mut out: Vec<CompletionOut>, prefix: &str) -> Vec<CompletionOut> {
    if !prefix.is_empty() {
        out.retain(|c| c.label.starts_with(prefix));
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

pub(super) fn push_item(out: &mut Vec<CompletionOut>, c: CompletionOut) {
    if !out.iter().any(|x| x.label == c.label) {
        out.push(c);
    }
}

// ---- the LSP mapping ----

/// plain completion data → the LSP item (kind icons, detail, docs,
/// the auto-import tier's sort text + additional edits) — the one
/// mapper every host shares
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
