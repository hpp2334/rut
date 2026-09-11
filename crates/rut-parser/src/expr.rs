//! Expressions (RFC 0030 §4): the embedded Pratt engine — an operator
//! stack plus an operand stack of `NodeId`s, reductions popping two
//! operands and pushing one node, bottom-up (C1). Binding powers follow
//! the §4 table: assignment right (1), `||` (3), `&&` (4), `== !=` (5),
//! relational + `is` non-associative (6), `| ^` (7), `&` (8), `<< >>`
//! (9), `+ - &+ &-` (10), `* / % &*` (11), unary right (12), postfix
//! (13). Atoms (primary + postfix chains) are built by `AtomFrame`; the
//! two §4.2 scans live in `ty.rs`.

use rut_ast::ast::*;
use rut_lexer::span::Span;
use rut_lexer::token::{FPart, FStrTok, Tok, Token};

use crate::frame::{Done, Frame, Step};
use crate::item::ParamsFrame;
use crate::stmt::{BlockFrame, WhenFrame};
use crate::ty::TypeFrame;
use crate::{is_reserved_kw, Parser, EXPR_MAX};

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
    operands: Vec<NodeHandle<AnyExpr>>,
    /// prefix operators swept for the operand being fetched
    prefixes: Vec<(Pfx, Span)>,
    /// the binding level of the last <=6-level operator shifted — the
    /// non-associativity scope for relational/`is` (RFC 0012 §3)
    last_level: u8,
    pending_assign: Option<BinOp>,
}

