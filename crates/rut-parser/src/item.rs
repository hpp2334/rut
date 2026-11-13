//! Declarations: items, pub visibility, use statements, type
//! bodies, fn/method signatures, surface stubs — as frames. The
//! grammar logic is the frame machine's own; construction is
//! ALLOCATE-COMPLETE over the capsule store (every node whole, docs
//! grafted at allocation, exact spans), and every member list fences
//! its broken regions as ErrorNodes beside the survivors.

use rut_ast::{
    AliasNode, BlockNode, BuiltinImplNode, BuiltinPrimitiveNode, BuiltinTyNode, ClassNode, DocData,
    EnumNode, FieldDeclNode, FieldRef, FnNode, Handle, IdentId, ImplNode, InterfaceNode, ItemRef,
    Linkage, MemberRef, MethodDeclNode, MethodRef, ModDeclNode, ModInlineNode, ModKind,
    ModifierNode, ModuleLetNode, ParamNode, ParamRef, SelfParamNode, StructNode, SurfaceFnNode,
    SurfaceStructNode, TyRef, UseNode, Vis,
};
use rut_lexer::span::Span;
use rut_lexer::token::Tok;

use crate::expr::{ExprFrame, ExprMode};
use crate::frame::{pub_scope, Done, Frame, PubFrame, Step};
use crate::stmt::BlockFrame;
use crate::ty::TypeFrame;
use crate::{with_doc, Mode, Name, Parser};

// ---- item dispatch ----

