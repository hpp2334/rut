//! Statements, blocks, `when` heads, and patterns: frames
//! for the statement grammar — keyword-led dispatch (peek 1), shared
//! when-heads for statement and expression positions, and the pattern
//! grammar. Blocks fence broken statements as `StmtRef::Error` slots
//! beside the survivors.

use rut_ast::{
    BlockNode, BreakNode, ContinueNode, ElseBranch, ExprRef, ForCNode, ForOfNode, Handle, IfNode,
    PatCtorNode, PatElseNode, PatLitNode, PatPathNode, PatRef, PatWildNode, PathSeg, ReturnNode,
    StmtRef, TyRef, WhenArmNode, WhenStmtNode, WhileNode,
};
use rut_lexer::span::Span;
use rut_lexer::token::Tok;

use crate::expr::{AtomFrame, AtomMode, ExprFrame, ExprMode};
use crate::frame::{Done, Frame, Step};
use crate::ty::TypeFrame;
use crate::{Name, Parser};

// ---- blocks ----

pub(crate) struct BlockFrame {
    lo: u32,
    /// a strict block bails when `{` is missing; a lax one diagnoses
    /// and continues (v1's fn/method bodies had already expected the
    /// brace themselves)
    lax: bool,
    stmts: Vec<StmtRef>,
    /// the tail expression: the last statement closed by the block's
    /// `}` (no `;`) — the block's implicit value, Rust-style
    tail: Option<ExprRef>,
    before: usize,
    dmark: usize,
}