enum ExprStage {
    Start,
    /// waiting for an atom (or the select form) — the operand
    Operand,
    /// waiting for the `is` right-hand type
    IsTy,
    /// waiting for the right-hand side of an assignment (a full
    /// expression — assignment is right-associative through recursion)
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
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        debug_assert!(matches!(self.stage, ExprStage::Start));
        if self.mode == ExprMode::Full {
            // lambda scan 1: `( params ) (: Type)? =>` — scan read-only (§4.2)
            if matches!(p.tok(), Tok::LParen) && p.scan_is_lambda() {
                self.stage = ExprStage::Operand;
                return Step::Push(Frame::Lambda(LambdaFrame::paren(self.lo)));
            }
            // lambda scan 2: single-ident form `x =>`
            if let Tok::Ident(name) = p.tok().clone() {
                if !is_reserved_kw(&name) && name != "else" && matches!(p.peek(1).tok, Tok::FatArrow) {
                    let sp = p.span();
                    p.bump(); // ident
                    p.bump(); // =>
                    let id = p.interner.intern(&name);
                    let param = p.member(
                        MemberKind::Param(ParamData { is_mut: false, name: id, ty: None }),
                        sp,
                    );
                    self.stage = ExprStage::Operand;
                    return Step::Push(Frame::Lambda(LambdaFrame::single(self.lo, vec![param])));
                }
            }
        }
        self.fetch_operand(p)
    }

    /// sweep prefix operators (`-` `!` `~` `await`), then fetch an atom.
    /// `await select {..}` is the one prefix form that replaces its
    /// operand outright (RFC 0019 §3).
    fn fetch_operand(&mut self, p: &mut Parser) -> Step {
        self.stage = ExprStage::Operand;
        if let Some(f) = self.sweep(p) {
            return Step::Push(f);
        }
        Step::Push(Frame::Atom(AtomFrame::new(self.lo, AtomMode::Postfix)))
    }

    fn sweep(&mut self, p: &mut Parser) -> Option<Frame> {
        loop {
            let pfx = match p.tok() {
                Tok::Minus => Some(Pfx::Un(UnOp::Neg)),
                Tok::Bang => Some(Pfx::Un(UnOp::Not)),
                Tok::Tilde => Some(Pfx::Un(UnOp::BitNot)),
                _ if p.at_kw("await") => {
                    let is_select = matches!(&p.peek(1).tok, Tok::Ident(s) if s == "select")
                        && matches!(p.peek(2).tok, Tok::LBrace);
                    if is_select {
                        return Some(Frame::Select(SelectFrame::new(p.span())));
                    }
                    Some(Pfx::Await)
                }
                _ => None,
            };
            let Some(pfx) = pfx else { break };
            // C3: unary chains carry the expression budget — one clean
            // Diag, the tree stays bounded, the cursor still advances
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
                            Pfx::Un(op) => p.expr(ExprKind::Unary { op, expr: e }, sp.to(p.span())),
                            Pfx::Await => p.expr(ExprKind::Await { expr: e }, sp.to(p.span())),
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
                    let sp = p.nodes[target.id().0 as usize].span;
                    match &p.nodes[target.id().0 as usize].kind {
                        Kind::Expr(ExprKind::Path { .. } | ExprKind::Field { .. } | ExprKind::Index { .. }) => {}
                        _ => {
                            p.err(sp, "invalid assignment target —expected a path, field, or index");
                        }
                    }
                    let node = p.expr(
                        ExprKind::Assign { op: self.pending_assign, target, value: e },
                        sp.to(p.span()),
                    );
                    self.operands.push(node);
                    self.oploop(p)
                }
                _ => unreachable!("expr frame received an operand at the wrong stage"),
            },
            Done::Ty(t) => {
                debug_assert!(matches!(self.stage, ExprStage::IsTy));
                let lhs = self.operands.pop().expect("`is` without a left-hand side");
                let sp = p.nodes[lhs.id().0 as usize].span;
                let node = p.expr(ExprKind::Is { expr: lhs, ty: t }, sp.to(p.span()));
                self.operands.push(node);
                self.last_level = 6;
                self.oploop(p)
            }
            Done::Failed => Step::Pop(Done::Failed),
            _ => unreachable!("expr frame receives expressions or types"),
        }
    }

    /// the operator loop: shift binary/assign/`is` operators with
    /// precedence-driven reduction, or finish by reducing everything
    fn oploop(&mut self, p: &mut Parser) -> Step {
        loop {
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
                Tok::AmpPlus => (BinOp::WrapAdd, 10, 10),
                Tok::AmpMinus => (BinOp::WrapSub, 10, 10),
                Tok::Star => (BinOp::Mul, 11, 11),
                Tok::Slash => (BinOp::Div, 11, 11),
                Tok::Percent => (BinOp::Mod, 11, 11),
                Tok::AmpStar => (BinOp::WrapMul, 11, 11),
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
                Tok::AmpPlusEq => return self.shift_assign(p, Some(BinOp::WrapAdd)),
                Tok::AmpMinusEq => return self.shift_assign(p, Some(BinOp::WrapSub)),
                Tok::AmpStarEq => return self.shift_assign(p, Some(BinOp::WrapMul)),
                Tok::AmpShlEq => return self.shift_assign(p, Some(BinOp::WrapShl)),
                _ if p.at_kw("is") => {
                    // `is` takes a TYPE as its right-hand side (naming
                    // position, RFC 0012 §3) — non-associative at level 6
                    if self.last_level == 6 {
                        break;
                    }
                    p.bump(); // the `is` keyword
                    self.reduce_while(p, 6);
                    self.last_level = 6;
                    if p.at_kw("dyn") {
                        p.err_here("the `is` right-hand side is a naming position —no `dyn` prefix (RFC 0012 §3)");
                        p.bump();
                    }
                    self.stage = ExprStage::IsTy;
                    return Step::Push(Frame::Type(TypeFrame::new(p)));
                }
                _ => break,
            };
            // non-associativity scope (RFC 0012 §3): one relational per
            // context — a second level-6 op is left for the caller's
            // expect, exactly as v1's parse_rel did
            if level == 6 && self.last_level == 6 {
                break;
            }
            p.bump(); // the operator token
            self.reduce_while(p, bp);
            self.ops.push(OpEntry { op, bp });
            if level <= 6 {
                self.last_level = level;
            }
            return self.fetch_operand(p);
        }
        while !self.ops.is_empty() {
            self.reduce_one(p);
        }
        Step::Pop(Done::Expr(self.operands.pop().expect("expr frame without a result")))
    }

    fn shift_assign(&mut self, p: &mut Parser, op: Option<BinOp>) -> Step {
        p.bump(); // the assignment operator token
        // right-associative: reduce only strictly tighter bindings
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
        let top = self.ops.pop().expect("reduction on an empty operator stack");
        let rhs = self.operands.pop().unwrap();
        let lhs = self.operands.pop().unwrap();
        let lo = p.nodes[lhs.id().0 as usize].span.lo;
        let node = p.expr(ExprKind::Binary { op: top.op, lhs, rhs }, Span::new(lo, p.span().hi));
        self.operands.push(node);
    }
}

