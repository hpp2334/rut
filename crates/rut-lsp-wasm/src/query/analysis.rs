//! Per-document analysis — one pure function over the text and its URI:
//! normalize → lex (token source) → parse (AST + diags) → classify /
//! symbols → LSP values, every location stamped with the document's
//! identity at construction. The server layer only routes and publishes.

use ls_types::*;
use rut_semantic::{Diag, LineIndex, Span};
use std::collections::HashSet;
use std::str::FromStr;

use crate::completion;
use crate::definition;
use crate::hover;
use crate::inlay;
use crate::references;
use crate::signature_help;
use rut_semantic::{SymKind, Symbol, TokenType};

/// the analysis envelope — the per-document rows (diags, tokens,
/// symbols) plus the hover index, related-info URIs + provenance
/// stamped at construction. A plain record: the rows exist to move to
/// the wire, so the server lane reads the fields directly.
pub struct Analysis {
    pub diags: Vec<Diagnostic>,
    pub tokens: Vec<SemanticToken>,
    pub symbols: Vec<DocumentSymbol>,
    /// the hover definition index (origin stamped at construction from
    /// the document's URI)
    pub index: hover::DefIndex,
}

/// moved to `rut-semantic` (the document layer owns the filename
/// convention); re-exported so the internal callers keep their spelling
pub use rut_semantic::mode_of;

/// one ParsedDoc → the LSP `Analysis` envelope — related info + hover
/// provenance stamped at construction from the document's URI (the
/// degenerate twin below rides the placeholder)
fn analysis_of(doc: &rut_semantic::ParsedDoc, uri: &Uri) -> Analysis {
    let src = doc.src();
    let mut index = hover::index(src, doc.ast(), doc.toks());
    index.origin = uri.as_str().to_string();
    Analysis {
        diags: doc
            .diags()
            .iter()
            .map(|d| to_diagnostic(d, src, doc.lines(), uri))
            .collect(),
        tokens: encode(doc.spans(), src, doc.lines()),
        symbols: to_document_symbols(doc.symbols(), src, doc.lines()),
        index,
    }
}

/// the degenerate-URI fallback (a uri that fails to parse as `Uri`
/// still analyzes — the MODE derives from the raw string, `mode_of`
/// keeping the filename convention; related info + provenance ride the
/// placeholder)
pub fn analyze_unstamped(uri: &str, src: &str) -> Analysis {
    analysis_of(
        &rut_semantic::ParsedDoc::parse(mode_of(uri), src),
        &placeholder_uri(),
    )
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

pub(crate) fn to_diagnostic(d: &Diag, src: &str, index: &LineIndex, uri: &Uri) -> Diagnostic {
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
                uri: uri.clone(), // the document's identity — stamped at construction
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

/// the degenerate lane's stand-in — a raw uri that failed `Uri::from_str`
/// has no identity to stamp; related info rides it openly, the check lane
/// drops related info rather than guess (its never-guess law)
pub(crate) fn placeholder_uri() -> Uri {
    Uri::from_str("file:///").unwrap()
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

/// The server's entry point: the analysis stamped with the document's
/// URI (related info + hover provenance).
pub fn analyze_at(uri: &Uri, src: &str) -> Analysis {
    analysis_of(
        &rut_semantic::ParsedDoc::parse(mode_of(uri.as_str()), src),
        uri,
    )
}

// ---- document-level queries (the hosts' entry points) ----
//
// The wasm-bindgen bodies in lib.rs and the tests run the *same*
// hover / completion queries over the same indexes — defined here
// once, so no host can grow its own dialect.

/// per-query pipeline: normalize → lex → parse → the document's index
/// (origin stamped). The queries below only add position + lookup —
/// they hold the doc and borrow its parts (the capsule's read doors).
fn doc_ctx(uri: &str, src: &str) -> (rut_semantic::ParsedDoc, hover::DefIndex) {
    // ONE derivation per query: the facade parses (mode from the
    // filename convention) and the hover index builds over its parts —
    // the old shape parsed the same text twice (its own parse +
    // index_at's)
    let d = rut_semantic::ParsedDoc::parse(mode_of(uri), src);
    let mut idx = hover::index(d.src(), d.ast(), d.toks());
    idx.origin = uri.to_string();
    (d, idx)
}

/// the definition index for one document (origin stamped with `uri`)
pub fn index_at(uri: &str, src: &str) -> hover::DefIndex {
    let d = rut_semantic::ParsedDoc::parse(mode_of(uri), src);
    let mut idx = hover::index(d.src(), d.ast(), d.toks());
    idx.origin = uri.to_string();
    idx
}

/// the lookup chain — the open document first, then `extra` (the
/// prelude + the workspace)
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
    let (d, doc) = doc_ctx(uri, src);
    let idxs = doc_idxs(&doc, extra);
    let h = hover::hover(&idxs, d.toks(), d.ast(), dm, byte_at(d.src(), line, ch))?;
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: h.markdown,
        }),
        range: Some(range_of(d.lines(), d.src(), h.span)),
    })
}

