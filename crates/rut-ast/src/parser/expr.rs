//! Expressions: the embedded Pratt engine — an operator
//! stack plus an operand stack of expr SLOTS, reductions popping two
//! operands and pushing one node, bottom-up. Binding powers follow
//! the §4 table. Atoms (primary plus postfix chains) are built by
//! `AtomFrame`; the two §4.2 far-decision scans live in `ty.rs`.
//!
//! v2: operands are `ExprRef` slots; a failed operand under a pending
//! operator becomes an `Error` SLOT — the binary completes with its
//! hole (the recovery ladder's part tier).

use crate::{
    ArrayLitNode, ArrayRepeatNode, AssignNode, AsyncBlockNode, AwaitNode, BinOp, BinaryNode,
    CallNode, CastNode, ExprRef, FPartAst, FStrNode, FieldNode, IfExprNode, IndexNode, IsNode,
    LambdaNode, Lit, LitNode, MethodNode, ParamNode, ParamRef, PathNode, PathSeg, StructLitNode,
    TryNode, TupleNode, UnOp, UnaryNode, WhenExprNode,
};
use rut_lexer::span::Span;
use rut_lexer::token::{FPart, FStrTok, FloatSuffix, Tok, Token};

use crate::parser::frame::{Done, Frame, Step};
use crate::parser::item::ParamsFrame;
use crate::parser::stmt::{BlockFrame, IfFrame, WhenFrame};
use crate::parser::ty::TypeFrame;
use crate::parser::{is_reserved_kw, Parser, EXPR_MAX};
use rut_core::sym;

/// The ten numeric primitive names — the only spellings an `as` cast
/// accepts as its right-hand side.
fn is_numeric_prim(s: &str) -> bool {
    matches!(
        s,
        "u8" | "u16" | "u32" | "u64" | "i8" | "i16" | "i32" | "i64" | "f32" | "f64"
    )
}

fn peek_is_numeric_prim(p: &Parser) -> bool {
    matches!(p.peek(1).tok, Tok::Ident(ref n) if is_numeric_prim(n))
}

/// how the frame was entered — `parse_expr` vs `parse_unary` in v1
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExprMode {
    /// full expression: lambdas, assignment, binary operators
    Full,
    /// one unary-level operand: prefix sweep + postfix atom only
    UnaryOnly,
}

#[derive(Clone, Copy)]
enum Pfx {
    Un(UnOp),
    Await,
}

struct OpEntry {
    op: BinOp,
    bp: u8,
}