// ---- atoms: primary + postfix (RFC 0030 §3, prec 12–13) ----

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
    cur_field: Option<IdentId>,
}

enum AtomStage {
    Primary,
    Paren { first: bool, e: Option<NodeHandle<AnyExpr>>, discard: bool },
    Array { elems: Vec<NodeHandle<AnyExpr>> },
    Struct { ty: NodeHandle<AnyTy>, fields: Vec<(IdentId, NodeHandle<AnyExpr>)> },
    PathDots { segs: Vec<PathSeg> },
    PathGen { segs: Vec<PathSeg>, args: Vec<NodeHandle<AnyTy>> },
    PathDotGen { segs: Vec<PathSeg>, name: IdentId, args: Vec<NodeHandle<AnyTy>> },
    PathDotCall { segs: Vec<PathSeg>, name: IdentId, generics: Vec<NodeHandle<AnyTy>>, args: Vec<NodeHandle<AnyExpr>> },
    Postfix { e: NodeHandle<AnyExpr> },
    PostDotGen { recv: NodeHandle<AnyExpr>, name: IdentId, args: Vec<NodeHandle<AnyTy>> },
    PostDotCall { recv: NodeHandle<AnyExpr>, name: IdentId, generics: Vec<NodeHandle<AnyTy>>, args: Vec<NodeHandle<AnyExpr>> },
    PostCall { callee: NodeHandle<AnyExpr>, args: Vec<NodeHandle<AnyExpr>> },
    PostIndex { recv: NodeHandle<AnyExpr> },
}

impl AtomFrame {
    pub(crate) fn new(lo: Span, mode: AtomMode) -> Self {
        AtomFrame { lo, mode, stage: AtomStage::Primary, cur_field: None }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        debug_assert!(matches!(self.stage, AtomStage::Primary));
        let sp = p.span();
        match p.tok().clone() {
            Tok::Int(v, sfx) => {
                p.bump();
                let e = p.expr(ExprKind::Lit(Lit::Int(v, sfx)), sp);
                self.finish(p, e)
            }
            Tok::Float(bits, sfx) => {
                p.bump();
                let e = p.expr(ExprKind::Lit(Lit::Float(bits, sfx)), sp);
                self.finish(p, e)
            }
            Tok::Str(s) => {
                p.bump();
                let e = p.expr(ExprKind::Lit(Lit::Str(s)), sp);
                self.finish(p, e)
            }
            Tok::RawStr(s) => {
                p.bump();
                let e = p.expr(ExprKind::Lit(Lit::RawStr(s)), sp);
                self.finish(p, e)
            }
            Tok::Char(c) => {
                p.bump();
                let e = p.expr(ExprKind::Lit(Lit::Char(c)), sp);
                self.finish(p, e)
            }
            Tok::Bool(b) => {
                p.bump();
                let e = p.expr(ExprKind::Lit(Lit::Bool(b)), sp);
                self.finish(p, e)
            }
            Tok::FStr(f) => {
                p.bump();
                Step::Push(Frame::FStr(FStrFrame::new(f, sp)))
            }
            Tok::LParen => {
                p.bump();
                self.stage = AtomStage::Paren { first: true, e: None, discard: false };
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
                if name == "self" {
                    p.bump();
                    let seg = PathSeg { name: p.interner.intern("self"), generics: Vec::new() };
                    let e = p.expr(ExprKind::Path { segs: vec![seg] }, sp);
                    return self.finish(p, e);
                }
                if name == "Self" {
                    p.bump();
                    // `Self { .. }` — the class-private literal (RFC 0010 §1)
                    if matches!(p.tok(), Tok::LBrace) {
                        let ty_name = p.interner.intern("Self");
                        return self.struct_enter(p, ty_name, true);
                    }
                    let seg = PathSeg { name: p.interner.intern("Self"), generics: Vec::new() };
                    let e = p.expr(ExprKind::Path { segs: vec![seg] }, sp);
                    return self.finish(p, e);
                }
                // struct literal: `Ident {` (dataclass only — classes have
                // no instance literal; the checker rejects `Circle { .. }`)
                if matches!(p.peek(1).tok, Tok::LBrace) && !is_reserved_kw(&name) {
                    let ty_name = p.interner.intern(&name);
                    return self.struct_enter(p, ty_name, false);
                }
                p.bump();
                let first = PathSeg { name: p.interner.intern(&name), generics: Vec::new() };
                let segs = vec![first];
                // generic args on the head segment: `Vec<f32>(1024)`,
                // `downcast<Point>(o)`, `MyMap<K, V>.new(..)`, `Option<T>.Some(x)`
                if matches!(p.tok(), Tok::Lt) && p.scan_is_generic_args(false) {
                    p.bump();
                    self.stage = AtomStage::PathGen { segs, args: Vec::new() };
                    return self.genarg_top(p);
                }
                self.stage = AtomStage::PathDots { segs };
                self.pathdots_top(p)
            }
            _ => {
                let found = p.peek(0).describe();
                p.err_here(format!("expected an expression, found {found}"));
                Step::Pop(Done::Failed)
            }
        }
    }

