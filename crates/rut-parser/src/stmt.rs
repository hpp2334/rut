//! Statements, blocks, `when` heads, and patterns (RFC 0030 §2): frames
//! for the statement grammar — keyword-led dispatch (peek 1), shared
//! when-heads for statement and expression positions, and the pattern
//! grammar (literals, paths, constructors).

use rut_ast::ast::*;
use rut_lexer::span::Span;
use rut_lexer::token::Tok;

use crate::expr::{AtomFrame, AtomMode, ExprFrame, ExprMode};
use crate::frame::{Done, Frame, Step};
use crate::ty::TypeFrame;
use crate::Parser;

// ---- blocks ----

pub(crate) struct BlockFrame {
    lo: u32,
    /// a strict block bails when `{` is missing (v1's `parse_block_stmt`
    /// `?`); a lax one diagnoses and continues (v1's fn/method bodies had
    /// already expected the brace themselves)
    lax: bool,
    stmts: Vec<NodeHandle<AnyStmt>>,
    before: usize,
}

impl BlockFrame {
    pub(crate) fn strict(p: &Parser) -> Self {
        BlockFrame { lo: p.span().lo, lax: false, stmts: Vec::new(), before: p.pos }
    }
    /// a lax block expects its own `{` but keeps the caller's `lo` for
    /// the span — v1's fn/method bodies were `parse_block_body(lo)` with
    /// the `fn` token's lo and the brace pre-expected by the caller
    pub(crate) fn lax(p: &Parser, lo: u32) -> Self {
        BlockFrame { lo, lax: true, stmts: Vec::new(), before: p.pos }
    }
    /// the degraded block of the NEST_MAX budget (C3): empty, so the
    /// enclosing item can still complete
    pub(crate) fn empty(&self, p: &mut Parser) -> NodeHandle<BlockNode> {
        p.block(Vec::new(), Span::new(self.lo, p.span().hi))
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        if self.lax {
            if let Some(sp) = p.expect(Tok::LBrace) {
                self.lo = sp.lo;
            }
        } else if p.expect(Tok::LBrace).is_none() {
            return Step::Pop(Done::Failed);
        }
        self.next_stmt(p)
    }

    fn next_stmt(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::RBrace) || p.at_eof() {
            let stmts = std::mem::take(&mut self.stmts);
            return Step::Pop(Done::Block(p.block(stmts, Span::new(self.lo, p.span().hi))));
        }
        self.before = p.pos;
        Step::Push(Frame::Stmt(StmtFrame::new()))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match d {
            Done::Stmt(s) => self.stmts.push(s),
            Done::If(h) => self.stmts.push(h.into()),
            Done::Failed => {
                // v1: no progress guarantees a bump, then resync
                if p.pos == self.before {
                    p.bump();
                }
                p.sync_stmt();
            }
            _ => unreachable!("block frame receives statements"),
        }
        self.next_stmt(p)
    }
}

// ---- statements ----

pub(crate) struct StmtFrame {
    lo: Span,
    stage: StmtStage,
    /// the `while` condition, held between its expression and its body
    while_cond: Option<NodeHandle<AnyExpr>>,
}

enum StmtStage {
    Dispatch,
    /// `let` — waiting for the optional type
    LetTy { is_mut: bool, name: IdentId },
    /// `let` — waiting for the initializer
    LetInit { is_mut: bool, name: IdentId, ty: Option<NodeHandle<AnyTy>> },
    WhileCond,
    WhileBody,
    ForIter { var: IdentId },
    ForInit { var: IdentId },
    ForCond { var: IdentId, init: NodeHandle<AnyExpr> },
    ForUpdate { var: IdentId, init: NodeHandle<AnyExpr>, cond: NodeHandle<AnyExpr> },
    ForBodyOf { var: IdentId, iter: NodeHandle<AnyExpr> },
    ForBodyC { var: IdentId, init: NodeHandle<AnyExpr>, cond: NodeHandle<AnyExpr>, update: NodeHandle<AnyExpr> },
    /// `return` — waiting for the value
    RetVal,
    /// a bare expression statement
    Expr,
    /// `when` — waiting for the head
    When,
}

