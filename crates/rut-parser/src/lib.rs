//! Parser — RFC 0030 §4: the explicit-frame machine (contract C2 — no
//! native recursion).
//!
//! One `run` loop (§4) drives two mechanisms: a **frame stack** for
//! structure — one frame kind per grammar rule, each holding the children
//! it has collected and the stage it suspended at — and an embedded
//! **Pratt engine** for expressions (an operator stack plus an operand
//! stack of `NodeId`s, reductions building nodes bottom-up). A frame that
//! needs a child pushes a frame; a frame that completes pops and hands
//! its result (`Done`) to the parent through the inbox. Host stack usage
//! is constant regardless of input.
//!
//! Contracts honored here: **C1** flat arena AST (built bottom-up),
//! **C3** depth budgets — exceeding one is a normal `Diag`, never a host
//! stack overflow (expressions `EXPR_MAX = 64`, brackets/blocks/
//! `NEST_MAX = 1024`; the expression cap stays at 64 because downstream
//! walks over the tree — dump, typecheck — are themselves recursive in
//! this milestone), **C4** monotone cursor + lookahead discipline (local
//! decisions peek ≤ 4 tokens; the two far decisions — lambda vs paren,
//! generic-call vs `<` — are read-only balancing *scans*, §4.2; there is
//! no checkpoint/rollback API, so backtracking is unrepresentable).

use rut_ast::ast::*;
use rut_lexer::diag::Diag;
use rut_lexer::lexer::lex;
use rut_lexer::span::{Span, NEST_MAX};
use rut_lexer::token::{Tok, Token};

mod expr;
mod frame;
mod item;
mod stmt;
mod ty;

use frame::{Done, Frame, Step};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// `.rut` — full grammar, no `host`/`extern`
    Impl,
    /// `.d.rut` — same grammar + surface declarations, bodies forbidden
    Decl,
}

/// Expression nesting budget (C3). Recursive descent cost ~13 host frames
/// per level, so 64 fit a 1 MiB stack with margin; the frame machine has
/// no host-stack cost of its own, but the *downstream* recursive walks
/// (AST dump, fused typecheck) do — the cap keeps trees they visit shallow.
pub(crate) const EXPR_MAX: u32 = 64;

pub fn parse(src: &str, mode: Mode) -> (Ast, Vec<Diag>) {
    let (toks, mut diags) = lex(src);
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
        nodes: Vec::new(),
        interner: Interner::default(),
    };
    let root = p.run();
    diags.append(&mut p.diags);
    let ast = Ast {
        nodes: p.nodes,
        interner: p.interner,
        root,
    };
    (ast, diags)
}

pub(crate) struct Parser {
    pub(crate) toks: Vec<Token>,
    pub(crate) pos: usize,
    /// debug monotone-cursor witness (C4): `pos` never moves backwards
    /// within one token slice (the f-string hole slice switch resets it)
    max_pos: usize,
    diags: Vec<Diag>,
    pub(crate) mode: Mode,
    frames: Vec<Frame>,
    /// the result a popped child frame handed to its parent
    inbox: Option<Done>,
    /// nesting-kind frames on the stack (blocks, types, unary-only exprs)
    nest: u32,
    nest_reported: bool,
    /// full-expression frames on the stack (the EXPR_MAX budget)
    expr_depth: u32,
    expr_reported: bool,
    // arena
    pub(crate) nodes: Vec<Node>,
    pub(crate) interner: Interner,
}

// ---- span/cursor helpers (C4: cursor only ever moves forward) ----

