//! Per-document analysis — one pure function over the text: normalize →
//! lex (token source) → parse (AST + diags) → classify / symbols → LSP
//! values. The server layer only routes URIs and publishes.

use rut_lexer::diag::Diag;
use rut_lexer::lexer::{lex, normalize};
use rut_lexer::span::Span;
use rut_parser::{parse, Mode};
use std::str::FromStr;
use tower_lsp_server::ls_types::*;

use crate::hover;
use crate::line_index::LineIndex;
use crate::semantic::{self, RawSymbol, SymKind, TokenType};

pub struct Analysis {
    pub diags: Vec<Diagnostic>,
    pub tokens: Vec<SemanticToken>,
    pub symbols: Vec<DocumentSymbol>,
    /// the hover definition index (origin patched by the caller —
    /// `analyze_at` stamps the URI)
    pub index: hover::DefIndex,
}

/// `.d.rut` parses in declaration mode (RFC 0030 §3); everything else is
/// full `Mode::Impl`.
pub fn mode_of(path: &str) -> Mode {
    if path.ends_with(".d.rut") {
        Mode::Decl
    } else {
        Mode::Impl
    }
}

pub fn analyze(src: &str, mode: Mode) -> Analysis {
    let src = normalize(src);
    let (toks, _) = lex(&src);
    let (ast, diags) = parse(&src, mode);
    let index = LineIndex::new(&src);
    let spans = semantic::classify(&toks, &ast);
    let tokens = encode(&spans, &src, &index);
    let symbols = to_document_symbols(semantic::symbols(&toks, &ast), &src, &index);
    let diags = diags.iter().map(|d| to_diagnostic(d, &src, &index)).collect();
    Analysis {
        diags,
        tokens,
        symbols,
        index: hover::index(&src, &ast),
    }
}

// ---- spans -> LSP ranges ----

pub fn range_of(index: &LineIndex, src: &str, span: Span) -> Range {
    let (l1, c1) = index.position(src, span.lo);
    let (l2, c2) = index.position(src, span.hi);
    Range {
        start: Position { line: l1, character: c1 },
        end: Position { line: l2, character: c2 },
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

fn to_diagnostic(d: &Diag, src: &str, index: &LineIndex) -> Diagnostic {
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
        related_information: if related.is_empty() { None } else { Some(related) },
        tags: None,
        data: None,
    }
}

/// Patch the (placeholder) related-info URIs after the fact — `analyze` is
/// URI-agnostic; the server entry point below knows the document.
fn set_related_uri(diags: &mut [Diagnostic], uri: &Uri) {
    for d in diags {
        if let Some(rel) = &mut d.related_information {
            for r in rel.iter_mut() {
                r.location.uri = uri.clone();
            }
        }
    }
}

/// The server's entry point: `analyze` + URI wiring for related info
/// and hover provenance.
pub fn analyze_at(uri: &Uri, src: &str, mode: Mode) -> Analysis {
    let mut a = analyze(src, mode);
    set_related_uri(&mut a.diags, uri);
    a.index.origin = uri.as_str().to_string();
    a
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
    let inside = (selection.start.line, selection.start.character) >= (range.start.line, range.start.character)
        && (selection.end.line, selection.end.character) <= (range.end.line, range.end.character);
    if !inside {
        selection = range.clone();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_end_to_end() {
        let a = analyze("fn main() -> nil {\n    let x = ;\n}\n", Mode::Impl);
        assert!(!a.diags.is_empty(), "parse error must surface");
        assert!(a.diags[0].severity == Some(DiagnosticSeverity::ERROR));
        assert_eq!(a.diags[0].source.as_deref(), Some("rut"));

        let clean = analyze("enum Color { Red }\n", Mode::Impl);
        assert!(clean.diags.is_empty());
        assert_eq!(clean.symbols.len(), 1);
        assert_eq!(clean.symbols[0].name, "Color");
    }

    #[test]
    fn encode_is_delta_correct() {
        let src = "enum Color { Red }\nfn f(x: i32) -> nil {}\n";
        let a = analyze(src, Mode::Impl);
        // decode back: every token's absolute (line, char, len) must land
        // on the exact text it classified
        let mut line = 0u32;
        let mut ch = 0u32;
        for t in &a.tokens {
            line += t.delta_line;
            ch = if t.delta_line == 0 { ch + t.delta_start } else { t.delta_start };
            let byte_lo = {
                let ix = LineIndex::new(&normalize(src));
                ix.byte(&normalize(src), line, ch)
            };
            let text = &normalize(src)[byte_lo as usize..(byte_lo + t.length) as usize];
            assert!(!text.is_empty());
            assert!(
                text.chars().all(|c| !c.is_whitespace()),
                "token must not span whitespace: {text:?}"
            );
        }
    }

    #[test]
    fn decl_mode_for_dot_d_rut() {
        assert_eq!(mode_of("x/y/plugin.d.rut"), Mode::Decl);
        assert_eq!(mode_of("x/y/main.rut"), Mode::Impl);
    }
}
