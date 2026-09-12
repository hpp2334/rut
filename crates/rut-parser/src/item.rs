//! Declarations (RFC 0030 §2): items, exports, imports, type bodies,
//! fn/method signatures, surface stubs — as frames. `classify_item` is
//! v1's `parse_item` dispatch (peek 1, keyword-led); the frames carry
//! the state each rule accumulated across child pushes.

use rut_ast::ast::*;
use rut_lexer::span::Span;
use rut_lexer::token::Tok;

use crate::expr::{ExprFrame, ExprMode};
use crate::frame::{Done, ExportFrame, Frame, Step};
use crate::stmt::BlockFrame;
use crate::ty::TypeFrame;
use crate::{is_primitive_ty, Mode, Parser};

// ---- item dispatch (v1's parse_item) ----

pub(crate) fn classify_item(p: &mut Parser) -> Option<Frame> {
    let sp = p.span();
    match p.tok().clone() {
        Tok::Ident(kw) => match kw.as_str() {
            "import" => Some(Frame::Import(ImportFrame::new())),
            "export" => Some(Frame::Export(ExportFrame::new())),
            "let" => Some(Frame::ModuleLet(ModuleLetFrame::new(Vis::Self_))),
            "enum" => Some(Frame::Enum(EnumFrame::new(Vis::Self_))),
            "dataclass" => Some(Frame::Dataclass(TyDeclFrame::new(false, Vis::Self_))),
            "class" => Some(Frame::Class(TyDeclFrame::new(true, Vis::Self_))),
            "trait" => Some(Frame::Trait(TraitFrame::new(Vis::Self_))),
            "impl" => Some(Frame::Impl(ImplFrame::new())),
            "suspend" if p.at_kw2("fn") => {
                // RFC 0030 §2: `suspend fn` — M1 parses it; the compiler
                // rejects with a targeted M3 message
                p.bump();
                Some(Frame::Fn(FnFrame::new(Vis::Self_, true, false)))
            }
            "fn" => Some(Frame::Fn(FnFrame::new(Vis::Self_, false, false))),
            // `entry fn` — the host-callable surface (RFC 0035 §3):
            // contextual; only special directly before `fn`
            "entry" if p.at_kw2("fn") => {
                p.bump();
                Some(Frame::Fn(FnFrame::new(Vis::Self_, false, true)))
            }
            "host" | "extern" => Some(Frame::Surface(SurfaceFrame::new())),
            // statement keywords at module scope: RFC 0003 §1
            "if" | "while" | "for" | "return" | "when" | "break" | "continue" | "await" => {
                p.err(
                    sp,
                    "statements are not allowed at module scope —modules contain declarations only (RFC 0003 §1)",
                );
                None
            }
            _ => {
                p.err(sp, format!("expected a declaration, found `{kw}`"));
                None
            }
        },
        _ => {
            let found = p.peek(0).describe();
            p.err(sp, format!("expected a declaration, found {found}"));
            None
        }
    }
}

/// v1's parse_export_item inner dispatch (the visibility was parsed by
/// the export frame itself)
pub(crate) fn classify_export(p: &mut Parser, vis: Vis) -> Option<Frame> {
    match p.tok().clone() {
        Tok::Ident(kw) => match kw.as_str() {
            "let" => Some(Frame::ModuleLet(ModuleLetFrame::new(vis))),
            "enum" => Some(Frame::Enum(EnumFrame::new(vis))),
            "dataclass" => Some(Frame::Dataclass(TyDeclFrame::new(false, vis))),
            "class" => Some(Frame::Class(TyDeclFrame::new(true, vis))),
            "trait" => Some(Frame::Trait(TraitFrame::new(vis))),
            "suspend" if p.at_kw2("fn") => {
                p.bump();
                Some(Frame::Fn(FnFrame::new(vis, true, false)))
            }
            "fn" => Some(Frame::Fn(FnFrame::new(vis, false, false))),
            "host" | "extern" => Some(Frame::Surface(SurfaceFrame::new())),
            _ => {
                p.err_here(format!("`export` must precede a declaration, found `{kw}`"));
                None
            }
        },
        _ => {
            let found = p.peek(0).describe();
            p.err_here(format!("`export` must precede a declaration, found {found}"));
            None
        }
    }
}

// ---- import ----

pub(crate) struct ImportFrame {
    lo: u32,
    names: Vec<IdentId>,
}

impl ImportFrame {
    pub(crate) fn new() -> Self {
        ImportFrame { lo: 0, names: Vec::new() }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = p.bump().span.lo; // import
        if p.expect(Tok::LBrace).is_none() {
            return Step::Pop(Done::Failed);
        }
        loop {
            if p.eat_punct(Tok::RBrace) {
                break;
            }
            let Some(n) = p.expect_ident("an imported name") else {
                p.sync_stmt();
                return Step::Pop(Done::Failed);
            };
            self.names.push(n);
            if !p.eat_punct(Tok::Comma) {
                p.expect(Tok::RBrace);
                break;
            }
        }
        if !p.at_kw("from") {
            p.err_here("expected `from` after the import list");
        } else {
            p.bump();
        }
        let from = match p.tok().clone() {
            Tok::Str(s) | Tok::RawStr(s) => {
                p.bump();
                s
            }
            _ => {
                p.err_here("expected a module specifier string after `from`");
                String::new()
            }
        };
        p.expect(Tok::Semi);
        let names = std::mem::take(&mut self.names);
        Step::Pop(Done::Item(p.item(
            ItemKind::Import { names, from },
            Span::new(self.lo, p.span().hi),
        )))
    }

