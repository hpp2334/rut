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
//! ladder); docs graft at allocation when `include_comments` is set
//! (the grammar never consults comments); node spans end at the item's
//! own terminator (`prev_end` — exact spans).

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

/// The AST layer's opts — one knob: docs in the tree. `recover` is NOT
/// a knob; the ENTRY is the choice (`parse` fail-fast /
/// `parse_with_recover` always a tree).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParseOpts {
    /// graft doc chains onto decl nodes at allocation (D7). The grammar
    /// never consults comments; tree shape never depends on them.
    pub include_comments: bool,
}

/// The pair artifact: the tree + ITS exact text. `src` is the
/// NORMALIZED text every span indexes — the join target, never the
/// caller's raw input. The halves ride named doors (the capsule law):
/// reads borrow, `into_parts` moves.
pub struct Parsed {
    ast: crate::Ast,
    src: String,
}

impl Parsed {
    /// the tree half
    pub fn ast(&self) -> &crate::Ast {
        &self.ast
    }

    /// the text half — the NORMALIZED text every span indexes
    pub fn src(&self) -> &str {
        &self.src
    }

    /// the consuming door — both halves as owned values (the lanes
    /// that store the pair's parts)
    pub fn into_parts(self) -> (crate::Ast, String) {
        (self.ast, self.src)
    }
}

/// The fail-fast entry: `Err` carries the FIRST diag and no tree was
/// built; `Ok` implies a clean tree (`ast.is_clean()`). Takes the
/// source BY VALUE — it moves into the artifact (the pair law:
/// `Parsed::src` is the normalized text every span indexes).
pub fn parse(src: String, mode: Mode, opts: ParseOpts) -> Result<Parsed, Diag> {
    let p = parse_with_recover(src, mode, opts);
    match p.ast.errors().first().cloned() {
        Some(first) => Err(first),
        None => Ok(p),
    }
}

/// The recovering entry: ALWAYS a tree. Unparseable regions are
/// `ErrorNode`s in the lists they belong to; every diag rides
/// `ast.errors()` (the flat view) and the in-stream slots (the
/// positional view). Takes the source BY VALUE and rides
/// [`rut_lexer::lexer::lex_canonical`]: a clean (LF-only) input moves
/// through with ZERO copies, a dirty one pays the one normalize pass —
/// and `Parsed::src` is exactly the text the tokens (and every span)
/// index.
pub fn parse_with_recover(src: String, mode: Mode, opts: ParseOpts) -> Parsed {
    let (normalized, toks, diags, comments) = rut_lexer::lexer::lex_canonical(src, true);
    let comments = if opts.include_comments {
        Some(comments)
    } else {
        None
    };
    let mut p = Parser {
        toks,
        pos: 0,
        max_pos: 0,
        diags: Vec::new(),
        mode,
        frames: Vec::new(),
        inbox: None,
        nest: 0,
        nest_reported: false,
        expr_depth: 0,
        expr_reported: false,
        ast: crate::Ast::new(),
        src: normalized,
        comments,
        prev_end: 0,
    };
    // lex rows have no grammar home: one root-level ErrorNode each,
    // pre-seeded into the module (the ladder's LEX tier)
    let mut pre_items = Vec::new();
    for d in &diags {
        let h = ErrorNode {
            span: d.span,
            diags: vec![d.clone()],
        }
        .alloc_plain(&mut p.ast);
        pre_items.push(ItemRef::Error(h));
    }
    let root = p.run(pre_items);
    // the ordered flat lane: lex rows first (as today), then the parse rows
    let mut all = diags;
    all.append(&mut p.diags);
    p.ast.set_errors(all);
    p.ast.set_root(root);
    Parsed {
        ast: p.ast,
        src: p.src,
    }
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
    /// the NORMALIZED text (spans index it; moved into `Parsed`)
    pub(crate) src: String,
    /// the comment table — `Some` ⇔ include_comments (docs graft)
    pub(crate) comments: Option<Vec<Comment>>,
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
            let want = want.trim_matches('`').to_string();
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
    /// byte-identical to the hover lane this replaces. Empty unless
    /// `include_comments`.
    pub(crate) fn doc_above(&self, lo: u32) -> DocData {
        let Some(cs) = &self.comments else {
            return DocData::default();
        };
        let src = &self.src;
        let line_initial = |c: &Comment| {
            let line_start = src[..c.span.lo as usize]
                .rfind('\n')
                .map(|i| i + 1)
                .unwrap_or(0);
            src[line_start..c.span.lo as usize].trim().is_empty()
        };
        let mut out: Vec<CommentData> = Vec::new();
        let mut bound = lo;
        let mut first = true;
        while let Some(c) = cs.iter().rev().find(|c| {
            c.span.hi <= bound && src[c.span.lo as usize..].starts_with("//") && line_initial(c)
        }) {
            let gap = &src[c.span.hi as usize..bound as usize];
            let attaches =
                gap.chars().all(char::is_whitespace) && (first || gap.matches('\n').count() <= 1);
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
    "mut",
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
    "extern",
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