impl Parser {
    pub(crate) fn peek(&self, n: usize) -> &Token {
        self.toks.get(self.pos + n).unwrap_or_else(|| self.toks.last().unwrap())
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
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    /// switch to an f-string hole's sub-slice (RFC 0030 §4.4) — the one
    /// sanctioned `pos` reset: a different token stream, not backtracking.
    /// Returns the outer slice/position/budgets for the later restore.
    pub(crate) fn switch_to_hole(&mut self, toks: Vec<Token>) -> (Vec<Token>, usize, u32, u32) {
        let old_toks = std::mem::replace(&mut self.toks, toks);
        let old_pos = std::mem::replace(&mut self.pos, 0);
        self.max_pos = 0;
        let ed = std::mem::replace(&mut self.expr_depth, 0);
        let ne = std::mem::replace(&mut self.nest, 0);
        (old_toks, old_pos, ed, ne)
    }

    /// restore the outer token slice and budgets after a hole parsed
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

    /// `at` a keyword (keywords are Idents — RFC 0030 §1)
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
            let want = Token { tok: tok.clone(), span: Span::new(0, 0) }.describe();
            let want = want.trim_matches('`').to_string();
            self.err_here(format!("expected {want}, found {found}"));
            None
        }
    }

    pub(crate) fn expect_ident(&mut self, what: &str) -> Option<IdentId> {
        if let Tok::Ident(name) = self.tok().clone() {
            let sp = self.bump().span;
            let _ = sp;
            return Some(self.interner.intern(&name));
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

    /// Expect `>`, splitting a maximal-munch `>>` (span arithmetic — RFC
    /// 0030 §4.2: `Vec<Vec<i32>>` needs no re-lexing and no glued tokens).
    pub(crate) fn expect_gt(&mut self) -> bool {
        match self.tok().clone() {
            Tok::Gt => {
                self.bump();
                true
            }
            Tok::Shr => {
                let sp = self.peek(0).span;
                // Shr becomes TWO Gt tokens (span arithmetic, RFC 0030 4.2):
                // `Vec<Vec<i32>>` closes both levels
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

    // ---- arena ----
    //
    // Typed constructors: a child slot's type is enforced at every
    // construction site — the parser cannot build a wrong-category link.

    fn push_raw(&mut self, kind: Kind, span: Span) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(Node { span, kind });
        id
    }
    pub(crate) fn item(&mut self, k: ItemKind, span: Span) -> NodeHandle<AnyItem> {
        NodeHandle::new(self.push_raw(Kind::Item(k), span))
    }
    pub(crate) fn stmt(&mut self, k: StmtKind, span: Span) -> NodeHandle<AnyStmt> {
        NodeHandle::new(self.push_raw(Kind::Stmt(k), span))
    }
    pub(crate) fn expr(&mut self, k: ExprKind, span: Span) -> NodeHandle<AnyExpr> {
        NodeHandle::new(self.push_raw(Kind::Expr(k), span))
    }
    pub(crate) fn pat(&mut self, k: PatKind, span: Span) -> NodeHandle<AnyPat> {
        NodeHandle::new(self.push_raw(Kind::Pat(k), span))
    }
    pub(crate) fn typ(&mut self, k: TypeKind, span: Span) -> NodeHandle<AnyTy> {
        NodeHandle::new(self.push_raw(Kind::Type(k), span))
    }
    pub(crate) fn arm(&mut self, k: ArmKind, span: Span) -> NodeHandle<AnyArm> {
        NodeHandle::new(self.push_raw(Kind::Arm(k), span))
    }
    pub(crate) fn member(&mut self, k: MemberKind, span: Span) -> NodeHandle<AnyParam> {
        NodeHandle::new(self.push_raw(Kind::Member(k), span))
    }
    pub(crate) fn field_decl(&mut self, d: FieldDeclData, span: Span) -> NodeHandle<FieldDeclNode> {
        NodeHandle::new(self.push_raw(Kind::Member(MemberKind::FieldDecl(d)), span))
    }
    pub(crate) fn method_decl(&mut self, d: MethodDeclData, span: Span) -> NodeHandle<MethodDeclNode> {
        NodeHandle::new(self.push_raw(Kind::Member(MemberKind::MethodDecl(d)), span))
    }
    pub(crate) fn fn_decl(&mut self, d: FnData, span: Span) -> NodeHandle<FnNode> {
        NodeHandle::new(self.push_raw(Kind::Item(ItemKind::Fn(d)), span))
    }
    pub(crate) fn module(&mut self, items: Vec<NodeHandle<AnyItem>>, span: Span) -> NodeHandle<ModuleNode> {
        NodeHandle::new(self.push_raw(Kind::Item(ItemKind::Module { items }), span))
    }
    pub(crate) fn block(&mut self, stmts: Vec<NodeHandle<AnyStmt>>, span: Span) -> NodeHandle<BlockNode> {
        NodeHandle::new(self.push_raw(Kind::Expr(ExprKind::Block { stmts }), span))
    }
    pub(crate) fn if_stmt(&mut self, cond: NodeHandle<AnyExpr>, then: NodeHandle<BlockNode>, els: Option<ElseBranch>, span: Span) -> NodeHandle<IfNode> {
        NodeHandle::new(self.push_raw(Kind::Stmt(StmtKind::If { cond, then, els }), span))
    }

    // ---- the driver loop (RFC 0030 §4) ----

    fn run(&mut self) -> NodeHandle<ModuleNode> {
        self.frames.push(Frame::Module(ModuleFrame::new()));
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

    /// Push a child frame, enforcing the depth budgets (C3). Over budget
    /// is a normal (latched, once-per-parse) Diag plus a site-specific
    /// degraded result — never a push, never a host stack overflow.
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
            // degraded results mirror the old enter()-failure sites:
            // a block consumes its balanced braces and yields an empty
            // block so the enclosing item can continue; everything else
            // fails and lets the parent's recovery run
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

    /// the EXPR_MAX recovery skim: skip forward to the matching closer
    /// (or statement boundary); resync forward only
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

    /// consume through the matching `}` (block-budget recovery)
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

    // ---- recovery (RFC 0030 §6): resync forward at `;` / `}` / balanced block

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

/// Reserved words of the grammar (RFC 0002 §4) — keywords are `Ident`s
/// matched by interner text (RFC 0002 §4/§5). Public: the LSP classifier
/// and any tooling that needs the keyword set share this one table.
pub fn is_reserved_kw(s: &str) -> bool {
    matches!(s, "let" | "mut" | "if" | "else" | "while" | "for" | "of" | "return" | "when" | "enum" | "class" | "dataclass" | "interface" | "impl" | "requires" | "import" | "pub" | "from" | "static" | "suspend" | "await" | "extern" | "where" | "is" | "host" | "fn" | "true" | "false" | "select")
}

/// The primitive types (RFC 0002 §3) — contextual type names, matched by
/// interner text. Public and canonical: `host primitive` decls (RFC 0029
/// §2) and the LSP classifier share this one table. `string` included —
/// it is a primitive, not a class; its native member surface is declared
/// with `host primitive string { .. }` in the std `.d.rut`. `bytes`
/// (RFC 0004) is the immutable binary primitive alongside `string`.
pub fn is_primitive_ty(s: &str) -> bool {
    matches!(
        s,
        "bool" | "string" | "bytes" | "unit" | "f32" | "f64"
            | "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64"
    )
}

use crate::frame::ModuleFrame;