impl BlockFrame {
    pub(crate) fn strict(p: &Parser) -> Self {
        BlockFrame {
            lo: p.span().lo,
            lax: false,
            stmts: Vec::new(),
            tail: None,
            before: p.pos,
            dmark: p.mark(),
        }
    }
    /// a lax block expects its own `{` but keeps the caller's `lo` for
    /// the span
    pub(crate) fn lax(p: &Parser, lo: u32) -> Self {
        BlockFrame {
            lo,
            lax: true,
            stmts: Vec::new(),
            tail: None,
            before: p.pos,
            dmark: p.mark(),
        }
    }
    /// the degraded block of the NEST_MAX budget (C3): empty, so the
    /// enclosing item can still complete
    pub(crate) fn empty(&self, p: &mut Parser) -> Handle<BlockNode> {
        BlockNode {
            span: Span::new(self.lo, p.span().hi),
            stmts: Vec::new(),
            tail: None,
        }
        .alloc_plain(&mut p.ast)
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
            let node = BlockNode {
                span: Span::new(self.lo, p.span().hi),
                stmts: std::mem::take(&mut self.stmts),
                tail: self.tail.take(),
            }
            .alloc_plain(&mut p.ast);
            return Step::Pop(Done::Block(node));
        }
        self.before = p.pos;
        self.dmark = p.mark();
        Step::Push(Frame::Stmt(StmtFrame::new()))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match d {
            Done::Stmt(s) => self.stmts.push(s),
            Done::If(h) => self.stmts.push(StmtRef::If(h)),
            Done::TailExpr(e) => self.tail = Some(e),
            Done::Failed => {
                // the stmt-level fence: guarantee progress, resync, the
                // region becomes an ErrorNode IN the block
                if p.pos == self.before {
                    p.bump();
                }
                p.sync_stmt();
                let (before, mark) = (self.before, self.dmark);
                self.stmts.push(StmtRef::Error(p.fence(before, mark)));
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
    while_cond: Option<ExprRef>,
    /// the stmt start + diag watermark (the init-hole fence's extent)
    before: usize,
    dmark: usize,
}

enum StmtStage {
    Dispatch,
    /// `let` — waiting for the optional type
    LetTy {
        is_mut: bool,
        name: Name,
        destructure: Option<Vec<rut_ast::IdentId>>,
    },
    /// `let` — waiting for the initializer
    LetInit {
        is_mut: bool,
        name: Name,
        destructure: Option<Vec<rut_ast::IdentId>>,
        ty: Option<TyRef>,
    },
    WhileCond,
    WhileBody,
    ForIter {
        var: Name,
        is_mut: bool,
    },
    ForInit {
        var: Name,
        is_mut: bool,
    },
    ForCond {
        var: Name,
        is_mut: bool,
        init: ExprRef,
    },
    ForUpdate {
        var: Name,
        is_mut: bool,
        init: ExprRef,
        cond: ExprRef,
    },
    ForBodyOf {
        var: Name,
        is_mut: bool,
        iter: ExprRef,
    },
    ForBodyC {
        var: Name,
        is_mut: bool,
        init: ExprRef,
        cond: ExprRef,
        update: ExprRef,
    },
    RetVal,
    Expr,
    When,
}

impl StmtFrame {
    pub(crate) fn new() -> Self {
        StmtFrame {
            lo: Span::new(0, 0),
            stage: StmtStage::Dispatch,
            while_cond: None,
            before: 0,
            dmark: 0,
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = p.span();
        self.before = p.pos;
        self.dmark = p.mark();
        match p.tok().clone() {
            Tok::Ident(kw) => match kw.as_str() {
                "let" => {
                    p.bump(); // let
                    let is_mut = p.at_kw("mut") && {
                        p.bump();
                        true
                    };
                    // `let (a, b) = ..` — tuple destructuring; the row's
                    // binding name is the FIRST list member (no extra
                    // token is consumed — v1's shape)
                    let mut destructure_first: Option<Name> = None;
                    let destructure = if matches!(p.tok(), Tok::LParen) {
                        p.bump();
                        let mut names = Vec::new();
                        loop {
                            let Some(n) = p.expect_ident("a binding name") else {
                                return Step::Pop(Done::Failed);
                            };
                            if destructure_first.is_none() {
                                destructure_first = Some(n);
                            }
                            names.push(n.id);
                            if !p.eat_punct(Tok::Comma) {
                                break;
                            }
                            if matches!(p.tok(), Tok::RParen) {
                                break; // trailing comma
                            }
                        }
                        p.expect(Tok::RParen);
                        Some(names)
                    } else {
                        None
                    };
                    let name = match destructure_first {
                        Some(n) => n,
                        None => {
                            let Some(name) = p.expect_ident("a binding name") else {
                                return Step::Pop(Done::Failed);
                            };
                            name
                        }
                    };
                    if p.eat_punct(Tok::Colon) {
                        self.stage = StmtStage::LetTy {
                            is_mut,
                            name,
                            destructure,
                        };
                        return Step::Push(Frame::Type(TypeFrame::new(p)));
                    }
                    self.let_eq(p, is_mut, name, destructure, None)
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
                            ReturnNode {
                                span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                                value: None,
                            }
                            .allocate(&mut p.ast),
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
                    Step::Pop(Done::Stmt(
                        BreakNode {
                            span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                        }
                        .allocate(&mut p.ast),
                    ))
                }
                "continue" => {
                    p.bump();
                    p.expect(Tok::Semi);
                    Step::Pop(Done::Stmt(
                        ContinueNode {
                            span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                        }
                        .allocate(&mut p.ast),
                    ))
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
        name: Name,
        destructure: Option<Vec<rut_ast::IdentId>>,
        ty: Option<TyRef>,
    ) -> Step {
        p.expect(Tok::Eq);
        self.stage = StmtStage::LetInit {
            is_mut,
            name,
            destructure,
            ty,
        };
        Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
    }

    #[allow(clippy::too_many_arguments)]
    fn let_done(
        &mut self,
        p: &mut Parser,
        is_mut: bool,
        name: Name,
        destructure: Option<Vec<rut_ast::IdentId>>,
        ty: Option<TyRef>,
        init: ExprRef,
    ) -> Step {
        p.expect(Tok::Semi);
        let node = rut_ast::LetStmtNode {
            span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
            is_mut,
            name: name.id,
            name_span: name.span,
            destructure,
            ty,
            init,
        }
        .allocate(&mut p.ast);
        Step::Pop(Done::Stmt(node))
    }

    fn let_hole(
        &mut self,
        p: &mut Parser,
        is_mut: bool,
        name: Name,
        destructure: Option<Vec<rut_ast::IdentId>>,
        ty: Option<TyRef>,
    ) -> Step {
        // the ladder's part tier: the init failed — the LET survives
        // with an Error slot
        p.sync_stmt();
        let (before, mark) = (self.before, self.dmark);
        let hole = ExprRef::Error(p.fence(before, mark));
        self.let_done(p, is_mut, name, destructure, ty, hole)
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
        let is_mut = p.at_kw("mut") && {
            p.bump();
            true
        };
        let Some(var) = p.expect_ident("a loop variable") else {
            return Step::Pop(Done::Failed);
        };
        // for-of vs for-c: peek `of` vs `=`
        if p.at_kw("of") {
            p.bump();
            self.stage = StmtStage::ForIter { var, is_mut };
            return Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)));
        }
        p.expect(Tok::Eq);
        self.stage = StmtStage::ForInit { var, is_mut };
        Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match (std::mem::replace(&mut self.stage, StmtStage::Dispatch), d) {
            (
                StmtStage::LetTy {
                    is_mut,
                    name,
                    destructure,
                },
                Done::Ty(t),
            ) => self.let_eq(p, is_mut, name, destructure, Some(t)),
            (
                StmtStage::LetInit {
                    is_mut,
                    name,
                    destructure,
                    ty,
                },
                Done::Expr(e),
            ) => self.let_done(p, is_mut, name, destructure, ty, e),
            // the failed init: the let survives with an Error slot
            (
                StmtStage::LetInit {
                    is_mut,
                    name,
                    destructure,
                    ty,
                },
                Done::Failed,
            ) => self.let_hole(p, is_mut, name, destructure, ty),
            (StmtStage::WhileCond, Done::Expr(cond)) => {
                p.expect(Tok::RParen);
                self.while_cond = Some(cond);
                self.stage = StmtStage::WhileBody;
                Step::Push(Frame::Block(BlockFrame::strict(p)))
            }
            (StmtStage::WhileBody, Done::Block(body)) => {
                let cond = self.while_cond.take().expect("while without a condition");
                Step::Pop(Done::Stmt(
                    WhileNode {
                        span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                        cond,
                        body,
                    }
                    .allocate(&mut p.ast),
                ))
            }
            (StmtStage::ForIter { var, is_mut }, Done::Expr(iter)) => {
                p.expect(Tok::RParen);
                self.stage = StmtStage::ForBodyOf { var, is_mut, iter };
                Step::Push(Frame::Block(BlockFrame::strict(p)))
            }
            (StmtStage::ForInit { var, is_mut }, Done::Expr(init)) => {
                p.expect(Tok::Semi);
                self.stage = StmtStage::ForCond { var, is_mut, init };
                Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
            }
            (StmtStage::ForCond { var, is_mut, init }, Done::Expr(cond)) => {
                p.expect(Tok::Semi);
                self.stage = StmtStage::ForUpdate {
                    var,
                    is_mut,
                    init,
                    cond,
                };
                Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
            }
            (
                StmtStage::ForUpdate {
                    var,
                    is_mut,
                    init,
                    cond,
                },
                Done::Expr(update),
            ) => {
                p.expect(Tok::RParen);
                self.stage = StmtStage::ForBodyC {
                    var,
                    is_mut,
                    init,
                    cond,
                    update,
                };
                Step::Push(Frame::Block(BlockFrame::strict(p)))
            }
            (StmtStage::ForBodyOf { var, is_mut, iter }, Done::Block(body)) => {
                Step::Pop(Done::Stmt(
                    ForOfNode {
                        span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                        var: var.id,
                        name_span: var.span,
                        is_mut,
                        iter,
                        body,
                    }
                    .allocate(&mut p.ast),
                ))
            }
            (
                StmtStage::ForBodyC {
                    var,
                    is_mut,
                    init,
                    cond,
                    update,
                },
                Done::Block(body),
            ) => Step::Pop(Done::Stmt(
                ForCNode {
                    span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                    var: var.id,
                    name_span: var.span,
                    is_mut,
                    init,
                    cond,
                    update,
                    body,
                }
                .allocate(&mut p.ast),
            )),
            (StmtStage::RetVal, Done::Expr(v)) => {
                p.expect(Tok::Semi);
                Step::Pop(Done::Stmt(
                    ReturnNode {
                        span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                        value: Some(v),
                    }
                    .allocate(&mut p.ast),
                ))
            }
            (StmtStage::Expr, Done::Expr(e)) => {
                // the tail law: the `;` decides. A `}` IMMEDIATELY after
                // the expression makes it the block's tail (no `;` — the
                // implicit value, Rust-style); anything else still
                // demands the `;`
                if matches!(p.tok(), Tok::RBrace) {
                    return Step::Pop(Done::TailExpr(e));
                }
                p.expect(Tok::Semi);
                Step::Pop(Done::Stmt(StmtRef::Expr(e)))
            }
            (StmtStage::When, Done::WhenParts { scrut, arms }) => Step::Pop(Done::Stmt(
                WhenStmtNode {
                    span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                    scrut,
                    arms,
                }
                .allocate(&mut p.ast),
            )),
            // the `if` child (pushed from Dispatch) completes the statement.
            // The `;` decides (the expression law): a trailing `;` after
            // the statement form only ever DISCARDS
            (StmtStage::Dispatch, Done::If(h)) => {
                p.eat_punct(Tok::Semi);
                Step::Pop(Done::Stmt(StmtRef::If(h)))
            }
            (_, Done::Failed) => Step::Pop(Done::Failed),
            (_, d) => unreachable!("statement frame received the wrong child: {:?}", d),
        }
    }
}

// ---- if / else-if chains ----

pub(crate) struct IfFrame {
    lo: u32,
    stage: IfStage,
    cond: Option<ExprRef>,
    then: Option<Handle<BlockNode>>,
}

enum IfStage {
    Cond,
    Then,
    Else,
}

impl IfFrame {
    pub(crate) fn new() -> Self {
        IfFrame {
            lo: 0,
            stage: IfStage::Cond,
            cond: None,
            then: None,
        }
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
        let node = IfNode {
            span: Span::new(self.lo, p.prev_end.max(self.lo)),
            cond,
            then,
            els,
        }
        .alloc_plain(&mut p.ast);
        Step::Pop(Done::If(node))
    }
}

// ---- `when ( expr ) { arms }` — shared by stmt and expr positions ----

pub(crate) struct WhenFrame {
    lo: Span,
    stage: WhenStage,
    scrut: Option<ExprRef>,
    arms: Vec<Handle<WhenArmNode>>,
    /// the arm under construction
    arm_sp: Span,
    arm_pats: Vec<PatRef>,
    arm_is_block: bool,
    before: usize,
    dmark: usize,
}

enum WhenStage {
    Scrut,
    Arms,
    Pats,
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
            dmark: 0,
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
                            self.step_body(p)
                        } else {
                            Step::Push(Frame::Pattern(PatternFrame::new()))
                        }
                    } else {
                        self.step_body(p)
                    }
                }
                // a failed pattern fences its slot; the arm continues
                Done::Failed => {
                    let (before, mark) = (self.before, self.dmark);
                    self.arm_pats.push(PatRef::Error(p.fence(before, mark)));
                    self.step_body(p)
                }
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
        self.dmark = p.mark();
        self.arm_sp = p.span();
        self.arm_pats = Vec::new();
        self.stage = WhenStage::Pats;
        Step::Push(Frame::Pattern(PatternFrame::new()))
    }

    fn step_body(&mut self, p: &mut Parser) -> Step {
        p.expect(Tok::Arrow);
        // body: `{` ⇒ block arm (peek 1); else expr arm
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
            Done::Block(b) => ExprRef::Block(b),
            Done::Failed => return Step::Pop(Done::Failed),
            _ => unreachable!("when arm body is an expression or block"),
        };
        // comma required between expression arms, optional after block
        // arms (the corpus uses `,` after both)
        let had_comma = p.eat_punct(Tok::Comma);
        if !self.arm_is_block && !had_comma && !matches!(p.tok(), Tok::RBrace) {
            p.err_here("expression arms must be comma-separated");
        }
        let node = WhenArmNode {
            span: p.span(),
            pats: std::mem::take(&mut self.arm_pats),
            body,
        }
        .alloc_plain(&mut p.ast);
        self.arms.push(node);
        if p.pos == self.before {
            p.bump(); // guarantee progress
        }
        self.stage = WhenStage::Arms;
        self.arms_top(p)
    }
}