impl StmtFrame {
    pub(crate) fn new() -> Self {
        StmtFrame { lo: Span::new(0, 0), stage: StmtStage::Dispatch, while_cond: None }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = p.span();
        match p.tok().clone() {
            Tok::Ident(kw) => match kw.as_str() {
                "let" => {
                    p.bump(); // let
                    let is_mut = p.at_kw("mut") && {
                        p.bump();
                        true
                    };
                    let Some(name) = p.expect_ident("a binding name") else {
                        return Step::Pop(Done::Failed);
                    };
                    if p.eat_punct(Tok::Colon) {
                        self.stage = StmtStage::LetTy { is_mut, name };
                        return Step::Push(Frame::Type(TypeFrame::new(p)));
                    }
                    self.let_eq(p, is_mut, name, None)
                }
                "if" => Step::Push(Frame::If(IfFrame::new())),
                "while" => {
                    p.bump();
                    p.expect(Tok::LParen);
                    self.stage = StmtStage::WhileCond;
                    Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
                }
                "for" => self.for_header(p),
                "return" => {
                    p.bump();
                    if matches!(p.tok(), Tok::Semi) {
                        p.expect(Tok::Semi);
                        return Step::Pop(Done::Stmt(
                            p.stmt(StmtKind::Return { value: None }, self.lo.to(p.span())),
                        ));
                    }
                    self.stage = StmtStage::RetVal;
                    Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
                }
                "when" => {
                    self.stage = StmtStage::When;
                    Step::Push(Frame::When(WhenFrame::new()))
                }
                "break" => {
                    p.bump();
                    p.expect(Tok::Semi);
                    Step::Pop(Done::Stmt(p.stmt(StmtKind::Break, self.lo)))
                }
                "continue" => {
                    p.bump();
                    p.expect(Tok::Semi);
                    Step::Pop(Done::Stmt(p.stmt(StmtKind::Continue, self.lo)))
                }
                _ => self.expr_stmt(p),
            },
            _ => self.expr_stmt(p),
        }
    }

    fn expr_stmt(&mut self, p: &mut Parser) -> Step {
        self.stage = StmtStage::Expr;
        Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
    }

    fn let_eq(
        &mut self,
        p: &mut Parser,
        is_mut: bool,
        name: IdentId,
        ty: Option<NodeHandle<AnyTy>>,
    ) -> Step {
        p.expect(Tok::Eq);
        self.stage = StmtStage::LetInit { is_mut, name, ty };
        Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
    }

    fn let_done(
        &mut self,
        p: &mut Parser,
        is_mut: bool,
        name: IdentId,
        ty: Option<NodeHandle<AnyTy>>,
        init: NodeHandle<AnyExpr>,
    ) -> Step {
        p.expect(Tok::Semi);
        Step::Pop(Done::Stmt(p.stmt(
            StmtKind::LetStmt { is_mut, name, ty, init },
            self.lo.to(p.span()),
        )))
    }

