//! Parser — the explicit-frame machine (contract C2 — no
//! native recursion).
//!
//! One `run` loop drives two mechanisms: a **frame stack** for
//! structure — one frame kind per grammar rule, each holding the children
//! it has collected and the stage it suspended at — and an embedded
//! **Pratt engine** for expressions. A frame that needs a child pushes
//! a frame; a frame that completes pops and hands its result (`Done`)
//! to the parent through the inbox. Host stack usage is constant.
//!
//! v2 laws: construction is ALLOCATE-COMPLETE over the capsule store —
//! every node value is built whole (span + name_span + doc inside) and
//! takes its slot once; recovery FENCES unparseable regions as
//! `ErrorNode`s into the very list they belong to (the recovery
//! ladder); docs graft at allocation — always on, from the enriched
//! `Graft` rows (the grammar never consults comments); node spans end
//! at the item's own terminator (`prev_end` — exact spans), the
//! token-less ones included: a node with no token of its own — a block
//! that never received its `{` — spans nothing. The ONE
//! engine entry is `parse_item_window`: a cold parse is the window
//! covering everything, the `Doc` edit lane rides it with the dirty
//! tail; `parse`/`parse_with_recover` are wrappers over `Doc::parse`.

use crate::{CommentData, DocData, ErrorNode, Handle, ItemRef, ModuleNode};
use rut_lexer::diag::Diag;
use rut_lexer::span::{Span, NEST_MAX};
use rut_lexer::token::{Comment, Tok, Token};

mod expr;
mod frame;
mod item;
mod stmt;
mod ty;

use frame::{Done, Frame, Step};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// `.rut` — full grammar, no `host`/`builtin`
    Impl,
    /// `.d.rut` — same grammar + surface declarations, bodies forbidden
    Decl,
}

/// Expression nesting budget (C3). Recursive descent cost ~13 host frames
/// per level, so 64 fit a 1 MiB stack with margin; the frame machine has
/// no host-stack cost of its own, but the *downstream* recursive walks
/// (AST dump, fused typecheck) do — the cap keeps trees they visit shallow.
pub(crate) const EXPR_MAX: u32 = 64;

/// The fail-fast entry: `Err` carries the FIRST diag and no tree was
/// built; `Ok` implies a clean tree (`ast.is_clean()`). Takes the
/// source BY VALUE — it moves into the artifact (the [`Doc`](crate::Doc)
/// law: the document owns the normalized text every span indexes).
/// A wrapper over [`crate::Doc::parse`] kept for the "a diag means
/// stop" callers (its own tests).
pub fn parse(src: String, mode: Mode) -> Result<crate::Doc, Diag> {
    let doc = crate::Doc::parse(mode, src);
    match doc.ast().errors().first().cloned() {
        Some(first) => Err(first),
        None => Ok(doc),
    }
}

/// The recovering entry: ALWAYS a tree. Unparseable regions are
/// `ErrorNode`s in the lists they belong to; every diag rides
/// `ast.errors()` (the flat view) and the in-stream slots (the
/// positional view). Takes the source BY VALUE — a wrapper over
/// [`crate::Doc::parse`] (the one grammar engine; see there).
pub fn parse_with_recover(src: String, mode: Mode) -> crate::Doc {
    crate::Doc::parse(mode, src)
}

