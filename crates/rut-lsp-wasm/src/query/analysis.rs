//! Per-document analysis — one pure function over the text: normalize →
//! lex (token source) → parse (AST + diags) → classify / symbols → LSP
//! values. The server layer only routes URIs and publishes.

use ls_types::*;
use rut_ast::ast::Ast;
use rut_semantic::{Diag, LineIndex, Span, Token};
use std::str::FromStr;

use crate::completion;
use crate::definition;
use crate::hover;
use crate::inlay;
use crate::references;
use crate::signature_help;
use rut_semantic::{RawSymbol, SymKind, TokenType};

pub struct Analysis {
    pub diags: Vec<Diagnostic>,
    pub tokens: Vec<SemanticToken>,
    pub symbols: Vec<DocumentSymbol>,
    /// the hover definition index (origin patched by the caller —
    /// `analyze_at` stamps the URI)
    pub index: hover::DefIndex,
}

/// moved to `rut-semantic` (the document layer owns the filename
/// convention); re-exported so the internal callers keep their spelling
pub use rut_semantic::mode_of;

/// one ParsedDoc → the LSP `Analysis` envelope (no URI wiring — the
/// stamped twin below owns that)
fn analysis_of(doc: &rut_semantic::ParsedDoc) -> Analysis {
    let src = doc.src();
    Analysis {
        diags: doc
            .diags
            .iter()
            .map(|d| to_diagnostic(d, src, &doc.lines))
            .collect(),
        tokens: encode(&doc.spans, src, &doc.lines),
        symbols: to_document_symbols(doc.symbols.clone(), src, &doc.lines),
        index: hover::index(src, &doc.ast, &doc.toks),
    }
}

/// the degenerate-URI fallback (a uri that fails to parse as `Uri`
/// still analyzes — origin stamped from the raw string, no related-info
/// wiring)
pub fn analyze_unstamped(uri: &str, src: &str) -> Analysis {
    let mut a = analysis_of(&rut_semantic::ParsedDoc::parse(uri, src));
    a.index.origin = uri.to_string();
    a
}

// ---- spans -> LSP ranges ----

pub fn range_of(index: &LineIndex, src: &str, span: Span) -> Range {
    let (l1, c1) = index.position(src, span.lo);
    let (l2, c2) = index.position(src, span.hi);
    Range {
        start: Position {
            line: l1,
            character: c1,
        },
        end: Position {
            line: l2,
            character: c2,
        },
    }
}

// ---- semantic tokens -> the delta encoding ----

/// `spans` must be sorted and non-overlapping (the classifier's contract).
/// Lengths and columns are UTF-16 code units — LSP's position currency.
pub fn encode(spans: &[(Span, TokenType)], src: &str, index: &LineIndex) -> Vec<SemanticToken> {
    let mut out = Vec::with_capacity(spans.len());
    let (mut prev_line, mut prev_char) = (0u32, 0u32);
    for (sp, ty) in spans {
        let (line, ch) = index.position(src, sp.lo);
        let len: u32 = src[sp.lo as usize..sp.hi as usize]
            .chars()
            .map(|c| c.len_utf16() as u32)
            .sum();
        let delta_line = line - prev_line;
        let delta_char = if delta_line == 0 { ch - prev_char } else { ch };
        out.push(SemanticToken {
            delta_line,
            delta_start: delta_char,
            length: len,
            token_type: ty.index(),
            token_modifiers_bitset: 0,
        });
        prev_line = line;
        prev_char = ch;
    }
    out
}

// ---- diagnostics ----

pub(crate) fn to_diagnostic(d: &Diag, src: &str, index: &LineIndex) -> Diagnostic {
    let mut message = d.msg.clone();
    for n in &d.notes {
        message.push_str("\n\nnote: ");
        message.push_str(n);
    }
    let related: Vec<DiagnosticRelatedInformation> = d
        .labels
        .iter()
        .map(|(sp, label)| DiagnosticRelatedInformation {
            location: Location {
                uri: Uri::from_str("file:///").unwrap(), // patched by the server layer
                range: range_of(index, src, *sp),
            },
            message: label.clone(),
        })
        .collect();
    Diagnostic {
        range: range_of(index, src, d.span),
        severity: Some(DiagnosticSeverity::ERROR),
        code: None,
        code_description: None,
        source: Some("rut".to_string()),
        message,
        related_information: if related.is_empty() {
            None
        } else {
            Some(related)
        },
        tags: None,
        data: None,
    }
}

