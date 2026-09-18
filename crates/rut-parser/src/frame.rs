//! The frame stack (RFC 0030 §4): `Done` (a completed child's result,
//! handed to the parent through the parser's inbox), `Step` (a frame's
//! next action — push a child, or pop with a result), and `Frame` (one
//! kind per grammar rule; each holds the children collected so far and
//! the stage it suspended at). The structure frames live here and in
//! `item.rs`/`stmt.rs`; the Pratt engine and its helpers live in
//! `expr.rs`; types in `ty.rs`.

use rut_ast::ast::*;
use rut_lexer::span::Span;
use rut_lexer::token::Tok;

use crate::expr::{AtomFrame, ExprFrame, ExprMode, FStrFrame, LambdaFrame, SelectFrame};
use crate::item::{
    classify_item, classify_pub, EnumFrame, FnFrame, ImplFrame, TypeAliasFrame, UseFrame,
    MethodFrame, ModuleLetFrame, ParamsFrame, SurfaceFrame, TraitFrame, TyDeclFrame, TypeBodyFrame,
};
use crate::stmt::{BlockFrame, IfFrame, PatternFrame, StmtFrame, WhenFrame};
use crate::ty::TypeFrame;
use crate::Parser;

/// A popped frame's result. `Failed` means the child bailed after its
/// own diagnostic and recovery — the propagation semantics of the `?`
/// operator in the recursive-descent v1.
#[derive(Debug)]
pub(crate) enum Done {
    Root(NodeHandle<ModuleNode>),
    Item(NodeHandle<AnyItem>),
    Stmt(NodeHandle<AnyStmt>),
    If(NodeHandle<IfNode>),
    Expr(NodeHandle<AnyExpr>),
    Block(NodeHandle<BlockNode>),
    Ty(NodeHandle<AnyTy>),
    Pat(NodeHandle<AnyPat>),
    Members(Vec<NodeHandle<AnyParam>>),
    Method(NodeHandle<MethodDeclNode>),
    /// a struct/class/trait/impl body: (fields, methods)
    Body(Vec<NodeHandle<FieldDeclNode>>, Vec<NodeHandle<MethodDeclNode>>),
    WhenParts { scrut: NodeHandle<AnyExpr>, arms: Vec<NodeHandle<AnyArm>> },
    Failed,
}

pub(crate) enum Step {
    Push(Frame),
    Pop(Done),
}

pub(crate) enum Frame {
    Module(ModuleFrame),
    Pub(PubFrame),
    Use(UseFrame),
    Alias(TypeAliasFrame),
    ModuleLet(ModuleLetFrame),
    Enum(EnumFrame),
    Dataclass(TyDeclFrame),
    Class(TyDeclFrame),
    Trait(TraitFrame),
    Impl(ImplFrame),
    Fn(FnFrame),
    Method(MethodFrame),
    Surface(SurfaceFrame),
    TypeBody(TypeBodyFrame),
    Params(ParamsFrame),
    Block(BlockFrame),
    Stmt(StmtFrame),
    If(IfFrame),
    When(WhenFrame),
    Pattern(PatternFrame),
    Type(TypeFrame),
    Expr(ExprFrame),
    Atom(AtomFrame),
    Lambda(LambdaFrame),
    FStr(FStrFrame),
    Select(SelectFrame),
}

impl Frame {
    pub(crate) fn step(&mut self, p: &mut Parser, done: Option<Done>) -> Step {
        match done {
            None => match self {
                Frame::Module(f) => f.step(p),
                Frame::Pub(f) => f.step(p),
                Frame::Use(f) => f.step(p),
                Frame::Alias(f) => f.step(p),
                Frame::ModuleLet(f) => f.step(p),
                Frame::Enum(f) => f.step(p),
                Frame::Dataclass(f) => f.step(p),
                Frame::Class(f) => f.step(p),
                Frame::Trait(f) => f.step(p),
                Frame::Impl(f) => f.step(p),
                Frame::Fn(f) => f.step(p),
                Frame::Method(f) => f.step(p),
                Frame::Surface(f) => f.step(p),
                Frame::TypeBody(f) => f.step(p),
                Frame::Params(f) => f.step(p),
                Frame::Block(f) => f.step(p),
                Frame::Stmt(f) => f.step(p),
                Frame::If(f) => f.step(p),
                Frame::When(f) => f.step(p),
                Frame::Pattern(f) => f.step(p),
                Frame::Type(f) => f.step(p),
                Frame::Expr(f) => f.step(p),
                Frame::Atom(f) => f.step(p),
                Frame::Lambda(f) => f.step(p),
                Frame::FStr(f) => f.step(p),
                Frame::Select(f) => f.step(p),
            },
            Some(d) => match self {
                Frame::Module(f) => f.absorb(p, d),
                Frame::Pub(f) => f.absorb(p, d),
                Frame::Use(f) => f.absorb(p, d),
                Frame::Alias(f) => f.absorb(p, d),
                Frame::ModuleLet(f) => f.absorb(p, d),
                Frame::Enum(f) => f.absorb(p, d),
                Frame::Dataclass(f) => f.absorb(p, d),
                Frame::Class(f) => f.absorb(p, d),
                Frame::Trait(f) => f.absorb(p, d),
                Frame::Impl(f) => f.absorb(p, d),
                Frame::Fn(f) => f.absorb(p, d),
                Frame::Method(f) => f.absorb(p, d),
                Frame::Surface(f) => f.absorb(p, d),
                Frame::TypeBody(f) => f.absorb(p, d),
                Frame::Params(f) => f.absorb(p, d),
                Frame::Block(f) => f.absorb(p, d),
                Frame::Stmt(f) => f.absorb(p, d),
                Frame::If(f) => f.absorb(p, d),
                Frame::When(f) => f.absorb(p, d),
                Frame::Pattern(f) => f.absorb(p, d),
                Frame::Type(f) => f.absorb(p, d),
                Frame::Expr(f) => f.absorb(p, d),
                Frame::Atom(f) => f.absorb(p, d),
                Frame::Lambda(f) => f.absorb(p, d),
                Frame::FStr(f) => f.absorb(p, d),
                Frame::Select(f) => f.absorb(p, d),
            },
        }
    }