    pub(crate) fn absorb(&mut self, _p: &mut Parser, _d: Done) -> Step {
        unreachable!("import frame pushes no children")
    }
}

// ---- module-level `let` ----

pub(crate) struct ModuleLetFrame {
    vis: Vis,
    lo: u32,
    stage: MlStage,
    name: IdentId,
    ty: Option<NodeHandle<AnyTy>>,
}

#[derive(Clone, Copy)]
enum MlStage {
    Ty,
    Init,
}

impl ModuleLetFrame {
    pub(crate) fn new(vis: Vis) -> Self {
        ModuleLetFrame { vis, lo: 0, stage: MlStage::Ty, name: IdentId(0), ty: None }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = p.bump().span.lo; // let
        let Some(name) = p.expect_ident("a binding name") else {
            return Step::Pop(Done::Failed);
        };
        self.name = name;
        if p.eat_punct(Tok::Colon) {
            return Step::Push(Frame::Type(TypeFrame::new(p)));
        }
        self.eq(p)
    }

    fn eq(&mut self, p: &mut Parser) -> Step {
        p.expect(Tok::Eq);
        self.stage = MlStage::Init;
        Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match (self.stage, d) {
            (MlStage::Ty, Done::Ty(t)) => {
                self.ty = Some(t);
                self.eq(p)
            }
            (MlStage::Init, Done::Expr(init)) => {
                p.expect(Tok::Semi);
                let node = p.item(
                    ItemKind::ModuleLet {
                        vis: self.vis,
                        name: self.name,
                        ty: self.ty.take(),
                        init,
                    },
                    Span::new(self.lo, p.span().hi),
                );
                Step::Pop(Done::Item(node))
            }
            (_, Done::Failed) => Step::Pop(Done::Failed),
            _ => unreachable!("module let receives a type or an initializer"),
        }
    }
}

// ---- enum ----

pub(crate) struct EnumFrame {
    vis: Vis,
    lo: u32,
    name: IdentId,
    members: Vec<(IdentId, Option<i64>)>,
}

impl EnumFrame {
    pub(crate) fn new(vis: Vis) -> Self {
        EnumFrame { vis, lo: 0, name: IdentId(0), members: Vec::new() }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = p.bump().span.lo; // enum
        let Some(name) = p.expect_ident("an enum name") else {
            return Step::Pop(Done::Failed);
        };
        self.name = name;
        p.expect(Tok::LBrace);
        loop {
            if p.eat_punct(Tok::RBrace) {
                break;
            }
            let Some(m) = p.expect_ident("an enum member") else {
                p.sync_stmt();
                return Step::Pop(Done::Failed);
            };
            let mut val = None;
            if p.eat_punct(Tok::Eq) {
                let neg = p.eat_punct(Tok::Minus);
                match p.tok().clone() {
                    Tok::Int(v, _) => {
                        p.bump();
                        val = Some(v as i64 * if neg { -1 } else { 1 });
                    }
                    _ => p.err_here("expected an integer after `=` in an enum member"),
                }
            }
            self.members.push((m, val));
            if !p.eat_punct(Tok::Comma) {
                p.expect(Tok::RBrace);
                break;
            }
        }
        let members = std::mem::take(&mut self.members);
        let node = p.item(
            ItemKind::Enum { vis: self.vis, name: self.name, members },
            Span::new(self.lo, p.span().hi),
        );
        Step::Pop(Done::Item(node))
    }

    pub(crate) fn absorb(&mut self, _p: &mut Parser, _d: Done) -> Step {
        unreachable!("enum frame pushes no children")
    }
}

// ---- generic parameter list (inline — no child frames) ----

fn generic_params(p: &mut Parser) -> Vec<IdentId> {
    p.bump(); // < (the caller checked)
    let mut out = Vec::new();
    loop {
        if p.eat_punct(Tok::Gt) {
            break;
        }
        let Some(n) = p.expect_ident("a generic parameter") else {
            return out;
        };
        out.push(n);
        if !p.eat_punct(Tok::Comma) {
            p.expect_gt();
            break;
        }
    }
    out
}

// ---- dataclass / class ----

pub(crate) struct TyDeclFrame {
    is_class: bool,
    vis: Vis,
    lo: u32,
    name: IdentId,
    generics: Vec<IdentId>,
}

