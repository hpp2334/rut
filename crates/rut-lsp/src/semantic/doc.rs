//! `Document` — the document layer's ONE artifact: the editor's whole
//! per-document derivation, computed once. The shell rides rut-ast's
//! [`Doc`] (the pipeline owner — one lex, the tree, the token stream,
//! and the one `Doc::edit` mutator inside); the expensive extras —
//! the semantic-token spans and the symbol tree — ride LAZY
//! (`OnceCell`): lanes that never read them (the dep-walk index pass
//! parses many dep files and reads only ast/toks/src) never pay for
//! them. Every query above this layer consumes `&Document` parts —
//! no consumer re-runs the pipeline by hand — and every span×src×lines
//! join rides the doc's own doors (`text`/`position`/`byte`), one per
//! axis.
//!
//! The editor law is baked here, exactly once: recovery always on (the
//! tree survives broken code — the queries answer on best effort),
//! docs always (rut-ast grafts them at allocation — always on there
//! too). The MODE is the caller's input — `mode_of` spells the
//! filename convention (a `.d.rut` path parses in declaration mode,
//! everything else full `Mode::Impl`), and every uri-holding call site
//! derives through it. `src` is the Doc's own normalized text (the
//! artifact law: the tree carries no text) — every span indexes it.
use std::cell::OnceCell;

use rut_ast::parser::Mode;
use rut_ast::{Ast, Doc};
use rut_lexer::diag::Diag;
use rut_lexer::source::Edit;
use rut_lexer::span::Span;
use rut_lexer::token::Token;

use super::line_index::LineIndex;
use super::{classify, symbols, Symbol, TokenType};

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

/// the editor's per-document bundle — see the module doc. A capsule:
/// ONE construction lane (`parse`), every part read through named
/// doors; consumers hold the doc and borrow (`&Document` parts).
/// The pipeline itself (text, tokens, tree, the edit door) lives in
/// the rut-ast `Doc` this shell holds.
pub struct Document {
    /// the pipeline — text + tokens + tree + the one mutator
    doc: Doc,
    /// byte offset ⇄ (line, UTF-16 column) over the Doc's text
    lines: LineIndex,
    /// `classify` over (toks, ast), on first read — the semantic-token spans
    spans: OnceCell<Vec<(Span, TokenType)>>,
    /// `symbols` over the tree, on first read — the document-symbol tree
    symbols: OnceCell<Vec<Symbol>>,
    /// the inlay pass over the whole document, on first read — the
    /// per-version lazy view (dropped by sync; the viewport filter is
    /// the caller's presentation)
    inlays: OnceCell<Vec<crate::query::inlay::RawInlay>>,
}

impl Document {
    /// the editor entry: mode supplied by the caller (`mode_of` spells
    /// the filename convention), editor opts baked — recovering always
    /// (rut-ast's only entry is the recovering one), docs always (they
    /// graft onto the decl nodes at allocation). Takes the source BY
    /// VALUE — it moves into the artifact; a caller holding an owned
    /// String clones to move, a fresh read moves for free.
    pub fn parse(mode: Mode, src: String) -> Document {
        let doc = Doc::parse(mode, src);
        let lines = LineIndex::of(&doc);
        Document {
            doc,
            lines,
            spans: OnceCell::new(),
            symbols: OnceCell::new(),
            inlays: OnceCell::new(),
        }
    }

    /// the AST artifact — the capsule store (the text lives in the Doc)
    pub fn ast(&self) -> &Ast {
        self.doc.ast()
    }

    /// THE SYNC LANE — one edit in, a coherent view out: the host's
    /// new text diffs against the Doc's own (`Edit::between`), the one
    /// `Doc::edit` mutator repairs the pipeline (text + tokens + tree
    /// inside), the UTF-16 view rebuilds over the fresh text, and the
    /// lazy views drop (the first read after the change re-derives
    /// them — never yesterday's spans). The shell sequences NOTHING
    /// else: every query above reads the same coherent document.
    pub fn sync(&mut self, text: &str) {
        let e = Edit::between(self.src(), text);
        self.doc.edit(e);
        self.lines = LineIndex::of(&self.doc);
        self.spans = OnceCell::new();
        self.symbols = OnceCell::new();
        self.inlays = OnceCell::new();
    }

    /// the leaf layer, lexed once (the Doc's own token stream)
    pub fn toks(&self) -> &[Token] {
        self.doc.tokens_in(self.doc.span())
    }

    /// byte offset ⇄ (line, UTF-16 column) over the Doc's text
    pub fn lines(&self) -> &LineIndex {
        &self.lines
    }

    /// `classify` over (toks, ast) — LAZY: the first read computes it
    /// (`OnceCell`), and a lane that never reads it never pays. Sorted
    /// by start and non-overlapping — the classifier's contract.
    pub fn spans(&self) -> &[(Span, TokenType)] {
        self.spans.get_or_init(|| classify(self.toks(), self.ast()))
    }

    /// `symbols` over the tree — LAZY: the first read computes it
    /// (`OnceCell`), and a lane that never reads it never pays.
    pub fn symbols(&self) -> &[Symbol] {
        self.symbols.get_or_init(|| symbols(self.ast()))
    }

    /// the inlay pass — LAZY, per version: `compute` runs at the first
    /// read after a sync (the caller closure carries the chain the
    /// resolver doors need); the viewport filter over the cached rows
    /// is the caller's presentation
    pub(crate) fn inlays_view(
        &self,
        compute: impl FnOnce() -> Vec<crate::query::inlay::RawInlay>,
    ) -> &[crate::query::inlay::RawInlay] {
        self.inlays.get_or_init(compute)
    }

    /// the artifact's own normalized text — every span indexes this
    pub fn src(&self) -> &str {
        self.doc.text(self.doc.span())
    }

    /// the locals pass — lazy per version over the pipeline (the rows
    /// carry written annotations; inference is the resolver's lane)
    pub fn bindings(&self) -> &[rut_ast::Binding] {
        self.doc.bindings()
    }

    /// the fence residue — the lexeme under the cursor when the tree
    /// landed on an `ErrorNode` (the token store's one public read)
    pub fn name_at(&self, pos: u32) -> Option<(Span, &str)> {
        self.doc.name_at(pos)
    }

    // the join doors — span × src × lines, one door each: no consumer
    // pairs `lines()`+`src()` by hand

    /// the text a span indexes, over the artifact's own src (the
    /// span×src join)
    pub fn text(&self, span: Span) -> &str {
        self.doc.text(span)
    }

    /// byte offset -> (line, UTF-16 column) over the artifact's src
    pub fn position(&self, byte: u32) -> (u32, u32) {
        self.lines.position(self.src(), byte)
    }

    /// (line, UTF-16 column) -> byte offset over the artifact's src —
    /// the position joins' one door (no consumer pairs
    /// `lines()`+`src()` by hand)
    pub fn byte(&self, line: u32, ch: u32) -> u32 {
        self.lines.byte(self.src(), line, ch)
    }

    /// the 0-based line `byte` sits on — the Doc's own line geometry
    /// (the per-row scan this replaces built a table for; the geometry
    /// was lexed once and lives in the pipeline)
    pub fn line_of(&self, byte: u32) -> u32 {
        self.doc.line_of(byte)
    }

    /// the parse diags, ALL of them (the recovering parse yields the
    /// tree regardless) — the FLAT view on `ast.errors()`; the
    /// positional view is the `ErrorNode`s in the streams
    pub fn diags(&self) -> &[Diag] {
        self.doc.ast().errors()
    }
}