/// Patch the (placeholder) related-info URIs after the fact — `analyze` is
/// URI-agnostic; the server entry point below knows the document.
pub(crate) fn set_related_uri(diags: &mut [Diagnostic], uri: &Uri) {
    for d in diags {
        if let Some(rel) = &mut d.related_information {
            for r in rel.iter_mut() {
                r.location.uri = uri.clone();
            }
        }
    }
}

/// Merge two diagnostic lists for one document, dropping TRUE
/// duplicates — same range AND same message (the check lane re-parses
/// the document, so every parse error arrives again). Distinct sites
/// are never collapsed: the checker reports multiple sites per body
/// (the B3 emission law), and each carries its own range.
pub fn merge_diags(base: Vec<Diagnostic>, extra: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let mut out = base;
    for d in extra {
        let dup = out
            .iter()
            .any(|e| e.range == d.range && e.message == d.message);
        if !dup {
            out.push(d);
        }
    }
    out
}

/// The server's entry point: `analyze` + URI wiring for related info
/// and hover provenance.
pub fn analyze_at(uri: &Uri, src: &str) -> Analysis {
    let mut a = analysis_of(&rut_semantic::ParsedDoc::parse(uri.as_str(), src));
    set_related_uri(&mut a.diags, uri);
    a.index.origin = uri.as_str().to_string();
    a
}

// ---- document-level queries (the hosts' entry points) ----
//
// The wasm-bindgen bodies in lib.rs and the tests run the *same*
// hover / completion queries over the same indexes — defined here
// once, so no host can grow its own dialect.

/// per-query pipeline: normalize → lex → parse → the document's index
/// (origin stamped). The queries below only add position + lookup.
fn doc_ctx(uri: &str, src: &str) -> (String, Vec<Token>, Ast, hover::DefIndex) {
    // ONE derivation per query: the facade parses (mode derived from
    // the uri) and the hover index builds over its parts — the old
    // shape parsed the same text twice (its own parse + index_at's)
    let d = rut_semantic::ParsedDoc::parse(uri, src);
    let mut idx = hover::index(d.src(), &d.ast, &d.toks);
    idx.origin = uri.to_string();
    let rut_semantic::ParsedDoc { ast, toks, .. } = d;
    // one String clone per query — the consumers want the text and the
    // tree as separate values (the parse itself dwarfs this)
    (ast.src.clone(), toks, ast, idx)
}

/// the definition index for one document (origin stamped with `uri`)
pub fn index_at(uri: &str, src: &str) -> hover::DefIndex {
    let d = rut_semantic::ParsedDoc::parse(uri, src);
    let mut idx = hover::index(d.src(), &d.ast, &d.toks);
    idx.origin = uri.to_string();
    idx
}

/// the lookup chain — the open document first, then `extra` (the std
/// surface + the workspace)
fn doc_idxs<'a>(
    doc: &'a hover::DefIndex,
    extra: &'a [hover::DefIndex],
) -> Vec<&'a hover::DefIndex> {
    std::iter::once(doc).chain(extra.iter()).collect()
}

/// byte offset of an LSP position over the normalized source
fn byte_at(normalized: &str, line: u32, ch: u32) -> u32 {
    LineIndex::new(normalized).byte(normalized, line, ch)
}