    /// Frames whose stack depth is bounded by the shared NEST_MAX budget
    /// (C3). These are the rules whose v1 counterparts called `enter()` —
    /// plus types, which v1 left unbounded (a latent C3 hole: deeply
    /// nested generic arguments recursed `parse_type` without any guard).
    pub(crate) fn nests(&self) -> bool {
        match self {
            Frame::Block(_) | Frame::Type(_) => true,
            Frame::Expr(f) => f.mode == ExprMode::UnaryOnly,
            _ => false,
        }
    }

    /// Full-expression frames — the EXPR_MAX budget.
    pub(crate) fn counts_full_expr(&self) -> bool {
        match self {
            Frame::Expr(f) => f.mode == ExprMode::Full,
            _ => false,
        }
    }
}

// ---- module (RFC 0003 §1: declarations only) ----

pub(crate) struct ModuleFrame {
    items: Vec<NodeHandle<AnyItem>>,
    /// cursor position when the current item started (progress guard)
    before: usize,
}

impl ModuleFrame {
    pub(crate) fn new() -> Self {
        ModuleFrame { items: Vec::new(), before: 0 }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        loop {
            if p.at_eof() {
                let hi = p.span().hi;
                let m = p.module(std::mem::take(&mut self.items), Span::new(0, hi));
                return Step::Pop(Done::Root(m));
            }
            self.before = p.pos;
            match classify_item(p) {
                Some(fr) => return Step::Push(fr),
                None => self.recover(p),
            }
        }
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match d {
            Done::Item(i) => self.items.push(i),
            Done::Failed => self.recover(p),
            _ => unreachable!("module frame receives items"),
        }
        self.step(p)
    }

    /// parse_module's None-arm: `classify_item` already diagnosed itself;
    /// guarantee progress and resync at the module level
    fn recover(&mut self, p: &mut Parser) {
        if p.pos == self.before {
            p.err_here("expected a declaration");
            p.bump();
        }
        p.sync_item();
    }
}

// ---- pub (RFC 0003 §2) ----

pub(crate) struct PubFrame {
    vis: Vis,
}

impl PubFrame {
    pub(crate) fn new() -> Self {
        PubFrame { vis: Vis::Self_ }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        p.bump(); // pub
        self.vis = pub_scope(p);
        match classify_pub(p, self.vis) {
            Some(fr) => Step::Push(fr),
            None => Step::Pop(Done::Failed),
        }
    }

    pub(crate) fn absorb(&mut self, _p: &mut Parser, d: Done) -> Step {
        match d {
            Done::Item(i) => Step::Pop(Done::Item(i)),
            _ => Step::Pop(Done::Failed),
        }
    }
}

/// The optional scope after `pub` (RFC 0003 §2): nothing = public,
/// `(mod|super|self)` = the package/parent/module scopes. Shared by
/// module items and class members (RFC 0010 §2).
pub(crate) fn pub_scope(p: &mut Parser) -> Vis {
    if p.eat_punct(Tok::LParen) {
        let v = match p.tok().clone() {
            Tok::Ident(m) if m == "mod" => Vis::Mod,
            Tok::Ident(m) if m == "super" => Vis::Super,
            Tok::Ident(m) if m == "self" => Vis::Self_,
            _ => {
                p.err_here("expected `mod`, `super`, or `self` in pub(..)");
                p.bump();
                p.expect(Tok::RParen);
                return Vis::Self_;
            }
        };
        p.bump();
        p.expect(Tok::RParen);
        v
    } else {
        Vis::Pub
    }
}