impl TyDeclFrame {
    pub(crate) fn new(is_class: bool, vis: Vis) -> Self {
        TyDeclFrame { is_class, vis, lo: 0, name: IdentId(0), generics: Vec::new() }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = p.bump().span.lo; // dataclass | class
        let what = if self.is_class { "a class name" } else { "a dataclass name" };
        let Some(name) = p.expect_ident(what) else {
            return Step::Pop(Done::Failed);
        };
        self.name = name;
        if matches!(p.tok(), Tok::Lt) {
            self.generics = generic_params(p);
        }
        Step::Push(Frame::TypeBody(TypeBodyFrame::new(BodyMode::Class {
            allow_private: self.is_class,
            is_dataclass: !self.is_class,
        })))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match d {
            Done::Body(fields, methods) => {
                let (vis, name, generics) = (self.vis, self.name, std::mem::take(&mut self.generics));
                let kind = if self.is_class {
                    ItemKind::Class { vis, name, generics, fields, methods }
                } else {
                    ItemKind::Dataclass { vis, name, generics, fields, methods }
                };
                Step::Pop(Done::Item(p.item(kind, Span::new(self.lo, p.span().hi))))
            }
            Done::Failed => Step::Pop(Done::Failed),
            _ => unreachable!("type declarations receive a body"),
        }
    }
}

// ---- trait ----

pub(crate) struct TraitFrame {
    vis: Vis,
    lo: u32,
    stage: TrStage,
    name: IdentId,
    generics: Vec<IdentId>,
    requires: Vec<NodeHandle<AnyTy>>,
}

#[derive(Clone, Copy)]
enum TrStage {
    Requires,
    Body,
}