    fn for_header(&mut self, p: &mut Parser) -> Step {
        p.bump(); // for
        p.expect(Tok::LParen);
        if !p.at_kw("let") {
            p.err_here("expected `let` in a for header");
            p.sync_stmt();
            return Step::Pop(Done::Failed);
        }
        p.bump();
        let Some(var) = p.expect_ident("a loop variable") else {
            return Step::Pop(Done::Failed);
        };
        // for-of vs for-c: peek `of` vs `=` (peek 4, RFC 0030 §4.1)
        if p.at_kw("of") {
            p.bump();
            self.stage = StmtStage::ForIter { var };
            return Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)));
        }
        p.expect(Tok::Eq);
        self.stage = StmtStage::ForInit { var };
        Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match (std::mem::replace(&mut self.stage, StmtStage::Dispatch), d) {
            (StmtStage::LetTy { is_mut, name }, Done::Ty(t)) => self.let_eq(p, is_mut, name, Some(t)),
            (StmtStage::LetInit { is_mut, name, ty }, Done::Expr(e)) => {
                self.let_done(p, is_mut, name, ty, e)
            }
            (StmtStage::WhileCond, Done::Expr(cond)) => {
                p.expect(Tok::RParen);
                self.while_cond = Some(cond);
                self.stage = StmtStage::WhileBody;
                Step::Push(Frame::Block(BlockFrame::strict(p)))
            }
            (StmtStage::WhileBody, Done::Block(body)) => {
                let cond = self.while_cond.take().expect("while without a condition");
                Step::Pop(Done::Stmt(p.stmt(
                    StmtKind::While { cond, body },
                    self.lo.to(p.span()),
                )))
            }
            (StmtStage::ForIter { var }, Done::Expr(iter)) => {
                p.expect(Tok::RParen);
                self.stage = StmtStage::ForBodyOf { var, iter };
                Step::Push(Frame::Block(BlockFrame::strict(p)))
            }
            (StmtStage::ForInit { var }, Done::Expr(init)) => {
                p.expect(Tok::Semi);
                self.stage = StmtStage::ForCond { var, init };
                Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
            }
            (StmtStage::ForCond { var, init }, Done::Expr(cond)) => {
                p.expect(Tok::Semi);
                self.stage = StmtStage::ForUpdate { var, init, cond };
                Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
            }
            (StmtStage::ForUpdate { var, init, cond }, Done::Expr(update)) => {
                p.expect(Tok::RParen);
                self.stage = StmtStage::ForBodyC { var, init, cond, update };
                Step::Push(Frame::Block(BlockFrame::strict(p)))
            }
            (StmtStage::ForBodyOf { var, iter }, Done::Block(body)) => Step::Pop(Done::Stmt(
                p.stmt(StmtKind::ForOf { var, iter, body }, self.lo.to(p.span())),
            )),
            (StmtStage::ForBodyC { var, init, cond, update }, Done::Block(body)) => {
                Step::Pop(Done::Stmt(p.stmt(
                    StmtKind::ForC { var, init, cond, update, body },
                    self.lo.to(p.span()),
                )))
            }
            (StmtStage::RetVal, Done::Expr(v)) => {
                p.expect(Tok::Semi);
                Step::Pop(Done::Stmt(p.stmt(
                    StmtKind::Return { value: Some(v) },
                    self.lo.to(p.span()),
                )))
            }
            (StmtStage::Expr, Done::Expr(e)) => {
                p.expect(Tok::Semi);
                Step::Pop(Done::Stmt(p.stmt(StmtKind::ExprStmt(e), self.lo.to(p.span()))))
            }
            (StmtStage::When, Done::WhenParts { scrut, arms }) => Step::Pop(Done::Stmt(p.stmt(
                StmtKind::WhenStmt { scrut, arms },
                self.lo.to(p.span()),
            ))),
            // the `if` child (pushed from Dispatch) completes the statement
            (StmtStage::Dispatch, Done::If(h)) => Step::Pop(Done::Stmt(h.into())),
            (_, Done::Failed) => Step::Pop(Done::Failed),
            (_, d) => unreachable!("statement frame received the wrong child: {:?}", d),
        }
    }
}

// ---- if / else-if chains (RFC 0008 §1) ----

pub(crate) struct IfFrame {
    lo: u32,
    stage: IfStage,
    cond: Option<NodeHandle<AnyExpr>>,
    then: Option<NodeHandle<BlockNode>>,
}

enum IfStage {
    Cond,
    Then,
    Else,
}