pub(crate) struct ExprFrame {
    pub(crate) mode: ExprMode,
    lo: Span,
    stage: ExprStage,
    ops: Vec<OpEntry>,
    operands: Vec<ExprRef>,
    /// prefix operators swept for the operand being fetched
    prefixes: Vec<(Pfx, Span)>,
    /// the binding level of the last <=6-level operator shifted — the
    /// non-associativity scope for relational/`is`
    last_level: u8,
    pending_assign: Option<BinOp>,
    /// the operand fetch's start + diag watermark (the Error slot's
    /// extent when the fetch fails under a pending operator)
    operand_pos: usize,
    operand_mark: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExprStage {
    Start,
    /// waiting for an atom — the operand
    Operand,
    /// waiting for the `is` right-hand type
    IsTy,
    /// waiting for the `as` right-hand type (the numeric cast)
    AsTy,
    /// waiting for the right-hand side of an assignment
    AssignRhs,
}

impl ExprFrame {
    pub(crate) fn new(p: &Parser, mode: ExprMode) -> Self {
        ExprFrame {
            mode,
            lo: p.span(),
            stage: ExprStage::Start,
            ops: Vec::new(),
            operands: Vec::new(),
            prefixes: Vec::new(),
            last_level: 0,
            pending_assign: None,
            operand_pos: p.pos,
            operand_mark: p.mark(),
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        debug_assert!(matches!(self.stage, ExprStage::Start));
        if self.mode == ExprMode::Full {
            // lambda scan 1: `( params ) (: Type)? =>` — read-only (§4.2)
            if matches!(p.tok(), Tok::LParen) && p.scan_is_lambda() {
                self.stage = ExprStage::Operand;
                return Step::Push(Frame::Lambda(LambdaFrame::paren(self.lo)));
            }
            // lambda scan 2: single-ident form `x =>`
            if let Tok::Ident(name) = p.tok().clone() {
                if !is_reserved_kw(&name)
                    && name != "else"
                    && matches!(p.peek(1).tok, Tok::FatArrow)
                {
                    let sp = p.span();
                    p.bump(); // ident
                    p.bump(); // =>
                    let id = p.ast.intern(&name);
                    let param = ParamNode {
                        span: sp,
                        is_mut: false,
                        name: id,
                        name_span: sp,
                        ty: None,
                    }
                    .allocate(&mut p.ast);
                    self.stage = ExprStage::Operand;
                    return Step::Push(Frame::Lambda(LambdaFrame::single(self.lo, vec![param])));
                }
                // anonymous closure: `fn (params) -> T { .. }`
                if name == "fn" && matches!(p.peek(1).tok, Tok::LParen) {
                    p.bump(); // fn
                    self.stage = ExprStage::Operand;
                    return Step::Push(Frame::Lambda(LambdaFrame::anon(self.lo)));
                }
            }
        }
        self.fetch_operand(p)
    }

    /// sweep prefix operators, then fetch an atom.
    fn fetch_operand(&mut self, p: &mut Parser) -> Step {
        self.stage = ExprStage::Operand;
        self.operand_pos = p.pos;
        self.operand_mark = p.mark();
        if let Some(f) = self.sweep(p) {
            return Step::Push(f);
        }
        Step::Push(Frame::Atom(AtomFrame::new(self.lo, AtomMode::Postfix)))
    }

    fn sweep(&mut self, p: &mut Parser) -> Option<Frame> {
        loop {
            // the removed `*x` / `&x` prefix forms diagnose themselves
            match p.tok() {
                Tok::Star | Tok::Amp => {
                    p.err_here("`*x` / `&x` were removed — bindings share by reference now — pass `x` directly");
                    p.bump();
                    continue;
                }
                _ => {}
            }
            let pfx = match p.tok() {
                Tok::Minus => Some(Pfx::Un(UnOp::Neg)),
                Tok::Bang => Some(Pfx::Un(UnOp::Not)),
                Tok::Tilde => Some(Pfx::Un(UnOp::BitNot)),
                _ if p.at_kw("await") => Some(Pfx::Await),
                _ => None,
            };
            let Some(pfx) = pfx else { break };
            // C3: unary chains carry the expression budget
            if p.expr_nesting() + self.prefixes.len() as u32 >= EXPR_MAX {
                p.expr_nesting_diag();
                p.bump();
                continue;
            }
            self.prefixes.push((pfx, p.span()));
            p.bump();
        }
        None
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match d {
            Done::Expr(e) => match self.stage {
                ExprStage::Operand => {
                    // wrap the swept prefixes, innermost first
                    let mut e = e;
                    while let Some((pfx, sp)) = self.prefixes.pop() {
                        e = match pfx {
                            Pfx::Un(op) => UnaryNode {
                                span: Span::new(sp.lo, e.span(&p.ast).hi.max(sp.lo)),
                                op,
                                expr: e,
                            }
                            .allocate(&mut p.ast),
                            Pfx::Await => AwaitNode {
                                span: Span::new(sp.lo, e.span(&p.ast).hi.max(sp.lo)),
                                expr: e,
                            }
                            .allocate(&mut p.ast),
                        };
                    }
                    if self.mode == ExprMode::UnaryOnly {
                        return Step::Pop(Done::Expr(e));
                    }
                    self.operands.push(e);
                    self.oploop(p)
                }
                ExprStage::AssignRhs => {
                    let target = self.operands.pop().expect("assignment without a target");
                    let sp = target.span(&p.ast);
                    match target {
                        ExprRef::Path(_) | ExprRef::Field(_) | ExprRef::Index(_) => {}
                        _ => {
                            p.err(
                                sp,
                                "invalid assignment target —expected a path, field, or index",
                            );
                        }
                    }
                    let node = AssignNode {
                        span: Span::new(sp.lo, p.prev_end.max(sp.lo)),
                        op: self.pending_assign,
                        target,
                        value: e,
                    }
                    .allocate(&mut p.ast);
                    self.operands.push(node);
                    self.oploop(p)
                }
                _ => unreachable!("expr frame received an operand at the wrong stage"),
            },
            Done::Ty(t) => {
                let cast = matches!(self.stage, ExprStage::AsTy);
                debug_assert!(matches!(self.stage, ExprStage::IsTy | ExprStage::AsTy));
                let lhs = self
                    .operands
                    .pop()
                    .expect("`is`/`as` without a left-hand side");
                let sp = lhs.span(&p.ast);
                let node = if cast {
                    CastNode {
                        span: Span::new(sp.lo, p.prev_end.max(sp.lo)),
                        expr: lhs,
                        ty: t,
                    }
                    .allocate(&mut p.ast)
                } else {
                    IsNode {
                        span: Span::new(sp.lo, p.prev_end.max(sp.lo)),
                        expr: lhs,
                        ty: t,
                    }
                    .allocate(&mut p.ast)
                };
                self.operands.push(node);
                if !cast {
                    // the cast sits above the relational non-associativity
                    // scope (level 12), so only `is` marks it
                    self.last_level = 6;
                }
                self.oploop(p)
            }
            Done::Failed => {
                // a failed operand: under a PENDING operator it becomes
                // an Error SLOT (the binary completes with its hole);
                // otherwise the failure propagates to the parent's fence
                if self.ops.is_empty() && self.stage == ExprStage::Operand {
                    return Step::Pop(Done::Failed);
                }
                let (pos, mark) = (self.operand_pos, self.operand_mark);
                self.operands.push(ExprRef::Error(p.fence(pos, mark)));
                if self.stage == ExprStage::AssignRhs {
                    let target = self.operands.pop().expect("assignment without a target");
                    let sp = target.span(&p.ast);
                    let node = AssignNode {
                        span: Span::new(sp.lo, p.prev_end.max(sp.lo)),
                        op: self.pending_assign,
                        target,
                        value: ExprRef::Error(p.fence(pos, mark)),
                    }
                    .allocate(&mut p.ast);
                    self.operands.push(node);
                }
                self.oploop(p)
            }
            _ => unreachable!("expr frame receives expressions or types"),
        }
    }

    /// the operator step: shift one binary/assign/`is` operator with
    /// precedence-driven reduction, or finish by reducing everything
    fn oploop(&mut self, p: &mut Parser) -> Step {
        let (op, bp, level) = match p.tok() {
            Tok::PipePipe => (BinOp::Or, 3u8, 3u8),
            Tok::AmpAmp => (BinOp::And, 4, 4),
            Tok::EqEq => (BinOp::Eq, 5, 5),
            Tok::NotEq => (BinOp::Ne, 5, 5),
            Tok::Lt => (BinOp::Lt, 6, 6),
            Tok::Gt => (BinOp::Gt, 6, 6),
            Tok::LtEq => (BinOp::Le, 6, 6),
            Tok::GtEq => (BinOp::Ge, 6, 6),
            Tok::Pipe => (BinOp::BitOr, 7, 7),
            Tok::Caret => (BinOp::BitXor, 7, 7),
            Tok::Amp => (BinOp::BitAnd, 8, 8),
            Tok::Shl => (BinOp::Shl, 9, 9),
            Tok::Shr => (BinOp::Shr, 9, 9),
            Tok::Plus => (BinOp::Add, 10, 10),
            Tok::Minus => (BinOp::Sub, 10, 10),
            Tok::Star => (BinOp::Mul, 11, 11),
            Tok::Slash => (BinOp::Div, 11, 11),
            Tok::Percent => (BinOp::Mod, 11, 11),
            Tok::Eq => return self.shift_assign(p, None),
            Tok::PlusEq => return self.shift_assign(p, Some(BinOp::Add)),
            Tok::MinusEq => return self.shift_assign(p, Some(BinOp::Sub)),
            Tok::StarEq => return self.shift_assign(p, Some(BinOp::Mul)),
            Tok::SlashEq => return self.shift_assign(p, Some(BinOp::Div)),
            Tok::PercentEq => return self.shift_assign(p, Some(BinOp::Mod)),
            Tok::AmpEq => return self.shift_assign(p, Some(BinOp::BitAnd)),
            Tok::PipeEq => return self.shift_assign(p, Some(BinOp::BitOr)),
            Tok::CaretEq => return self.shift_assign(p, Some(BinOp::BitXor)),
            Tok::ShlEq => return self.shift_assign(p, Some(BinOp::Shl)),
            Tok::ShrEq => return self.shift_assign(p, Some(BinOp::Shr)),
            _ if p.at_kw("is") => {
                // `is` takes a TYPE as its right-hand side —
                // non-associative at level 6
                if self.last_level == 6 {
                    return self.finish_op(p);
                }
                p.bump(); // the `is` keyword
                self.reduce_while(p, 6);
                self.last_level = 6;
                self.stage = ExprStage::IsTy;
                return Step::Push(Frame::Type(TypeFrame::new(p)));
            }
            _ if p.at_kw("as") && peek_is_numeric_prim(p) => {
                // `expr as T` — the numeric cast
                p.bump(); // the `as` keyword
                self.reduce_while(p, 12);
                self.stage = ExprStage::AsTy;
                return Step::Push(Frame::Type(TypeFrame::new(p)));
            }
            _ => return self.finish_op(p),
        };
        // non-associativity scope: one relational per context
        if level == 6 && self.last_level == 6 {
            return self.finish_op(p);
        }
        p.bump(); // the operator token
        self.reduce_while(p, bp);
        self.ops.push(OpEntry { op, bp });
        if level <= 6 {
            self.last_level = level;
        }
        self.fetch_operand(p)
    }

    /// reduce the pending operator stack and hand the finished expression up
    fn finish_op(&mut self, p: &mut Parser) -> Step {
        while !self.ops.is_empty() {
            self.reduce_one(p);
        }
        Step::Pop(Done::Expr(
            self.operands.pop().expect("expr frame without a result"),
        ))
    }

    fn shift_assign(&mut self, p: &mut Parser, op: Option<BinOp>) -> Step {
        p.bump(); // the assignment operator token
        while let Some(top) = self.ops.last() {
            if top.bp <= 1 {
                break;
            }
            self.reduce_one(p);
        }
        self.last_level = 1;
        self.pending_assign = op;
        self.stage = ExprStage::AssignRhs;
        Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
    }

    fn reduce_while(&mut self, p: &mut Parser, bp: u8) {
        while self.ops.last().map(|t| t.bp >= bp).unwrap_or(false) {
            self.reduce_one(p);
        }
    }

    fn reduce_one(&mut self, p: &mut Parser) {
        let top = self
            .ops
            .pop()
            .expect("reduction on an empty operator stack");
        let rhs = self.operands.pop().unwrap();
        let lhs = self.operands.pop().unwrap();
        let lo = lhs.span(&p.ast).lo;
        let node = BinaryNode {
            span: Span::new(lo, p.prev_end.max(lo)),
            op: top.op,
            lhs,
            rhs,
        }
        .allocate(&mut p.ast);
        self.operands.push(node);
    }
}

// ---- atoms: primary + postfix (prec 12–13) ----

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AtomMode {
    /// primary + the postfix loop (parse_unary's operand)
    Postfix,
    /// one primary, nothing else (pattern literals use parse_primary)
    Bare,
}

pub(crate) struct AtomFrame {
    lo: Span,
    mode: AtomMode,
    stage: AtomStage,
    cur_field: Option<crate::IdentId>,
}

enum AtomStage {
    Primary,
    Paren {
        first: bool,
        e: Option<ExprRef>,
        elems: Vec<ExprRef>,
    },
    Array {
        elems: Vec<ExprRef>,
    },
    /// `[v; n]` — the value was parsed, the count is being fetched
    Repeat {
        value: ExprRef,
    },
    Struct {
        ty: crate::TyRef,
        fields: Vec<(crate::IdentId, ExprRef)>,
    },
    /// a `::`-chain path: segments collected so far
    PathColons {
        segs: Vec<PathSeg>,
    },
    /// generic args on the HEAD segment (`Vec<u32>::from(..)`)
    PathGen {
        segs: Vec<PathSeg>,
        args: Vec<crate::TyRef>,
    },
    /// generic args on a later `::` segment (`Option<T>::Some(x)`)
    PathSegGen {
        segs: Vec<PathSeg>,
        name: crate::IdentId,
        args: Vec<crate::TyRef>,
    },
    Postfix {
        e: ExprRef,
    },
    PostDotGen {
        recv: ExprRef,
        name: crate::IdentId,
        args: Vec<crate::TyRef>,
    },
    PostDotCall {
        recv: ExprRef,
        name: crate::IdentId,
        generics: Vec<crate::TyRef>,
        args: Vec<ExprRef>,
    },
    PostCall {
        callee: ExprRef,
        args: Vec<ExprRef>,
    },
    PostIndex {
        recv: ExprRef,
    },
    /// `async { .. }` — the block body is being fetched
    AsyncBody {
        lo: u32,
    },
}

impl AtomFrame {
    pub(crate) fn new(lo: Span, mode: AtomMode) -> Self {
        AtomFrame {
            lo,
            mode,
            stage: AtomStage::Primary,
            cur_field: None,
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        debug_assert!(matches!(self.stage, AtomStage::Primary));
        let sp = p.span();
        match p.tok().clone() {
            Tok::Int(v, sfx) => {
                p.bump();
                let e = LitNode {
                    span: sp,
                    lit: Lit::Int(v, sfx),
                }
                .allocate(&mut p.ast);
                self.finish(p, e)
            }
            Tok::Float(bits, sfx) => {
                p.bump();
                let e = LitNode {
                    span: sp,
                    lit: Lit::Float(bits, sfx),
                }
                .allocate(&mut p.ast);
                self.finish(p, e)
            }
            Tok::Str(s) => {
                p.bump();
                let e = LitNode {
                    span: sp,
                    lit: Lit::Str(s),
                }
                .allocate(&mut p.ast);
                self.finish(p, e)
            }
            Tok::RawStr(s) => {
                p.bump();
                let e = LitNode {
                    span: sp,
                    lit: Lit::RawStr(s),
                }
                .allocate(&mut p.ast);
                self.finish(p, e)
            }
            Tok::Bool(b) => {
                p.bump();
                let e = LitNode {
                    span: sp,
                    lit: Lit::Bool(b),
                }
                .allocate(&mut p.ast);
                self.finish(p, e)
            }
            Tok::FStr(f) => {
                p.bump();
                Step::Push(Frame::FStr(FStrFrame::new(f, sp)))
            }
            Tok::LParen => {
                p.bump();
                self.stage = AtomStage::Paren {
                    first: true,
                    e: None,
                    elems: Vec::new(),
                };
                self.paren_top(p)
            }
            Tok::LBracket => {
                p.bump();
                self.stage = AtomStage::Array { elems: Vec::new() };
                self.array_top(p)
            }
            Tok::Ident(name) => {
                if name == "when" && matches!(p.peek(1).tok, Tok::LParen) {
                    return Step::Push(Frame::When(WhenFrame::new()));
                }
                // `if (cond) { .. }` — the if ATOM (the expression form)
                if name == "if" && matches!(p.peek(1).tok, Tok::LParen) {
                    return Step::Push(Frame::If(IfFrame::new()));
                }
                // `nil` — the empty value
                if name == "nil" {
                    p.bump();
                    let e = LitNode {
                        span: sp,
                        lit: Lit::Nil,
                    }
                    .allocate(&mut p.ast);
                    return self.finish(p, e);
                }
                // `inf` — the infinity float literal (the ident fold,
                // the `true`/`false` precedent — no lexer keyword; the
                // family is f64 END-TO-END, so the row carries the f64
                // suffix and no f32-default question ever opens).
                // `-inf` is unary negation on this literal, folded at
                // the const lowering. `inf` as a NAME stays a plain
                // ident — only expression position folds.
                if name == "inf" {
                    p.bump();
                    let e = LitNode {
                        span: sp,
                        lit: Lit::Float(f64::INFINITY.to_bits(), Some(FloatSuffix::F64)),
                    }
                    .allocate(&mut p.ast);
                    return self.finish(p, e);
                }
                // anonymous closure: handled at the expression start
                if name == "fn" && matches!(p.peek(1).tok, Tok::LParen) {
                    p.err_here("closures are expressions — `fn (..) { .. }` cannot appear here");
                }
                if name == "self" {
                    p.bump();
                    let e = PathNode {
                        span: sp,
                        segs: vec![PathSeg {
                            name: sym::SELF,
                            generics: Vec::new(),
                        }],
                    }
                    .allocate(&mut p.ast);
                    return self.finish(p, e);
                }
                if name == "Self" {
                    p.bump();
                    // `Self { .. }` — the class-private literal
                    if matches!(p.tok(), Tok::LBrace) {
                        let ty_name = sym::SELF_TY;
                        return self.struct_enter(p, ty_name, true);
                    }
                    let e = PathNode {
                        span: sp,
                        segs: vec![PathSeg {
                            name: sym::SELF_TY,
                            generics: Vec::new(),
                        }],
                    }
                    .allocate(&mut p.ast);
                    return self.finish(p, e);
                }
                // struct literal: `Ident {` (struct only)
                if matches!(p.peek(1).tok, Tok::LBrace) && !is_reserved_kw(&name) {
                    let ty_name = p.ast.intern(&name);
                    return self.struct_enter(p, ty_name, false);
                }
                // `async { .. }` — the async block primitive
                if name == "async" && matches!(p.peek(1).tok, Tok::LBrace) {
                    let lo = sp.lo;
                    p.bump();
                    self.stage = AtomStage::AsyncBody { lo };
                    return Step::Push(Frame::Block(BlockFrame::strict(p)));
                }
                p.bump();
                let first = PathSeg {
                    name: p.ast.intern(&name),
                    generics: Vec::new(),
                };
                let segs = vec![first];
                // generic args on the head segment: `Vec<f32>(1024)`
                if matches!(p.tok(), Tok::Lt) && p.scan_is_generic_args(false) {
                    p.bump();
                    self.stage = AtomStage::PathGen {
                        segs,
                        args: Vec::new(),
                    };
                    return self.genarg_top(p);
                }
                self.stage = AtomStage::PathColons { segs };
                self.pathcolons_top(p)
            }
            _ => {
                let found = p.peek(0).describe();
                p.err_here(format!("expected an expression, found {found}"));
                Step::Pop(Done::Failed)
            }
        }
    }

    /// a completed atom: into the postfix loop, or straight out (Bare)
    fn finish(&mut self, p: &mut Parser, e: ExprRef) -> Step {
        if self.mode == AtomMode::Bare {
            return Step::Pop(Done::Expr(e));
        }
        self.stage = AtomStage::Postfix { e };
        self.postfix_top(p)
    }

    // -- postfix loop: `.name` `.name<..>(..)` `(..)` `[..]` `?` --

    fn postfix_top(&mut self, p: &mut Parser) -> Step {
        let mut e = match &self.stage {
            AtomStage::Postfix { e } => *e,
            _ => unreachable!("postfix loop outside the postfix stage"),
        };
        loop {
            match p.tok() {
                Tok::Dot => {
                    p.bump();
                    // `.0`/`.1` — tuple field access
                    let name = match p.tok().clone() {
                        Tok::Int(v, None) => {
                            p.bump();
                            p.ast.intern(&v.to_string())
                        }
                        _ => {
                            let Some(n) = p.expect_ident("a member name") else {
                                return Step::Pop(Done::Failed);
                            };
                            n.id
                        }
                    };
                    if matches!(p.tok(), Tok::Lt) && p.scan_is_generic_args(false) {
                        p.bump();
                        self.stage = AtomStage::PostDotGen {
                            recv: e,
                            name,
                            args: Vec::new(),
                        };
                        return self.genarg_top(p);
                    }
                    if matches!(p.tok(), Tok::LParen) {
                        p.bump();
                        self.stage = AtomStage::PostDotCall {
                            recv: e,
                            name,
                            generics: Vec::new(),
                            args: Vec::new(),
                        };
                        return self.call_top(p);
                    }
                    e = FieldNode {
                        span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                        recv: e,
                        name,
                    }
                    .allocate(&mut p.ast);
                }
                Tok::LParen => {
                    p.bump();
                    self.stage = AtomStage::PostCall {
                        callee: e,
                        args: Vec::new(),
                    };
                    return self.call_top(p);
                }
                Tok::LBracket => {
                    p.bump();
                    self.stage = AtomStage::PostIndex { recv: e };
                    return Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)));
                }
                Tok::Question => {
                    p.bump();
                    e = TryNode {
                        span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                        expr: e,
                    }
                    .allocate(&mut p.ast);
                }
                _ => return Step::Pop(Done::Expr(e)),
            }
        }
    }