pub(crate) fn classify_item(p: &mut Parser) -> Option<Frame> {
    let sp = p.span();
    match p.tok().clone() {
        Tok::Ident(kw) => match kw.as_str() {
            "use" => Some(Frame::Use(UseFrame::new(Vis::Self_))),
            "mod" => Some(Frame::ModDecl(ModDeclFrame::new(Vis::Self_))),
            "pub" => Some(Frame::Pub(PubFrame::new())),
            "type" => Some(Frame::Alias(TypeAliasFrame::new(Vis::Self_))),
            "let" => Some(Frame::ModuleLet(ModuleLetFrame::new(Vis::Self_))),
            "enum" => Some(Frame::Enum(EnumFrame::new(Vis::Self_))),
            "struct" => Some(Frame::Struct(TyDeclFrame::new(false, Vis::Self_))),
            "class" => Some(Frame::Class(TyDeclFrame::new(true, Vis::Self_))),
            "interface" => Some(Frame::Interface(InterfaceFrame::new(Vis::Self_))),
            "impl" => Some(Frame::Impl(ImplFrame::new())),
            // `async fn` — the async declaration
            "async" if p.at_kw2("fn") => {
                p.bump();
                Some(Frame::Fn(FnFrame::new(Vis::Self_, true, false)))
            }
            "fn" => Some(Frame::Fn(FnFrame::new(Vis::Self_, false, false))),
            // `entry fn` — the host-callable surface:
            // contextual; only special directly before `fn`
            "entry" if p.at_kw2("fn") => {
                p.bump();
                Some(Frame::Fn(FnFrame::new(Vis::Self_, false, true)))
            }
            // `host` — the one native linkage left: the embedding Rust.
            // `extern` is removed — the loader + `entry fn`
            // cover it.
            "host" => Some(Frame::Surface(SurfaceFrame::new(Linkage::Host, Vis::Self_))),
            // `prelude builtin` — the AMBIENT engine surface (core only,
            // compiler-lowered). `prelude` is contextual: a legal
            // identifier everywhere except directly before `builtin`
            "prelude" if p.at_kw2("builtin") => {
                p.bump();
                Some(Frame::Surface(SurfaceFrame::new(
                    Linkage::Builtin { ambient: true },
                    Vis::Self_,
                )))
            }
            // bare `builtin` — the old spelling is gone: diagnosed, then
            // recovered as ambient so downstream diagnostics stay sane
            "builtin" => {
                p.err(
                    sp,
                    "`builtin` must be spelled `prelude builtin` (ambient) or `pub builtin` (import-gated)",
                );
                Some(Frame::Surface(SurfaceFrame::new(
                    Linkage::Builtin { ambient: true },
                    Vis::Self_,
                )))
            }
            "extern" => {
                p.err(
                    sp,
                    "`extern` linkage is removed —`host` is the only native surface (embedding Rust); rut packages arrive through the module loader",
                );
                None
            }
            // statement keywords at module scope
            "if" | "while" | "for" | "return" | "when" | "break" | "continue" | "await" => {
                p.err(
                    sp,
                    "statements are not allowed at module scope —modules contain declarations only",
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
            "mod" => Some(Frame::ModDecl(ModDeclFrame::new(vis))),
            // `pub use` — the re-export: the imported names join the
            // module's public surface (a private `use` imports only)
            "use" => Some(Frame::Use(UseFrame::new(vis))),
            "type" => Some(Frame::Alias(TypeAliasFrame::new(vis))),
            "enum" => Some(Frame::Enum(EnumFrame::new(vis))),
            "struct" => Some(Frame::Struct(TyDeclFrame::new(false, vis))),
            "class" => Some(Frame::Class(TyDeclFrame::new(true, vis))),
            "interface" => Some(Frame::Interface(InterfaceFrame::new(vis))),
            // `async fn` — the async declaration
            "async" if p.at_kw2("fn") => {
                p.bump();
                Some(Frame::Fn(FnFrame::new(vis, true, false)))
            }
            "fn" => Some(Frame::Fn(FnFrame::new(vis, false, false))),
            "host" => Some(Frame::Surface(SurfaceFrame::new(Linkage::Host, vis))),
            // `pub builtin` — the IMPORT-GATED engine surface spelling:
            // the decl resolves only through `use <pkg>::{ .. }` (the
            // ambient twin is `prelude builtin`)
            "builtin" => Some(Frame::Surface(SurfaceFrame::new(
                Linkage::Builtin { ambient: false },
                vis,
            ))),
            "extern" => {
                p.err_here(
                    "`extern` linkage is removed —`host` is the only native surface (embedding Rust); rut packages arrive through the module loader",
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

/// `pub? use pkg::A::B::{C, D};` / `use pkg::A::B::C;` — the path walks
/// `ident (:: ident)*` (`path[0]` is the package, the middle segments
/// name modules), then the leaf is a brace list or a single name (the
/// LAST path segment pops into names). The flat forms —
/// `use pkg::{A, B};` / `use pkg::A;` — degenerate to `path = [pkg]`.
/// The span starts at the `use` keyword (the `pub` prefix rides in
/// `vis`, the other vis-carrying items' convention).
pub(crate) struct UseFrame {
    vis: Vis,
    lo: u32,
    path: Vec<IdentId>,
    path_spans: Vec<Span>,
    names: Vec<IdentId>,
    name_spans: Vec<Span>,
}

impl UseFrame {
    pub(crate) fn new(vis: Vis) -> Self {
        UseFrame {
            vis,
            lo: 0,
            path: Vec::new(),
            path_spans: Vec::new(),
            names: Vec::new(),
            name_spans: Vec::new(),
        }
    }

    fn push_seg(&mut self, n: Name) {
        self.path.push(n.id);
        self.path_spans.push(n.span);
    }
    fn push_name(&mut self, n: Name) {
        self.names.push(n.id);
        self.name_spans.push(n.span);
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = p.bump().span.lo; // use
        let Some(pkg) = p.expect_ident("a package name") else {
            p.sync_stmt();
            return Step::Pop(Done::Failed);
        };
        self.push_seg(pkg);
        if !p.eat_punct(Tok::Colon2) {
            p.err_here(
                "expected `::` after the package name — use paths spell `use <pkg>::{A, B};`",
            );
            p.sync_stmt();
            return Step::Pop(Done::Failed);
        }
        if p.eat_punct(Tok::LBrace) {
            if !self.brace_names(p) {
                p.sync_stmt();
                return Step::Pop(Done::Failed);
            }
        } else {
            // the module path: `ident (:: ident)*`, then either the
            // brace leaf or the `::ident` single-name leaf
            loop {
                let Some(seg) = p.expect_ident("a path segment") else {
                    p.sync_stmt();
                    return Step::Pop(Done::Failed);
                };
                self.push_seg(seg);
                if !p.eat_punct(Tok::Colon2) {
                    break;
                }
                if p.eat_punct(Tok::LBrace) {
                    if !self.brace_names(p) {
                        p.sync_stmt();
                        return Step::Pop(Done::Failed);
                    }
                    break;
                }
            }
        }
        if self.names.is_empty() {
            // no brace leaf: the single-name leaf is the LAST path
            // segment — `use pkg::A;` (the flat form) and
            // `use pkg::A::B::C;` pop it; `use pkg::A::B;` spells a mod
            // path with nothing at the leaf
            match self.path.len() {
                0 | 1 => {} // `use pkg::{};` — names stay empty, as today
                3 => p.err_here(
                    "`use pkg::A::B;` imports nothing — spell a name at the leaf (`::{ C }` or `::C`)",
                ),
                _ => {
                    let id = self.path.pop().expect("path depth checked above");
                    let sp = self.path_spans.pop().expect("spans parallel the ids");
                    self.names.push(id);
                    self.name_spans.push(sp);
                }
            }
        }
        p.expect(Tok::Semi);
        let node = with_doc!(
            p,
            UseNode {
                span: p.span_from(self.lo),
                vis: self.vis,
                path: std::mem::take(&mut self.path),
                name_span: self.path_spans.first().copied().unwrap_or(Span::new(0, 0)),
                names: std::mem::take(&mut self.names),
                name_spans: std::mem::take(&mut self.name_spans),
                doc: DocData::default(),
            }
        );
        Step::Pop(Done::Item(node.allocate(&mut p.ast)))
    }

    /// the brace leaf — `::{A, B}` — one or more idents. `false` when
    /// the list failed to parse (the caller resyncs).
    fn brace_names(&mut self, p: &mut Parser) -> bool {
        loop {
            if p.eat_punct(Tok::RBrace) {
                return true;
            }
            let Some(n) = p.expect_ident("a used name") else {
                return false;
            };
            self.push_name(n);
            if !p.eat_punct(Tok::Comma) {
                p.expect(Tok::RBrace);
                return true;
            }
        }
    }

    pub(crate) fn absorb(&mut self, _p: &mut Parser, _d: Done) -> Step {
        unreachable!("use frame pushes no children")
    }
}

// ---- mod decl ----

/// `pub? mod NAME;` — a file-module DECLARATION: the body lives in the
/// sibling `NAME/mod.rut`; the loader mounts it. In implementation
/// files the inline `mod NAME { .. }` block form is gone — one
/// mechanism, files only — and diagnosing it still produces the decl
/// (the stray block is skipped so the tail keeps parsing). In a
/// DECLARATION file (`Mode::Decl`) the inline form IS legal and the
/// block parses as a `ModInlineNode`.
pub(crate) struct ModDeclFrame {
    vis: Vis,
    lo: u32,
    name: Name,
    /// the inline form is out for its block (Decl mode): the frame
    /// relays the inline module's item upward when it pops
    inline: bool,
}

impl ModDeclFrame {
    pub(crate) fn new(vis: Vis) -> Self {
        ModDeclFrame {
            vis,
            lo: 0,
            name: Name {
                id: IdentId(0),
                span: Span::new(0, 0),
            },
            inline: false,
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = p.bump().span.lo; // mod
        let Some(name) = p.expect_ident("a module name") else {
            return Step::Pop(Done::Failed);
        };
        self.name = name;
        if p.eat_punct(Tok::LBrace) {
            if p.mode == Mode::Decl {
                // the inline declaration module: the frame collects the
                // nested declarations (it consumed the `{` above); this
                // frame stays underneath and relays its item
                self.inline = true;
                let (vis, lo, name) = (self.vis, self.lo, self.name);
                return Step::Push(Frame::ModInline(ModInlineFrame::from_decl(vis, lo, name)));
            }
            p.err_here(
                "inline `mod { }` blocks are gone — the body lives in `NAME/mod.rut` beside this file",
            );
            // skip the stray block so the module's tail still parses —
            // one diag, and the declaration itself survives
            let mut depth = 1usize;
            while depth > 0 && !p.at_eof() {
                if p.eat_punct(Tok::LBrace) {
                    depth += 1;
                } else if p.eat_punct(Tok::RBrace) {
                    depth -= 1;
                } else {
                    p.bump();
                }
            }
        } else {
            p.expect(Tok::Semi);
        }
        let node = with_doc!(
            p,
            ModDeclNode {
                span: p.span_from(self.lo),
                vis: self.vis,
                name: self.name.id,
                name_span: self.name.span,
                doc: DocData::default(),
            }
        );
        Step::Pop(Done::Item(node.allocate(&mut p.ast)))
    }

    pub(crate) fn absorb(&mut self, _p: &mut Parser, d: Done) -> Step {
        if self.inline {
            // the inline module popped — relay its item upward
            match d {
                Done::Item(i) => return Step::Pop(Done::Item(i)),
                Done::Failed => return Step::Pop(Done::Failed),
                _ => unreachable!("mod decl frame's inline child yields an item"),
            }
        }
        unreachable!("mod decl frame pushes no children")
    }
}

/// The inline declaration module's frame — `pub? mod NAME { <decls> }`
/// in a `.d.rut` (Decl mode only). Declarations only, recursively.
pub(crate) struct ModInlineFrame {
    vis: Vis,
    lo: u32,
    name: Name,
    items: Vec<ItemRef>,
    /// cursor position + diag watermark when the current item started
    before: usize,
    dmark: usize,
}

impl ModInlineFrame {
    /// The `{` was already consumed by [`ModDeclFrame::step`].
    fn from_decl(vis: Vis, lo: u32, name: Name) -> Self {
        ModInlineFrame {
            vis,
            lo,
            name,
            items: Vec::new(),
            before: 0,
            dmark: 0,
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        loop {
            if p.eat_punct(Tok::RBrace) || p.at_eof() {
                let node = with_doc!(
                    p,
                    ModInlineNode {
                        span: p.span_from(self.lo),
                        vis: self.vis,
                        name: self.name.id,
                        name_span: self.name.span,
                        items: std::mem::take(&mut self.items),
                        doc: DocData::default(),
                    }
                );
                return Step::Pop(Done::Item(node.allocate(&mut p.ast)));
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
            _ => unreachable!("inline mod frame receives items"),
        }
        self.step(p)
    }

    /// the item-level fence, scoped to the block: guarantee progress,
    /// resync (an unbalanced `}` is left for the frame's own loop top)
    fn recover(&mut self, p: &mut Parser) {
        if p.pos == self.before {
            p.err_here("expected a declaration");
            p.bump();
        }
        p.sync_stmt();
        let (before, mark) = (self.before, self.dmark);
        self.items.push(ItemRef::Error(p.fence(before, mark)));
    }
}

// ---- type alias ----

/// `pub(..)? type Name = Target;`: a transparent alias, or a
/// `Target` spelled `A | B` for the bound-only union. Aliases are
/// NON-GENERIC — `type Name<K, ..>` is a parse error.
pub(crate) struct TypeAliasFrame {
    vis: Vis,
    lo: u32,
    name: Name,
}

impl TypeAliasFrame {
    pub(crate) fn new(vis: Vis) -> Self {
        TypeAliasFrame {
            vis,
            lo: 0,
            name: Name {
                id: IdentId(0),
                span: Span::new(0, 0),
            },
        }
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
            Done::Ty(ty) => {
                p.expect(Tok::Semi);
                let node = with_doc!(
                    p,
                    AliasNode {
                        span: p.span_from(self.lo),
                        vis: self.vis,
                        name: self.name.id,
                        name_span: self.name.span,
                        target: ty,
                        doc: DocData::default(),
                    }
                );
                Step::Pop(Done::Item(node.allocate(&mut p.ast)))
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
    name: Name,
    ty: Option<TyRef>,
}

#[derive(Clone, Copy)]
enum MlStage {
    Ty,
    Init,
}

impl ModuleLetFrame {
    pub(crate) fn new(vis: Vis) -> Self {
        ModuleLetFrame {
            vis,
            lo: 0,
            stage: MlStage::Ty,
            name: Name {
                id: IdentId(0),
                span: Span::new(0, 0),
            },
            ty: None,
        }
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
                let node = with_doc!(
                    p,
                    ModuleLetNode {
                        span: p.span_from(self.lo),
                        vis: self.vis,
                        name: self.name.id,
                        name_span: self.name.span,
                        ty: self.ty.take(),
                        init,
                        doc: DocData::default(),
                    }
                );
                Step::Pop(Done::Item(node.allocate(&mut p.ast)))
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
    name: Name,
    members: Vec<MemberRef>,
}

impl EnumFrame {
    pub(crate) fn new(vis: Vis) -> Self {
        EnumFrame {
            vis,
            lo: 0,
            name: Name {
                id: IdentId(0),
                span: Span::new(0, 0),
            },
            members: Vec::new(),
        }
    }

    /// resync inside a member list: to the `,`, the `}`, or EOF
    fn sync_member(p: &mut Parser) {
        while !matches!(p.tok(), Tok::Comma | Tok::RBrace | Tok::Eof) {
            p.bump();
        }
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
            let (before, dmark) = (p.pos, p.mark());
            match p.expect_ident("an enum member") {
                Some(m) => {
                    let node = with_doc!(
                        p,
                        rut_ast::EnumMemberNode {
                            span: m.span,
                            name: m.id,
                            name_span: m.span,
                            doc: DocData::default(),
                        }
                    );
                    self.members.push(node.allocate(&mut p.ast));
                    if !p.eat_punct(Tok::Comma) {
                        if !p.eat_punct(Tok::RBrace) {
                            let found = p.peek(0).describe();
                            p.err_here(format!("expected `,` or `}}`, found {found}"));
                            // skip the stray tokens so the member list's tail
                            // (`, B }`) still parses — one diag, full recovery
                            Self::sync_member(p);
                            if p.eat_punct(Tok::Comma) {
                                continue;
                            }
                            // the stray run may end at the enum's own `}` — take it
                            p.eat_punct(Tok::RBrace);
                        }
                        break;
                    }
                }
                None => {
                    // the ladder's member tier: a broken member fences
                    // IN the list — the enum survives with its hole
                    if p.pos == before {
                        p.bump();
                    }
                    Self::sync_member(p);
                    self.members.push(MemberRef::Error(p.fence(before, dmark)));
                    if !p.eat_punct(Tok::Comma) {
                        p.eat_punct(Tok::RBrace);
                        break;
                    }
                }
            }
        }
        let node = with_doc!(
            p,
            EnumNode {
                span: p.span_from(self.lo),
                vis: self.vis,
                name: self.name.id,
                name_span: self.name.span,
                members: std::mem::take(&mut self.members),
                doc: DocData::default(),
            }
        );
        Step::Pop(Done::Item(node.allocate(&mut p.ast)))
    }

    pub(crate) fn absorb(&mut self, _p: &mut Parser, _d: Done) -> Step {
        unreachable!("enum frame pushes no children")
    }
}

// ---- generic parameter list ----
//
// fn/method generic parameters may carry inline admission
// bounds (`<T requires A | B>`), and so do CLASS generics; struct/
// interface/surface generics reject them. A bound suspends the
// enclosing frame at a GenBound stage — the bound itself is a child
// Type frame.

fn generic_params(p: &mut Parser, allow_bounds: bool, owner: &str) -> (Vec<IdentId>, Option<Name>) {
    p.bump(); // < (the caller checked)
    let mut out = Vec::new();
    let pending = generic_params_more(p, allow_bounds, owner, &mut out);
    (out, pending)
}

/// Continue the list at a parameter name — after `<`, a comma, or an
/// absorbed bound. `Some(n)` when `n requires` suspends the list.
fn generic_params_more(
    p: &mut Parser,
    allow_bounds: bool,
    owner: &str,
    out: &mut Vec<IdentId>,
) -> Option<Name> {
    p.eat_punct(Tok::Comma); // after an absorbed bound the separator is still ahead
    loop {
        if p.eat_punct(Tok::Gt) {
            return None;
        }
        let n = p.expect_ident("a generic parameter")?;
        out.push(n.id);
        if p.at_kw("requires") {
            p.bump();
            if allow_bounds {
                return Some(n);
            }
            p.err_here(format!(
                "`{owner}` generic parameters take no `requires` bounds — inline bounds bind fn/method/class generics only"
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

pub(crate) struct TyDeclFrame {
    is_class: bool,
    vis: Vis,
    lo: u32,
    stage: TdStage,
    name: Name,
    generics: Vec<IdentId>,
    /// the class's admission bounds, in `requires` order (empty for
    /// structs — their generics never carry bounds)
    requires: Vec<(IdentId, TyRef)>,
    /// the generic whose `requires` bound is being parsed (GenBound stage)
    pending: Option<IdentId>,
}

#[derive(Clone, Copy)]
enum TdStage {
    GenBound,
    /// the wrapped type of the positional decl (`class Name(Ty);`) —
    /// a child Type frame
    NewtypeTy,
    Body,
}

impl TyDeclFrame {
    pub(crate) fn new(is_class: bool, vis: Vis) -> Self {
        TyDeclFrame {
            is_class,
            vis,
            lo: 0,
            stage: TdStage::Body,
            name: Name {
                id: IdentId(0),
                span: Span::new(0, 0),
            },
            generics: Vec::new(),
            requires: Vec::new(),
            pending: None,
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = p.bump().span.lo; // struct | class
        let what = if self.is_class {
            "a class name"
        } else {
            "a struct name"
        };
        let Some(name) = p.expect_ident(what) else {
            return Step::Pop(Done::Failed);
        };
        self.name = name;
        if matches!(p.tok(), Tok::Lt) {
            let owner = if self.is_class { "class" } else { "struct" };
            let (gens, pending) = generic_params(p, self.is_class, owner);
            self.generics = gens;
            if let Some(n) = pending {
                self.pending = Some(n.id);
                self.stage = TdStage::GenBound;
                return Step::Push(Frame::Type(TypeFrame::new_bound(p)));
            }
        }
        self.push_body(p)
    }

    fn push_body(&mut self, p: &mut Parser) -> Step {
        // the positional one-field form — `class Name(Wrapped);` — the
        // newtype decl
        if self.is_class && matches!(p.tok(), Tok::LParen) {
            p.bump(); // (
            if p.eat_punct(Tok::RParen) {
                p.err_here("a newtype class wraps exactly one type — `class Name(Wrapped);`");
                p.sync_item();
                return Step::Pop(Done::Failed);
            }
            self.stage = TdStage::NewtypeTy;
            return Step::Push(Frame::Type(TypeFrame::new(p)));
        }
        self.stage = TdStage::Body;
        Step::Push(Frame::TypeBody(TypeBodyFrame::new(BodyMode::Class {
            allow_pub: self.is_class,
            is_struct: !self.is_class,
        })))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match (self.stage, d) {
            (TdStage::GenBound, Done::Ty(t)) => {
                let g = self.pending.take().expect("bound without a parameter");
                self.requires.push((g, t));
                let owner = if self.is_class { "class" } else { "struct" };
                let pending = generic_params_more(p, self.is_class, owner, &mut self.generics);
                if let Some(n) = pending {
                    self.pending = Some(n.id);
                    return Step::Push(Frame::Type(TypeFrame::new_bound(p)));
                }
                self.push_body(p)
            }
            (TdStage::GenBound, Done::Failed) => Step::Pop(Done::Failed),
            (TdStage::NewtypeTy, Done::Ty(t)) => {
                // `class Foo(Bar);` ≡ `class Foo { inner: Bar }` plus the
                // compiler-provided constructor — the desugar happens
                // HERE, as one synthesized field
                p.expect(Tok::RParen);
                if p.eat_punct(Tok::LBrace) {
                    p.err_here("a newtype class has no body — the wrapped type is the one field; methods live in `impl` blocks");
                    p.sync_item();
                    return Step::Pop(Done::Failed);
                }
                p.expect(Tok::Semi);
                let inner = p.ast.intern("inner");
                let ty_span = t.span(&p.ast);
                // SYNTHESIZED — no doc graft (it would steal the class's
                // own chain above)
                let field = FieldDeclNode {
                    span: ty_span,
                    modifiers: Vec::new(),
                    name: inner,
                    name_span: ty_span,
                    ty: t,
                    init: None,
                    doc: DocData::default(),
                }
                .allocate(&mut p.ast);
                let node = with_doc!(
                    p,
                    ClassNode {
                        span: p.span_from(self.lo),
                        vis: self.vis,
                        name: self.name.id,
                        name_span: self.name.span,
                        generics: std::mem::take(&mut self.generics),
                        requires: std::mem::take(&mut self.requires),
                        newtype: true,
                        fields: vec![field],
                        methods: Vec::new(),
                        doc: DocData::default(),
                    }
                );
                Step::Pop(Done::Item(node.allocate(&mut p.ast)))
            }
            (TdStage::NewtypeTy, Done::Failed) => Step::Pop(Done::Failed),
            (TdStage::Body, Done::Body(fields, methods)) => {
                let item = if self.is_class {
                    with_doc!(
                        p,
                        ClassNode {
                            span: p.span_from(self.lo),
                            vis: self.vis,
                            name: self.name.id,
                            name_span: self.name.span,
                            generics: std::mem::take(&mut self.generics),
                            requires: std::mem::take(&mut self.requires),
                            newtype: false,
                            fields,
                            methods,
                            doc: DocData::default(),
                        }
                    )
                    .allocate(&mut p.ast)
                } else {
                    with_doc!(
                        p,
                        StructNode {
                            span: p.span_from(self.lo),
                            vis: self.vis,
                            name: self.name.id,
                            name_span: self.name.span,
                            generics: std::mem::take(&mut self.generics),
                            fields,
                            methods,
                            doc: DocData::default(),
                        }
                    )
                    .allocate(&mut p.ast)
                };
                Step::Pop(Done::Item(item))
            }
            (TdStage::Body, Done::Failed) => Step::Pop(Done::Failed),
            _ => unreachable!("type declarations receive a body"),
        }
    }
}

// ---- interface ----

pub(crate) struct InterfaceFrame {
    vis: Vis,
    lo: u32,
    name: Name,
    generics: Vec<IdentId>,
}

impl InterfaceFrame {
    pub(crate) fn new(vis: Vis) -> Self {
        InterfaceFrame {
            vis,
            lo: 0,
            name: Name {
                id: IdentId(0),
                span: Span::new(0, 0),
            },
            generics: Vec::new(),
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = p.bump().span.lo; // interface
        let Some(name) = p.expect_ident("an interface name") else {
            return Step::Pop(Done::Failed);
        };
        self.name = name;
        if matches!(p.tok(), Tok::Lt) {
            let (gens, pending) = generic_params(p, false, "interface");
            self.generics = gens;
            debug_assert!(pending.is_none(), "rejected bounds never suspend");
        }
        Step::Push(Frame::TypeBody(TypeBodyFrame::new(BodyMode::Interface)))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match d {
            Done::Body(_, methods) => {
                let node = with_doc!(
                    p,
                    InterfaceNode {
                        span: p.span_from(self.lo),
                        vis: self.vis,
                        name: self.name.id,
                        name_span: self.name.span,
                        generics: std::mem::take(&mut self.generics),
                        methods,
                        doc: DocData::default(),
                    }
                );
                Step::Pop(Done::Item(node.allocate(&mut p.ast)))
            }
            Done::Failed => Step::Pop(Done::Failed),
            _ => unreachable!("interface frame receives a body"),
        }
    }
}

// ---- impl ----

pub(crate) struct ImplFrame {
    lo: u32,
    stage: ImStage,
    generics: Vec<IdentId>,
    bounds: Vec<(IdentId, TyRef)>,
    pending: Option<IdentId>,
    target: Option<TyRef>,
}

#[derive(Clone, Copy)]
enum ImStage {
    GenBound,
    Head,
    Methods,
}

impl ImplFrame {
    pub(crate) fn new() -> Self {
        ImplFrame {
            lo: 0,
            stage: ImStage::Head,
            generics: Vec::new(),
            bounds: Vec::new(),
            pending: None,
            target: None,
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = p.bump().span.lo; // impl
        if p.mode == Mode::Decl {
            p.err(
                Span::new(self.lo, self.lo + 4),
                "implementation in a declaration file —`impl` blocks live in `.rut`",
            );
        }
        if matches!(p.tok(), Tok::Lt) {
            let (gens, pending) = generic_params(p, true, "impl");
            self.generics = gens;
            if let Some(n) = pending {
                self.pending = Some(n.id);
                self.stage = ImStage::GenBound;
                return Step::Push(Frame::Type(TypeFrame::new(p)));
            }
        }
        Step::Push(Frame::Type(TypeFrame::new(p)))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match (self.stage, d) {
            (ImStage::GenBound, Done::Ty(t)) => {
                let g = self.pending.take().expect("bound without a parameter");
                self.bounds.push((g, t));
                let pending = generic_params_more(p, true, "impl", &mut self.generics);
                if let Some(n) = pending {
                    self.pending = Some(n.id);
                    return Step::Push(Frame::Type(TypeFrame::new(p)));
                }
                self.stage = ImStage::Head;
                Step::Push(Frame::Type(TypeFrame::new(p)))
            }
            (ImStage::GenBound, Done::Failed) => Step::Pop(Done::Failed),
            (ImStage::Head, Done::Ty(t)) => {
                self.target = Some(t);
                self.stage = ImStage::Methods;
                Step::Push(Frame::TypeBody(TypeBodyFrame::new(BodyMode::Inherent)))
            }
            (ImStage::Methods, Done::Body(_, methods)) => {
                let node = ImplNode {
                    span: p.span_from(self.lo),
                    generics: std::mem::take(&mut self.generics),
                    bounds: std::mem::take(&mut self.bounds),
                    target: self.target.take().expect("impl without a target"),
                    methods,
                }
                .allocate(&mut p.ast);
                Step::Pop(Done::Item(node))
            }
            (_, Done::Failed) => Step::Pop(Done::Failed),
            _ => unreachable!("impl frame receives types or a body"),
        }
    }
}

// ---- struct/class/interface/impl bodies ----

pub(crate) enum BodyMode {
    /// struct/class body: FIELDS ONLY — methods live in `impl` blocks
    Class { allow_pub: bool, is_struct: bool },
    /// `host struct` body: fields only, no initializers
    HostStruct,
    /// interface body: bodiless method signatures
    Interface,
    /// `impl T` body: inherent methods
    Inherent,
}

pub(crate) struct TypeBodyFrame {
    mode: BodyMode,
    fields: Vec<FieldRef>,
    methods: Vec<MethodRef>,
    stage: TbStage,
    /// the member under construction
    f_lo: Span,
    /// the member's spelled modifiers, in source order
    f_mods: Vec<ModifierNode>,
    f_name: Name,
    f_ty: Option<TyRef>,
    /// member start + diag watermark (the fence's extent)
    before: usize,
    dmark: usize,
}

#[derive(Clone, Copy)]
enum TbStage {
    Open,
    Members,
    FieldTy,
    FieldInit,
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
            f_mods: Vec::new(),
            f_name: Name {
                id: IdentId(0),
                span: Span::new(0, 0),
            },
            f_ty: None,
            before: 0,
            dmark: 0,
        }
    }

    fn mode_noun(&self) -> &'static str {
        match self.mode {
            BodyMode::Interface => "interfaces declare method signatures",
            BodyMode::Inherent => "inherent impl blocks declare methods",
            BodyMode::HostStruct => "host structs declare fields",
            BodyMode::Class { .. } => "type bodies declare fields —methods live in `impl` blocks",
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        let strict = matches!(
            self.mode,
            BodyMode::Class { .. } | BodyMode::HostStruct | BodyMode::Interface
        );
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

    /// The bracket marker's PARSE half: `[word]` before a member's
    /// modifiers — any contextual ident; the CHECKER validates the
    /// closed marker set. The modifier's SPAN (brackets included)
    /// rides along — it is a ModifierNode's own span.
    fn parse_member_marker(p: &mut Parser) -> Option<(IdentId, Span)> {
        if !matches!(p.tok(), Tok::LBracket) {
            return None;
        }
        let br = p.span(); // the `[` token — the diag anchor
        let lo = br.lo;
        p.bump(); // [
        let word = p
            .expect_ident("a marker name after `[`")
            .map(|n| n.id)
            .unwrap_or(IdentId(0));
        if p.expect(Tok::RBracket).is_none() {
            p.err(
                br,
                "unterminated marker — `[disposal]` / `[iterable]` / `[constructor]`",
            );
        }
        Some((word, Span::new(lo, p.prev_end.max(lo))))
    }

    fn members_top(&mut self, p: &mut Parser) -> Step {
        loop {
            if p.eat_punct(Tok::RBrace) || p.at_eof() {
                let fields = std::mem::take(&mut self.fields);
                let methods = std::mem::take(&mut self.methods);
                return Step::Pop(Done::Body(fields, methods));
            }
            self.before = p.pos;
            self.dmark = p.mark();
            match self.mode {
                BodyMode::HostStruct => {
                    let lo = p.span();
                    while let Tok::Ident(m) = p.tok().clone() {
                        match m.as_str() {
                            "pub" => {
                                p.bump();
                                let _ = pub_scope(p);
                                p.err(
                                    lo,
                                    "host struct fields carry no visibility —all fields are public",
                                );
                            }
                            "static" | "async" => {
                                p.bump();
                                p.err(lo, format!("host struct fields carry no modifiers —`{m}` is not declarable here"));
                            }
                            _ => break,
                        }
                    }
                    match p.tok().clone() {
                        // parse (bodiless) to stay in sync; the methods are
                        // dropped — SurfaceStruct keeps fields only
                        Tok::Ident(kw) if kw == "fn" => {
                            p.err(lo, "host structs declare fields only —methods live in rut wrapper classes");
                            self.stage = TbStage::Method;
                            return Step::Push(Frame::Method(MethodFrame::new(
                                Vec::new(),
                                lo.lo,
                                false,
                                false,
                            )));
                        }
                        Tok::Ident(_) => {
                            let name = p.expect_ident("a field name").unwrap_or(Name {
                                id: IdentId(0),
                                span: lo,
                            });
                            p.expect(Tok::Colon);
                            self.stage = TbStage::FieldTy;
                            self.f_lo = lo;
                            self.f_mods = Vec::new();
                            self.f_name = name;
                            return Step::Push(Frame::Type(TypeFrame::new(p)));
                        }
                        _ => {
                            let found = p.peek(0).describe();
                            p.err_here(format!("expected a field, found {found}"));
                            p.sync_stmt();
                            let (before, mark) = (self.before, self.dmark);
                            self.fields.push(FieldRef::Error(p.fence(before, mark)));
                        }
                    }
                }
                BodyMode::Class {
                    allow_pub,
                    is_struct,
                } => {
                    let lo = p.span();
                    let mut mods: Vec<ModifierNode> = Vec::new();
                    while let Tok::Ident(m) = p.tok().clone() {
                        match m.as_str() {
                            "pub" => {
                                let m_lo = p.span().lo;
                                p.bump();
                                let v = pub_scope(p);
                                mods.push(ModifierNode {
                                    span: Span::new(m_lo, p.prev_end.max(m_lo)),
                                    kind: ModKind::Pub(v),
                                });
                                if !allow_pub {
                                    p.err(
                                        lo,
                                        "structs have no field visibility —all fields are public",
                                    );
                                }
                            }
                            "static" => {
                                let sp = p.span();
                                p.bump();
                                mods.push(ModifierNode {
                                    span: sp,
                                    kind: ModKind::Static,
                                });
                                if is_struct {
                                    p.err(lo, "structs have no `static` members");
                                }
                            }
                            "async" => {
                                p.bump();
                                p.err(lo, "unexpected `async` —type bodies declare fields only");
                            }
                            _ => break,
                        }
                    }
                    match p.tok().clone() {
                        Tok::Ident(kw) if kw == "fn" => {
                            if p.mode != Mode::Decl {
                                p.err(
                                    lo,
                                    "methods live in `impl` blocks —type bodies declare fields only",
                                );
                            }
                            if mods.iter().any(|m| matches!(m.kind, ModKind::Static)) {
                                p.err(
                                    lo,
                                    "there is no `static fn` —a method without `self` IS a class method",
                                );
                            }
                            self.stage = TbStage::Method;
                            return Step::Push(Frame::Method(MethodFrame::new(
                                mods, lo.lo, true, true,
                            )));
                        }
                        Tok::Ident(_) => {
                            let name = p.expect_ident("a field name").unwrap_or(Name {
                                id: IdentId(0),
                                span: lo,
                            });
                            p.expect(Tok::Colon);
                            self.stage = TbStage::FieldTy;
                            self.f_lo = lo;
                            self.f_mods = mods;
                            self.f_name = name;
                            return Step::Push(Frame::Type(TypeFrame::new(p)));
                        }
                        _ => {
                            let found = p.peek(0).describe();
                            p.err_here(format!("expected a field, found {found}"));
                            p.sync_stmt();
                            let (before, mark) = (self.before, self.dmark);
                            self.fields.push(FieldRef::Error(p.fence(before, mark)));
                        }
                    }
                }
                BodyMode::Interface => {
                    let lo = p.span();
                    let mut mods: Vec<ModifierNode> = Vec::new();
                    let marker = Self::parse_member_marker(p);
                    while let Tok::Ident(m) = p.tok().clone() {
                        match m.as_str() {
                            "pub" => {
                                p.bump();
                                let _ = pub_scope(p);
                                p.err(lo, "interface members carry no `pub` —the interface's visibility rules");
                            }
                            "async" => {
                                let sp = p.span();
                                p.bump();
                                mods.push(ModifierNode {
                                    span: sp,
                                    kind: ModKind::Async,
                                });
                            }
                            _ => break,
                        }
                    }
                    if let Some((word, _)) = marker {
                        p.err(lo, format!(
                            "`[{}]` marks an inherent impl member — engine contracts are declared on `impl Type {{ .. }}` blocks, never an interface",
                            p.ast.name(word)
                        ));
                    }
                    if p.at_kw("fn") {
                        self.stage = TbStage::Method;
                        return Step::Push(Frame::Method(MethodFrame::new(
                            mods, lo.lo, false, true,
                        )));
                    }
                    if mods.iter().any(|m| matches!(m.kind, ModKind::Async)) {
                        p.err(lo, "expected `fn` after `async`");
                    }
                    let (what, found) = (self.mode_noun(), p.peek(0).describe());
                    p.err_here(format!("{what} —expected `fn`, found {found}"));
                    p.sync_stmt();
                }
                BodyMode::Inherent => {
                    // `impl T { .. }` — inherent methods: the bracket
                    // marker FIRST, then `pub`/`async`
                    let lo = p.span();
                    let mut mods: Vec<ModifierNode> = Vec::new();
                    if let Some((word, sp)) = Self::parse_member_marker(p) {
                        mods.push(ModifierNode {
                            span: sp,
                            kind: ModKind::Marker(word),
                        });
                    }
                    while let Tok::Ident(m) = p.tok().clone() {
                        match m.as_str() {
                            "pub" => {
                                let m_lo = p.span().lo;
                                p.bump();
                                let v = pub_scope(p);
                                mods.push(ModifierNode {
                                    span: Span::new(m_lo, p.prev_end.max(m_lo)),
                                    kind: ModKind::Pub(v),
                                });
                            }
                            "async" => {
                                let sp = p.span();
                                p.bump();
                                mods.push(ModifierNode {
                                    span: sp,
                                    kind: ModKind::Async,
                                });
                            }
                            _ => break,
                        }
                    }
                    if matches!(p.tok(), Tok::LBracket) {
                        p.err(lo, "the bracket marker comes first — e.g. `[constructor] pub fn ..` (marker-before-visibility)");
                    }
                    if p.at_kw("fn") {
                        self.stage = TbStage::Method;
                        return Step::Push(Frame::Method(MethodFrame::new(
                            mods, lo.lo, true, true,
                        )));
                    }
                    if mods.iter().any(|m| matches!(m.kind, ModKind::Async)) {
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
                    if matches!(self.mode, BodyMode::HostStruct) {
                        p.err(
                            p.span(),
                            "host struct fields have no initializers —the host constructs the record",
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
                // the ladder's member tier: a failed method/field-type
                // fences IN its list; the body survives
                TbStage::Method => {
                    p.sync_stmt();
                    let (before, mark) = (self.before, self.dmark);
                    self.methods.push(MethodRef::Error(p.fence(before, mark)));
                    self.stage = TbStage::Members;
                    self.members_top(p)
                }
                TbStage::FieldTy => {
                    p.sync_stmt();
                    let (before, mark) = (self.before, self.dmark);
                    self.fields.push(FieldRef::Error(p.fence(before, mark)));
                    self.stage = TbStage::Members;
                    self.members_top(p)
                }
                // v1: a failed field initializer bails the whole body
                TbStage::FieldInit => Step::Pop(Done::Failed),
                _ => unreachable!(),
            },
            _ => unreachable!("type body receives methods, types, or expressions"),
        }
    }

    fn field_done(&mut self, p: &mut Parser, init: Option<rut_ast::ExprRef>) -> Step {
        let node = with_doc!(
            p,
            FieldDeclNode {
                span: Span::new(self.f_lo.lo, p.prev_end.max(self.f_lo.lo)),
                modifiers: std::mem::take(&mut self.f_mods),
                name: self.f_name.id,
                name_span: self.f_name.span,
                ty: self.f_ty.take().expect("field without a type"),
                init,
                doc: DocData::default(),
            }
        );
        self.fields.push(node.allocate(&mut p.ast));
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
    /// the member's first-token lo — the span law starts the decl there
    /// (the leading modifier when one is spelled, `fn` otherwise)
    lo: u32,
    /// the spelled modifiers, in source order
    modifiers: Vec<ModifierNode>,
    with_body: bool,
    allow_bounds: bool,
    stage: MeStage,
    name: Name,
    generics: Vec<IdentId>,
    pending: Option<IdentId>,
    bounds: Vec<(IdentId, TyRef)>,
    params: Option<Vec<ParamRef>>,
    ret: Option<TyRef>,
}

#[derive(Clone, Copy)]
enum MeStage {
    GenBound,
    Params,
    Ret,
    Body,
}

impl MethodFrame {
    pub(crate) fn new(
        modifiers: Vec<ModifierNode>,
        member_lo: u32,
        with_body: bool,
        allow_bounds: bool,
    ) -> Self {
        MethodFrame {
            lo: member_lo,
            modifiers,
            with_body,
            allow_bounds,
            stage: MeStage::Params,
            name: Name {
                id: IdentId(0),
                span: Span::new(0, 0),
            },
            generics: Vec::new(),
            pending: None,
            bounds: Vec::new(),
            params: None,
            ret: None,
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        p.bump(); // fn — the span already starts at the member's first token
        let Some(name) = p.expect_ident("a method name") else {
            return Step::Pop(Done::Failed);
        };
        self.name = name;
        if matches!(p.tok(), Tok::Lt) {
            let (gens, pending) = generic_params(p, self.allow_bounds, "method");
            self.generics = gens;
            if let Some(n) = pending {
                self.pending = Some(n.id);
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

    /// A `where` clause is gone: diagnose with the inline
    /// replacement and skip past the stray clause.
    fn stray_where(&mut self, p: &mut Parser) {
        if p.at_kw("where") {
            p.err_here(
                "`where` clauses are removed — write the bound inline: `fn m<T requires B>(..)`",
            );
            while !matches!(p.tok(), Tok::LBrace | Tok::Semi | Tok::Eof) {
                p.bump();
            }
        }
    }

    fn pop(&mut self, p: &mut Parser, body: Option<Handle<BlockNode>>) -> Step {
        let d = with_doc!(
            p,
            MethodDeclNode {
                span: p.span_from(self.lo),
                modifiers: std::mem::take(&mut self.modifiers),
                name: self.name.id,
                name_span: self.name.span,
                generics: std::mem::take(&mut self.generics),
                bounds: std::mem::take(&mut self.bounds),
                params: self.params.take().expect("method without params"),
                ret: self.ret.take(),
                body,
                doc: DocData::default(),
            }
        );
        Step::Pop(Done::Method(d.allocate(&mut p.ast)))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match (self.stage, d) {
            (MeStage::GenBound, Done::Ty(t)) => {
                let g = self.pending.take().expect("bound without a parameter");
                self.bounds.push((g, t));
                let pending =
                    generic_params_more(p, self.allow_bounds, "method", &mut self.generics);
                if let Some(n) = pending {
                    self.pending = Some(n.id);
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
    name: Name,
    generics: Vec<IdentId>,
    pending: Option<IdentId>,
    bounds: Vec<(IdentId, TyRef)>,
    params: Option<Vec<ParamRef>>,
    ret: Option<TyRef>,
    /// the bodiless spelling's `;` end (Decl mode) — the empty body's span
    semi_hi: u32,
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
            name: Name {
                id: IdentId(0),
                span: Span::new(0, 0),
            },
            generics: Vec::new(),
            pending: None,
            bounds: Vec::new(),
            params: None,
            ret: None,
            semi_hi: 0,
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
        let Some(name) = p.expect_ident("a function name") else {
            return Step::Pop(Done::Failed);
        };
        self.name = name;
        if matches!(p.tok(), Tok::Lt) {
            let (gens, pending) = generic_params(p, true, "fn");
            self.generics = gens;
            if let Some(n) = pending {
                self.pending = Some(n.id);
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

    /// The fn body — a stray `where` clause diagnoses and is skipped.
    /// In a DECLARATION file a bodiless signature (`fn f(..);`) is the
    /// legal spelling; a braced body is implementation and diagnoses.
    fn body(&mut self, p: &mut Parser) -> Step {
        if p.at_kw("where") {
            p.err_here(
                "`where` clauses are removed — write the bound inline: `fn f<T requires B>(..)`",
            );
            while !matches!(p.tok(), Tok::LBrace | Tok::Eof) {
                p.bump();
            }
        }
        if p.mode == Mode::Decl {
            if matches!(p.tok(), Tok::Semi) {
                self.semi_hi = p.span().hi;
                p.bump();
                return self.pop(p);
            }
            p.err(
                Span::new(self.lo, self.lo + 2),
                "implementation in a declaration file",
            );
        }
        self.stage = FnSStage::Body;
        Step::Push(Frame::Block(BlockFrame::lax(p, self.lo)))
    }

    /// The bodiless pop (Decl mode only): the fn node carries an EMPTY
    /// block (synthesized, no doc of its own).
    fn pop(&mut self, p: &mut Parser) -> Step {
        let at = self.semi_hi;
        let body = BlockNode {
            span: Span::new(at, at),
            stmts: Vec::new(),
            tail: None,
        }
        .alloc_plain(&mut p.ast);
        let f = with_doc!(
            p,
            FnNode {
                span: p.span_from(self.lo),
                vis: self.vis,
                is_async: self.is_async,
                entry: self.entry,
                name: self.name.id,
                name_span: self.name.span,
                generics: std::mem::take(&mut self.generics),
                params: self.params.take().expect("fn without params"),
                ret: self.ret.take(),
                bounds: std::mem::take(&mut self.bounds),
                body,
                doc: DocData::default(),
            }
        );
        Step::Pop(Done::Item(f.allocate(&mut p.ast)))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match (self.stage, d) {
            (FnSStage::GenBound, Done::Ty(t)) => {
                let g = self.pending.take().expect("bound without a parameter");
                self.bounds.push((g, t));
                let pending = generic_params_more(p, true, "fn", &mut self.generics);
                if let Some(n) = pending {
                    self.pending = Some(n.id);
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
                let f = with_doc!(
                    p,
                    FnNode {
                        span: p.span_from(self.lo),
                        vis: self.vis,
                        is_async: self.is_async,
                        entry: self.entry,
                        name: self.name.id,
                        name_span: self.name.span,
                        generics: std::mem::take(&mut self.generics),
                        params: self.params.take().expect("fn without params"),
                        ret: self.ret.take(),
                        bounds: std::mem::take(&mut self.bounds),
                        body,
                        doc: DocData::default(),
                    }
                );
                Step::Pop(Done::Item(f.allocate(&mut p.ast)))
            }
            (FnSStage::Body, Done::Failed) => Step::Pop(Done::Failed),
            _ => unreachable!("fn frame receives params, types, or a body"),
        }
    }
}

// ---- host/builtin surface declarations (.d.rut) ----

pub(crate) struct SurfaceFrame {
    linkage: Linkage,
    vis: Vis,
    lo: u32,
    stage: SuStage,
    is_async: bool,
    is_impl: bool,
    is_primitive: bool,
    name: Name,
    generics: Vec<IdentId>,
    methods: Vec<MethodRef>,
    params: Option<Vec<ParamRef>>,
    ret: Option<TyRef>,
}

#[derive(Clone, Copy)]
enum SuStage {
    Params,
    Ret,
    Members,
}

impl SurfaceFrame {
    pub(crate) fn new(linkage: Linkage, vis: Vis) -> Self {
        SurfaceFrame {
            linkage,
            vis,
            lo: 0,
            stage: SuStage::Params,
            is_async: false,
            is_impl: false,
            is_primitive: false,
            name: Name {
                id: IdentId(0),
                span: Span::new(0, 0),
            },
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
                "declaration keyword in an implementation file —`host`/`builtin` belong in a `.d.rut`",
            );
        }
        self.lo = p.bump().span.lo;
        match self.linkage {
            Linkage::Host => self.host_step(p),
            Linkage::Builtin { .. } => self.builtin_step(p),
        }
    }

    fn host_step(&mut self, p: &mut Parser) -> Step {
        match p.tok().clone() {
            Tok::Ident(k) if k == "async" => {
                p.bump();
                self.is_async = true;
                match p.tok().clone() {
                    Tok::Ident(k) if k == "fn" => self.host_fn_tail(p),
                    _ => {
                        let found = p.peek(0).describe();
                        p.err_here(format!("expected `fn` after `host async`, found {found}"));
                        Step::Pop(Done::Failed)
                    }
                }
            }
            Tok::Ident(k) if k == "fn" => self.host_fn_tail(p),
            Tok::Ident(k) if k == "struct" => {
                p.bump();
                let Some(name) = p.expect_ident("a struct name") else {
                    return Step::Pop(Done::Failed);
                };
                self.name = name;
                self.stage = SuStage::Members;
                Step::Push(Frame::TypeBody(TypeBodyFrame::new(BodyMode::HostStruct)))
            }
            Tok::Ident(k) if k == "class" => {
                p.err(
                    p.span(),
                    "`host class` is removed —declare `host fn`s and wrap native state in a rut `class` over `opaque`",
                );
                Step::Pop(Done::Failed)
            }
            _ => {
                let found = p.peek(0).describe();
                p.err_here(format!(
                    "expected `fn` or `struct` after `host`, found {found}"
                ));
                Step::Pop(Done::Failed)
            }
        }
    }

    /// The `fn` spelling after `host` (with or without the `async`
    /// marker): the name, the generic rejection, then the params.
    fn host_fn_tail(&mut self, p: &mut Parser) -> Step {
        p.bump();
        let Some(name) = p.expect_ident("a function name") else {
            return Step::Pop(Done::Failed);
        };
        self.name = name;
        if matches!(p.tok(), Tok::Lt) {
            p.err(
                p.span(),
                "host fn signatures are concrete —generic parameters cannot cross the boundary",
            );
            let (gens, pending) = generic_params(p, false, "host fn");
            self.generics = gens;
            debug_assert!(pending.is_none(), "rejected bounds never suspend");
        }
        self.stage = SuStage::Params;
        Step::Push(Frame::Params(ParamsFrame::new()))
    }

    fn builtin_step(&mut self, p: &mut Parser) -> Step {
        match p.tok().clone() {
            Tok::Ident(k) if k == "fn" => {
                p.bump();
                let Some(name) = p.expect_ident("a function name") else {
                    return Step::Pop(Done::Failed);
                };
                self.name = name;
                if matches!(p.tok(), Tok::Lt) {
                    let (gens, pending) = generic_params(p, false, "builtin fn");
                    self.generics = gens;
                    debug_assert!(pending.is_none(), "rejected bounds never suspend");
                }
                self.stage = SuStage::Params;
                Step::Push(Frame::Params(ParamsFrame::new()))
            }
            Tok::Ident(k) if k == "class" || k == "trait" => {
                if k == "trait" {
                    p.err(p.span(), "`builtin trait` is removed — engine contracts are spelled on types now: `Future`/`RunContext` are `builtin class` rows, and `Iterable`/`Disposal` are bracket markers (`[iterable] fn` / `[disposal] fn` in an inherent `impl`)");
                }
                p.bump();
                let Some(name) = p.expect_ident("a builtin type name") else {
                    return Step::Pop(Done::Failed);
                };
                self.name = name;
                if matches!(p.tok(), Tok::Lt) {
                    let owner = "builtin class";
                    let (gens, pending) = generic_params(p, false, owner);
                    self.generics = gens;
                    debug_assert!(pending.is_none(), "rejected bounds never suspend");
                }
                self.stage = SuStage::Members;
                p.expect(Tok::LBrace);
                self.members_top(p)
            }
            Tok::Ident(k) if k == "primitive" => {
                p.bump();
                let Some(name) = p.expect_ident("a primitive name") else {
                    return Step::Pop(Done::Failed);
                };
                self.name = name;
                self.is_primitive = true;
                if matches!(p.tok(), Tok::Lt) {
                    p.err(p.span(), "a primitive takes no generic arguments");
                    while !matches!(p.tok(), Tok::LBrace | Tok::Semi | Tok::Eof) {
                        p.bump();
                    }
                }
                self.stage = SuStage::Members;
                p.expect(Tok::LBrace);
                self.members_top(p)
            }
            Tok::Ident(k) if k == "impl" => {
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
                    "`builtin` spells its kind — `builtin primitive <name> { .. }`, `builtin class Name { .. }`, or `builtin impl <prim> { .. }` (`builtin trait` is removed — engine contracts are `builtin class` rows and bracket markers now)",
                );
                Step::Pop(Done::Failed)
            }
            _ => {
                let found = p.peek(0).describe();
                p.err_here(format!(
                    "expected `fn`, `primitive`, `class`, or `impl` after `builtin`, found {found}"
                ));
                Step::Pop(Done::Failed)
            }
        }
    }

    fn members_top(&mut self, p: &mut Parser) -> Step {
        loop {
            if p.eat_punct(Tok::RBrace) || p.at_eof() {
                let ambient = matches!(self.linkage, Linkage::Builtin { ambient: true });
                let item = if self.is_impl {
                    BuiltinImplNode {
                        span: p.span_from(self.lo),
                        vis: Vis::Self_,
                        prim: self.name.id,
                        name_span: self.name.span,
                        methods: std::mem::take(&mut self.methods),
                    }
                    .allocate(&mut p.ast)
                } else if self.is_primitive {
                    with_doc!(
                        p,
                        BuiltinPrimitiveNode {
                            span: p.span_from(self.lo),
                            ambient,
                            name: self.name.id,
                            name_span: self.name.span,
                            members: std::mem::take(&mut self.methods),
                            doc: DocData::default(),
                        }
                    )
                    .allocate(&mut p.ast)
                } else {
                    with_doc!(
                        p,
                        BuiltinTyNode {
                            span: p.span_from(self.lo),
                            vis: Vis::Self_,
                            ambient,
                            name: self.name.id,
                            name_span: self.name.span,
                            generics: std::mem::take(&mut self.generics),
                            members: std::mem::take(&mut self.methods),
                            doc: DocData::default(),
                        }
                    )
                    .allocate(&mut p.ast)
                };
                return Step::Pop(Done::Item(item));
            }
            let member_lo = p.span().lo;
            let mut mods: Vec<ModifierNode> = Vec::new();
            if p.at_kw("async") {
                let sp = p.span();
                p.bump();
                mods.push(ModifierNode {
                    span: sp,
                    kind: ModKind::Async,
                });
            }
            if p.at_kw("fn") {
                self.stage = SuStage::Members;
                return Step::Push(Frame::Method(MethodFrame::new(
                    mods, member_lo, false, false,
                )));
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
                let node = with_doc!(
                    p,
                    SurfaceStructNode {
                        span: p.span_from(self.lo),
                        vis: Vis::Self_,
                        name: self.name.id,
                        name_span: self.name.span,
                        fields,
                        doc: DocData::default(),
                    }
                );
                Step::Pop(Done::Item(node.allocate(&mut p.ast)))
            }
            (SuStage::Members, Done::Failed) => {
                p.sync_stmt();
                match self.linkage {
                    Linkage::Host => Step::Pop(Done::Failed),
                    Linkage::Builtin { .. } => self.members_top(p),
                }
            }
            (_, d) => unreachable!("surface frame received the wrong child: {d:?}"),
        }
    }

    fn pop_fn(&mut self, p: &mut Parser) -> Step {
        let node = with_doc!(
            p,
            SurfaceFnNode {
                span: p.span_from(self.lo),
                vis: self.vis,
                linkage: self.linkage,
                is_async: self.is_async,
                name: self.name.id,
                name_span: self.name.span,
                generics: std::mem::take(&mut self.generics),
                params: self.params.take().expect("surface fn without params"),
                ret: self.ret.take(),
                doc: DocData::default(),
            }
        );
        Step::Pop(Done::Item(node.allocate(&mut p.ast)))
    }
}

// ---- parameter list ----

pub(crate) struct ParamsFrame {
    out: Vec<ParamRef>,
    stage: PsStage,
    /// the parameter under construction
    p_lo: Span,
    p_mut: bool,
    p_name: Name,
    /// param start + diag watermark (the fence's extent)
    before: usize,
    dmark: usize,
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
            p_name: Name {
                id: IdentId(0),
                span: Span::new(0, 0),
            },
            before: 0,
            dmark: 0,
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        if p.expect(Tok::LParen).is_none() {
            return Step::Pop(Done::Failed);
        }
        self.list_top(p)
    }

    /// resync inside a param list: to the `,`, the `)`, or EOF
    fn sync_param(p: &mut Parser) {
        while !matches!(p.tok(), Tok::Comma | Tok::RParen | Tok::Eof) {
            p.bump();
        }
    }

    fn list_top(&mut self, p: &mut Parser) -> Step {
        loop {
            if p.eat_punct(Tok::RParen) {
                return Step::Pop(Done::Members(std::mem::take(&mut self.out)));
            }
            let (before, dmark) = (p.pos, p.mark());
            self.before = before;
            self.dmark = dmark;
            let lo = p.span();
            let mut is_mut = false;
            if p.at_kw("mut") && p.at_kw2("self") {
                is_mut = true;
                p.bump();
            }
            if p.at_kw("self") {
                let sp = p.bump().span;
                let node = SelfParamNode {
                    span: Span::new(lo.lo, sp.hi.max(lo.lo)),
                    is_mut,
                }
                .allocate(&mut p.ast);
                self.out.push(node);
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
                // the ladder's param tier: a broken param fences IN the
                // list — the signature survives with its hole
                Self::sync_param(p);
                self.out.push(ParamRef::Error(p.fence(before, dmark)));
                if !p.eat_punct(Tok::Comma) {
                    p.expect(Tok::RParen);
                    return Step::Pop(Done::Members(std::mem::take(&mut self.out)));
                }
                continue;
            };
            self.p_lo = lo;
            self.p_mut = is_mut;
            self.p_name = name;
            if p.eat_punct(Tok::Colon) {
                self.stage = PsStage::Ty;
                return Step::Push(Frame::Type(TypeFrame::new(p)));
            }
            let node = ParamNode {
                span: Span::new(lo.lo, p.prev_end.max(lo.lo)),
                is_mut,
                name: name.id,
                name_span: name.span,
                ty: None,
            }
            .allocate(&mut p.ast);
            self.out.push(node);
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
                let node = ParamNode {
                    span: Span::new(lo.lo, p.prev_end.max(lo.lo)),
                    is_mut,
                    name: name.id,
                    name_span: name.span,
                    ty: Some(t),
                }
                .allocate(&mut p.ast);
                self.out.push(node);
                if !p.eat_punct(Tok::Comma) {
                    p.expect(Tok::RParen);
                    return Step::Pop(Done::Members(std::mem::take(&mut self.out)));
                }
                self.list_top(p)
            }
            Done::Failed => {
                // the failed param's TYPE: fence in place, continue the
                // list (the ladder's param tier)
                Self::sync_param(p);
                let (before, mark) = (self.before, self.dmark);
                self.out.push(ParamRef::Error(p.fence(before, mark)));
                self.stage = PsStage::List;
                if !p.eat_punct(Tok::Comma) {
                    p.expect(Tok::RParen);
                    return Step::Pop(Done::Members(std::mem::take(&mut self.out)));
                }
                self.list_top(p)
            }
            _ => unreachable!("params frame receives types"),
        }
    }
}