/// hover at an LSP position: the document's index first, then `extra`.
/// Returns the full LSP value — hosts hand it to their client as-is.
/// `doc` is the open document's file-module context (the face's
/// knowledge) — an own-package use-path segment hover resolves through
/// it; the default keeps the cross-package lanes only.
pub fn hover_at(
    uri: &str,
    src: &str,
    extra: &[hover::DefIndex],
    dm: &crate::mods::DocMods,
    line: u32,
    ch: u32,
) -> Option<Hover> {
    let (normalized, toks, ast, doc) = doc_ctx(uri, src);
    let idxs = doc_idxs(&doc, extra);
    let h = hover::hover(&idxs, &toks, &ast, dm, byte_at(&normalized, line, ch))?;
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: h.markdown,
        }),
        range: Some(range_of(&LineIndex::new(&normalized), &normalized, h.span)),
    })
}

/// completions at an LSP position: the document's index first, then
/// `extra`. Returns full LSP items. `doc` is the open document's
/// file-module context (`crate::mods::DocMods`) — the face's knowledge
/// of where the file sits in its package's mod tree; the default
/// (unknown) keeps the position-path tier off and changes nothing
/// else.
pub fn complete_at(
    uri: &str,
    src: &str,
    extra: &[hover::DefIndex],
    line: u32,
    ch: u32,
    doc: &crate::mods::DocMods,
) -> Vec<CompletionItem> {
    let (normalized, toks, ast, doc_index) = doc_ctx(uri, src);
    let idxs = doc_idxs(&doc_index, extra);
    completion::complete(
        &idxs,
        &toks,
        &ast,
        byte_at(&normalized, line, ch),
        &normalized,
        doc,
    )
    .into_iter()
    .map(completion::lsp_item)
    .collect()
}

/// definitions at an LSP position: the survey §3.3 layers (binding pass
/// → decl layer → use graph → std surface). Empty = nothing resolves;
/// multiple entries = the candidate list. Targets carry the doc URI, a
/// workspace origin, or the std surface's true `rut/...` source path —
/// the face resolves them (server: against the workspace root; wasm
/// shim: the extension does). `doc` is the open document's file-module
/// context — the mod-edge clicks and the own-package use lanes key on
/// it; the default keeps them off.
pub fn definition_at(
    uri: &str,
    src: &str,
    extra: &[hover::DefIndex],
    dm: &crate::mods::DocMods,
    line: u32,
    ch: u32,
) -> Vec<definition::DefLocation> {
    let (normalized, toks, ast, doc) = doc_ctx(uri, src);
    let idxs = doc_idxs(&doc, extra);
    let ctx = definition::Ctx {
        doc_uri: uri,
        toks: &toks,
        ast: &ast,
        idxs: &idxs,
        doc_mods: dm,
    };
    definition::definition(&ctx, byte_at(&normalized, line, ch))
}

/// type definitions at an LSP position: the expression's type head →
/// that type's declaration (the cheap shape the survey priced)
pub fn type_definition_at(
    uri: &str,
    src: &str,
    extra: &[hover::DefIndex],
    dm: &crate::mods::DocMods,
    line: u32,
    ch: u32,
) -> Vec<definition::DefLocation> {
    let (normalized, toks, ast, doc) = doc_ctx(uri, src);
    let idxs = doc_idxs(&doc, extra);
    let ctx = definition::Ctx {
        doc_uri: uri,
        toks: &toks,
        ast: &ast,
        idxs: &idxs,
        doc_mods: dm,
    };
    definition::type_definition(&ctx, byte_at(&normalized, line, ch))
}

/// the token + AST half of `doc_ctx` — `references` re-parses candidate
/// files over their indexed source (doc-sized files; the per-query
/// re-parse is the house style — hover already rides it)
pub(crate) fn parse_at(uri: &str, src: &str) -> (Vec<Token>, Ast) {
    let rut_semantic::ParsedDoc { ast, toks, .. } = rut_semantic::ParsedDoc::parse(uri, src);
    (toks, ast)
}

