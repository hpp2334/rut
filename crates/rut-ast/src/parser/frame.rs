//! The frame stack: `Done` (a completed child's result,
//! handed to the parent through the parser's inbox), `Step` (a frame's
//! next action — push a child, or pop with a result), and `Frame` (one
//! kind per grammar rule; each holds the children collected so far and
//! the stage it suspended at). The structure frames live here and in
//! `item.rs`/`stmt.rs`; the Pratt engine and its helpers live in
//! `expr.rs`; types in `ty.rs`.
//!
//! v2: children travel as SLOTS (`ItemRef`, `StmtRef`, …); a failed
//! child FENCES — the parent's recovery allocates an `ErrorNode` into
//! the very list the region belongs to, carrying the region's diags.

use crate::{
    BlockNode, ExprRef, Handle, IfNode, ItemRef, ModuleNode, ParamRef, PatRef, StmtRef, TyRef, Vis,
};
use rut_lexer::span::Span;
use rut_lexer::token::Tok;

use crate::parser::expr::{AtomFrame, ExprFrame, ExprMode, FStrFrame, LambdaFrame};
use crate::parser::item::{
    classify_item, classify_pub, EnumFrame, FnFrame, ImplFrame, InterfaceFrame, MethodFrame,
    ModDeclFrame, ModInlineFrame, ModuleLetFrame, ParamsFrame, SurfaceFrame, TyDeclFrame,
    TypeAliasFrame, TypeBodyFrame, UseFrame,
};
use crate::parser::stmt::{BlockFrame, IfFrame, PatternFrame, StmtFrame, WhenFrame};
use crate::parser::ty::TypeFrame;
use crate::parser::Parser;

/// A popped frame's result. `Failed` means the child bailed after its
/// own diagnostic and recovery — the parent fences the region.
#[derive(Debug)]
pub(crate) enum Done {
    Root(Handle<ModuleNode>),
    Item(ItemRef),
    Stmt(StmtRef),
    If(Handle<IfNode>),
    Expr(ExprRef),
    /// a block's tail expression — the `}`-closed last statement
    /// (no `;`): handed to the enclosing `BlockFrame`, which records it
    /// as the block's implicit value
    TailExpr(ExprRef),
    Block(Handle<BlockNode>),
    Ty(TyRef),
    Pat(PatRef),
    Members(Vec<ParamRef>),
    Method(crate::MethodRef),
    /// a struct/class/interface/impl body: (fields, methods) — slots
    Body(Vec<crate::FieldRef>, Vec<crate::MethodRef>),
    WhenParts {
        scrut: ExprRef,
        arms: Vec<Handle<crate::WhenArmNode>>,
    },
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
    ModDecl(ModDeclFrame),
    ModInline(ModInlineFrame),
    Alias(TypeAliasFrame),
    ModuleLet(ModuleLetFrame),
    Enum(EnumFrame),
    Struct(TyDeclFrame),
    Class(TyDeclFrame),
    Interface(InterfaceFrame),
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
}

impl Frame {
    pub(crate) fn step(&mut self, p: &mut Parser, done: Option<Done>) -> Step {
        match done {
            None => match self {
                Frame::Module(f) => f.step(p),
                Frame::Pub(f) => f.step(p),
                Frame::Use(f) => f.step(p),
                Frame::ModDecl(f) => f.step(p),
                Frame::ModInline(f) => f.step(p),
                Frame::Alias(f) => f.step(p),
                Frame::ModuleLet(f) => f.step(p),
                Frame::Enum(f) => f.step(p),
                Frame::Struct(f) => f.step(p),
                Frame::Class(f) => f.step(p),
                Frame::Interface(f) => f.step(p),
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
            },
            Some(d) => match self {
                Frame::Module(f) => f.absorb(p, d),
                Frame::Pub(f) => f.absorb(p, d),
                Frame::Use(f) => f.absorb(p, d),
                Frame::ModDecl(f) => f.absorb(p, d),
                Frame::ModInline(f) => f.absorb(p, d),
                Frame::Alias(f) => f.absorb(p, d),
                Frame::ModuleLet(f) => f.absorb(p, d),
                Frame::Enum(f) => f.absorb(p, d),
                Frame::Struct(f) => f.absorb(p, d),
                Frame::Class(f) => f.absorb(p, d),
                Frame::Interface(f) => f.absorb(p, d),
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
            },
        }
    }

    /// Frames whose stack depth is bounded by the shared NEST_MAX budget
    /// (C3).
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

// ---- module (declarations only) ----

pub(crate) struct ModuleFrame {
    items: Vec<ItemRef>,
    /// cursor position + diag watermark when the current item started
    /// (the fence's extent)
    before: usize,
    dmark: usize,
}

impl ModuleFrame {
    pub(crate) fn new(pre_items: Vec<ItemRef>) -> Self {
        ModuleFrame {
            items: pre_items,
            before: 0,
            dmark: 0,
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        loop {
            if p.at_eof() {
                let m = crate::ModuleNode {
                    span: Span::new(0, p.span().hi),
                    items: std::mem::take(&mut self.items),
                }
                .alloc_plain(&mut p.ast);
                return Step::Pop(Done::Root(m));
            }
            self.before = p.pos;
            self.dmark = p.mark();
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

    /// the item-level FENCE: progress guarantee, resync, then the
    /// region becomes an ErrorNode in the items list — identity damage
    /// (or a failed item) never vanishes
    fn recover(&mut self, p: &mut Parser) {
        if p.pos == self.before {
            p.err_here("expected a declaration");
            p.bump();
        }
        p.sync_item();
        let (before, mark) = (self.before, self.dmark);
        self.items.push(ItemRef::Error(p.fence(before, mark)));
    }
}

// ---- pub ----

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

/// The optional scope after `pub`: nothing = public, `(super|pkg)` =
/// the parent-module / package scopes.
pub(crate) fn pub_scope(p: &mut Parser) -> Vis {
    if p.eat_punct(Tok::LParen) {
        let v = match p.tok().clone() {
            Tok::Ident(m) if m == "super" => Vis::Super,
            Tok::Ident(m) if m == "pkg" => Vis::Pkg,
            Tok::Ident(m) if m == "mod" || m == "self" => {
                p.err_here(
                    "`pub(mod)`/`pub(self)` are gone — bare private is the default; widen with `pub(super)`, `pub(pkg)`, or `pub`",
                );
                Vis::Self_
            }
            _ => {
                p.err_here("expected `super` or `pkg` in pub(..)");
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
