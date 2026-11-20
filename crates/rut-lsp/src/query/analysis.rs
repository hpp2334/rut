//! Per-document analysis — the LSP values over the session's versioned
//! cache: the entry's document (one parse per change — the sync lane's)
//! is the single source for diags / tokens / symbols, and every query
//! reads it through the collection's chain. The server layer only
//! routes and publishes; nothing here re-parses per request.

use crate::hover::collection::Chain;
use crate::semantic::{Diag, LineIndex, Span};
use ls_types::*;

use crate::query::check;
use crate::query::definition;
use crate::query::inlay;
use crate::query::references;
use crate::query::signature_help;
use crate::semantic::{SymKind, Symbol};

/// the analysis envelope — the per-document rows (diags, tokens,
/// symbols) read off the entry's kept document. A plain record: the
/// rows exist to move to the wire, so the server lane reads the fields
/// directly.
pub struct Analysis {
    pub diags: Vec<Diagnostic>,
    pub tokens: Vec<SemanticToken>,
    pub symbols: Vec<DocumentSymbol>,
}

/// the sealed `semantic` module owns the filename convention — the
/// unit tests spell it here (the production lanes go through the
/// sync lane's own derivation)
#[cfg(test)]
pub use crate::semantic::mode_of;

/// one Document → the LSP `Analysis` envelope — every row rides the
/// doc's own lazy views (the classifier and the symbol tree compute
/// once per version, on the sync lane's entry)
pub fn analysis_of(doc: &crate::semantic::Document) -> Analysis {
    Analysis {
        diags: doc.diags().iter().map(|d| to_diagnostic(d, doc)).collect(),
        tokens: encode(doc),
        symbols: to_document_symbols(doc),
    }
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

/// the doc-door range — both ends through `Document::position`, no
/// manual `lines()`+`src()` pairing at the call site (the lanes above
/// hold parts — definition's spans over a mounted file — and keep
/// [`range_of`])
fn range_on(doc: &crate::semantic::Document, span: Span) -> Range {
    let (l1, c1) = doc.position(span.lo);
    let (l2, c2) = doc.position(span.hi);
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

/// `Document`'s spans → the delta encoding, every position and length
/// joined through the doc's own doors (`position`/`text` — no manual
/// `lines()`+`src()` pairing). Spans must be sorted and
/// non-overlapping (the classifier's contract). Lengths and columns
/// are UTF-16 code units — LSP's position currency.
pub fn encode(doc: &crate::semantic::Document) -> Vec<SemanticToken> {
    let mut out = Vec::with_capacity(doc.spans().len());
    let (mut prev_line, mut prev_char) = (0u32, 0u32);
    for (sp, ty) in doc.spans() {
        let (line, ch) = doc.position(sp.lo);
        let len: u32 = doc.text(*sp).chars().map(|c| c.len_utf16() as u32).sum();
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

/// The doc-door diagnostic — the positions join through the
/// document's own doors (the one builder, [`to_diagnostic_parts`],
/// under a `Document`).
pub(crate) fn to_diagnostic(d: &Diag, doc: &crate::semantic::Document) -> Diagnostic {
    to_diagnostic_parts(d, doc.src(), doc.lines())
}

/// The diagnostic builder over PARTS — the check lane's shape: its
/// compiler-diag rows name DIFFERENT files (one per-text `LineIndex`
/// each), so they never hold one `Document` to ride its doors.
pub(crate) fn to_diagnostic_parts(d: &Diag, src: &str, index: &LineIndex) -> Diagnostic {
    Diagnostic {
        range: range_of(index, src, d.span),
        severity: Some(DiagnosticSeverity::ERROR),
        code: None,
        code_description: None,
        source: Some("rut".to_string()),
        message: d.msg.clone(),
        related_information: None,
        tags: None,
        data: None,
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

// ---- document-level queries (the hosts' entry points) ----
//
// The wasm-bindgen bodies in lib.rs and the tests run the *same*
// hover / completion queries over the same chains — defined here
// once, so no host can grow its own dialect. Every query reads the
// session's SYNCED entry (the chain head's kept document) — the old
// per-request lex→parse→index pipeline is gone.

/// byte offset of an LSP position over the document's own text
fn byte_at(doc: &crate::semantic::Document, line: u32, ch: u32) -> u32 {
    doc.lines().byte(doc.src(), line, ch)
}

/// hover at an LSP position: the open document first, then the
/// mounted world. Returns the full LSP value — hosts hand it to their
/// client as-is. `dm` is the open document's file-module context (the
/// face's knowledge) — an own-package use-path segment hover resolves
/// through it; the default keeps the cross-package lanes only.
pub fn hover_on(
    doc: &crate::semantic::Document,
    chain: &Chain,
    dm: &crate::mods::DocMods,
    line: u32,
    ch: u32,
) -> Option<Hover> {
    let h = crate::hover::hover(chain, dm, byte_at(doc, line, ch))?;
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: h.markdown,
        }),
        range: Some(range_of(doc.lines(), doc.src(), h.span)),
    })
}

/// definitions at an LSP position: the survey §3.3 layers (binding pass
/// → decl layer → use graph → the mounted world). Empty = nothing
/// resolves; multiple entries = the candidate list. Targets carry the
/// doc URI, a workspace origin, or the prelude's true `rut/...` source
/// path — the face resolves them (server: against the workspace root;
/// wasm shim: the extension does). `dm` is the open document's
/// file-module context — the mod-edge clicks and the own-package use
/// lanes key on it; the default keeps them off.
pub fn definition_on(
    doc: &crate::semantic::Document,
    chain: &Chain,
    dm: &crate::mods::DocMods,
    line: u32,
    ch: u32,
) -> Vec<definition::DefLocation> {
    let ctx = definition::Ctx {
        doc_uri: chain.head().origin(),
        doc,
        chain,
        doc_mods: dm,
    };
    definition::definition(&ctx, byte_at(doc, line, ch))
}

/// type definitions at an LSP position: the expression's type head →
/// that type's declaration (the cheap shape the survey priced)
pub fn type_definition_on(
    doc: &crate::semantic::Document,
    chain: &Chain,
    dm: &crate::mods::DocMods,
    line: u32,
    ch: u32,
) -> Vec<definition::DefLocation> {
    let ctx = definition::Ctx {
        doc_uri: chain.head().origin(),
        doc,
        chain,
        doc_mods: dm,
    };
    definition::type_definition(&ctx, byte_at(doc, line, ch))
}

/// inlay hints over a document range (the survey §4.3): TYPE hints on
/// unannotated bindings (the binding pass / decl layer's inference —
/// the same text hover shows), PARAMETER hints at exact-arity call
/// sites. Returns full LSP values; hosts hand them to their
/// client as-is.
pub fn inlay_hints_on(
    doc: &crate::semantic::Document,
    chain: &Chain,
    range: Range,
) -> Vec<InlayHint> {
    let ctx = inlay::Ctx {
        doc: chain.head(),
        chain,
    };
    let index = doc.lines();
    let lo = index.byte(doc.src(), range.start.line, range.start.character);
    let hi = index.byte(doc.src(), range.end.line, range.end.character);
    inlay::hints(&ctx, lo, hi)
}

/// references at an LSP position (the survey §4.4): the definition
/// index read BACKWARDS — a token is a reference iff go-to-definition
/// from it lands on the declaration under the cursor. Within-file
/// through the binding pass (shadow-aware), cross-file through the use
/// graph's reverse edges — over the mounted files' KEPT documents (no
/// re-parse lane). `include_declaration` per the LSP spec.
pub fn references_on(
    doc: &crate::semantic::Document,
    chain: &Chain,
    dm: &crate::mods::DocMods,
    line: u32,
    ch: u32,
    include_declaration: bool,
) -> Vec<definition::DefLocation> {
    let ctx = definition::Ctx {
        doc_uri: chain.head().origin(),
        doc,
        chain,
        doc_mods: dm,
    };
    references::references(&ctx, byte_at(doc, line, ch), include_declaration)
}

/// signature help at an LSP position (the survey §4.4): the callee
/// resolves through the receiver machinery (free calls by unique
/// recorded params, methods through the receiver's type head), the
/// signature renders verbatim, the active parameter comes from the
/// comma/paren depth at the request position. Ambiguity or arity
/// mismatch → no help, never wrong help.
pub fn signature_help_on(chain: &Chain, line: u32, ch: u32) -> Option<SignatureHelp> {
    let head = chain.head();
    let ctx = signature_help::Ctx { chain };
    signature_help::signature_help(&ctx, head.doc.lines().byte(head.doc.src(), line, ch))
}

/// the check lane's merged answer for one document — the mounted
/// world's package groups feed the compiler run; the doc's package
/// context (its stamped twin names it) decides parse-only vs merged.
pub(crate) fn merged_diags_for(
    uri: &str,
    src: &str,
    dm: &crate::mods::DocMods,
    files: &[crate::hover::collection::MountedFile],
    parse_diags: Vec<Diagnostic>,
) -> Vec<Diagnostic> {
    let groups = if dm.pkg.is_some() {
        check::groups_from_files(files)
    } else {
        Vec::new()
    };
    check::merged_doc_diags(
        uri,
        src,
        dm.pkg.clone(),
        dm.mod_path.clone(),
        groups,
        parse_diags,
    )
}

// ---- symbols ----

fn to_document_symbols(doc: &crate::semantic::Document) -> Vec<DocumentSymbol> {
    symbols_on(doc, doc.symbols())
}

fn symbols_on(doc: &crate::semantic::Document, raw: &[Symbol]) -> Vec<DocumentSymbol> {
    raw.iter().map(|r| convert(r.clone(), doc)).collect()
}

#[allow(deprecated)] // `DocumentSymbol::deprecated` — kept for older clients; tags are the modern way
fn convert(r: Symbol, doc: &crate::semantic::Document) -> DocumentSymbol {
    // selection ⊆ range holds by construction at every symbol site and is
    // test-gated (semantic tests + the lsp-wasm corpus check) — no clamp here
    DocumentSymbol {
        name: r.name,
        detail: None,
        kind: symbol_kind(r.kind),
        tags: None,
        deprecated: None,
        range: range_on(doc, r.range),
        selection_range: range_on(doc, r.selection),
        children: if r.children.is_empty() {
            None
        } else {
            Some(symbols_on(doc, &r.children))
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