/// THE GRAMMAR ENGINE'S ONE ENTRY — a token WINDOW in, root-level
/// items out. The whole stream is just the window that happens to
/// cover everything: a cold parse rides it with the full range, the
/// `Doc` edit lane rides it with the dirty tail (`[dirty item .. EOF]`)
/// into FRESH SLOTS of the live store. One engine, no second parser to
/// drift (law 3).
///
/// `toks` and `ast` are borrowed and swapped out for the frame
/// machine's duration, then put back — the fresh nodes land in the
/// caller's store. `grafts` is the enriched comment table the doc
/// chains graft from ([`graft_table`]; the grammar never consults
/// comments, but decl nodes carry their chains — always on). `pre_items`
/// are the lex-tier fences seeded ahead of the window (cold parses
/// only — an edit window's fences live in the `Doc`'s managed prefix).
/// `prev_end` seeds the exact-span helper: 0 for a cold parse, the
/// window's first token lo for an edit (a degenerate fence must not
/// reach back before the window). `deep_already` presets the once-only
/// nesting flags when the surviving subtrees already carry a "nesting
/// too deep" row — the fresh parse then stays ≡ cold (the flat panel
/// reports the global-first site, which survives the edit).
///
/// Returns the root-level ITEM LIST (the throwaway module frame's
/// payload — its slot is punched back) and the window's parse diags.
pub(crate) fn parse_item_window(
    mode: Mode,
    toks: &mut Vec<Token>,
    ast: &mut crate::Ast,
    grafts: Vec<Graft>,
    pre_items: Vec<ItemRef>,
    prev_end: u32,
    deep_already: bool,
) -> (Vec<ItemRef>, Vec<Diag>) {
    let window = std::mem::take(toks);
    let store = std::mem::replace(ast, crate::Ast::new());
    // the parser gets a SCRATCH copy of the window: the grammar may
    // rewrite it locally (expect_gt splits a `>>` into two `>`), and
    // the caller's stream must stay the COLD LEX — the splice lane's
    // convergence and the edit lane's mappings both key on it
    let mut p = Parser {
        toks: window.clone(),
        pos: 0,
        max_pos: 0,
        diags: Vec::new(),
        mode,
        frames: Vec::new(),
        inbox: None,
        nest: 0,
        nest_reported: deep_already,
        expr_depth: 0,
        expr_reported: deep_already,
        ast: store,
        comments: grafts,
        prev_end,
    };
    let root = p.run(pre_items);
    // the item list is the payload: take the throwaway module node out
    // of its arena (the hole is reused by the next allocation) and
    // hand the items back
    let module = <ModuleNode as crate::NodeStore>::arena_mut(&mut p.ast)
        .remove(root.idx)
        .expect("the module frame always allocates its root");
    // the PRISTINE window rides back out (the scratch copy — split
    // tokens and all — is the parser's own and dies here)
    *toks = window;
    *ast = p.ast;
    (module.items, p.diags)
}

/// One enriched comment row — the doc-graft's fuel. [`Parser::doc_above`]
/// must answer "which `//` lines attach above this decl?" WITHOUT a
/// text to slice, so the text-derived verdicts are computed ONCE per
/// parse-lane entry (cold or edit window) into these rows: whether the
/// comment starts its line, where the next code byte sits, and how
/// many newlines the whitespace gap after the comment carries.
pub(crate) struct Graft {
    /// the comment's span, marker included
    span: Span,
    /// the rustc doc convention (`///` `//!` `/**` `/*!`)
    doc: bool,
    /// a `//` line comment — only line comments attach (the
    /// Go-convention rule this graft implements)
    line: bool,
    /// only whitespace precedes the comment on its line
    line_initial: bool,
    /// the first non-whitespace byte at/after the comment's end —
    /// `u32::MAX` when only whitespace follows (the EOF edge)
    next_code: u32,
    /// newlines between the comment's end and `next_code`
    nl_after: u32,
}

/// Enrich the lex lane's comment table against the NORMALIZED text —
/// the one place the doc-graft touches text. Called at the parse
/// lane's entry (cold: `Doc::parse`; edit: the refreshed window
/// table), so the grammar-side [`Parser::doc_above`] rides pure rows.
pub(crate) fn graft_table(src: &str, comments: Vec<Comment>) -> Vec<Graft> {
    let mut out = Vec::with_capacity(comments.len());
    // the newline frontier — the lowest byte whose '\n' status is not
    // yet recorded (comments arrive in source order, so one forward
    // walk serves every row)
    let mut frontier = 0usize;
    let mut last_nl: Option<usize> = None;
    for c in comments {
        let lo = c.span.lo as usize;
        let hi = c.span.hi as usize;
        while frontier < lo {
            if src.as_bytes()[frontier] == b'\n' {
                last_nl = Some(frontier);
            }
            frontier += 1;
        }
        let line_start = last_nl.map(|i| i + 1).unwrap_or(0);
        let line_initial = src[line_start..lo].trim().is_empty();
        let (next_code, nl_after) =
            match src[hi..].char_indices().find(|(_, ch)| !ch.is_whitespace()) {
                Some((i, _)) => {
                    let at = hi + i;
                    let nls = src[hi..at].matches('\n').count() as u32;
                    (at as u32, nls)
                }
                None => (u32::MAX, 0),
            };
        out.push(Graft {
            span: c.span,
            doc: c.doc,
            line: src[lo..].starts_with("//"),
            line_initial,
            next_code,
            nl_after,
        });
    }
    out
}