/// completions at an LSP position: the document's index first, then
/// `extra`. Returns full LSP items. `doc` is the open document's
/// file-module context (`crate::mods::DocMods`) — the face's knowledge
/// of where the file sits in its package's mod tree; the default
/// (unknown) keeps the position-path tier off and changes nothing
/// else. `direct_deps` is the open document's own dep set (its
/// manifest's rows) — the auto-import tier derives its gate from it
/// live.
pub fn complete_at(
    uri: &str,
    src: &str,
    extra: &[hover::DefIndex],
    line: u32,
    ch: u32,
    doc: &crate::mods::DocMods,
    direct_deps: &HashSet<String>,
) -> Vec<CompletionItem> {
    let (d, doc_index) = doc_ctx(uri, src);
    let idxs = doc_idxs(&doc_index, extra);
    completion::complete(
        &idxs,
        d.toks(),
        d.ast(),
        byte_at(d.src(), line, ch),
        d.src(),
        doc,
        direct_deps,
    )
    .into_iter()
    .map(completion::lsp_item)
    .collect()
}

/// definitions at an LSP position: the survey §3.3 layers (binding pass
/// → decl layer → use graph → the chain's dep indexes). Empty = nothing
/// resolves; multiple entries = the candidate list. Targets carry the
/// doc URI, a workspace origin, or the prelude's true `rut/...` source
/// path —
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
    let (d, doc) = doc_ctx(uri, src);
    let idxs = doc_idxs(&doc, extra);
    let ctx = definition::Ctx {
        doc_uri: uri,
        toks: d.toks(),
        ast: d.ast(),
        idxs: &idxs,
        doc_mods: dm,
    };
    definition::definition(&ctx, byte_at(d.src(), line, ch))
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
    let (d, doc) = doc_ctx(uri, src);
    let idxs = doc_idxs(&doc, extra);
    let ctx = definition::Ctx {
        doc_uri: uri,
        toks: d.toks(),
        ast: d.ast(),
        idxs: &idxs,
        doc_mods: dm,
    };
    definition::type_definition(&ctx, byte_at(d.src(), line, ch))
}

/// the doc itself — `references` re-parses candidate files over their
/// indexed source (doc-sized files; the per-query re-parse is the
/// house style — hover already rides it)
pub(crate) fn parse_at(uri: &str, src: &str) -> rut_semantic::ParsedDoc {
    rut_semantic::ParsedDoc::parse(mode_of(uri), src)
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
    let (d, doc) = doc_ctx(uri, src);
    let idxs = doc_idxs(&doc, extra);
    let index = d.lines();
    let ctx = inlay::Ctx {
        ast: d.ast(),
        toks: d.toks(),
        doc: &doc,
        idxs: &idxs,
        index,
        src: d.src(),
    };
    let lo = index.byte(d.src(), range.start.line, range.start.character);
    let hi = index.byte(d.src(), range.end.line, range.end.character);
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
    let (d, doc) = doc_ctx(uri, src);
    let idxs = doc_idxs(&doc, extra);
    let ctx = definition::Ctx {
        doc_uri: uri,
        toks: d.toks(),
        ast: d.ast(),
        idxs: &idxs,
        doc_mods: dm,
    };
    references::references(&ctx, byte_at(d.src(), line, ch), include_declaration)
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
    let (d, doc) = doc_ctx(uri, src);
    let idxs = doc_idxs(&doc, extra);
    let ctx = signature_help::Ctx {
        toks: d.toks(),
        ast: d.ast(),
        idxs: &idxs,
    };
    signature_help::signature_help(&ctx, byte_at(d.src(), line, ch))
}

// ---- symbols ----

fn to_document_symbols(raw: &[Symbol], src: &str, index: &LineIndex) -> Vec<DocumentSymbol> {
    raw.iter().map(|r| convert(r.clone(), src, index)).collect()
}

#[allow(deprecated)] // `DocumentSymbol::deprecated` — kept for older clients; tags are the modern way
fn convert(r: Symbol, src: &str, index: &LineIndex) -> DocumentSymbol {
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
            Some(to_document_symbols(&r.children, src, index))
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