impl IfFrame {
    pub(crate) fn new() -> Self {
        IfFrame { lo: 0, stage: IfStage::Cond, cond: None, then: None }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = p.bump().span.lo; // if
        p.expect(Tok::LParen);
        Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match self.stage {
            IfStage::Cond => match d {
                Done::Expr(cond) => {
                    p.expect(Tok::RParen);
                    self.cond = Some(cond);
                    self.stage = IfStage::Then;
                    Step::Push(Frame::Block(BlockFrame::strict(p)))
                }
                Done::Failed => Step::Pop(Done::Failed),
                _ => unreachable!("if condition is an expression"),
            },
            IfStage::Then => match d {
                Done::Block(then) => {
                    self.then = Some(then);
                    self.stage = IfStage::Else;
                    self.else_branch(p)
                }
                Done::Failed => Step::Pop(Done::Failed),
                _ => unreachable!("if then-branch is a block"),
            },
            IfStage::Else => match d {
                Done::Block(els) => self.finish(p, Some(ElseBranch::Block(els))),
                Done::If(els) => self.finish(p, Some(ElseBranch::If(els))),
                Done::Failed => Step::Pop(Done::Failed),
                _ => unreachable!("if else-branch is a block or a nested if"),
            },
        }
    }

    fn else_branch(&mut self, p: &mut Parser) -> Step {
        if p.at_kw("else") {
            p.bump();
            if p.at_kw("if") {
                Step::Push(Frame::If(IfFrame::new()))
            } else {
                Step::Push(Frame::Block(BlockFrame::strict(p)))
            }
        } else {
            self.finish(p, None)
        }
    }

    fn finish(&mut self, p: &mut Parser, els: Option<ElseBranch>) -> Step {
        let cond = self.cond.take().expect("if without a condition");
        let then = self.then.take().expect("if without a then-block");
        Step::Pop(Done::If(p.if_stmt(cond, then, els, Span::new(self.lo, p.span().hi))))
    }
}

// ---- `when ( expr ) { arms }` — shared by stmt and expr positions ----

pub(crate) struct WhenFrame {
    lo: Span,
    stage: WhenStage,
    scrut: Option<NodeHandle<AnyExpr>>,
    arms: Vec<NodeHandle<AnyArm>>,
    /// the arm under construction
    arm_sp: Span,
    arm_pats: Vec<NodeHandle<AnyPat>>,
    arm_is_block: bool,
    before: usize,
}

enum WhenStage {
    Scrut,
    Arms,
    /// collecting the arm's patterns (comma list until `->`)
    Pats,
    /// waiting for the arm body (block or expression)
    Body,
}

impl WhenFrame {
    pub(crate) fn new() -> Self {
        WhenFrame {
            lo: Span::new(0, 0),
            stage: WhenStage::Scrut,
            scrut: None,
            arms: Vec::new(),
            arm_sp: Span::new(0, 0),
            arm_pats: Vec::new(),
            arm_is_block: false,
            before: 0,
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = p.span();
        p.bump(); // when
        p.expect(Tok::LParen);
        Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match self.stage {
            WhenStage::Scrut => match d {
                Done::Expr(scrut) => {
                    p.expect(Tok::RParen);
                    p.expect(Tok::LBrace);
                    self.scrut = Some(scrut);
                    self.stage = WhenStage::Arms;
                    self.arms_top(p)
                }
                Done::Failed => Step::Pop(Done::Failed),
                _ => unreachable!("when scrutinee is an expression"),
            },
            WhenStage::Pats => match d {
                Done::Pat(pat) => {
                    self.arm_pats.push(pat);
                    if p.eat_punct(Tok::Comma) {
                        if matches!(p.tok(), Tok::Arrow) {
                            self.to_body(p)
                        } else {
                            Step::Push(Frame::Pattern(PatternFrame::new()))
                        }
                    } else {
                        self.to_body(p)
                    }
                }
                // v1: a failed pattern breaks the pattern list
                Done::Failed => self.to_body(p),
                _ => unreachable!("when-arm patterns are patterns"),
            },
            WhenStage::Body => self.arm_body(p, d),
            WhenStage::Arms => unreachable!("when arms stage has no pending child"),
        }
    }

    fn arms_top(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::RBrace) {
            let scrut = self.scrut.take().expect("when without a scrutinee");
            let arms = std::mem::take(&mut self.arms);
            return Step::Pop(Done::WhenParts { scrut, arms });
        }
        self.before = p.pos;
        self.arm_sp = p.span();
        self.arm_pats = Vec::new();
        self.stage = WhenStage::Pats;
        Step::Push(Frame::Pattern(PatternFrame::new()))
    }