    // -- parenthesized expression: `( expr )` — no tuples --

    fn paren_top(&mut self, p: &mut Parser) -> Step {
        loop {
            if p.eat_punct(Tok::RParen) {
                let (e, elems) = match &mut self.stage {
                    AtomStage::Paren { e, elems, .. } => (e.take(), std::mem::take(elems)),
                    _ => unreachable!(),
                };
                // `(a)` groups; `(a, b)` are tuples; `()` is
                // the nil value and must be spelled `nil` (v1.2)
                if elems.is_empty() {
                    return match e {
                        Some(v) => self.finish(p, v),
                        None => {
                            p.err_here(
                                "`()` is not a value — the empty value is spelled `nil` (v1.2)",
                            );
                            let node = LitNode {
                                span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                                lit: Lit::Nil,
                            }
                            .allocate(&mut p.ast);
                            Step::Pop(Done::Expr(node))
                        }
                    };
                }
                let mut elems = elems;
                if let Some(v) = e {
                    elems.push(v);
                }
                let node = TupleNode {
                    span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                    elems,
                }
                .allocate(&mut p.ast);
                return Step::Pop(Done::Expr(node));
            }
            let (first, has_e) = match &self.stage {
                AtomStage::Paren { first, e, .. } => (*first, e.is_some()),
                _ => unreachable!(),
            };
            if !first && matches!(p.tok(), Tok::Comma) {
                // tuple element separator: fold the element, parse the next
                if has_e {
                    if let AtomStage::Paren { e, elems, .. } = &mut self.stage {
                        elems.push(e.take().unwrap());
                    }
                }
                p.bump();
                return Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)));
            }
            if first && matches!(p.tok(), Tok::Comma) {
                p.err_here("expected an expression");
                p.bump();
                continue;
            }
            return Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)));
        }
    }

    // -- array literals: `[ e1, .., en ]` and the repeat `[ v; n ]` --

    fn array_top(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::RBracket) {
            let elems = match &mut self.stage {
                AtomStage::Array { elems } => std::mem::take(elems),
                _ => unreachable!(),
            };
            let node = ArrayLitNode {
                span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                elems,
            }
            .allocate(&mut p.ast);
            return Step::Pop(Done::Expr(node));
        }
        Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
    }

    // -- struct literal: `Name { field: expr, .. }` / `Name<Ty> { .. }` --

    fn struct_enter(
        &mut self,
        p: &mut Parser,
        ty_name: crate::IdentId,
        ident_consumed: bool,
    ) -> Step {
        self.struct_enter_gen(p, ty_name, Vec::new(), ident_consumed)
    }

    fn struct_enter_gen(
        &mut self,
        p: &mut Parser,
        ty_name: crate::IdentId,
        generics: Vec<crate::TyRef>,
        ident_consumed: bool,
    ) -> Step {
        if !ident_consumed {
            p.bump(); // the type ident
        }
        p.bump(); // {
        let ty = crate::TyPathNode {
            span: self.lo,
            segs: vec![PathSeg {
                name: ty_name,
                generics,
            }],
        }
        .allocate(&mut p.ast);
        self.stage = AtomStage::Struct {
            ty,
            fields: Vec::new(),
        };
        self.struct_top(p)
    }

    fn struct_top(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::RBrace) {
            return self.struct_pop(p);
        }
        // the field-name token's span — a field-init shorthand's value
        // span (the desugared path must spell exactly the name token)
        let id_sp = p.span();
        let Some(f) = p.expect_ident("a field name") else {
            p.sync_stmt();
            return self.struct_pop(p);
        };
        // `Name { x, .. }` — the field-init shorthand
        if matches!(p.tok(), Tok::Comma | Tok::RBrace) {
            let e = PathNode {
                span: id_sp,
                segs: vec![PathSeg {
                    name: f.id,
                    generics: Vec::new(),
                }],
            }
            .allocate(&mut p.ast);
            return self.struct_field_done(p, f.id, e);
        }
        p.expect(Tok::Colon);
        self.cur_field = Some(f.id);
        Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
    }

    fn struct_field_done(&mut self, p: &mut Parser, f: crate::IdentId, e: ExprRef) -> Step {
        match &mut self.stage {
            AtomStage::Struct { fields, .. } => fields.push((f, e)),
            _ => unreachable!("struct field at the wrong stage"),
        }
        if p.eat_punct(Tok::Comma) {
            self.struct_top(p)
        } else {
            p.expect(Tok::RBrace);
            self.struct_pop(p)
        }
    }

    fn struct_pop(&mut self, p: &mut Parser) -> Step {
        let (ty, fields) = match &mut self.stage {
            AtomStage::Struct { ty, fields } => (*ty, std::mem::take(fields)),
            _ => unreachable!(),
        };
        let node = StructLitNode {
            span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
            ty,
            fields,
        }
        .allocate(&mut p.ast);
        Step::Pop(Done::Expr(node))
    }

    // -- path building: `a::b::c` --

    fn pathcolons_top(&mut self, p: &mut Parser) -> Step {
        loop {
            if !p.eat_punct(Tok::Colon2) {
                let segs = match &mut self.stage {
                    AtomStage::PathColons { segs } => std::mem::take(segs),
                    _ => unreachable!(),
                };
                let node = PathNode {
                    span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                    segs,
                }
                .allocate(&mut p.ast);
                return self.finish(p, node);
            }
            let Some(name) = p.expect_ident("a path segment") else {
                return Step::Pop(Done::Failed);
            };
            if matches!(p.tok(), Tok::Lt) && p.scan_is_generic_args(false) {
                p.bump();
                let segs = match &mut self.stage {
                    AtomStage::PathColons { segs } => std::mem::take(segs),
                    _ => unreachable!(),
                };
                self.stage = AtomStage::PathSegGen {
                    segs,
                    name: name.id,
                    args: Vec::new(),
                };
                return self.genarg_top(p);
            }
            if let AtomStage::PathColons { segs } = &mut self.stage {
                segs.push(PathSeg {
                    name: name.id,
                    generics: Vec::new(),
                });
            }
        }
    }

    /// generic-argument list top: `>` finishes, otherwise fetch an arg
    fn genarg_top(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::Gt) {
            return self.genarg_done(p);
        }
        if matches!(p.tok(), Tok::Int(..)) {
            Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::UnaryOnly)))
        } else {
            Step::Push(Frame::Type(TypeFrame::new(p)))
        }
    }

    fn genarg_after_arg(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::Comma) {
            self.genarg_top(p)
        } else {
            p.expect_gt();
            self.genarg_done(p)
        }
    }

    fn genarg_done(&mut self, p: &mut Parser) -> Step {
        match &mut self.stage {
            AtomStage::PathGen { segs, args } => {
                let args = std::mem::take(args);
                let mut segs = std::mem::take(segs);
                segs[0].generics = args;
                // `Name<Ty> { .. }` — the generic record literal
                if segs.len() == 1 && matches!(p.tok(), Tok::LBrace) {
                    let name = segs[0].name;
                    let generics = std::mem::take(&mut segs[0].generics);
                    return self.struct_enter_gen(p, name, generics, true);
                }
                self.stage = AtomStage::PathColons { segs };
                self.pathcolons_top(p)
            }
            AtomStage::PathSegGen { segs, name, args } => {
                let args = std::mem::take(args);
                let name = *name;
                let mut segs = std::mem::take(segs);
                segs.push(PathSeg {
                    name,
                    generics: args,
                });
                self.stage = AtomStage::PathColons { segs };
                self.pathcolons_top(p)
            }
            AtomStage::PostDotGen { recv, name, args } => {
                let recv = *recv;
                let name = *name;
                let generics = std::mem::take(args);
                if matches!(p.tok(), Tok::LParen) {
                    p.bump();
                    self.stage = AtomStage::PostDotCall {
                        recv,
                        name,
                        generics,
                        args: Vec::new(),
                    };
                    self.call_top(p)
                } else {
                    let node = FieldNode {
                        span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                        recv,
                        name,
                    }
                    .allocate(&mut p.ast);
                    self.stage = AtomStage::Postfix { e: node };
                    self.postfix_top(p)
                }
            }
            _ => unreachable!("generic args at the wrong stage"),
        }
    }

    /// call-argument list top: `)` finishes, otherwise an expression
    fn call_top(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::RParen) {
            return self.call_done(p);
        }
        Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
    }

    fn call_after_arg(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::Comma) {
            self.call_top(p)
        } else {
            p.expect(Tok::RParen);
            self.call_done(p)
        }
    }

    fn call_done(&mut self, p: &mut Parser) -> Step {
        match &mut self.stage {
            AtomStage::PostCall { callee, args } => {
                let callee = *callee;
                let args = std::mem::take(args);
                let node = CallNode {
                    span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                    callee,
                    args,
                }
                .allocate(&mut p.ast);
                self.stage = AtomStage::Postfix { e: node };
                self.postfix_top(p)
            }
            AtomStage::PostDotCall {
                recv,
                name,
                generics,
                args,
            } => {
                let recv = *recv;
                let name = *name;
                let generics = std::mem::take(generics);
                let args = std::mem::take(args);
                let node = MethodNode {
                    span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                    recv,
                    name,
                    generics,
                    args,
                }
                .allocate(&mut p.ast);
                self.stage = AtomStage::Postfix { e: node };
                self.postfix_top(p)
            }
            _ => unreachable!("call args at the wrong stage"),
        }
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match d {
            // the async block's body landed: the block IS the expression
            Done::Block(b) => match self.stage {
                AtomStage::AsyncBody { lo } => {
                    let e = AsyncBlockNode {
                        span: Span::new(lo, p.prev_end.max(lo)),
                        body: b,
                    }
                    .allocate(&mut p.ast);
                    self.finish(p, e)
                }
                _ => unreachable!("atom frame received a block outside the async body"),
            },
            Done::Expr(e) => match &mut self.stage {
                AtomStage::Primary => self.finish(p, e), // an f-string child
                AtomStage::AsyncBody { .. } => {
                    unreachable!("atom frame received an expr inside the async body")
                }
                AtomStage::Paren {
                    first,
                    e: slot,
                    elems,
                } => {
                    if *first {
                        *slot = Some(e);
                        *first = false;
                    } else {
                        elems.push(e);
                    }
                    self.paren_top(p)
                }
                AtomStage::Array { elems } => {
                    elems.push(e);
                    // `;` switches to the repeat form `[ v; n ]`
                    if p.eat_punct(Tok::Semi) {
                        let value = elems.pop().expect("repeat value");
                        self.stage = AtomStage::Repeat { value };
                        return Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)));
                    }
                    if p.eat_punct(Tok::Comma) {
                        self.array_top(p)
                    } else {
                        p.expect(Tok::RBracket);
                        let elems = std::mem::take(elems);
                        let node = ArrayLitNode {
                            span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                            elems,
                        }
                        .allocate(&mut p.ast);
                        Step::Pop(Done::Expr(node))
                    }
                }
                AtomStage::Repeat { value } => {
                    let value = *value;
                    p.expect(Tok::RBracket);
                    let node = ArrayRepeatNode {
                        span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                        value,
                        count: e,
                    }
                    .allocate(&mut p.ast);
                    Step::Pop(Done::Expr(node))
                }
                AtomStage::Struct { .. } => {
                    let f = self.cur_field.take().expect("struct field without a name");
                    self.struct_field_done(p, f, e)
                }
                AtomStage::PathGen { args, .. }
                | AtomStage::PathSegGen { args, .. }
                | AtomStage::PostDotGen { args, .. } => {
                    // const-generic argument: integer expression
                    args.push(
                        crate::TyConstNode {
                            span: e.span(&p.ast),
                            expr: e,
                        }
                        .allocate(&mut p.ast),
                    );
                    self.genarg_after_arg(p)
                }
                AtomStage::PostCall { args, .. } | AtomStage::PostDotCall { args, .. } => {
                    args.push(e);
                    self.call_after_arg(p)
                }
                AtomStage::PostIndex { recv } => {
                    let recv = *recv;
                    p.expect(Tok::RBracket);
                    let node = IndexNode {
                        span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                        recv,
                        idx: e,
                    }
                    .allocate(&mut p.ast);
                    self.stage = AtomStage::Postfix { e: node };
                    self.postfix_top(p)
                }
                AtomStage::Postfix { .. } | AtomStage::PathColons { .. } => {
                    unreachable!("atom frame received an operand at the wrong stage")
                }
            },
            Done::Ty(t) => {
                match &mut self.stage {
                    AtomStage::PathGen { args, .. }
                    | AtomStage::PathSegGen { args, .. }
                    | AtomStage::PostDotGen { args, .. } => args.push(t),
                    _ => unreachable!("atom frame received a type at the wrong stage"),
                }
                self.genarg_after_arg(p)
            }
            Done::WhenParts { scrut, arms } => {
                debug_assert!(matches!(self.stage, AtomStage::Primary));
                let node = WhenExprNode {
                    span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                    scrut,
                    arms,
                }
                .allocate(&mut p.ast);
                self.finish(p, node)
            }
            Done::If(h) => {
                debug_assert!(matches!(self.stage, AtomStage::Primary));
                // the if frame minted the statement node; its field set
                // IS the expression's — re-mint as an expression. An
                // `else if` chain stays `ElseBranch::If` — statement
                // handles all the way down.
                let n = h.get(&p.ast);
                let node = IfExprNode {
                    span: n.span,
                    cond: n.cond,
                    then: n.then,
                    els: n.els,
                }
                .allocate(&mut p.ast);
                self.finish(p, node)
            }
            Done::Failed => match self.stage {
                AtomStage::Paren { first, .. } => {
                    // v1: a failed first element re-enters the loop top;
                    // a failed later element ends the parenthesized form
                    if first {
                        if let AtomStage::Paren { first, .. } = &mut self.stage {
                            *first = false;
                        }
                        self.paren_top(p)
                    } else {
                        let e = match &mut self.stage {
                            AtomStage::Paren { e, .. } => e.take(),
                            _ => unreachable!(),
                        };
                        match e {
                            Some(v) => self.finish(p, v),
                            None => Step::Pop(Done::Failed),
                        }
                    }
                }
                _ => Step::Pop(Done::Failed),
            },
            _ => unreachable!("atom frame receives expressions, types, or when/if parts"),
        }
    }
}