// ---- patterns ----

pub(crate) struct PatternFrame {
    sp: Span,
    stage: PatStage,
    segs: Vec<PathSeg>,
    genargs: Vec<TyRef>,
    ctor_args: Vec<Option<rut_ast::IdentId>>,
}

enum PatStage {
    /// waiting for a literal (a bare atom)
    Lit,
    /// waiting for a negative literal (a unary-level expression)
    NegLit,
    /// building a (possibly dotted, possibly generic) path
    Path,
    /// inside the path's `<..>`
    GenArgs { seg_name: rut_ast::IdentId },
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
            Tok::Int(..) | Tok::Float(..) | Tok::Bool(_) | Tok::Str(_) | Tok::RawStr(_) => {
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
                Step::Pop(Done::Pat(
                    PatElseNode { span: self.sp }.allocate(&mut p.ast),
                ))
            }
            Tok::Ident(name) if name == "_" => {
                p.bump();
                Step::Pop(Done::Pat(
                    PatWildNode { span: self.sp }.allocate(&mut p.ast),
                ))
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

    /// `::`-separated path segments; generic args pause at a child frame
    /// and resume through `genargs_done` — the loops are iterative (C2).
    fn path_run(&mut self, p: &mut Parser) -> Step {
        loop {
            let ident_sp = p.span();
            let Some(name) = p.expect_ident("a pattern name") else {
                return Step::Pop(Done::Failed);
            };
            let _ = ident_sp;
            self.sp = self.sp.to(name.span);
            if matches!(p.tok(), Tok::Lt) && p.scan_is_generic_args(true) {
                p.bump();
                self.stage = PatStage::GenArgs { seg_name: name.id };
                return self.genargs_top(p);
            }
            self.segs.push(PathSeg {
                name: name.id,
                generics: Vec::new(),
            });
            if !p.eat_punct(Tok::Colon2) {
                return self.path_end(p);
            }
        }
    }

    fn path_resume(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::Colon2) {
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
            Step::Pop(Done::Pat(
                PatPathNode {
                    span: self.sp,
                    segs,
                }
                .allocate(&mut p.ast),
            ))
        }
    }