    fn to_body(&mut self, p: &mut Parser) -> Step {
        p.expect(Tok::Arrow);
        // body: `{` ⇒ block arm (peek 1 — RFC 0030 §4.1); else expr arm
        self.arm_is_block = matches!(p.tok(), Tok::LBrace);
        self.stage = WhenStage::Body;
        if self.arm_is_block {
            Step::Push(Frame::Block(BlockFrame::strict(p)))
        } else {
            Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
        }
    }

    fn arm_body(&mut self, p: &mut Parser, d: Done) -> Step {
        let body = match d {
            Done::Expr(e) => e,
            Done::Block(b) => b.into(),
            Done::Failed => return Step::Pop(Done::Failed),
            _ => unreachable!("when arm body is an expression or block"),
        };
        // comma required between expression arms, optional after block
        // arms (RFC 0008 §2 — the corpus uses `,` after both)
        let had_comma = p.eat_punct(Tok::Comma);
        if !self.arm_is_block && !had_comma && !matches!(p.tok(), Tok::RBrace) {
            p.err_here("expression arms must be comma-separated (RFC 0008 §2)");
        }
        let node = p.arm(
            ArmKind::WhenArm { pats: std::mem::take(&mut self.arm_pats), body },
            p.span(),
        );
        self.arms.push(node);
        if p.pos == self.before {
            p.bump(); // guarantee progress
        }
        self.stage = WhenStage::Arms;
        self.arms_top(p)
    }
}

// ---- patterns (RFC 0008 §2) ----

pub(crate) struct PatternFrame {
    sp: Span,
    stage: PatStage,
    segs: Vec<PathSeg>,
    genargs: Vec<NodeHandle<AnyTy>>,
    ctor_args: Vec<Option<IdentId>>,
}

enum PatStage {
    /// waiting for a literal (a bare atom)
    Lit,
    /// waiting for a negative literal (a unary-level expression)
    NegLit,
    /// building a (possibly dotted, possibly generic) path
    Path,
    /// inside the path's `<..>`
    GenArgs { seg_name: IdentId },
}