// ---- lambdas: `( params ) (-> Type)? => expr | block` ----

pub(crate) struct LambdaFrame {
    sp: Span,
    params: Option<Vec<ParamRef>>,
    ret: Option<crate::TyRef>,
    /// anonymous-fn form `fn (params) -> T { .. }` — no `=>`, block required
    anon: bool,
}

impl LambdaFrame {
    /// `(a, b) -> T => ..` — params come from a ParamsFrame child
    pub(crate) fn paren(sp: Span) -> Self {
        LambdaFrame {
            sp,
            params: None,
            ret: None,
            anon: false,
        }
    }
    /// `x => ..` — the single param was pre-seeded by the caller
    pub(crate) fn single(sp: Span, params: Vec<ParamRef>) -> Self {
        LambdaFrame {
            sp,
            params: Some(params),
            ret: None,
            anon: false,
        }
    }
    /// `fn (params) -> T { .. }` — the `(` was not consumed
    pub(crate) fn anon(sp: Span) -> Self {
        LambdaFrame {
            sp,
            params: None,
            ret: None,
            anon: true,
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        if self.params.is_none() {
            return Step::Push(Frame::Params(ParamsFrame::new()));
        }
        self.body(p)
    }

    fn after_params(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::Arrow) {
            return Step::Push(Frame::Type(TypeFrame::new(p)));
        }
        if !self.anon {
            p.expect(Tok::FatArrow);
        }
        self.body(p)
    }