    /// a completed atom: into the postfix loop, or straight out (Bare)
    fn finish(&mut self, p: &mut Parser, e: NodeHandle<AnyExpr>) -> Step {
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
                    let Some(name) = p.expect_ident("a member name") else {
                        return Step::Pop(Done::Failed);
                    };
                    if matches!(p.tok(), Tok::Lt) && p.scan_is_generic_args(false) {
                        p.bump();
                        self.stage = AtomStage::PostDotGen { recv: e, name, args: Vec::new() };
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
                    e = p.expr(ExprKind::Field { recv: e, name }, self.lo.to(p.span()));
                }
                Tok::LParen => {
                    p.bump();
                    self.stage = AtomStage::PostCall { callee: e, args: Vec::new() };
                    return self.call_top(p);
                }
                Tok::LBracket => {
                    p.bump();
                    self.stage = AtomStage::PostIndex { recv: e };
                    return Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)));
                }
                Tok::Question => {
                    p.bump();
                    e = p.expr(ExprKind::Try { expr: e }, self.lo.to(p.span()));
                }
                _ => return Step::Pop(Done::Expr(e)),
            }
        }
    }

    // -- parenthesized expression: `( expr )` — no tuples (RFC 0009) --

    fn paren_top(&mut self, p: &mut Parser) -> Step {
        loop {
            if p.eat_punct(Tok::RParen) {
                let e = match &mut self.stage {
                    AtomStage::Paren { e, .. } => e.take(),
                    _ => unreachable!(),
                };
                return match e {
                    Some(v) => self.finish(p, v),
                    None => Step::Pop(Done::Failed), // `()` — v1 returned None silently
                };
            }
            let (first, discard) = match &self.stage {
                AtomStage::Paren { first, discard, .. } => (*first, *discard),
                _ => unreachable!(),
            };
            if !first && !discard && matches!(p.tok(), Tok::Comma) {
                // RFC 0030 §4.2: a comma inside parens that is not a
                // lambda errors AT THE COMMA — there are no tuples
                p.err_here("there are no tuples (RFC 0009) —if you meant a lambda, add `=>`");
                p.bump();
                if let AtomStage::Paren { discard, .. } = &mut self.stage {
                    *discard = true;
                }
                continue;
            }
            return Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)));
        }
    }

    // -- array literal: `[ e1, .., en ]` --

    fn array_top(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::RBracket) {
            let elems = match &mut self.stage {
                AtomStage::Array { elems } => std::mem::take(elems),
                _ => unreachable!(),
            };
            return Step::Pop(Done::Expr(p.expr(ExprKind::ArrayLit { elems }, self.lo.to(p.span()))));
        }
        Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
    }

    // -- struct literal: `Name { field: expr, .. }` / `Self { .. }` --

    fn struct_enter(&mut self, p: &mut Parser, ty_name: IdentId, ident_consumed: bool) -> Step {
        if !ident_consumed {
            p.bump(); // the type ident
        }
        p.bump(); // {
        let ty = p.typ(
            TypeKind::TyPath { segs: vec![PathSeg { name: ty_name, generics: Vec::new() }], is_dyn: false },
            self.lo,
        );
        self.stage = AtomStage::Struct { ty, fields: Vec::new() };
        self.struct_top(p)
    }

    fn struct_top(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::RBrace) {
            return self.struct_pop(p);
        }
        let Some(f) = p.expect_ident("a field name") else {
            p.sync_stmt();
            return self.struct_pop(p);
        };
        p.expect(Tok::Colon);
        self.cur_field = Some(f);
        Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
    }

    fn struct_pop(&mut self, p: &mut Parser) -> Step {
        let (ty, fields) = match &mut self.stage {
            AtomStage::Struct { ty, fields } => (*ty, std::mem::take(fields)),
            _ => unreachable!(),
        };
        Step::Pop(Done::Expr(p.expr(ExprKind::Struct { ty, fields }, self.lo.to(p.span()))))
    }

    // -- path building: `a.b.c`, generic args on segments, method calls --

    fn pathdots_top(&mut self, p: &mut Parser) -> Step {
        loop {
            if !p.eat_punct(Tok::Dot) {
                let segs = match &mut self.stage {
                    AtomStage::PathDots { segs } => std::mem::take(segs),
                    _ => unreachable!(),
                };
                let node = p.expr(ExprKind::Path { segs }, self.lo.to(p.span()));
                return self.finish(p, node);
            }
            let Some(name) = p.expect_ident("a member name") else {
                return Step::Pop(Done::Failed);
            };
            if matches!(p.tok(), Tok::Lt) && p.scan_is_generic_args(false) {
                p.bump();
                let segs = match &mut self.stage {
                    AtomStage::PathDots { segs } => std::mem::take(segs),
                    _ => unreachable!(),
                };
                self.stage = AtomStage::PathDotGen { segs, name, args: Vec::new() };
                return self.genarg_top(p);
            }
            if matches!(p.tok(), Tok::LParen) {
                // a call after the segment: `p.area(..)`, `Vec.from(..)`,
                // `MyMap<K, V>.new(..)` — a METHOD node over the path
                // built so far (the resolver decides static vs instance)
                p.bump();
                let segs = match &mut self.stage {
                    AtomStage::PathDots { segs } => std::mem::take(segs),
                    _ => unreachable!(),
                };
                self.stage = AtomStage::PathDotCall {
                    segs,
                    name,
                    generics: Vec::new(),
                    args: Vec::new(),
                };
                return self.call_top(p);
            }
            if let AtomStage::PathDots { segs } = &mut self.stage {
                segs.push(PathSeg { name, generics: Vec::new() });
            }
        }
    }

    /// generic-argument list top: `>` finishes, otherwise fetch an arg
    /// (an integer expression is a const-generic arg, else a type)
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
                segs[0].generics = args;
                let segs = std::mem::take(segs);
                self.stage = AtomStage::PathDots { segs };
                self.pathdots_top(p)
            }
            AtomStage::PathDotGen { segs, name, args } => {
                let args = std::mem::take(args);
                let name = *name;
                let mut segs = std::mem::take(segs);
                segs.push(PathSeg { name, generics: args });
                self.stage = AtomStage::PathDots { segs };
                self.pathdots_top(p)
            }
            AtomStage::PostDotGen { recv, name, args } => {
                let recv = *recv;
                let name = *name;
                let generics = std::mem::take(args);
                if matches!(p.tok(), Tok::LParen) {
                    p.bump();
                    self.stage = AtomStage::PostDotCall { recv, name, generics, args: Vec::new() };
                    self.call_top(p)
                } else {
                    let node = p.expr(ExprKind::Field { recv, name }, self.lo.to(p.span()));
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
                let node = p.expr(ExprKind::Call { callee, args }, self.lo.to(p.span()));
                self.stage = AtomStage::Postfix { e: node };
                self.postfix_top(p)
            }
            AtomStage::PostDotCall { recv, name, generics, args } => {
                let recv = *recv;
                let name = *name;
                let generics = std::mem::take(generics);
                let args = std::mem::take(args);
                let node = p.expr(
                    ExprKind::Method { recv, name, generics, args },
                    self.lo.to(p.span()),
                );
                self.stage = AtomStage::Postfix { e: node };
                self.postfix_top(p)
            }
            AtomStage::PathDotCall { segs, name, generics, args } => {
                let name = *name;
                let generics = std::mem::take(generics);
                let args = std::mem::take(args);
                let segs = std::mem::take(segs);
                let recv = p.expr(ExprKind::Path { segs }, self.lo);
                let node = p.expr(
                    ExprKind::Method { recv, name, generics, args },
                    self.lo.to(p.span()),
                );
                self.finish(p, node)
            }
            _ => unreachable!("call args at the wrong stage"),
        }
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match d {
            Done::Expr(e) => match &mut self.stage {
                AtomStage::Primary => self.finish(p, e), // an f-string child
                AtomStage::Paren { first, e: slot, discard } => {
                    if *discard {
                        *discard = false;
                    } else if *first {
                        *slot = Some(e);
                        *first = false;
                    }
                    self.paren_top(p)
                }
                AtomStage::Array { elems } => {
                    elems.push(e);
                    if p.eat_punct(Tok::Comma) {
                        self.array_top(p)
                    } else {
                        p.expect(Tok::RBracket);
                        let elems = std::mem::take(elems);
                        Step::Pop(Done::Expr(p.expr(ExprKind::ArrayLit { elems }, self.lo.to(p.span()))))
                    }
                }
                AtomStage::Struct { fields, .. } => {
                    let f = self.cur_field.take().expect("struct field without a name");
                    fields.push((f, e));
                    if p.eat_punct(Tok::Comma) {
                        self.struct_top(p)
                    } else {
                        p.expect(Tok::RBrace);
                        self.struct_pop(p)
                    }
                }
                AtomStage::PathGen { args, .. }
                | AtomStage::PathDotGen { args, .. }
                | AtomStage::PostDotGen { args, .. } => {
                    // const-generic argument (RFC 0005): integer expression
                    args.push(p.typ(TypeKind::TyConst(e), p.span()));
                    self.genarg_after_arg(p)
                }
                AtomStage::PostCall { args, .. }
                | AtomStage::PostDotCall { args, .. }
                | AtomStage::PathDotCall { args, .. } => {
                    args.push(e);
                    self.call_after_arg(p)
                }
                AtomStage::PostIndex { recv } => {
                    let recv = *recv;
                    p.expect(Tok::RBracket);
                    let node = p.expr(ExprKind::Index { recv, idx: e }, self.lo.to(p.span()));
                    self.stage = AtomStage::Postfix { e: node };
                    self.postfix_top(p)
                }
                AtomStage::Postfix { .. } | AtomStage::PathDots { .. } => {
                    unreachable!("atom frame received an operand at the wrong stage")
                }
            },
            Done::Ty(t) => {
                match &mut self.stage {
                    AtomStage::PathGen { args, .. }
                    | AtomStage::PathDotGen { args, .. }
                    | AtomStage::PostDotGen { args, .. } => args.push(t),
                    _ => unreachable!("atom frame received a type at the wrong stage"),
                }
                self.genarg_after_arg(p)
            }
            Done::WhenParts { scrut, arms } => {
                debug_assert!(matches!(self.stage, AtomStage::Primary));
                let node = p.expr(ExprKind::WhenExpr { scrut, arms }, self.lo.to(p.span()));
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
            _ => unreachable!("atom frame receives expressions, types, or when-parts"),
        }
    }
}

