//! Declarations (RFC 0030 §2): items, pub visibility, use statements, type
//! bodies,
//! fn/method signatures, surface stubs — as frames. `classify_item` is
//! v1's `parse_item` dispatch (peek 1, keyword-led); the frames carry
//! the state each rule accumulated across child pushes.

use rut_ast::ast::*;
use rut_lexer::span::Span;
use rut_lexer::token::Tok;

use crate::expr::{ExprFrame, ExprMode};
use crate::frame::{pub_scope, Done, PubFrame, Frame, Step};
use crate::stmt::BlockFrame;
use crate::ty::TypeFrame;
use crate::{Mode, Parser};

// ---- item dispatch (v1's parse_item) ----

pub(crate) fn classify_item(p: &mut Parser) -> Option<Frame> {
    let sp = p.span();
    match p.tok().clone() {
        Tok::Ident(kw) => match kw.as_str() {
            "use" => Some(Frame::Use(UseFrame::new())),
            "pub" => Some(Frame::Pub(PubFrame::new())),
            "type" => Some(Frame::Alias(TypeAliasFrame::new(Vis::Self_))),
            "let" => Some(Frame::ModuleLet(ModuleLetFrame::new(Vis::Self_))),
            "enum" => Some(Frame::Enum(EnumFrame::new(Vis::Self_))),
            "struct" => Some(Frame::Dataclass(TyDeclFrame::new(false, Vis::Self_))),
            "class" => Some(Frame::Class(TyDeclFrame::new(true, Vis::Self_))),
            "trait" => Some(Frame::Trait(TraitFrame::new(Vis::Self_))),
            "impl" => Some(Frame::Impl(ImplFrame::new())),
            // `async fn` — the async declaration (RFC 0018 §2)
            "async" if p.at_kw2("fn") => {
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
            // `host` — the one native linkage left: the embedding Rust
            // (RFC 0025). `extern` is removed — the loader + `entry fn`
            // cover it.
            "host" => Some(Frame::Surface(SurfaceFrame::new(Linkage::Host))),
            // `builtin` — contextual (a legal identifier everywhere else):
            // the ENGINE surface, core only, compiler-lowered
            "builtin" => Some(Frame::Surface(SurfaceFrame::new(Linkage::Builtin))),
            "extern" => {
                p.err(
                    sp,
                    "`extern` linkage is removed —`host` is the only native surface (embedding Rust); rut packages arrive through the module loader (RFC 0029 §2)",
                );
                None
            }
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
/// the pub frame itself)
pub(crate) fn classify_pub(p: &mut Parser, vis: Vis) -> Option<Frame> {
    match p.tok().clone() {
        Tok::Ident(kw) => match kw.as_str() {
            "let" => Some(Frame::ModuleLet(ModuleLetFrame::new(vis))),
            "type" => Some(Frame::Alias(TypeAliasFrame::new(vis))),
            "enum" => Some(Frame::Enum(EnumFrame::new(vis))),
            "struct" => Some(Frame::Dataclass(TyDeclFrame::new(false, vis))),
            "class" => Some(Frame::Class(TyDeclFrame::new(true, vis))),
            "trait" => Some(Frame::Trait(TraitFrame::new(vis))),
            // `async fn` — the async declaration (RFC 0018 §2)
            "async" if p.at_kw2("fn") => {
                p.bump();
                Some(Frame::Fn(FnFrame::new(vis, true, false)))
            }
            "fn" => Some(Frame::Fn(FnFrame::new(vis, false, false))),
            "host" => Some(Frame::Surface(SurfaceFrame::new(Linkage::Host))),
            // `builtin` dropped `pub` (builtin-surface phase 1): builtin
            // names are AMBIENT — the engine's surface carries no
            // visibility, so the old `pub builtin` spelling diagnoses
            "builtin" => {
                p.err_here(
                    "`pub builtin` is removed —builtin names are ambient (no `use`, no visibility): write `builtin fn` / `builtin primitive` / `builtin impl` / `builtin trait` without `pub`",
                );
                None
            }
            "extern" => {
                p.err_here(
                    "`extern` linkage is removed —`host` is the only native surface (embedding Rust); rut packages arrive through the module loader (RFC 0029 §2)",
                );
                None
            }
            _ => {
                p.err_here(format!("`pub` must precede a declaration, found `{kw}`"));
                None
            }
        },
        _ => {
            let found = p.peek(0).describe();
            p.err_here(format!("`pub` must precede a declaration, found {found}"));
            None
        }
    }
}

// ---- use ----

/// `use <pkg>::{A, B};` / `use <pkg>::A;` (RFC 0029 §2): the package is
/// one bare identifier, the names are one or more idents. There is no
/// string specifier and no `from` clause — resolution is the driver's
/// exact-match against mounted module names.
pub(crate) struct UseFrame {
    lo: u32,
    pkg: IdentId,
    names: Vec<IdentId>,
}

impl UseFrame {
    pub(crate) fn new() -> Self {
        UseFrame { lo: 0, pkg: IdentId(0), names: Vec::new() }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = p.bump().span.lo; // use
        let Some(pkg) = p.expect_ident("a package name") else {
            p.sync_stmt();
            return Step::Pop(Done::Failed);
        };
        self.pkg = pkg;
        if !p.eat_punct(Tok::Colon) {
            p.err_here("expected `::` after the package name — use paths spell `use <pkg>::{A, B};`");
            p.sync_stmt();
            return Step::Pop(Done::Failed);
        }
        p.expect(Tok::Colon);
        if p.eat_punct(Tok::LBrace) {
            loop {
                if p.eat_punct(Tok::RBrace) {
                    break;
                }
                let Some(n) = p.expect_ident("a used name") else {
                    p.sync_stmt();
                    return Step::Pop(Done::Failed);
                };
                self.names.push(n);
                if !p.eat_punct(Tok::Comma) {
                    p.expect(Tok::RBrace);
                    break;
                }
            }
        } else {
            // the single-name form: `use <pkg>::A;`
            let Some(n) = p.expect_ident("a used name") else {
                p.sync_stmt();
                return Step::Pop(Done::Failed);
            };
            self.names.push(n);
        }
        p.expect(Tok::Semi);
        let names = std::mem::take(&mut self.names);
        Step::Pop(Done::Item(p.item(
            ItemKind::Use { pkg: self.pkg, names },
            Span::new(self.lo, p.span().hi),
        )))
    }

    pub(crate) fn absorb(&mut self, _p: &mut Parser, _d: Done) -> Step {
        unreachable!("use frame pushes no children")
    }
}

// ---- type alias ----

/// `pub(..)? type Name = Target;` (RFC 0043): a transparent alias, or a
/// `Target` spelled `A | B` for the bound-only union. `vis` rides the
/// item like `TyDeclFrame`'s; the `use`-both rule stays the binding gate.
pub(crate) struct TypeAliasFrame {
    vis: Vis,
    lo: u32,
    name: IdentId,
}

impl TypeAliasFrame {
    pub(crate) fn new(vis: Vis) -> Self {
        TypeAliasFrame { vis, lo: 0, name: IdentId(0) }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = p.bump().span.lo; // type
        let Some(name) = p.expect_ident("a type alias name") else {
            return Step::Pop(Done::Failed);
        };
        self.name = name;
        p.expect(Tok::Eq);
        Step::Push(Frame::Type(TypeFrame::new_bound(p)))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match d {
            Done::Ty(target) => {
                p.expect(Tok::Semi);
                let node = p.item(
                    ItemKind::Alias(AliasData { vis: self.vis, name: self.name, target }),
                    Span::new(self.lo, p.span().hi),
                );
                Step::Pop(Done::Item(node))
            }
            Done::Failed => Step::Pop(Done::Failed),
            _ => unreachable!("type alias frame receives a type"),
        }
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

// ---- generic parameter list ----
//
// RFC 0043: fn/method generic parameters may carry inline admission
// bounds (`<T requires A | B>`), and so do CLASS generics (§A5 —
// `class HashMap<K requires Hashable, V>`); struct/trait/surface
// generics reject them. A bound suspends the enclosing frame at a
// GenBound stage — the bound itself is a child Type frame — and
// `generic_params_more` hands back the generic whose bound is being
// parsed (`Some`).

fn generic_params(p: &mut Parser, allow_bounds: bool, owner: &str) -> (Vec<IdentId>, Option<IdentId>) {
    p.bump(); // < (the caller checked)
    let mut out = Vec::new();
    let pending = generic_params_more(p, allow_bounds, owner, &mut out);
    (out, pending)
}

/// Continue the list at a parameter name — after `<`, a comma, or an
/// absorbed bound. `Some(g)` when `g requires` suspends the list.
fn generic_params_more(p: &mut Parser, allow_bounds: bool, owner: &str, out: &mut Vec<IdentId>) -> Option<IdentId> {
    p.eat_punct(Tok::Comma); // after an absorbed bound the separator is still ahead
    loop {
        if p.eat_punct(Tok::Gt) {
            return None;
        }
        let Some(n) = p.expect_ident("a generic parameter") else {
            return None;
        };
        out.push(n);
        if p.at_kw("requires") {
            p.bump();
            if allow_bounds {
                return Some(n);
            }
            p.err_here(format!(
                "`{owner}` generic parameters take no `requires` bounds — inline bounds bind fn/method/class generics only (RFC 0043)"
            ));
            // skip the rejected bound so the list continues cleanly
            while !matches!(
                p.tok(),
                Tok::Comma | Tok::Gt | Tok::Shr | Tok::LParen | Tok::LBrace | Tok::Eof
            ) {
                p.bump();
            }
        }
        if !p.eat_punct(Tok::Comma) {
            p.expect_gt();
            return None;
        }
    }
}

// ---- struct / class ----
//
// RFC 0043 §A5: class generics take inline admission bounds
// (`class HashMap<K requires Hashable, V>`); struct generics reject
// them. A bound suspends the frame at a GenBound stage — the bound
// itself is a child Type frame — exactly like the fn/method frames;
// the body frame only pushes once the `<..>` list is fully consumed.

pub(crate) struct TyDeclFrame {
    is_class: bool,
    vis: Vis,
    lo: u32,
    stage: TdStage,
    name: IdentId,
    generics: Vec<IdentId>,
    /// the class's admission bounds, in `requires` order (empty for
    /// structs — their generics never carry bounds)
    requires: Vec<(IdentId, NodeHandle<AnyTy>)>,
    /// the generic whose `requires` bound is being parsed (GenBound stage)
    pending: Option<IdentId>,
}

#[derive(Clone, Copy)]
enum TdStage {
    GenBound,
    Body,
}

impl TyDeclFrame {
    pub(crate) fn new(is_class: bool, vis: Vis) -> Self {
        TyDeclFrame {
            is_class,
            vis,
            lo: 0,
            stage: TdStage::Body,
            name: IdentId(0),
            generics: Vec::new(),
            requires: Vec::new(),
            pending: None,
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = p.bump().span.lo; // struct | class
        let what = if self.is_class { "a class name" } else { "a struct name" };
        let Some(name) = p.expect_ident(what) else {
            return Step::Pop(Done::Failed);
        };
        self.name = name;
        if matches!(p.tok(), Tok::Lt) {
            let owner = if self.is_class { "class" } else { "struct" };
            let (gens, pending) = generic_params(p, self.is_class, owner);
            self.generics = gens;
            if let Some(g) = pending {
                self.pending = Some(g);
                self.stage = TdStage::GenBound;
                return Step::Push(Frame::Type(TypeFrame::new_bound(p)));
            }
        }
        self.push_body()
    }

    fn push_body(&mut self) -> Step {
        self.stage = TdStage::Body;
        Step::Push(Frame::TypeBody(TypeBodyFrame::new(BodyMode::Class {
            allow_pub: self.is_class,
            is_dataclass: !self.is_class,
        })))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match (self.stage, d) {
            (TdStage::GenBound, Done::Ty(t)) => {
                let g = self.pending.take().expect("bound without a parameter");
                self.requires.push((g, t));
                let owner = if self.is_class { "class" } else { "struct" };
                let pending = generic_params_more(p, self.is_class, owner, &mut self.generics);
                if let Some(g) = pending {
                    self.pending = Some(g);
                    return Step::Push(Frame::Type(TypeFrame::new_bound(p)));
                }
                self.push_body()
            }
            (TdStage::GenBound, Done::Failed) => Step::Pop(Done::Failed),
            (TdStage::Body, Done::Body(fields, methods)) => {
                let (vis, name, generics) = (self.vis, self.name, std::mem::take(&mut self.generics));
                let kind = if self.is_class {
                    ItemKind::Class {
                        vis,
                        name,
                        generics,
                        requires: std::mem::take(&mut self.requires),
                        fields,
                        methods,
                    }
                } else {
                    ItemKind::Dataclass { vis, name, generics, fields, methods }
                };
                Step::Pop(Done::Item(p.item(kind, Span::new(self.lo, p.span().hi))))
            }
            (TdStage::Body, Done::Failed) => Step::Pop(Done::Failed),
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
            let (gens, pending) = generic_params(p, false, "trait");
            self.generics = gens;
            debug_assert!(pending.is_none(), "rejected bounds never suspend");
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
//
// The two impl forms (RFC 0012): `impl T { .. }` — inherent, the type's
// module only — and `impl I for T { .. }` — a trait impl, any module.
// The first type IS the target unless `for` follows it; bodies are
// braced, methods only.

pub(crate) struct ImplFrame {
    lo: u32,
    stage: ImStage,
    trait_ref: Option<NodeHandle<AnyTy>>,
    target: Option<NodeHandle<AnyTy>>,
}

#[derive(Clone, Copy)]
enum ImStage {
    /// the first type — target or trait ref, decided by `for`
    Head,
    /// the trait impl's target (after `impl Trait for`)
    Target,
    /// the braced method body
    Methods,
}

impl ImplFrame {
    pub(crate) fn new() -> Self {
        ImplFrame { lo: 0, stage: ImStage::Head, trait_ref: None, target: None }
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
            (ImStage::Head, Done::Ty(t)) => {
                if p.at_kw("for") {
                    p.bump();
                    self.trait_ref = Some(t);
                    self.stage = ImStage::Target;
                    Step::Push(Frame::Type(TypeFrame::new(p)))
                } else {
                    // inherent impl: `impl Type { .. }`
                    self.target = Some(t);
                    self.stage = ImStage::Methods;
                    Step::Push(Frame::TypeBody(TypeBodyFrame::new(BodyMode::Inherent)))
                }
            }
            (ImStage::Target, Done::Ty(t)) => {
                self.target = Some(t);
                self.stage = ImStage::Methods;
                Step::Push(Frame::TypeBody(TypeBodyFrame::new(BodyMode::Impl)))
            }
            (ImStage::Methods, Done::Body(_, methods)) => {
                let node = p.item(
                    ItemKind::Impl {
                        trait_ref: self.trait_ref.take(),
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

// ---- struct/class/trait/impl bodies ----

pub(crate) enum BodyMode {
    /// struct/class body: FIELDS ONLY — methods live in `impl` blocks
    /// (RFC 0012)
    Class { allow_pub: bool, is_dataclass: bool },
    /// `host struct` body: fields only, no initializers — the host
    /// constructs the record (RFC 0025)
    HostDataclass,
    /// trait body: bodiless method signatures (`async` and no-`self`
    /// signatures legal — no bodies, RFC 0012)
    Trait,
    /// `impl I for T` body: method implementations — `async` where the
    /// trait says so; no `pub` (visibility rides the trait); no fields
    Impl,
    /// `impl T` body: inherent methods — `pub` and `async` legal
    /// (checker restricts `pub` to classes, RFC 0009); no fields
    Inherent,
}

pub(crate) struct TypeBodyFrame {
    mode: BodyMode,
    fields: Vec<NodeHandle<FieldDeclNode>>,
    methods: Vec<NodeHandle<MethodDeclNode>>,
    stage: TbStage,
    /// the field under construction
    f_lo: Span,
    f_vis: Option<Vis>,
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
            f_vis: None,
            f_static: false,
            f_name: IdentId(0),
            f_ty: None,
        }
    }

    /// the leading clause for a malformed trait/impl member diagnostic
    fn mode_noun(&self) -> &'static str {
        match self.mode {
            BodyMode::Trait => "traits declare method signatures",
            BodyMode::Impl => "impl blocks contain trait methods",
            BodyMode::Inherent => "inherent impl blocks declare methods",
            BodyMode::HostDataclass => "host dataclasses declare fields",
            BodyMode::Class { .. } => "type bodies declare fields —methods live in `impl` blocks",
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        // v1 brace strictness: struct/class and trait bodies bail on a
        // missing `{`; impl and surface bodies had already expected it
        let strict = matches!(self.mode, BodyMode::Class { .. } | BodyMode::HostDataclass | BodyMode::Trait);
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
                BodyMode::HostDataclass => {
                    let lo = p.span();
                    while let Tok::Ident(m) = p.tok().clone() {
                        match m.as_str() {
                            "pub" => {
                                p.bump();
                                let _ = pub_scope(p);
                                p.err(lo, "host struct fields carry no visibility —all fields are public (RFC 0025)");
                            }
                            "static" | "async" => {
                                p.bump();
                                p.err(lo, format!("host struct fields carry no modifiers —`{m}` is not declarable here (RFC 0025)"));
                            }
                            _ => break,
                        }
                    }
                    match p.tok().clone() {
                        // parse (bodiless) to stay in sync; the methods are
                        // dropped — SurfaceDataclass keeps fields only
                        Tok::Ident(kw) if kw == "fn" => {
                            p.err(lo, "host dataclasses declare fields only —methods live in rut wrapper classes (RFC 0025)");
                            self.stage = TbStage::Method;
                            return Step::Push(Frame::Method(MethodFrame::new(None, false, false, false)));
                        }
                        Tok::Ident(_) => {
                            let name = p.expect_ident("a field name").unwrap_or(IdentId(0));
                            p.expect(Tok::Colon);
                            self.stage = TbStage::FieldTy;
                            self.f_lo = lo;
                            self.f_vis = None;
                            self.f_static = false;
                            self.f_name = name;
                            return Step::Push(Frame::Type(TypeFrame::new(p)));
                        }
                        _ => {
                            let found = p.peek(0).describe();
                            p.err_here(format!("expected a field, found {found}"));
                            p.sync_stmt();
                        }
                    }
                }
                BodyMode::Class { allow_pub, is_dataclass } => {
                    let lo = p.span();
                    // RFC 0003 §2 — members default to module-private;
                    // `pub` (+ scopes) exposes them. FIELDS ONLY since the
                    // impl blocks returned (RFC 0012): a `fn` member
                    // diagnoses and is parsed (bodiless) to stay in sync.
                    let mut vis: Option<Vis> = None;
                    let mut is_static = false;
                    while let Tok::Ident(m) = p.tok().clone() {
                        match m.as_str() {
                            "pub" => {
                                p.bump();
                                vis = Some(pub_scope(p));
                                if !allow_pub {
                                    p.err(
                                        lo,
                                        "dataclasses have no member visibility —all fields are public (RFC 0009)",
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
                            "async" => {
                                p.bump();
                                p.err(lo, "unexpected `async` —type bodies declare fields only (RFC 0012)");
                            }
                            _ => break,
                        }
                    }
                    match p.tok().clone() {
                        Tok::Ident(kw) if kw == "fn" => {
                            p.err(
                                lo,
                                "methods live in `impl` blocks —type bodies declare fields only (RFC 0012)",
                            );
                            if is_static {
                                p.err(
                                    lo,
                                    "there is no `static fn` —a method without `self` IS a class method (RFC 0010 §2)",
                                );
                            }
                            self.stage = TbStage::Method;
                            return Step::Push(Frame::Method(MethodFrame::new(vis, false, true, true)));
                        }
                        Tok::Ident(_) => {
                            let name = p.expect_ident("a field name").unwrap_or(IdentId(0));
                            p.expect(Tok::Colon);
                            self.stage = TbStage::FieldTy;
                            self.f_lo = lo;
                            self.f_vis = vis;
                            self.f_static = is_static;
                            self.f_name = name;
                            return Step::Push(Frame::Type(TypeFrame::new(p)));
                        }
                        _ => {
                            let found = p.peek(0).describe();
                            p.err_here(format!("expected a field, found {found}"));
                            p.sync_stmt();
                            // sync consumes at least one token (or stops
                            // at `}` / EOF, both caught at the loop top)
                        }
                    }
                }
                BodyMode::Trait | BodyMode::Impl => {
                    // modifier loop: `pub` rejected here (a trait impl
                    // rides the trait's visibility); `async` accepted on
                    // the method (RFC 0018 §2)
                    let lo = p.span();
                    let mut is_async = false;
                    while let Tok::Ident(m) = p.tok().clone() {
                        match m.as_str() {
                            "pub" => {
                                p.bump();
                                let _ = pub_scope(p);
                                let why = match self.mode {
                                    BodyMode::Impl => "trait impl methods carry no `pub` —they are as visible as the trait (RFC 0012)",
                                    _ => "trait methods carry no `pub` —the trait's visibility rules (RFC 0003 §2)",
                                };
                                p.err(lo, why);
                            }
                            "async" => {
                                is_async = true;
                                p.bump();
                            }
                            _ => break,
                        }
                    }
                    if p.at_kw("fn") {
                        self.stage = TbStage::Method;
                        let with_body = matches!(self.mode, BodyMode::Impl);
                        return Step::Push(Frame::Method(MethodFrame::new(None, is_async, with_body, true)));
                    }
                    if is_async {
                        p.err(lo, "expected `fn` after `async`");
                    }
                    let (what, found) = (self.mode_noun(), p.peek(0).describe());
                    p.err_here(format!("{what} —expected `fn`, found {found}"));
                    p.sync_stmt();
                }
                BodyMode::Inherent => {
                    // `impl T { .. }` — inherent methods: `pub`/`async`
                    // legal (the checker restricts `pub` to classes,
                    // RFC 0009); no fields
                    let lo = p.span();
                    let mut vis: Option<Vis> = None;
                    let mut is_async = false;
                    while let Tok::Ident(m) = p.tok().clone() {
                        match m.as_str() {
                            "pub" => {
                                p.bump();
                                vis = Some(pub_scope(p));
                            }
                            "async" => {
                                is_async = true;
                                p.bump();
                            }
                            _ => break,
                        }
                    }
                    if p.at_kw("fn") {
                        self.stage = TbStage::Method;
                        return Step::Push(Frame::Method(MethodFrame::new(vis, is_async, true, true)));
                    }
                    if is_async {
                        p.err(lo, "expected `fn` after `async`");
                    }
                    let (what, found) = (self.mode_noun(), p.peek(0).describe());
                    p.err_here(format!("{what} —expected `fn`, found {found}"));
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
                    if matches!(self.mode, BodyMode::HostDataclass) {
                        p.err(
                            p.span(),
                            "host struct fields have no initializers —the host constructs the record (RFC 0025)",
                        );
                    }
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
                vis: self.f_vis,
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
    vis: Option<Vis>,
    is_async: bool,
    with_body: bool,
    /// RFC 0043: rut methods take inline bounds; surface (.d.rut) members
    /// do not
    allow_bounds: bool,
    stage: MeStage,
    name: IdentId,
    generics: Vec<IdentId>,
    /// the generic whose `requires` bound is being parsed (GenBound stage)
    pending: Option<IdentId>,
    bounds: Vec<(IdentId, NodeHandle<AnyTy>)>,
    params: Option<Vec<NodeHandle<AnyParam>>>,
    ret: Option<NodeHandle<AnyTy>>,
}

#[derive(Clone, Copy)]
enum MeStage {
    GenBound,
    Params,
    Ret,
    Body,
}

impl MethodFrame {
    pub(crate) fn new(vis: Option<Vis>, is_async: bool, with_body: bool, allow_bounds: bool) -> Self {
        MethodFrame {
            lo: 0,
            vis,
            is_async,
            with_body,
            allow_bounds,
            stage: MeStage::Params,
            name: IdentId(0),
            generics: Vec::new(),
            pending: None,
            bounds: Vec::new(),
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
            let (gens, pending) = generic_params(p, self.allow_bounds, "method");
            self.generics = gens;
            if let Some(g) = pending {
                self.pending = Some(g);
                self.stage = MeStage::GenBound;
                return Step::Push(Frame::Type(TypeFrame::new_bound(p)));
            }
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
        self.stray_where(p);
        if self.with_body && p.mode == Mode::Impl {
            self.stage = MeStage::Body;
            Step::Push(Frame::Block(BlockFrame::lax(p, self.lo)))
        } else {
            p.expect(Tok::Semi);
            self.pop(p, None)
        }
    }

    /// A `where` clause is gone (RFC 0043): diagnose with the inline
    /// replacement and skip past the stray clause so the signature's
    /// tail (`;` or the body) still parses.
    fn stray_where(&mut self, p: &mut Parser) {
        if p.at_kw("where") {
            p.err_here(
                "`where` clauses are removed — write the bound inline: `fn m<T requires B>(..)` (RFC 0043)",
            );
            while !matches!(p.tok(), Tok::LBrace | Tok::Semi | Tok::Eof) {
                p.bump();
            }
        }
    }

    fn pop(&mut self, p: &mut Parser, body: Option<NodeHandle<BlockNode>>) -> Step {
        let d = MethodDeclData {
            vis: self.vis,
            is_async: self.is_async,
            name: self.name,
            generics: std::mem::take(&mut self.generics),
            bounds: std::mem::take(&mut self.bounds),
            params: self.params.take().expect("method without params"),
            ret: self.ret.take(),
            body,
        };
        Step::Pop(Done::Method(p.method_decl(d, Span::new(self.lo, p.span().hi))))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match (self.stage, d) {
            (MeStage::GenBound, Done::Ty(t)) => {
                let g = self.pending.take().expect("bound without a parameter");
                self.bounds.push((g, t));
                let pending = generic_params_more(p, self.allow_bounds, "method", &mut self.generics);
                if let Some(g) = pending {
                    self.pending = Some(g);
                    return Step::Push(Frame::Type(TypeFrame::new_bound(p)));
                }
                self.stage = MeStage::Params;
                Step::Push(Frame::Params(ParamsFrame::new()))
            }
            (MeStage::GenBound, Done::Failed) => Step::Pop(Done::Failed),
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
    pre_async: bool,
    entry: bool,
    lo: u32,
    stage: FnSStage,
    is_async: bool,
    name: IdentId,
    generics: Vec<IdentId>,
    /// the generic whose `requires` bound is being parsed (GenBound stage)
    pending: Option<IdentId>,
    bounds: Vec<(IdentId, NodeHandle<AnyTy>)>,
    params: Option<Vec<NodeHandle<AnyParam>>>,
    ret: Option<NodeHandle<AnyTy>>,
}

#[derive(Clone, Copy)]
enum FnSStage {
    GenBound,
    Params,
    Ret,
    Body,
}

impl FnFrame {
    pub(crate) fn new(vis: Vis, pre_async: bool, entry: bool) -> Self {
        FnFrame {
            vis,
            pre_async,
            entry,
            lo: 0,
            stage: FnSStage::Params,
            is_async: false,
            name: IdentId(0),
            generics: Vec::new(),
            pending: None,
            bounds: Vec::new(),
            params: None,
            ret: None,
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = p.bump().span.lo; // fn
        self.is_async = self.pre_async || {
            let s = p.at_kw("async");
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
            let (gens, pending) = generic_params(p, true, "fn");
            self.generics = gens;
            if let Some(g) = pending {
                self.pending = Some(g);
                self.stage = FnSStage::GenBound;
                return Step::Push(Frame::Type(TypeFrame::new_bound(p)));
            }
        }
        Step::Push(Frame::Params(ParamsFrame::new()))
    }

    fn after_params(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::Arrow) {
            self.stage = FnSStage::Ret;
            return Step::Push(Frame::Type(TypeFrame::new(p)));
        }
        self.body(p)
    }

    /// The fn body — a stray `where` clause (removed, RFC 0043) diagnoses
    /// with the inline replacement and is skipped so the `{` still parses.
    fn body(&mut self, p: &mut Parser) -> Step {
        if p.at_kw("where") {
            p.err_here(
                "`where` clauses are removed — write the bound inline: `fn f<T requires B>(..)` (RFC 0043)",
            );
            while !matches!(p.tok(), Tok::LBrace | Tok::Eof) {
                p.bump();
            }
        }
        self.stage = FnSStage::Body;
        Step::Push(Frame::Block(BlockFrame::lax(p, self.lo)))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match (self.stage, d) {
            (FnSStage::GenBound, Done::Ty(t)) => {
                let g = self.pending.take().expect("bound without a parameter");
                self.bounds.push((g, t));
                let pending = generic_params_more(p, true, "fn", &mut self.generics);
                if let Some(g) = pending {
                    self.pending = Some(g);
                    return Step::Push(Frame::Type(TypeFrame::new_bound(p)));
                }
                self.stage = FnSStage::Params;
                Step::Push(Frame::Params(ParamsFrame::new()))
            }
            (FnSStage::GenBound, Done::Failed) => Step::Pop(Done::Failed),
            (FnSStage::Params, Done::Members(ps)) => {
                self.params = Some(ps);
                self.after_params(p)
            }
            (FnSStage::Params, Done::Failed) => Step::Pop(Done::Failed),
            (FnSStage::Ret, Done::Ty(t)) => {
                self.ret = Some(t);
                self.body(p)
            }
            (FnSStage::Ret, Done::Failed) => Step::Pop(Done::Failed),
            (FnSStage::Body, Done::Block(body)) => {
                let f = p.fn_decl(
                    FnData {
                        vis: self.vis,
                        is_async: self.is_async,
                        entry: self.entry,
                        name: self.name,
                        generics: std::mem::take(&mut self.generics),
                        params: self.params.take().expect("fn without params"),
                        ret: self.ret.take(),
                        bounds: std::mem::take(&mut self.bounds),
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

// ---- host/builtin surface declarations (.d.rut, RFC 0030 §3) ----
//
// Two linkages (RFC 0025): `host` — the embedding Rust implements it;
// `builtin` — the engine itself (compiler-lowered, core only). What
// each may spell:
//
//     host fn name(params) -> T;        concrete signature over the
//                                       crossing set (RFC 0023 §1)
//     host struct Name { fields }    flat record; every field a
//                                       crossing type; host-constructed
//     builtin fn name<T>(params) -> T;  engine fn (generics fine —
//                                       nothing crosses)
//     builtin primitive <name> { .. }   a boot primitive's surface
//                                       statement (`str`/`bytes`/
//                                       `opaque`) — never a class
//     builtin class Name<T> { methods } engine type's member contract
//     builtin trait Name<T> { .. }      engine-woven contract (Index,
//                                       Iterator, Disposal)
//
// `host class` and `extern` are gone: native state crosses as `opaque`
// and rut wraps it in a class (the `Logger` pattern, RFC 0028).

pub(crate) struct SurfaceFrame {
    linkage: Linkage,
    lo: u32,
    stage: SuStage,
    /// `builtin trait Name { .. }` — collects members like BuiltinTy
    /// but emits the trait node
    is_trait: bool,
    /// `builtin impl i32 { .. }` — collects bodiless methods like
    /// BuiltinTy but emits the builtin-impl node (RFC 0032 §1.1 R2)
    is_impl: bool,
    /// `builtin primitive opaque { .. }` — collects bodiless methods
    /// like BuiltinTy but emits the builtin-primitive node (the boot
    /// primitives' surface statement, builtin-surface phase 1)
    is_primitive: bool,
    name: IdentId,
    generics: Vec<IdentId>,
    methods: Vec<NodeHandle<MethodDeclNode>>,
    params: Option<Vec<NodeHandle<AnyParam>>>,
    ret: Option<NodeHandle<AnyTy>>,
}

#[derive(Clone, Copy)]
enum SuStage {
    Params,
    Ret,
    Members,
}

impl SurfaceFrame {
    pub(crate) fn new(linkage: Linkage) -> Self {
        SurfaceFrame {
            linkage,
            lo: 0,
            stage: SuStage::Params,
            is_trait: false,
            is_impl: false,
            is_primitive: false,
            name: IdentId(0),
            generics: Vec::new(),
            methods: Vec::new(),
            params: None,
            ret: None,
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        let sp = p.span();
        if p.mode == Mode::Impl {
            p.err(
                sp,
                "declaration keyword in an implementation file —`host`/`builtin` belong in a `.d.rut` (RFC 0029 §2)",
            );
        }
        self.lo = p.bump().span.lo;
        match self.linkage {
            Linkage::Host => self.host_step(p),
            Linkage::Builtin => self.builtin_step(p),
        }
    }

    fn host_step(&mut self, p: &mut Parser) -> Step {
        match p.tok().clone() {
            Tok::Ident(k) if k == "fn" => {
                p.bump();
                let Some(name) = p.expect_ident("a function name") else {
                    return Step::Pop(Done::Failed);
                };
                self.name = name;
                if matches!(p.tok(), Tok::Lt) {
                    // RFC 0023 §1: a host fn's parameters and returns are
                    // built from the crossing set — a generic parameter
                    // has no shape the boundary could check
                    p.err(
                        p.span(),
                        "host fn signatures are concrete —generic parameters cannot cross the boundary (RFC 0023 §1)",
                    );
                    let (gens, pending) = generic_params(p, false, "host fn");
                    self.generics = gens;
                    debug_assert!(pending.is_none(), "rejected bounds never suspend");
                }
                self.stage = SuStage::Params;
                Step::Push(Frame::Params(ParamsFrame::new()))
            }
            Tok::Ident(k) if k == "struct" => {
                p.bump();
                let Some(name) = p.expect_ident("a struct name") else {
                    return Step::Pop(Done::Failed);
                };
                self.name = name;
                self.stage = SuStage::Members;
                Step::Push(Frame::TypeBody(TypeBodyFrame::new(BodyMode::HostDataclass)))
            }
            Tok::Ident(k) if k == "class" => {
                p.err(
                    p.span(),
                    "`host class` is removed —declare `host fn`s and wrap native state in a rut `class` over `opaque` (RFC 0025)",
                );
                Step::Pop(Done::Failed)
            }
            _ => {
                let found = p.peek(0).describe();
                p.err_here(format!("expected `fn` or `struct` after `host`, found {found}"));
                Step::Pop(Done::Failed)
            }
        }
    }

    fn builtin_step(&mut self, p: &mut Parser) -> Step {
        match p.tok().clone() {
            Tok::Ident(k) if k == "fn" => {
                p.bump();
                let Some(name) = p.expect_ident("a function name") else {
                    return Step::Pop(Done::Failed);
                };
                self.name = name;
                // engine fns are compiler-lowered (assert/panic, on_drop) —
                // generics are fine: nothing crosses a boundary
                if matches!(p.tok(), Tok::Lt) {
                    let (gens, pending) = generic_params(p, false, "builtin fn");
                    self.generics = gens;
                    debug_assert!(pending.is_none(), "rejected bounds never suspend");
                }
                self.stage = SuStage::Params;
                Step::Push(Frame::Params(ParamsFrame::new()))
            }
            Tok::Ident(k) if k == "class" || k == "trait" => {
                // `builtin class Name<T> { .. }` — an engine builtin type's
                // member contract; `builtin trait Name<T> { .. }` — an
                // engine-woven contract (RFC 0025). Builtin decls spell
                // their kind: a bare `builtin Name { .. }` is an error.
                let is_trait = k == "trait";
                p.bump();
                let Some(name) = p.expect_ident("a builtin type name") else {
                    return Step::Pop(Done::Failed);
                };
                self.name = name;
                self.is_trait = is_trait;
                if matches!(p.tok(), Tok::Lt) {
                    let owner = if is_trait { "builtin trait" } else { "builtin class" };
                    let (gens, pending) = generic_params(p, false, owner);
                    self.generics = gens;
                    debug_assert!(pending.is_none(), "rejected bounds never suspend");
                }
                self.stage = SuStage::Members;
                p.expect(Tok::LBrace);
                self.members_top(p)
            }
            Tok::Ident(k) if k == "primitive" => {
                // `builtin primitive <name> { .. }` — the boot primitives'
                // surface statement (`str`/`bytes`/`opaque`): NOT a class —
                // no construction literal, no fields; the members are the
                // compiler-lowered contracts users and the LSP see
                p.bump();
                let Some(name) = p.expect_ident("a primitive name") else {
                    return Step::Pop(Done::Failed);
                };
                self.name = name;
                self.is_primitive = true;
                if matches!(p.tok(), Tok::Lt) {
                    p.err(p.span(), "a primitive takes no generic arguments");
                    // skip the rejected list so parsing stays in sync
                    while !matches!(p.tok(), Tok::LBrace | Tok::Semi | Tok::Eof) {
                        p.bump();
                    }
                }
                self.stage = SuStage::Members;
                p.expect(Tok::LBrace);
                self.members_top(p)
            }
            Tok::Ident(k) if k == "impl" => {
                // `builtin impl i32 { fn wrapping_add(self, y: i32) -> i32; .. }`
                // — numeric methods ON a primitive type (RFC 0032 §1.1 R2):
                // lowered inline at the method call, ambient on the prim
                p.bump();
                let Some(prim) = p.expect_ident("a primitive type name") else {
                    return Step::Pop(Done::Failed);
                };
                self.name = prim;
                self.is_impl = true;
                self.stage = SuStage::Members;
                p.expect(Tok::LBrace);
                self.members_top(p)
            }
            Tok::Ident(_) => {
                p.err_here(
                    "`builtin` spells its kind — `builtin primitive <name> { .. }`, `builtin class Name { .. }`, `builtin trait Name { .. }`, or `builtin impl <prim> { .. }` (RFC 0025/0032)",
                );
                Step::Pop(Done::Failed)
            }
            _ => {
                let found = p.peek(0).describe();
                p.err_here(format!("expected `fn`, `primitive`, `class`, `trait`, or `impl` after `builtin`, found {found}"));
                Step::Pop(Done::Failed)
            }
        }
    }

    fn members_top(&mut self, p: &mut Parser) -> Step {
        loop {
            if p.eat_punct(Tok::RBrace) || p.at_eof() {
                let span = Span::new(self.lo, p.span().hi);
                let node = if self.is_trait {
                    p.item(
                        ItemKind::BuiltinTrait {
                            vis: Vis::Self_,
                            name: self.name,
                            generics: std::mem::take(&mut self.generics),
                            methods: std::mem::take(&mut self.methods),
                        },
                        span,
                    )
                } else if self.is_impl {
                    p.item(
                        ItemKind::BuiltinImpl {
                            vis: Vis::Self_,
                            prim: self.name,
                            methods: std::mem::take(&mut self.methods),
                        },
                        span,
                    )
                } else if self.is_primitive {
                    p.item(
                        ItemKind::BuiltinPrimitive {
                            name: self.name,
                            members: std::mem::take(&mut self.methods),
                        },
                        span,
                    )
                } else {
                    p.item(
                        ItemKind::BuiltinTy {
                            vis: Vis::Self_,
                            name: self.name,
                            generics: std::mem::take(&mut self.generics),
                            members: std::mem::take(&mut self.methods),
                        },
                        span,
                    )
                };
                return Step::Pop(Done::Item(node));
            }
            let is_async = if p.at_kw("async") {
                p.bump();
                true
            } else {
                false
            };
            if p.at_kw("fn") {
                self.stage = SuStage::Members;
                return Step::Push(Frame::Method(MethodFrame::new(None, is_async, false, false)));
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
            (SuStage::Members, Done::Method(m)) => {
                self.methods.push(m);
                self.members_top(p)
            }
            // a host struct body arrives as (fields, no methods) — any
            // `fn` member was diagnosed by the body frame and is dropped
            (SuStage::Members, Done::Body(fields, _methods)) => {
                let node = p.item(
                    ItemKind::SurfaceDataclass {
                        vis: Vis::Self_,
                        name: self.name,
                        fields,
                    },
                    Span::new(self.lo, p.span().hi),
                );
                Step::Pop(Done::Item(node))
            }
            (SuStage::Members, Done::Failed) => {
                p.sync_stmt();
                match self.linkage {
                    Linkage::Host => Step::Pop(Done::Failed),
                    Linkage::Builtin => self.members_top(p),
                }
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