    fn body(&mut self, p: &mut Parser) -> Step {
        if self.anon {
            // anonymous closures are block-bodied: `fn (..) { .. }`
            if !matches!(p.tok(), Tok::LBrace) {
                p.err_here("anonymous closures take a block —`fn (..) { .. }`");
            }
        }
        if matches!(p.tok(), Tok::LBrace) {
            Step::Push(Frame::Block(BlockFrame::strict(p)))
        } else {
            Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
        }
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match d {
            Done::Members(ps) => {
                self.params = Some(ps);
                self.after_params(p)
            }
            Done::Ty(t) => {
                self.ret = Some(t);
                if !self.anon {
                    p.expect(Tok::FatArrow);
                }
                self.body(p)
            }
            Done::Expr(e) => Step::Pop(Done::Expr(self.mk(p, e))),
            Done::Block(b) => Step::Pop(Done::Expr(self.mk(p, ExprRef::Block(b)))),
            Done::Failed => Step::Pop(Done::Failed),
            _ => unreachable!("lambda frame receives params/types/exprs"),
        }
    }

    fn mk(&mut self, p: &mut Parser, body: ExprRef) -> ExprRef {
        let params = self.params.take().expect("lambda without params");
        LambdaNode {
            span: Span::new(self.sp.lo, p.prev_end.max(self.sp.lo)),
            params,
            ret: self.ret.take(),
            body,
        }
        .allocate(&mut p.ast)
    }
}