    fn ctor_loop(&mut self, p: &mut Parser) -> Step {
        loop {
            let rp_sp = p.span();
            if p.eat_punct(Tok::RParen) {
                self.sp = self.sp.to(rp_sp);
                return self.ctor_pop(p);
            }
            let arg_sp = p.span();
            match p.tok().clone() {
                Tok::Ident(b) if b == "_" => {
                    p.bump();
                    self.sp = self.sp.to(arg_sp);
                    self.ctor_args.push(None);
                }
                Tok::Ident(b) => {
                    let sp = p.bump().span;
                    self.sp = self.sp.to(sp);
                    self.ctor_args.push(Some(p.ast.intern(&b)));
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
                if let Some(sp) = p.expect(Tok::RParen) {
                    self.sp = self.sp.to(sp);
                }
                return self.ctor_pop(p);
            }
        }
    }

    fn ctor_pop(&mut self, p: &mut Parser) -> Step {
        let segs = std::mem::take(&mut self.segs);
        let args = std::mem::take(&mut self.ctor_args);
        Step::Pop(Done::Pat(
            PatCtorNode {
                span: self.sp,
                segs,
                args,
            }
            .allocate(&mut p.ast),
        ))
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
        self.segs.push(PathSeg {
            name: seg_name,
            generics,
        });
        self.path_resume(p)
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match d {
            Done::Expr(e) => match self.stage {
                PatStage::Lit | PatStage::NegLit => Step::Pop(Done::Pat(
                    PatLitNode {
                        span: self.sp,
                        expr: e,
                    }
                    .allocate(&mut p.ast),
                )),
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