/// inlay hints over a document range (the survey §4.3): TYPE hints on
/// unannotated bindings (the binding pass / decl layer's inference —
/// the same text hover shows), PARAMETER hints at exact-arity call
/// sites. Returns full LSP values; hosts hand them to their
/// client as-is.
pub fn inlay_hints_at(
    uri: &str,
    src: &str,
    extra: &[hover::DefIndex],
    range: Range,
) -> Vec<InlayHint> {
    let (normalized, toks, ast, doc) = doc_ctx(uri, src);
    let idxs = doc_idxs(&doc, extra);
    let index = LineIndex::new(&normalized);
    let ctx = inlay::Ctx {
        ast: &ast,
        toks: &toks,
        doc: &doc,
        idxs: &idxs,
        index: &index,
        src: &normalized,
    };
    let lo = index.byte(&normalized, range.start.line, range.start.character);
    let hi = index.byte(&normalized, range.end.line, range.end.character);
    inlay::hints(&ctx, lo, hi)
}

/// references at an LSP position (the survey §4.4): the definition index
/// read BACKWARDS — a token is a reference iff go-to-definition from it
/// lands on the declaration under the cursor. Within-file through the
/// binding pass (shadow-aware), cross-file through the use graph's
/// reverse edges. `include_declaration` per the LSP spec.
pub fn references_at(
    uri: &str,
    src: &str,
    extra: &[hover::DefIndex],
    dm: &crate::mods::DocMods,
    line: u32,
    ch: u32,
    include_declaration: bool,
) -> Vec<definition::DefLocation> {
    let (normalized, toks, ast, doc) = doc_ctx(uri, src);
    let idxs = doc_idxs(&doc, extra);
    let ctx = definition::Ctx {
        doc_uri: uri,
        toks: &toks,
        ast: &ast,
        idxs: &idxs,
        doc_mods: dm,
    };
    references::references(&ctx, byte_at(&normalized, line, ch), include_declaration)
}

/// signature help at an LSP position (the survey §4.4): the callee
/// resolves through phase 3's machinery (free calls by unique recorded
/// params, methods through the receiver's type head), the signature
/// renders verbatim, the active parameter comes from the comma/paren
/// depth at the request position. Ambiguity or arity mismatch → no
/// help, never wrong help.
pub fn signature_help_at(
    uri: &str,
    src: &str,
    extra: &[hover::DefIndex],
    line: u32,
    ch: u32,
) -> Option<SignatureHelp> {
    let (normalized, toks, ast, doc) = doc_ctx(uri, src);
    let idxs = doc_idxs(&doc, extra);
    let ctx = signature_help::Ctx {
        toks: &toks,
        ast: &ast,
        idxs: &idxs,
    };
    signature_help::signature_help(&ctx, byte_at(&normalized, line, ch))
}

// ---- symbols ----

fn to_document_symbols(raw: Vec<RawSymbol>, src: &str, index: &LineIndex) -> Vec<DocumentSymbol> {
    raw.into_iter().map(|r| convert(r, src, index)).collect()
}

#[allow(deprecated)] // `DocumentSymbol::deprecated` — kept for older clients; tags are the modern way
fn convert(r: RawSymbol, src: &str, index: &LineIndex) -> DocumentSymbol {
    let range = range_of(index, src, r.range);
    let mut selection = range_of(index, src, r.selection);
    // selectionRange must stay inside range (LSP contract)
    let inside = (selection.start.line, selection.start.character)
        >= (range.start.line, range.start.character)
        && (selection.end.line, selection.end.character) <= (range.end.line, range.end.character);
    if !inside {
        selection = range;
    }
    DocumentSymbol {
        name: r.name,
        detail: None,
        kind: symbol_kind(r.kind),
        tags: None,
        deprecated: None,
        range,
        selection_range: selection,
        children: if r.children.is_empty() {
            None
        } else {
            Some(to_document_symbols(r.children, src, index))
        },
    }
}

fn symbol_kind(k: SymKind) -> SymbolKind {
    match k {
        SymKind::Function => SymbolKind::FUNCTION,
        SymKind::Variable => SymbolKind::VARIABLE,
        SymKind::Enum => SymbolKind::ENUM,
        SymKind::EnumMember => SymbolKind::ENUM_MEMBER,
        SymKind::Class => SymbolKind::CLASS,
        SymKind::Field => SymbolKind::FIELD,
        SymKind::Method => SymbolKind::METHOD,
        SymKind::Interface => SymbolKind::INTERFACE,
        SymKind::Module => SymbolKind::MODULE,
    }
}