impl TraitFrame {
    pub(crate) fn new(vis: Vis) -> Self {
        TraitFrame {
            vis,
            lo: 0,
            stage: TrStage::Requires,
            name: IdentId(0),
            generics: Vec::new(),
            requires: Vec::new(),
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = p.bump().span.lo; // trait
        let Some(name) = p.expect_ident("a trait name") else {
            return Step::Pop(Done::Failed);
        };
        self.name = name;
        if matches!(p.tok(), Tok::Lt) {
            self.generics = generic_params(p);
        }
        if p.at_kw("requires") {
            p.bump();
            return Step::Push(Frame::Type(TypeFrame::new(p)));
        }
        self.stage = TrStage::Body;
        Step::Push(Frame::TypeBody(TypeBodyFrame::new(BodyMode::Trait)))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match (self.stage, d) {
            (TrStage::Requires, Done::Ty(t)) => {
                self.requires.push(t);
                if p.eat_punct(Tok::Comma) {
                    Step::Push(Frame::Type(TypeFrame::new(p)))
                } else {
                    self.stage = TrStage::Body;
                    Step::Push(Frame::TypeBody(TypeBodyFrame::new(BodyMode::Trait)))
                }
            }
            // v1: a failed requires entry breaks to the body
            (TrStage::Requires, Done::Failed) => {
                self.stage = TrStage::Body;
                Step::Push(Frame::TypeBody(TypeBodyFrame::new(BodyMode::Trait)))
            }
            (TrStage::Body, Done::Body(_, methods)) => {
                let (vis, name, generics, requires) = (
                    self.vis,
                    self.name,
                    std::mem::take(&mut self.generics),
                    std::mem::take(&mut self.requires),
                );
                let node = p.item(
                    ItemKind::Trait { vis, name, generics, requires, methods },
                    Span::new(self.lo, p.span().hi),
                );
                Step::Pop(Done::Item(node))
            }
            (TrStage::Body, Done::Failed) => Step::Pop(Done::Failed),
            _ => unreachable!("trait frame receives types or a body"),
        }
    }
}

// ---- impl ----

pub(crate) struct ImplFrame {
    lo: u32,
    stage: ImStage,
    trait_ref: Option<NodeHandle<AnyTy>>,
    target: Option<NodeHandle<AnyTy>>,
}

#[derive(Clone, Copy)]
enum ImStage {
    TraitRef,
    Target,
    Methods,
}

impl ImplFrame {
    pub(crate) fn new() -> Self {
        ImplFrame { lo: 0, stage: ImStage::TraitRef, trait_ref: None, target: None }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = p.bump().span.lo; // impl
        if p.mode == Mode::Decl {
            p.err(
                Span::new(self.lo, self.lo + 4),
                "implementation in a declaration file —`impl` blocks live in `.rut` (RFC 0029 §2)",
            );
        }
        Step::Push(Frame::Type(TypeFrame::new(p)))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match (self.stage, d) {
            (ImStage::TraitRef, Done::Ty(t)) => {
                self.trait_ref = Some(t);
                if !p.at_kw("for") {
                    p.err_here("expected `for` in `impl Trait for Type`");
                } else {
                    p.bump();
                }
                self.stage = ImStage::Target;
                Step::Push(Frame::Type(TypeFrame::new(p)))
            }
            (ImStage::Target, Done::Ty(t)) => {
                self.target = Some(t);
                self.stage = ImStage::Methods;
                Step::Push(Frame::TypeBody(TypeBodyFrame::new(BodyMode::Impl)))
            }
            (ImStage::Methods, Done::Body(_, methods)) => {
                let node = p.item(
                    ItemKind::Impl {
                        trait_ref: self.trait_ref.take().expect("impl without a trait"),
                        target: self.target.take().expect("impl without a target"),
                        methods,
                    },
                    Span::new(self.lo, p.span().hi),
                );
                Step::Pop(Done::Item(node))
            }
            (_, Done::Failed) => Step::Pop(Done::Failed),
            _ => unreachable!("impl frame receives types or a body"),
        }
    }
}

// ---- dataclass/class/trait/impl bodies ----

pub(crate) enum BodyMode {
    /// dataclass/class body: fields and methods
    Class { allow_private: bool, is_dataclass: bool },
    /// trait body: method signatures only
    Trait,
    /// impl body: trait method implementations
    Impl,
}

pub(crate) struct TypeBodyFrame {
    mode: BodyMode,
    fields: Vec<NodeHandle<FieldDeclNode>>,
    methods: Vec<NodeHandle<MethodDeclNode>>,
    stage: TbStage,
    /// the field under construction
    f_lo: Span,
    f_private: bool,
    f_static: bool,
    f_name: IdentId,
    f_ty: Option<NodeHandle<AnyTy>>,
}

#[derive(Clone, Copy)]
enum TbStage {
    Open,
    Members,
    /// waiting for the current field's type
    FieldTy,
    /// waiting for the current field's initializer
    FieldInit,
    /// waiting for a method declaration
    Method,
}

impl TypeBodyFrame {
    pub(crate) fn new(mode: BodyMode) -> Self {
        TypeBodyFrame {
            mode,
            fields: Vec::new(),
            methods: Vec::new(),
            stage: TbStage::Open,
            f_lo: Span::new(0, 0),
            f_private: false,
            f_static: false,
            f_name: IdentId(0),
            f_ty: None,
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        // v1 brace strictness: dataclass/class and trait bodies bail on a
        // missing `{`; impl and surface bodies had already expected it
        let strict = matches!(self.mode, BodyMode::Class { .. } | BodyMode::Trait);
        if strict {
            if p.expect(Tok::LBrace).is_none() {
                return Step::Pop(Done::Failed);
            }
        } else {
            p.expect(Tok::LBrace);
        }
        self.stage = TbStage::Members;
        self.members_top(p)
    }

    fn members_top(&mut self, p: &mut Parser) -> Step {
        loop {
            if p.eat_punct(Tok::RBrace) || p.at_eof() {
                let fields = std::mem::take(&mut self.fields);
                let methods = std::mem::take(&mut self.methods);
                return Step::Pop(Done::Body(fields, methods));
            }
            match self.mode {
                BodyMode::Class { allow_private, is_dataclass } => {
                    let lo = p.span();
                    let mut is_private = false;
                    let mut is_static = false;
                    let mut is_suspend = false;
                    while let Tok::Ident(m) = p.tok().clone() {
                        match m.as_str() {
                            "private" => {
                                is_private = true;
                                p.bump();
                                if !allow_private {
                                    p.err(
                                        lo,
                                        "`private` is not allowed in a dataclass —all fields are public (RFC 0009); privacy is the class's job (RFC 0010)",
                                    );
                                }
                            }
                            "static" => {
                                is_static = true;
                                p.bump();
                                if is_dataclass {
                                    p.err(lo, "dataclasses have no `static` members (RFC 0009)");
                                }
                            }
                            "suspend" => {
                                is_suspend = true;
                                p.bump();
                            }
                            _ => break,
                        }
                    }
                    match p.tok().clone() {
                        Tok::Ident(kw) if kw == "fn" => {
                            if is_static {
                                p.err(
                                    lo,
                                    "there is no `static fn` —a method without `self` IS a class method (RFC 0010 §2)",
                                );
                            }
                            self.stage = TbStage::Method;
                            return Step::Push(Frame::Method(MethodFrame::new(is_private, is_suspend, true)));
                        }
                        Tok::Ident(_) => {
                            let name = p.expect_ident("a field name").unwrap_or(IdentId(0));
                            p.expect(Tok::Colon);
                            self.stage = TbStage::FieldTy;
                            self.f_lo = lo;
                            self.f_private = is_private;
                            self.f_static = is_static;
                            self.f_name = name;
                            return Step::Push(Frame::Type(TypeFrame::new(p)));
                        }
                        _ => {
                            let found = p.peek(0).describe();
                            p.err_here(format!("expected a field or method, found {found}"));
                            p.sync_stmt();
                            // sync consumes at least one token (or stops
                            // at `}` / EOF, both caught at the loop top)
                        }
                    }
                }
                BodyMode::Trait => {
                    if p.at_kw("fn") {
                        self.stage = TbStage::Method;
                        return Step::Push(Frame::Method(MethodFrame::new(false, false, false)));
                    }
                    let found = p.peek(0).describe();
                    p.err_here(format!("traits declare methods only —expected `fn`, found {found}"));
                    p.sync_stmt();
                }
                BodyMode::Impl => {
                    if p.at_kw("fn") {
                        self.stage = TbStage::Method;
                        return Step::Push(Frame::Method(MethodFrame::new(false, false, true)));
                    }
                    let found = p.peek(0).describe();
                    p.err_here(format!("impl blocks contain trait methods —expected `fn`, found {found}"));
                    p.sync_stmt();
                }
            }
        }
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match d {
            Done::Method(m) => {
                self.methods.push(m);
                self.stage = TbStage::Members;
                self.members_top(p)
            }
            Done::Ty(t) => {
                debug_assert!(matches!(self.stage, TbStage::FieldTy));
                self.f_ty = Some(t);
                if p.eat_punct(Tok::Eq) {
                    self.stage = TbStage::FieldInit;
                    Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::Full)))
                } else {
                    self.field_done(p, None)
                }
            }
            Done::Expr(e) => {
                debug_assert!(matches!(self.stage, TbStage::FieldInit));
                self.field_done(p, Some(e))
            }
            Done::Failed => match self.stage {
                // v1: a failed method or field type resyncs and continues
                TbStage::Method | TbStage::FieldTy => {
                    p.sync_stmt();
                    self.stage = TbStage::Members;
                    self.members_top(p)
                }
                // v1: a failed field initializer bails the whole body (`?`)
                TbStage::FieldInit => Step::Pop(Done::Failed),
                _ => unreachable!(),
            },
            _ => unreachable!("type body receives methods, types, or expressions"),
        }
    }

    fn field_done(&mut self, p: &mut Parser, init: Option<NodeHandle<AnyExpr>>) -> Step {
        let node = p.field_decl(
            FieldDeclData {
                is_private: self.f_private,
                is_static: self.f_static,
                name: self.f_name,
                ty: self.f_ty.take().expect("field without a type"),
                init,
            },
            self.f_lo.to(p.span()),
        );
        self.fields.push(node);
        if !(p.eat_punct(Tok::Semi) || p.eat_punct(Tok::Comma)) {
            // last field before `}` — allowed; otherwise complain
            if !matches!(p.tok(), Tok::RBrace) {
                p.err_here("expected `;` or `,` after a field");
                p.sync_stmt();
            }
        }
        self.stage = TbStage::Members;
        self.members_top(p)
    }
}