/// a name capture — the ident's interning id AND its span (the parse
/// verdict hover used to recover by token-walking; v2 records it).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Name {
    pub id: crate::IdentId,
    pub span: Span,
}

pub(crate) struct Parser {
    pub(crate) toks: Vec<Token>,
    pub(crate) pos: usize,
    /// debug monotone-cursor witness (C4)
    max_pos: usize,
    pub(crate) diags: Vec<Diag>,
    pub(crate) mode: Mode,
    frames: Vec<Frame>,
    inbox: Option<Done>,
    nest: u32,
    nest_reported: bool,
    expr_depth: u32,
    expr_reported: bool,
    // the capsule store — nodes land here, complete, once
    pub(crate) ast: crate::Ast,
    /// the enriched comment table ([`graft_table`]) — docs graft from
    /// these rows at allocation; the grammar itself never consults
    /// them (D7)
    pub(crate) comments: Vec<Graft>,
    /// hi of the last consumed token — exact item spans end here
    pub(crate) prev_end: u32,
}

// ---- span/cursor helpers (C4: cursor only ever moves forward) ----

impl Parser {
    pub(crate) fn peek(&self, n: usize) -> &Token {
        self.toks
            .get(self.pos + n)
            .unwrap_or_else(|| self.toks.last().unwrap())
    }
    pub(crate) fn tok(&self) -> &Tok {
        &self.peek(0).tok
    }
    pub(crate) fn span(&self) -> Span {
        self.peek(0).span
    }
    pub(crate) fn bump(&mut self) -> Token {
        debug_assert!(self.pos >= self.max_pos, "cursor moved backwards (C4)");
        self.max_pos = self.pos;
        let t = self.peek(0).clone();
        self.prev_end = t.span.hi;
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    /// exact span helper: [lo .. last consumed] (the item-span law,
    /// D6: a node's span ends at its OWN last consumed token)
    pub(crate) fn span_from(&self, lo: u32) -> Span {
        Span::new(lo, self.prev_end.max(lo))
    }

    pub(crate) fn switch_to_hole(&mut self, toks: Vec<Token>) -> (Vec<Token>, usize, u32, u32) {
        let old_toks = std::mem::replace(&mut self.toks, toks);
        let old_pos = std::mem::replace(&mut self.pos, 0);
        self.max_pos = 0;
        let ed = std::mem::replace(&mut self.expr_depth, 0);
        let ne = std::mem::replace(&mut self.nest, 0);
        (old_toks, old_pos, ed, ne)
    }

    pub(crate) fn restore_from_hole(&mut self, toks: Vec<Token>, pos: usize, ed: u32, ne: u32) {
        self.toks = toks;
        self.pos = pos;
        self.max_pos = pos;
        self.expr_depth = ed;
        self.nest = ne;
    }

    pub(crate) fn at_eof(&self) -> bool {
        matches!(self.tok(), Tok::Eof)
    }

    pub(crate) fn at_kw(&self, kw: &str) -> bool {
        matches!(self.tok(), Tok::Ident(s) if s == kw)
    }
    pub(crate) fn at_kw2(&self, kw: &str) -> bool {
        matches!(&self.peek(1).tok, Tok::Ident(s) if s.as_str() == kw)
    }

    pub(crate) fn err(&mut self, span: Span, msg: impl Into<String>) {
        self.diags.push(Diag::new(span, msg));
    }
    pub(crate) fn err_here(&mut self, msg: impl Into<String>) {
        let s = self.span();
        self.err(s, msg);
    }

    pub(crate) fn expect(&mut self, tok: Tok) -> Option<Span> {
        if *self.tok() == tok {
            Some(self.bump().span)
        } else {
            let found = self.peek(0).describe();
            let want = Token {
                tok: tok.clone(),
                span: Span::new(0, 0),
            }
            .describe();
            self.err_here(format!("expected {want}, found {found}"));
            None
        }
    }

    /// the ident consumption — returns the NAME (id + span), the
    /// verdict recorded at the grammar site
    pub(crate) fn expect_ident(&mut self, what: &str) -> Option<Name> {
        if let Tok::Ident(name) = self.tok().clone() {
            let sp = self.bump().span;
            return Some(Name {
                id: self.ast.intern(&name),
                span: sp,
            });
        }
        let found = self.peek(0).describe();
        self.err_here(format!("expected {what}, found {found}"));
        None
    }

    pub(crate) fn eat_punct(&mut self, tok: Tok) -> bool {
        if *self.tok() == tok {
            self.bump();
            true
        } else {
            false
        }
    }

    pub(crate) fn expect_gt(&mut self) -> bool {
        match self.tok().clone() {
            Tok::Gt => {
                self.bump();
                true
            }
            Tok::Shr => {
                let sp = self.peek(0).span;
                self.toks[self.pos] = Token {
                    tok: Tok::Gt,
                    span: Span::new(sp.lo, sp.lo + 1),
                };
                self.toks.insert(
                    self.pos + 1,
                    Token {
                        tok: Tok::Gt,
                        span: Span::new(sp.lo + 1, sp.hi),
                    },
                );
                self.bump();
                true
            }
            _ => {
                let found = self.peek(0).describe();
                self.err_here(format!("expected `>`, found {found}"));
                false
            }
        }
    }

    // ---- diags/fencing (the recovery ladder's plumbing) ----

    /// a diag watermark — `take_since` splits the rows a fenced region
    /// owns (they ride its ErrorNode; the flat lane gets them back at
    /// the parse boundary in order)
    pub(crate) fn mark(&self) -> usize {
        self.diags.len()
    }
    pub(crate) fn diags_since(&self, mark: usize) -> Vec<Diag> {
        if mark < self.diags.len() {
            self.diags[mark..].to_vec()
        } else {
            Vec::new()
        }
    }
    /// FENCE: allocate the ErrorNode covering [tok[start] .. last
    /// consumed], carrying the region's diags. Call AFTER resyncing.
    pub(crate) fn fence(&mut self, start: usize, mark: usize) -> Handle<ErrorNode> {
        let lo = self
            .toks
            .get(start)
            .map(|t| t.span.lo)
            .unwrap_or(self.prev_end);
        let span = Span::new(lo.min(self.prev_end), self.prev_end.max(lo));
        let diags = self.diags_since(mark);
        ErrorNode { span, diags }.alloc_plain(&mut self.ast)
    }

    // ---- docs at allocation (D7) ----

    /// the doc chain directly above `lo` — the Go-convention rules,
    /// byte-identical to the hover lane this replaced. Walks the
    /// ENRICHED rows backwards from `lo`: only `//` line comments that
    /// start their line attach, the gap to the next row (or the decl)
    /// must be whitespace, and past the first row at most one blank
    /// line may separate them.
    pub(crate) fn doc_above(&self, lo: u32) -> DocData {
        let cs = &self.comments;
        let mut out: Vec<CommentData> = Vec::new();
        let mut bound = lo;
        let mut first = true;
        while let Some(c) = cs
            .iter()
            .rev()
            .find(|c| c.span.hi <= bound && c.line && c.line_initial)
        {
            let attaches = c.next_code == bound && (first || c.nl_after <= 1);
            if !attaches {
                break;
            }
            out.push(CommentData {
                span: c.span,
                doc: c.doc,
            });
            bound = c.span.lo;
            first = false;
        }
        out.reverse();
        DocData(out)
    }

    // ---- the driver loop ----

    fn run(&mut self, pre_items: Vec<ItemRef>) -> Handle<ModuleNode> {
        self.frames
            .push(Frame::Module(frame::ModuleFrame::new(pre_items)));
        let mut root = None;
        while let Some(mut frame) = self.frames.pop() {
            let done = self.inbox.take();
            let step = frame.step(self, done);
            match step {
                Step::Push(child) => {
                    self.frames.push(frame);
                    self.push_child(child);
                }
                Step::Pop(d) => {
                    if frame.nests() {
                        self.nest -= 1;
                    }
                    if frame.counts_full_expr() {
                        self.expr_depth -= 1;
                    }
                    match d {
                        Done::Root(h) => root = Some(h),
                        d => self.inbox = Some(d),
                    }
                }
            }
        }
        root.expect("the module frame must complete the parse")
    }

    fn push_child(&mut self, child: Frame) {
        if child.counts_full_expr() && self.expr_depth >= EXPR_MAX {
            if !self.expr_reported {
                self.expr_reported = true;
                self.err(self.span(), "nesting too deep");
            }
            self.skim_to_closer();
            self.inbox = Some(Done::Failed);
            return;
        }
        if child.nests() && self.nest >= NEST_MAX {
            if !self.nest_reported {
                self.nest_reported = true;
                self.err(self.span(), "nesting too deep");
            }
            let d = match child {
                Frame::Block(bf) => {
                    self.skim_balanced();
                    Done::Block(bf.empty(self))
                }
                _ => Done::Failed,
            };
            self.inbox = Some(d);
            return;
        }
        if child.nests() {
            self.nest += 1;
        }
        if child.counts_full_expr() {
            self.expr_depth += 1;
        }
        self.frames.push(child);
    }

    pub(crate) fn expr_nesting(&self) -> u32 {
        self.expr_depth
    }
    pub(crate) fn expr_nesting_diag(&mut self) {
        if !self.expr_reported {
            self.expr_reported = true;
            self.err(self.span(), "nesting too deep");
        }
    }

    fn skim_to_closer(&mut self) {
        let mut depth = 0i32;
        loop {
            match self.tok() {
                Tok::LParen | Tok::LBracket | Tok::LBrace => depth += 1,
                Tok::RParen | Tok::RBracket | Tok::RBrace => {
                    depth -= 1;
                    if depth <= 0 {
                        if depth == 0 {
                            self.bump();
                        }
                        break;
                    }
                }
                Tok::Semi | Tok::Eof => break,
                _ => {}
            }
            self.bump();
        }
    }

    fn skim_balanced(&mut self) {
        let mut depth = 0i32;
        loop {
            match self.tok() {
                Tok::Eof => return,
                Tok::LBrace => depth += 1,
                Tok::RBrace => {
                    if depth == 0 {
                        self.bump();
                        return;
                    }
                    depth -= 1;
                }
                _ => {}
            }
            self.bump();
        }
    }

    pub(crate) fn sync_stmt(&mut self) {
        let mut brace = 0i32;
        loop {
            match self.tok() {
                Tok::Eof => return,
                Tok::Semi => {
                    self.bump();
                    return;
                }
                Tok::LBrace => {
                    brace += 1;
                    self.bump();
                }
                Tok::RBrace => {
                    if brace <= 0 {
                        return; // let the enclosing frame see it
                    }
                    brace -= 1;
                    self.bump();
                    if brace == 0 {
                        return;
                    }
                }
                _ => {
                    self.bump();
                }
            }
        }
    }

    pub(crate) fn sync_item(&mut self) {
        self.sync_stmt();
    }
}

/// fill a decl node's doc at allocation — `with_doc!(p, FnNode { .. })`
macro_rules! with_doc {
    ($p:expr, $node:expr) => {{
        let mut n = $node;
        n.doc = $p.doc_above(n.span.lo);
        n
    }};
}
pub(crate) use with_doc;

/// Reserved words of the grammar — keywords are `Ident`s
/// matched by interner text. Public: the LSP classifier
/// and any tooling that needs the keyword set share this one table.
pub const RESERVED_KW: &[&str] = &[
    "let",
    "const",
    "if",
    "else",
    "while",
    "for",
    "of",
    "return",
    "when",
    "enum",
    "class",
    "struct",
    "interface",
    "impl",
    "requires",
    "use",
    "pub",
    "static",
    "async",
    "await",
    "is",
    "host",
    "fn",
    "true",
    "false",
    "nil",
];

pub fn is_reserved_kw(s: &str) -> bool {
    RESERVED_KW.contains(&s)
}

/// The primitive type names, in canonical order — the ONE table.
pub const PRIMITIVE_TYS: &[&str] = &[
    "bool", "str", "bytes", "nil", "f32", "f64", "i8", "i16", "i32", "i64", "u8", "u16", "u32",
    "u64",
];

pub fn is_primitive_ty(s: &str) -> bool {
    PRIMITIVE_TYS.contains(&s)
}