// ---- f-strings: holes are sub-slice frames ----

pub(crate) struct FStrFrame {
    sp: Span,
    parts_in: Vec<FPart>,
    idx: usize,
    parts: Vec<FPartAst>,
    /// the outer token slice, position, and budgets, saved while a hole
    /// is being parsed by the same loop
    saved: Option<(Vec<Token>, usize, u32, u32)>,
}

impl FStrFrame {
    pub(crate) fn new(f: FStrTok, sp: Span) -> Self {
        FStrFrame {
            sp,
            parts_in: f.parts,
            idx: 0,
            parts: Vec::new(),
            saved: None,
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        while self.idx < self.parts_in.len() {
            match self.parts_in[self.idx].clone() {
                FPart::Lit(s) => {
                    self.parts.push(FPartAst::Lit(s));
                    self.idx += 1;
                }
                FPart::Hole(hole_toks) => {
                    // sub-parse over the hole's token slice
                    self.idx += 1;
                    let mut toks = hole_toks;
                    toks.push(Token {
                        tok: Tok::Eof,
                        span: self.sp,
                    });
                    self.saved = Some(p.switch_to_hole(toks));
                    return Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)));
                }
            }
        }
        Step::Pop(Done::Expr(
            FStrNode {
                span: self.sp,
                parts: std::mem::take(&mut self.parts),
            }
            .allocate(&mut p.ast),
        ))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match d {
            Done::Expr(e) => self.parts.push(FPartAst::Hole(e)),
            Done::Failed => {
                p.err(self.sp, "could not parse the format-string placeholder");
                self.parts.push(FPartAst::Lit(String::new()));
            }
            _ => unreachable!("f-string frame receives expressions"),
        }
        if let Some((toks, pos, ed, ne)) = self.saved.take() {
            p.restore_from_hole(toks, pos, ed, ne);
        }
        self.step(p)
    }
}