// ---- methods ----

pub(crate) struct MethodFrame {
    lo: u32,
    is_private: bool,
    is_suspend: bool,
    with_body: bool,
    stage: MeStage,
    name: IdentId,
    generics: Vec<IdentId>,
    params: Option<Vec<NodeHandle<AnyParam>>>,
    ret: Option<NodeHandle<AnyTy>>,
}

#[derive(Clone, Copy)]
enum MeStage {
    Params,
    Ret,
    Body,
}

impl MethodFrame {
    pub(crate) fn new(is_private: bool, is_suspend: bool, with_body: bool) -> Self {
        MethodFrame {
            lo: 0,
            is_private,
            is_suspend,
            with_body,
            stage: MeStage::Params,
            name: IdentId(0),
            generics: Vec::new(),
            params: None,
            ret: None,
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = p.bump().span.lo; // fn
        let Some(name) = p.expect_ident("a method name") else {
            return Step::Pop(Done::Failed);
        };
        self.name = name;
        if matches!(p.tok(), Tok::Lt) {
            self.generics = generic_params(p);
        }
        Step::Push(Frame::Params(ParamsFrame::new()))
    }

    fn after_params(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::Arrow) {
            self.stage = MeStage::Ret;
            return Step::Push(Frame::Type(TypeFrame::new(p)));
        }
        self.finish_sig(p)
    }

    fn finish_sig(&mut self, p: &mut Parser) -> Step {
        if self.with_body && p.mode == Mode::Impl {
            self.stage = MeStage::Body;
            Step::Push(Frame::Block(BlockFrame::lax(p, self.lo)))
        } else {
            p.expect(Tok::Semi);
            self.pop(p, None)
        }
    }

    fn pop(&mut self, p: &mut Parser, body: Option<NodeHandle<BlockNode>>) -> Step {
        let d = MethodDeclData {
            is_private: self.is_private,
            is_suspend: self.is_suspend,
            name: self.name,
            generics: std::mem::take(&mut self.generics),
            params: self.params.take().expect("method without params"),
            ret: self.ret.take(),
            body,
        };
        Step::Pop(Done::Method(p.method_decl(d, Span::new(self.lo, p.span().hi))))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match (self.stage, d) {
            (MeStage::Params, Done::Members(ps)) => {
                self.params = Some(ps);
                self.after_params(p)
            }
            (MeStage::Params, Done::Failed) => Step::Pop(Done::Failed),
            (MeStage::Ret, Done::Ty(t)) => {
                self.ret = Some(t);
                self.finish_sig(p)
            }
            (MeStage::Ret, Done::Failed) => Step::Pop(Done::Failed),
            (MeStage::Body, Done::Block(b)) => self.pop(p, Some(b)),
            (MeStage::Body, Done::Failed) => Step::Pop(Done::Failed),
            _ => unreachable!("method frame receives params, types, or a body"),
        }
    }
}

// ---- free functions ----

