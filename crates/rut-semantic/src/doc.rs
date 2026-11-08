//! `ParsedDoc` — the document layer's ONE artifact: the editor's whole
//! per-document derivation, computed once. The recovering entry
//! (`rut_parser::parse_with_recover`, mode derived from the uri — the
//! editor never fails fast) parses with `include_comments` so doc
//! chains graft onto the decl nodes at allocation (D7); the leaf layer
//! (tokens) is re-derived in the same linear pass (`lex_normalized`);
//! classification, symbols, and the line index ride precomputed. Every
//! query above this layer consumes `&ParsedDoc` parts — no consumer
//! re-runs the pipeline by hand.
//!
//! The editor law is baked here, exactly once: a `.d.rut` uri parses in
//! declaration mode, everything else full `Mode::Impl` (see `mode_of`);
//! recovery always on (the tree survives broken code — the queries
//! answer on best effort). `src` is the pair artifact's other half
//! (v2: the tree carries no text) — every span indexes it.
use rut_ast::Ast;
use rut_lexer::diag::Diag;
use rut_lexer::lexer::lex_normalized;
use rut_lexer::span::Span;
use rut_lexer::token::Token;
use rut_parser::{parse_with_recover, Mode, ParseOpts};

use crate::line_index::LineIndex;
use crate::{classify, symbols, RawSymbol, TokenType};

/// `.d.rut` parses in declaration mode; everything else is
/// full `Mode::Impl` — the filename convention, THE authority on which
/// grammar a document parses (the manifest/bundle rows follow it too).
pub fn mode_of(path: &str) -> Mode {
    if path.ends_with(".d.rut") {
        Mode::Decl
    } else {
        Mode::Impl
    }
}

/// the editor's per-document bundle — see the module doc.
pub struct ParsedDoc {
    /// the AST artifact — self-contained structure (the capsule store;
    /// the text lives in `src`, the pair's other half)
    pub ast: Ast,
    /// the NORMALIZED text every span indexes
    pub src: String,
    /// the leaf layer, lexed once
    pub toks: Vec<Token>,
    /// byte offset ⇄ (line, UTF-16 column) over `src`
    pub lines: LineIndex,
    /// `classify` over (toks, ast), once — the semantic-token spans
    pub spans: Vec<(Span, TokenType)>,
    /// `symbols` over (toks, ast), once — the document-symbol tree
    pub symbols: Vec<RawSymbol>,
}

impl ParsedDoc {
    /// the editor entry: mode derived from the uri, editor opts baked —
    /// recovering always, docs always (they graft onto the decl nodes).
    pub fn parse(uri: &str, src: &str) -> ParsedDoc {
        let p = parse_with_recover(
            src,
            mode_of(uri),
            ParseOpts {
                include_comments: true,
            },
        );
        let (toks, _, _) = lex_normalized(&p.src, false);
        let spans = classify(&toks, &p.ast);
        let syms = symbols(&toks, &p.ast);
        let lines = LineIndex::new(&p.src);
        ParsedDoc {
            ast: p.ast,
            toks,
            lines,
            spans,
            symbols: syms,
            src: p.src,
        }
    }

    /// the artifact's own normalized text — every span indexes this
    pub fn src(&self) -> &str {
        &self.src
    }

    /// the parse diags, ALL of them (the recovering parse yields the
    /// tree regardless) — the FLAT view on `ast.errors()`; the
    /// positional view is the `ErrorNode`s in the streams
    pub fn diags(&self) -> &[Diag] {
        self.ast.errors()
    }
}