// ---- lambdas (RFC 0013 §1): `( params ) (-> Type)? => expr | block` ----

pub(crate) struct LambdaFrame {
    sp: Span,
    params: Option<Vec<NodeHandle<AnyParam>>>,
    ret: Option<NodeHandle<AnyTy>>,
}

impl LambdaFrame {
    /// `(a, b) -> T => ..` — params come from a ParamsFrame child
    pub(crate) fn paren(sp: Span) -> Self {
        LambdaFrame { sp, params: None, ret: None }
    }
    /// `x => ..` — the single param was pre-seeded by the caller
    pub(crate) fn single(sp: Span, params: Vec<NodeHandle<AnyParam>>) -> Self {
        LambdaFrame { sp, params: Some(params), ret: None }
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
        p.expect(Tok::FatArrow);
        self.body(p)
    }

    fn body(&mut self, p: &mut Parser) -> Step {
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
                p.expect(Tok::FatArrow);
                self.body(p)
            }
            Done::Expr(e) => Step::Pop(Done::Expr(self.mk(p, e))),
            Done::Block(b) => Step::Pop(Done::Expr(self.mk(p, b.into()))),
            Done::Failed => Step::Pop(Done::Failed),
            _ => unreachable!("lambda frame receives params/types/exprs"),
        }
    }

    fn mk(&mut self, p: &mut Parser, body: NodeHandle<AnyExpr>) -> NodeHandle<AnyExpr> {
        let params = self.params.take().expect("lambda without params");
        p.expr(ExprKind::Lambda { params, ret: self.ret.take(), body }, self.sp.to(p.span()))
    }
}