pub(crate) struct FnFrame {
    vis: Vis,
    pre_suspend: bool,
    entry: bool,
    lo: u32,
    stage: FnSStage,
    is_suspend: bool,
    name: IdentId,
    generics: Vec<IdentId>,
    params: Option<Vec<NodeHandle<AnyParam>>>,
    ret: Option<NodeHandle<AnyTy>>,
    where_bounds: Vec<(IdentId, NodeHandle<AnyTy>)>,
    where_name: Option<IdentId>,
}

#[derive(Clone, Copy)]
enum FnSStage {
    Params,
    Ret,
    WhereBound,
    Body,
}

impl FnFrame {
    pub(crate) fn new(vis: Vis, pre_suspend: bool, entry: bool) -> Self {
        FnFrame {
            vis,
            pre_suspend,
            entry,
            lo: 0,
            stage: FnSStage::Params,
            is_suspend: false,
            name: IdentId(0),
            generics: Vec::new(),
            params: None,
            ret: None,
            where_bounds: Vec::new(),
            where_name: None,
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = p.bump().span.lo; // fn
        self.is_suspend = self.pre_suspend || {
            let s = p.at_kw("suspend");
            if s {
                p.bump();
            }
            s
        };
        if p.mode == Mode::Decl {
            p.err(Span::new(self.lo, self.lo + 2), "implementation in a declaration file (RFC 0029 §2)");
        }
        let Some(name) = p.expect_ident("a function name") else {
            return Step::Pop(Done::Failed);
        };
        self.name = name;
        if matches!(p.tok(), Tok::Lt) {
            self.generics = generic_params(p);
        }
        Step::Push(Frame::Params(ParamsFrame::new()))
    }

    fn after_params(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::Arrow) {
            self.stage = FnSStage::Ret;
            return Step::Push(Frame::Type(TypeFrame::new(p)));
        }
        self.where_entry(p)
    }

    /// the where-clause, if any (RFC 0013 §2)
    fn where_entry(&mut self, p: &mut Parser) -> Step {
        if p.at_kw("where") {
            p.bump();
            return self.where_top(p);
        }
        self.body(p)
    }

    fn where_top(&mut self, p: &mut Parser) -> Step {
        let Some(t) = p.expect_ident("a generic parameter name") else {
            return self.body(p);
        };
        if !p.at_kw("requires") {
            p.err_here("expected `requires` in a where clause (RFC 0013 §2)");
            return self.body(p);
        }
        p.bump();
        self.where_name = Some(t);
        self.stage = FnSStage::WhereBound;
        Step::Push(Frame::Type(TypeFrame::new(p)))
    }

    fn body(&mut self, p: &mut Parser) -> Step {
        self.stage = FnSStage::Body;
        Step::Push(Frame::Block(BlockFrame::lax(p, self.lo)))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match (self.stage, d) {
            (FnSStage::Params, Done::Members(ps)) => {
                self.params = Some(ps);
                self.after_params(p)
            }
            (FnSStage::Params, Done::Failed) => Step::Pop(Done::Failed),
            (FnSStage::Ret, Done::Ty(t)) => {
                self.ret = Some(t);
                self.where_entry(p)
            }
            (FnSStage::Ret, Done::Failed) => Step::Pop(Done::Failed),
            (FnSStage::WhereBound, Done::Ty(tr)) => {
                let t = self.where_name.take().expect("where bound without a name");
                self.where_bounds.push((t, tr));
                if p.eat_punct(Tok::Comma) {
                    self.where_top(p)
                } else {
                    self.body(p)
                }
            }
            // v1: a failed bound breaks to the body
            (FnSStage::WhereBound, Done::Failed) => self.body(p),
            (FnSStage::Body, Done::Block(body)) => {
                let f = p.fn_decl(
                    FnData {
                        vis: self.vis,
                        is_suspend: self.is_suspend,
                        entry: self.entry,
                        name: self.name,
                        generics: std::mem::take(&mut self.generics),
                        params: self.params.take().expect("fn without params"),
                        ret: self.ret.take(),
                        where_bounds: std::mem::take(&mut self.where_bounds),
                        body,
                    },
                    Span::new(self.lo, p.span().hi),
                );
                Step::Pop(Done::Item(f.into()))
            }
            (FnSStage::Body, Done::Failed) => Step::Pop(Done::Failed),
            _ => unreachable!("fn frame receives params, types, or a body"),
        }
    }
}

// ---- host/extern surface declarations (.d.rut, RFC 0030 §3) ----

pub(crate) struct SurfaceFrame {
    linkage: Linkage,
    lo: u32,
    stage: SuStage,
    is_class: bool,
    is_primitive: bool,
    name: IdentId,
    generics: Vec<IdentId>,
    extparams: Vec<(IdentId, Option<NodeHandle<AnyTy>>)>,
    methods: Vec<NodeHandle<MethodDeclNode>>,
    params: Option<Vec<NodeHandle<AnyParam>>>,
    ret: Option<NodeHandle<AnyTy>>,
    ext_name: Option<IdentId>,
}

#[derive(Clone, Copy)]
enum SuStage {
    Params,
    Ret,
    ExtBound,
    Members,
}