impl PatternFrame {
    pub(crate) fn new() -> Self {
        PatternFrame {
            sp: Span::new(0, 0),
            stage: PatStage::Path,
            segs: Vec::new(),
            genargs: Vec::new(),
            ctor_args: Vec::new(),
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.sp = p.span();
        match p.tok().clone() {
            Tok::Int(..) | Tok::Float(..) | Tok::Bool(_) | Tok::Char(_) | Tok::Str(_) | Tok::RawStr(_) => {
                self.stage = PatStage::Lit;
                Step::Push(Frame::Atom(AtomFrame::new(self.sp, AtomMode::Bare)))
            }
            Tok::Minus => {
                // negative literal pattern
                self.stage = PatStage::NegLit;
                Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::UnaryOnly)))
            }
            Tok::Ident(kw) if kw == "else" => {
                p.bump();
                Step::Pop(Done::Pat(p.pat(PatKind::PatElse, self.sp)))
            }
            Tok::Ident(name) if name == "_" => {
                p.bump();
                Step::Pop(Done::Pat(p.pat(PatKind::PatWild, self.sp)))
            }
            Tok::Ident(_) => {
                // path, possibly with generic args, possibly a constructor
                self.stage = PatStage::Path;
                self.path_run(p)
            }
            _ => {
                let found = p.peek(0).describe();
                p.err_here(format!("expected a pattern, found {found}"));
                Step::Pop(Done::Failed)
            }
        }
    }

    /// dotted path segments; generic args suspend to a child frame and
    /// resume through `genargs_done` — the loops are iterative (C2)
    fn path_run(&mut self, p: &mut Parser) -> Step {
        loop {
            let Some(name) = p.expect_ident("a pattern name") else {
                return Step::Pop(Done::Failed);
            };
            if matches!(p.tok(), Tok::Lt) && p.scan_is_generic_args(true) {
                p.bump();
                self.stage = PatStage::GenArgs { seg_name: name };
                return self.genargs_top(p);
            }
            self.segs.push(PathSeg { name, generics: Vec::new() });
            if !p.eat_punct(Tok::Dot) {
                return self.path_end(p);
            }
        }
    }

    fn path_resume(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::Dot) {
            self.path_run(p)
        } else {
            self.path_end(p)
        }
    }

    fn path_end(&mut self, p: &mut Parser) -> Step {
        if matches!(p.tok(), Tok::LParen) {
            // constructor pattern: args are bindings or `_`
            p.bump();
            self.ctor_loop(p)
        } else {
            let segs = std::mem::take(&mut self.segs);
            Step::Pop(Done::Pat(p.pat(PatKind::PatPath { segs }, self.sp)))
        }
    }

    fn ctor_loop(&mut self, p: &mut Parser) -> Step {
        loop {
            if p.eat_punct(Tok::RParen) {
                return self.ctor_pop(p);
            }
            match p.tok().clone() {
                Tok::Ident(b) if b == "_" => {
                    p.bump();
                    self.ctor_args.push(None);
                }
                Tok::Ident(b) => {
                    p.bump();
                    self.ctor_args.push(Some(p.interner.intern(&b)));
                }
                _ => {
                    let found = p.peek(0).describe();
                    p.err_here(format!(
                        "expected a binding name in a constructor pattern, found {found}"
                    ));
                    self.ctor_args.push(None);
                    p.bump();
                }
            }
            if !p.eat_punct(Tok::Comma) {
                p.expect(Tok::RParen);
                return self.ctor_pop(p);
            }
        }
    }

    fn ctor_pop(&mut self, p: &mut Parser) -> Step {
        let segs = std::mem::take(&mut self.segs);
        let args = std::mem::take(&mut self.ctor_args);
        Step::Pop(Done::Pat(p.pat(PatKind::PatCtor { segs, args }, self.sp)))
    }

    fn genargs_top(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::Gt) {
            return self.genargs_done(p);
        }
        Step::Push(Frame::Type(TypeFrame::new(p)))
    }

    fn genargs_done(&mut self, p: &mut Parser) -> Step {
        let seg_name = match &mut self.stage {
            PatStage::GenArgs { seg_name } => *seg_name,
            _ => unreachable!(),
        };
        let generics = std::mem::take(&mut self.genargs);
        self.stage = PatStage::Path;
        self.segs.push(PathSeg { name: seg_name, generics });
        self.path_resume(p)
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match d {
            Done::Expr(e) => match self.stage {
                PatStage::Lit | PatStage::NegLit => {
                    Step::Pop(Done::Pat(p.pat(PatKind::PatLit(e), self.sp)))
                }
                _ => unreachable!("pattern frame received a stray expression"),
            },
            Done::Ty(t) => {
                debug_assert!(matches!(self.stage, PatStage::GenArgs { .. }));
                self.genargs.push(t);
                if p.eat_punct(Tok::Comma) {
                    self.genargs_top(p)
                } else {
                    p.expect_gt();
                    self.genargs_done(p)
                }
            }
            Done::Failed => Step::Pop(Done::Failed),
            _ => unreachable!("pattern frame receives expressions or types"),
        }
    }
}