// ---- f-strings (RFC 0030 §1.1/§4.4): holes are sub-slice frames ----

pub(crate) struct FStrFrame {
    sp: Span,
    parts_in: Vec<FPart>,
    idx: usize,
    parts: Vec<FPartAst>,
    /// the outer token slice, position, and budgets, saved while a hole
    /// is being parsed by the same loop (no nested parse call)
    saved: Option<(Vec<Token>, usize, u32, u32)>,
}

impl FStrFrame {
    pub(crate) fn new(f: FStrTok, sp: Span) -> Self {
        FStrFrame { sp, parts_in: f.parts, idx: 0, parts: Vec::new(), saved: None }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        while self.idx < self.parts_in.len() {
            match self.parts_in[self.idx].clone() {
                FPart::Lit(s) => {
                    self.parts.push(FPartAst::Lit(s));
                    self.idx += 1;
                }
                FPart::Hole(hole_toks) => {
                    // sub-parse over the hole's token slice: the same
                    // loop, a frame over the sub-slice (RFC 0030 §4.4)
                    self.idx += 1;
                    let mut toks = hole_toks;
                    toks.push(Token { tok: Tok::Eof, span: self.sp });
                    self.saved = Some(p.switch_to_hole(toks));
                    return Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)));
                }
            }
        }
        Step::Pop(Done::Expr(p.expr(ExprKind::FStr { parts: std::mem::take(&mut self.parts) }, self.sp)))
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

