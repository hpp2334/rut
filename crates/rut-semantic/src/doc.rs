//! `ParsedDoc` — the document layer's ONE artifact: the editor's whole
//! per-document derivation, computed once. The AST facade
//! (`rut_parser::parse_opts`) parses (mode derived from the uri,
//! recovering ALWAYS — the editor never fails fast); the leaf layer
//! (tokens + comments) is re-derived in the same linear pass
//! (`lex_normalized`); classification, symbols, and the line index ride
//! precomputed. Every query above this layer consumes `&ParsedDoc`
//! parts — no consumer re-runs the pipeline by hand.
//!
//! The editor law is baked here, exactly once: a `.d.rut` uri parses in
//! declaration mode, everything else full `Mode::Impl` (see `mode_of`);
/// comments always collect (hover wants them); recovery always on (the
/// tree survives broken code — the queries answer on best effort).
use rut_ast::ast::Ast;
use rut_lexer::diag::Diag;
use rut_lexer::lexer::lex_normalized;
use rut_lexer::span::Span;
use rut_lexer::token::{Comment, Token};
use rut_parser::{parse_opts, Mode, ParseOpts};

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
    /// the AST artifact — self-contained (`ast.src` is the normalized
    /// text every span indexes)
    pub ast: Ast,
    /// the leaf layer, lexed once WITH comment collection
    pub toks: Vec<Token>,
    /// the comment side table (doc comments flagged — the rustc
    /// convention, see `rut_lexer::token::Comment`)
    pub comments: Vec<Comment>,
    /// parse diags (recovering: ALL of them, tree present regardless)
    pub diags: Vec<Diag>,
    /// byte offset ⇄ (line, UTF-16 column) over `ast.src`
    pub lines: LineIndex,
    /// `classify` over (toks, ast), once — the semantic-token spans
    pub spans: Vec<(Span, TokenType)>,
    /// `symbols` over (toks, ast), once — the document-symbol tree
    pub symbols: Vec<RawSymbol>,
}

impl ParsedDoc {
    /// the editor entry: mode derived from the uri, editor opts baked —
    /// recovering always, comments always.
    pub fn parse(uri: &str, src: &str) -> ParsedDoc {
        let p = parse_opts(src, mode_of(uri), ParseOpts { recover: true });
        let ast = p.ast.expect("the recovering parse always yields a tree");
        let (toks, _, comments) = lex_normalized(&ast.src, true);
        let spans = classify(&toks, &ast);
        let syms = symbols(&toks, &ast);
        let lines = LineIndex::new(&ast.src);
        ParsedDoc {
            ast,
            toks,
            comments,
            diags: p.diags,
            lines,
            spans,
            symbols: syms,
        }
    }

    /// the artifact's own normalized text — every span indexes this
    pub fn src(&self) -> &str {
        &self.ast.src
    }
}
