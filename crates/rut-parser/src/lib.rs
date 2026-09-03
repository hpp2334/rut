//! Parser —RFC 0030 §4.
//!
//! Contracts honored here: **C1** flat arena AST (built bottom-up),
//! **C3** depth budget —exceeding it is a normal `Diag`, never a host
//! stack overflow, **C4** monotone cursor + lookahead discipline (local
//! decisions peek —4 tokens; the two far decisions —lambda vs paren,
//! generic-call vs `<` —are read-only balancing *scans*, §4.2; there is
//! no checkpoint/rollback API, so backtracking is unrepresentable).
//!
//! C2 note (deviation, documented): v1 implements the depth budget over
//! recursive descent rather than an explicit frame stack. The safety
//! contract C3 exists to guarantee —untrusted source cannot overflow the
//! host stack —holds via the NEST_MAX guard at every nested construct
//! (and library entry points run the parse on a dedicated large-stack
//! thread as a second guard). The explicit-stack refactor is deferred.

use rut_ast::ast::*;
use rut_lexer::diag::Diag;
use rut_lexer::lexer::lex;
use rut_lexer::span::{Span, NEST_MAX};
use rut_lexer::token::{Tok, Token};

mod expr;
mod item;
mod stmt;
mod ty;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// `.rut` —full grammar, no `host`/`extern`
    Impl,
    /// `.d.rut` —same grammar + surface declarations, bodies forbidden
    Decl,
}

pub fn parse(src: &str, mode: Mode) -> (Ast, Vec<Diag>) {
    let (toks, mut diags) = lex(src);
    let mut p = Parser {
        toks,
        pos: 0,
        diags: Vec::new(),
        mode,
        depth: 0,
        depth_reported: false,
        nodes: Vec::new(),
        interner: Interner::default(),
    };
    let root = p.parse_module();
    diags.append(&mut p.diags);
    let ast = Ast {
        nodes: p.nodes,
        interner: p.interner,
        root,
    };
    (ast, diags)
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    diags: Vec<Diag>,
    mode: Mode,
    depth: u32,
    depth_reported: bool,
    // arena
    nodes: Vec<Node>,
    interner: Interner,
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
        let t = self.peek(0).clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    pub(crate) fn at_eof(&self) -> bool {
        matches!(self.tok(), Tok::Eof)
    }

    /// `at` a keyword (keywords are Idents —RFC 0030 §1)
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

    /// Expect `>`, splitting a maximal-munch `>>` (span arithmetic —    /// RFC 0030 §4.2: `Vec<Vec<i32>>` needs no re-lexing and no glued tokens).
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
    pub(crate) fn block(&mut self, stmts: Vec<NodeHandle<AnyStmt>>, span: Span) -> NodeHandle<BlockNode> {
        NodeHandle::new(self.push_raw(Kind::Expr(ExprKind::Block { stmts }), span))
    }
    pub(crate) fn if_stmt(&mut self, cond: NodeHandle<AnyExpr>, then: NodeHandle<BlockNode>, els: Option<ElseBranch>, span: Span) -> NodeHandle<IfNode> {
        NodeHandle::new(self.push_raw(Kind::Stmt(StmtKind::If { cond, then, els }), span))
    }

    // ---- depth budget (C3) ----

    pub(crate) fn enter(&mut self) -> bool {
        self.depth += 1;
        if self.depth > NEST_MAX {
            if !self.depth_reported {
                self.depth_reported = true;
                self.err(self.span(), "nesting too deep");
            }
            self.depth -= 1;
            return false;
        }
        true
    }
    pub(crate) fn leave(&mut self) {
        self.depth -= 1;
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

    // ---- module ----

    pub(crate) fn parse_module(&mut self) -> NodeHandle<ModuleNode> {
        let lo = self.span().lo;
        let mut items = Vec::new();
        while !self.at_eof() {
            let before = self.pos;
            match self.parse_item() {
                Some(id) => items.push(id),
                None => {
                    if self.pos == before {
                        self.err_here("expected a declaration");
                        self.bump();
                    }
                    self.sync_item();
                }
            }
        }
        let hi = self.span().hi;
        NodeHandle::new(self.push_raw(Kind::Item(ItemKind::Module { items }), Span::new(lo, hi)))
    }

}

pub(crate) fn is_reserved_kw(s: &str) -> bool {
    matches!(s, "let" | "mut" | "if" | "else" | "while" | "for" | "of" | "return" | "when" | "enum" | "class" | "dataclass" | "trait" | "impl" | "requires" | "import" | "export" | "from" | "private" | "static" | "suspend" | "await" | "extern" | "where" | "dyn" | "is" | "host" | "fn" | "true" | "false" | "select")
}