// ---- `await select { .. }` (RFC 0019 §3) ----

pub(crate) struct SelectFrame {
    sp: Span,
    arms: Vec<NodeHandle<AnyArm>>,
    cur: Option<(NodeHandle<AnyExpr>, Option<IdentId>)>,
    cur_sp: Span,
}

impl SelectFrame {
    pub(crate) fn new(sp: Span) -> Self {
        SelectFrame { sp, arms: Vec::new(), cur: None, cur_sp: sp }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        p.bump(); // await
        p.bump(); // select
        p.expect(Tok::LBrace);
        self.arms_top(p)
    }

    fn arms_top(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::RBrace) {
            let arms = std::mem::take(&mut self.arms);
            return Step::Pop(Done::Expr(p.expr(ExprKind::Select { arms }, self.sp.to(p.span()))));
        }
        self.cur_sp = p.span();
        Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match d {
            Done::Expr(e) => match self.cur.take() {
                None => {
                    // the future — `as name` binds it (RFC 0019 §3)
                    let bind = if p.at_kw("as") {
                        p.bump();
                        let Some(id) = p.expect_ident("a binding name") else {
                            return Step::Pop(Done::Failed);
                        };
                        Some(id)
                    } else {
                        None
                    };
                    p.expect(Tok::Arrow);
                    self.cur = Some((e, bind));
                    Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
                }
                Some((fut, bind)) => {
                    let node = p.arm(
                        ArmKind::SelectArm { fut, bind, body: e },
                        self.cur_sp.to(p.span()),
                    );
                    self.arms.push(node);
                    p.eat_punct(Tok::Comma);
                    self.arms_top(p)
                }
            },
            Done::Failed => Step::Pop(Done::Failed),
            _ => unreachable!("select frame receives expressions"),
        }
    }
}