impl SurfaceFrame {
    pub(crate) fn new() -> Self {
        SurfaceFrame {
            linkage: Linkage::Extern,
            lo: 0,
            stage: SuStage::Params,
            is_class: false,
            is_primitive: false,
            name: IdentId(0),
            generics: Vec::new(),
            extparams: Vec::new(),
            methods: Vec::new(),
            params: None,
            ret: None,
            ext_name: None,
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.linkage = match p.tok().clone() {
            Tok::Ident(k) if k == "host" => Linkage::Host,
            _ => Linkage::Extern,
        };
        let sp = p.span();
        if p.mode == Mode::Impl {
            p.err(
                sp,
                "declaration keyword in an implementation file —`host`/`extern` belong in a `.d.rut` (RFC 0029 §2)",
            );
        }
        self.lo = p.bump().span.lo;
        match p.tok().clone() {
            Tok::Ident(k) if k == "fn" => {
                p.bump();
                self.is_class = false;
                let Some(name) = p.expect_ident("a function name") else {
                    return Step::Pop(Done::Failed);
                };
                self.name = name;
                if matches!(p.tok(), Tok::Lt) {
                    self.generics = generic_params(p);
                }
                self.stage = SuStage::Params;
                Step::Push(Frame::Params(ParamsFrame::new()))
            }
            Tok::Ident(k) if k == "class" => {
                p.bump();
                self.is_class = true;
                let Some(name) = p.expect_ident("a class name") else {
                    return Step::Pop(Done::Failed);
                };
                self.name = name;
                // extparams: `K: Hashable` (corpus) or `K requires Hashable`
                // (RFC 0030 §3) — bounds are the `requires` form's meaning only
                if matches!(p.tok(), Tok::Lt) {
                    p.bump();
                    self.stage = SuStage::ExtBound;
                    return self.extparams_run(p);
                }
                self.stage = SuStage::Members;
                p.expect(Tok::LBrace);
                self.members_top(p)
            }
            Tok::Ident(k) if k == "primitive" => {
                // `host primitive string { ... }` — the native member
                // surface of a primitive (RFC 0029 §2); `primitive` is
                // contextual: only meaningful after the linkage keyword
                p.bump();
                self.is_class = true;
                self.is_primitive = true;
                let Some(name) = p.expect_ident("a primitive name") else {
                    return Step::Pop(Done::Failed);
                };
                if !is_primitive_ty(p.interner.name(name)) {
                    p.err_here(
                        "`host primitive` names a primitive type (`string`, `i32`, …) — for a host class use `host class`",
                    );
                }
                self.name = name;
                self.stage = SuStage::Members;
                p.expect(Tok::LBrace);
                self.members_top(p)
            }
            _ => {
                let found = p.peek(0).describe();
                p.err_here(format!(
                    "expected `fn`, `class`, or `primitive` after `host`/`extern`, found {found}"
                ));
                Step::Pop(Done::Failed)
            }
        }
    }

    /// the extparam list — `>` (or a failure) finishes; a bound suspends
    fn extparams_run(&mut self, p: &mut Parser) -> Step {
        loop {
            if p.eat_punct(Tok::Gt) {
                break;
            }
            let Some(name) = p.expect_ident("a generic parameter") else {
                break;
            };
            if p.eat_punct(Tok::Colon) || (p.at_kw("requires") && {
                p.bump();
                true
            }) {
                self.ext_name = Some(name);
                return Step::Push(Frame::Type(TypeFrame::new(p)));
            }
            self.extparams.push((name, None));
            if !p.eat_punct(Tok::Comma) {
                p.expect_gt();
                break;
            }
        }
        self.stage = SuStage::Members;
        p.expect(Tok::LBrace);
        self.members_top(p)
    }

    fn extparams_after_bound(&mut self, p: &mut Parser, bound: Option<NodeHandle<AnyTy>>) -> Step {
        let name = self.ext_name.take().expect("ext bound without a name");
        self.extparams.push((name, bound));
        if p.eat_punct(Tok::Comma) {
            self.extparams_run(p)
        } else {
            p.expect_gt();
            self.stage = SuStage::Members;
            p.expect(Tok::LBrace);
            self.members_top(p)
        }
    }

    fn members_top(&mut self, p: &mut Parser) -> Step {
        loop {
            if p.eat_punct(Tok::RBrace) || p.at_eof() {
                let node = if self.is_primitive {
                    p.item(
                        ItemKind::SurfacePrimitive {
                            vis: Vis::Self_,
                            linkage: self.linkage,
                            name: self.name,
                            members: std::mem::take(&mut self.methods),
                        },
                        Span::new(self.lo, p.span().hi),
                    )
                } else {
                    p.item(
                        ItemKind::SurfaceClass {
                            vis: Vis::Self_,
                            linkage: self.linkage,
                            name: self.name,
                            extparams: std::mem::take(&mut self.extparams),
                            members: std::mem::take(&mut self.methods),
                        },
                        Span::new(self.lo, p.span().hi),
                    )
                };
                return Step::Pop(Done::Item(node));
            }
            let is_suspend = if p.at_kw("suspend") {
                p.bump();
                true
            } else {
                false
            };
            if p.at_kw("fn") {
                self.stage = SuStage::Members;
                return Step::Push(Frame::Method(MethodFrame::new(false, is_suspend, false)));
            }
            let found = p.peek(0).describe();
            p.err_here(format!("expected a method declaration, found {found}"));
            p.sync_stmt();
        }
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match (self.stage, d) {
            (SuStage::Params, Done::Members(ps)) => {
                self.params = Some(ps);
                if p.eat_punct(Tok::Arrow) {
                    self.stage = SuStage::Ret;
                    return Step::Push(Frame::Type(TypeFrame::new(p)));
                }
                p.expect(Tok::Semi);
                self.pop_fn(p)
            }
            (SuStage::Params, Done::Failed) => Step::Pop(Done::Failed),
            (SuStage::Ret, Done::Ty(t)) => {
                self.ret = Some(t);
                p.expect(Tok::Semi);
                self.pop_fn(p)
            }
            (SuStage::Ret, Done::Failed) => Step::Pop(Done::Failed),
            (SuStage::ExtBound, Done::Ty(t)) => self.extparams_after_bound(p, Some(t)),
            // v1: a failed bound is simply an unbounded parameter
            (SuStage::ExtBound, Done::Failed) => self.extparams_after_bound(p, None),
            (SuStage::Members, Done::Method(m)) => {
                self.methods.push(m);
                self.members_top(p)
            }
            (SuStage::Members, Done::Failed) => {
                p.sync_stmt();
                self.members_top(p)
            }
            (_, d) => unreachable!("surface frame received the wrong child: {d:?}"),
        }
    }

    fn pop_fn(&mut self, p: &mut Parser) -> Step {
        let node = p.item(
            ItemKind::SurfaceFn {
                vis: Vis::Self_,
                linkage: self.linkage,
                name: self.name,
                generics: std::mem::take(&mut self.generics),
                params: self.params.take().expect("surface fn without params"),
                ret: self.ret.take(),
            },
            Span::new(self.lo, p.span().hi),
        );
        Step::Pop(Done::Item(node))
    }
}

// ---- parameter list ----

pub(crate) struct ParamsFrame {
    out: Vec<NodeHandle<AnyParam>>,
    stage: PsStage,
    /// the parameter under construction
    p_lo: Span,
    p_mut: bool,
    p_name: IdentId,
}

#[derive(Clone, Copy)]
enum PsStage {
    List,
    Ty,
}

impl ParamsFrame {
    pub(crate) fn new() -> Self {
        ParamsFrame {
            out: Vec::new(),
            stage: PsStage::List,
            p_lo: Span::new(0, 0),
            p_mut: false,
            p_name: IdentId(0),
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        if p.expect(Tok::LParen).is_none() {
            return Step::Pop(Done::Failed);
        }
        self.list_top(p)
    }

    fn list_top(&mut self, p: &mut Parser) -> Step {
        loop {
            if p.eat_punct(Tok::RParen) {
                return Step::Pop(Done::Members(std::mem::take(&mut self.out)));
            }
            let lo = p.span();
            let mut is_mut = false;
            if p.at_kw("mut") && p.at_kw2("self") {
                is_mut = true;
                p.bump();
            }
            if p.at_kw("self") {
                p.bump();
                self.out.push(p.member(MemberKind::SelfParam(SelfParamData { is_mut }), lo));
                if !p.eat_punct(Tok::Comma) {
                    p.expect(Tok::RParen);
                    return Step::Pop(Done::Members(std::mem::take(&mut self.out)));
                }
                continue;
            }
            if p.at_kw("mut") {
                is_mut = true;
                p.bump();
            }
            let Some(name) = p.expect_ident("a parameter name") else {
                p.sync_stmt();
                return Step::Pop(Done::Members(std::mem::take(&mut self.out)));
            };
            self.p_lo = lo;
            self.p_mut = is_mut;
            self.p_name = name;
            if p.eat_punct(Tok::Colon) {
                self.stage = PsStage::Ty;
                return Step::Push(Frame::Type(TypeFrame::new(p)));
            }
            self.out.push(p.member(
                MemberKind::Param(ParamData { is_mut, name, ty: None }),
                lo.to(p.span()),
            ));
            if !p.eat_punct(Tok::Comma) {
                p.expect(Tok::RParen);
                return Step::Pop(Done::Members(std::mem::take(&mut self.out)));
            }
        }
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match d {
            Done::Ty(t) => {
                debug_assert!(matches!(self.stage, PsStage::Ty));
                self.stage = PsStage::List;
                let (lo, is_mut, name) = (self.p_lo, self.p_mut, self.p_name);
                self.out.push(p.member(
                    MemberKind::Param(ParamData { is_mut, name, ty: Some(t) }),
                    lo.to(p.span()),
                ));
                if !p.eat_punct(Tok::Comma) {
                    p.expect(Tok::RParen);
                    return Step::Pop(Done::Members(std::mem::take(&mut self.out)));
                }
                self.list_top(p)
            }
            // v1: `Some(self.parse_type()?)` bails the whole list
            Done::Failed => Step::Pop(Done::Failed),
            _ => unreachable!("params frame receives types"),
        }
    }
}
