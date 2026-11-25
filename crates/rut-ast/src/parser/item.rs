//! Declarations: items, pub visibility, use statements, type
//! bodies, fn/method signatures, surface stubs — as frames. The
//! grammar logic is the frame machine's own; construction is
//! ALLOCATE-COMPLETE over the capsule store (every node whole, docs
//! grafted at allocation, exact spans), and every member list fences
//! its broken regions as ErrorNodes beside the survivors.

use crate::{
    AliasNode, BlockNode, BuiltinImplNode, BuiltinPrimitiveNode, BuiltinTyNode, ClassNode, DocData,
    EnumNode, FieldDeclNode, FieldRef, FnNode, Handle, IdentId, ImplNode, InterfaceNode, Linkage,
    MemberRef, MethodDeclNode, MethodRef, ModDeclNode, ModKind, ModifierNode, ModuleLetNode,
    ParamNode, ParamRef, SelfParamNode, StructNode, SurfaceFnNode, SurfaceStructNode, TyRef,
    UseNode,
};
use rut_lexer::span::Span;
use rut_lexer::token::Tok;

use crate::parser::expr::{ExprFrame, ExprMode};
use crate::parser::frame::{pub_scope, Done, Frame, PubFrame, Step};
use crate::parser::stmt::BlockFrame;
use crate::parser::ty::TypeFrame;
use crate::parser::{with_doc, Mode, Name, Parser};

// ---- item dispatch ----

/// the item-span law's lo: the first modifier's own lo when one is
/// spelled, the keyword's otherwise (v3.1: an item's span starts at
/// its FIRST source token)
pub(crate) fn item_lo(mods: &[ModifierNode], kw_lo: u32) -> u32 {
    mods.first().map(|m| m.span.lo).unwrap_or(kw_lo)
}

/// append one modifier node — the span covers exactly the modifier's
/// spelled tokens
fn with_mod(mut mods: Vec<ModifierNode>, span: Span, kind: ModKind) -> Vec<ModifierNode> {
    mods.push(ModifierNode { span, kind });
    mods
}

pub(crate) fn classify_item(p: &mut Parser) -> Option<Frame> {
    let sp = p.span();
    match p.tok().clone() {
        Tok::Ident(kw) => match kw.as_str() {
            "use" => Some(Frame::Use(UseFrame::new(Vec::new()))),
            "mod" => Some(Frame::ModDecl(ModDeclFrame::new(Vec::new()))),
            "pub" => Some(Frame::Pub(PubFrame::new())),
            "type" => Some(Frame::Alias(TypeAliasFrame::new(Vec::new()))),
            "let" => Some(Frame::ModuleLet(ModuleLetFrame::new(Vec::new()))),
            "enum" => Some(Frame::Enum(EnumFrame::new(Vec::new()))),
            "struct" => Some(Frame::Struct(TyDeclFrame::new(false, Vec::new()))),
            "class" => Some(Frame::Class(TyDeclFrame::new(true, Vec::new()))),
            "interface" => Some(Frame::Interface(InterfaceFrame::new(Vec::new()))),
            "impl" => Some(Frame::Impl(ImplFrame::new())),
            // `async fn` — the async declaration; the keyword rides as
            // the item's first modifier
            "async" if p.at_kw2("fn") => {
                p.bump();
                Some(Frame::Fn(FnFrame::new(with_mod(
                    Vec::new(),
                    sp,
                    ModKind::Async,
                ))))
            }
            "fn" => Some(Frame::Fn(FnFrame::new(Vec::new()))),
            // `entry fn` — the host-callable surface:
            // contextual; only special directly before `fn`
            "entry" if p.at_kw2("fn") => {
                p.bump();
                Some(Frame::Fn(FnFrame::new(with_mod(
                    Vec::new(),
                    sp,
                    ModKind::Entry,
                ))))
            }
            // `host` — the one native linkage left: the embedding Rust.
            // `extern` is removed — the loader + `entry fn`
            // cover it. The keyword is still ahead — the frame records
            // it as the item's own leading modifier
            "host" => Some(Frame::Surface(SurfaceFrame::new(Linkage::Host, Vec::new()))),
            // `prelude builtin` — the AMBIENT engine surface (core only,
            // compiler-lowered). `prelude` is contextual: a legal
            // identifier everywhere except directly before `builtin`.
            // The `prelude` half is recorded here; the frame widens the
            // modifier over the `builtin` token
            "prelude" if p.at_kw2("builtin") => {
                p.bump();
                Some(Frame::Surface(SurfaceFrame::new(
                    Linkage::Builtin { ambient: true },
                    with_mod(Vec::new(), sp, ModKind::Builtin { ambient: true }),
                )))
            }
            // statement keywords at module scope
            "if" | "while" | "for" | "return" | "when" | "break" | "continue" | "await" => {
                p.err(
                    sp,
                    "statements are not allowed at module scope — modules contain declarations only",
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

/// v1's parse_export_item inner dispatch — the `pub` frame records its
/// modifier (span covering the `(..)` scope) and hands the chain down;
/// every arm moves it into its item frame
pub(crate) fn classify_pub(p: &mut Parser, mods: Vec<ModifierNode>) -> Option<Frame> {
    match p.tok().clone() {
        Tok::Ident(kw) => match kw.as_str() {
            "let" => Some(Frame::ModuleLet(ModuleLetFrame::new(mods))),
            "mod" => Some(Frame::ModDecl(ModDeclFrame::new(mods))),
            // `pub use` — the re-export: the imported names join the
            // module's public surface (a private `use` imports only)
            "use" => Some(Frame::Use(UseFrame::new(mods))),
            "type" => Some(Frame::Alias(TypeAliasFrame::new(mods))),
            "enum" => Some(Frame::Enum(EnumFrame::new(mods))),
            "struct" => Some(Frame::Struct(TyDeclFrame::new(false, mods))),
            "class" => Some(Frame::Class(TyDeclFrame::new(true, mods))),
            "interface" => Some(Frame::Interface(InterfaceFrame::new(mods))),
            // `pub async fn` — the async keyword rides as the chain's
            // second modifier (`entry` does not combine with `pub` —
            // the documented law; classify_item owns the bare forms)
            "async" if p.at_kw2("fn") => {
                let sp = p.span();
                p.bump();
                Some(Frame::Fn(FnFrame::new(with_mod(mods, sp, ModKind::Async))))
            }
            "fn" => Some(Frame::Fn(FnFrame::new(mods))),
            "host" => Some(Frame::Surface(SurfaceFrame::new(Linkage::Host, mods))),
            // `pub builtin` — the IMPORT-GATED engine surface spelling:
            // the decl resolves only through `use <pkg>::{ .. }` (the
            // ambient twin is `prelude builtin`). The `builtin` keyword
            // is still ahead — the frame records it
            "builtin" => Some(Frame::Surface(SurfaceFrame::new(
                Linkage::Builtin { ambient: false },
                mods,
            ))),
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
/// The span starts at the item's FIRST token (the leading modifier
/// when one is spelled, `use` otherwise).
pub(crate) struct UseFrame {
    modifiers: Vec<ModifierNode>,
    lo: u32,
    path: Vec<crate::Name>,
    names: Vec<crate::Name>,
    /// the `{..}` group's span (braces included) — stamped only when
    /// the group's closing brace landed
    group: Option<Span>,
    /// the glob leaf's span — the `*` of a flat `pkg::*` walk or a
    /// brace list's star
    glob: Option<Span>,
}

impl UseFrame {
    pub(crate) fn new(modifiers: Vec<ModifierNode>) -> Self {
        UseFrame {
            modifiers,
            lo: 0,
            path: Vec::new(),
            names: Vec::new(),
            group: None,
            glob: None,
        }
    }

    fn push_seg(&mut self, n: Name) {
        self.path.push(crate::Name {
            ident: n.id,
            span: n.span,
        });
    }
    fn push_name(&mut self, n: Name) {
        self.names.push(crate::Name {
            ident: n.id,
            span: n.span,
        });
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = item_lo(&self.modifiers, p.span().lo);
        p.bump(); // use — the lo already covers a leading modifier
        let Some(pkg) = p.expect_ident("a package name") else {
            p.sync_stmt();
            // the incomplete rung: `use` alone at the typing edge — the
            // empty-path use keeps a real item at the cursor
            return self.partial(p);
        };
        self.push_seg(pkg);
        if !p.eat_punct(Tok::Colon2) {
            p.err_here(
                "expected `::` after the package name — use paths spell `use <pkg>::{A, B};`",
            );
            p.sync_stmt();
            // the incomplete rung: the path parsed, the leaf never
            // materialized — the partial use keeps a real node at the
            // cursor instead of a fence
            return self.partial(p);
        }
        if matches!(p.tok(), Tok::LBrace) {
            let brace_lo = p.span().lo;
            p.bump(); // {
            if !self.brace_names(p) {
                p.sync_stmt();
                return self.partial(p);
            }
            self.group = Some(Span::new(brace_lo, p.prev_end));
        } else {
            // the module path: `ident (:: ident)*`, then either the
            // brace leaf or the `::ident` single-name leaf
            loop {
                if matches!(p.tok(), Tok::Star) {
                    // the glob leaf — the star terminates the walk;
                    // nothing pops (the leaf is the star itself)
                    self.glob = Some(p.span());
                    p.bump();
                    break;
                }
                let Some(seg) = p.expect_ident("a path segment") else {
                    p.sync_stmt();
                    // the incomplete rung: the path continues (`use
                    // pkg::` at the typing edge) — the partial use
                    return self.partial(p);
                };
                self.push_seg(seg);
                if !p.eat_punct(Tok::Colon2) {
                    break;
                }
                if matches!(p.tok(), Tok::LBrace) {
                    let brace_lo = p.span().lo;
                    p.bump(); // {
                    if !self.brace_names(p) {
                        p.sync_stmt();
                        return self.partial(p);
                    }
                    self.group = Some(Span::new(brace_lo, p.prev_end));
                    break;
                }
            }
        }
        if self.names.is_empty() && self.glob.is_none() {
            // no brace leaf: the single-name leaf is the LAST path
            // segment — `use pkg::A;` (the flat form) and
            // `use pkg::A::B::C;` pop it; `use pkg::A::B;` spells a mod
            // path with nothing at the leaf. A glob leaf never runs
            // this — the star IS the leaf (`use pkg::A::B::*;` imports
            // the walked path's surface, nothing pops, no error).
            match self.path.len() {
                0 | 1 => {} // `use pkg::{};` — names stay empty, as today
                3 => p.err_here(
                    "`use pkg::A::B;` imports nothing — spell a name at the leaf (`::{ C }` or `::C`)",
                ),
                _ => {
                    let seg = self.path.pop().expect("path depth checked above");
                    self.names.push(seg);
                }
            }
        }
        p.expect(Tok::Semi);
        // the head's span BEFORE the take — name_span keys on it
        let name_span = self.path.first().map(|n| n.span).unwrap_or(Span::new(0, 0));
        let node = with_doc!(
            p,
            UseNode {
                span: p.span_from(self.lo),
                modifiers: std::mem::take(&mut self.modifiers),
                path: std::mem::take(&mut self.path),
                name_span,
                names: std::mem::take(&mut self.names),
                group: self.group.take(),
                glob: self.glob.take(),
                doc: DocData::default(),
            }
        );
        Step::Pop(Done::Item(node.allocate(&mut p.ast)))
    }

    /// the incomplete rung: the path parsed but the leaf never
    /// materialized — emit the partial use (absent group, no names)
    /// instead of fencing; the span runs to the cursor's token (the
    /// sync point — the region the typing continues in) so the
    /// cursor's walk lands on a real node, and the diag rows stay
    fn partial(&mut self, p: &mut Parser) -> Step {
        let name_span = self.path.first().map(|n| n.span).unwrap_or(Span::new(0, 0));
        let end = p.span().lo.max(self.lo);
        let node = with_doc!(
            p,
            UseNode {
                span: Span::new(self.lo, end),
                modifiers: std::mem::take(&mut self.modifiers),
                path: std::mem::take(&mut self.path),
                name_span,
                names: std::mem::take(&mut self.names),
                group: None,
                glob: self.glob.take(),
                doc: DocData::default(),
            }
        );
        Step::Pop(Done::Item(node.allocate(&mut p.ast)))
    }

    /// the brace leaf — `::{A, B}` — one or more idents, plus the
    /// brace star (`{ A, * }` / `{ *, A }` — the glob rides beside the
    /// names). `false` when the list failed to parse (the caller
    /// resyncs).
    fn brace_names(&mut self, p: &mut Parser) -> bool {
        loop {
            if p.eat_punct(Tok::RBrace) {
                return true;
            }
            if matches!(p.tok(), Tok::Star) {
                self.glob = Some(p.span());
                p.bump();
            } else {
                let Some(n) = p.expect_ident("a used name") else {
                    return false;
                };
                self.push_name(n);
            }
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
/// sibling `NAME/mod.rut` (a `.d.rut` tree: `NAME/mod.d.rut`); the
/// loader mounts it. ONE mod grammar, both modes — the inline
/// `mod NAME { .. }` block form is gone from the grammar, so a stray
/// block is the ordinary ``expected `;` `` error, declaration file or
/// implementation file alike.
pub(crate) struct ModDeclFrame {
    modifiers: Vec<ModifierNode>,
    lo: u32,
    name: Name,
}

impl ModDeclFrame {
    pub(crate) fn new(modifiers: Vec<ModifierNode>) -> Self {
        ModDeclFrame {
            modifiers,
            lo: 0,
            name: Name {
                id: IdentId(0),
                span: Span::new(0, 0),
            },
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = item_lo(&self.modifiers, p.span().lo);
        p.bump(); // mod
        let Some(name) = p.expect_ident("a module name") else {
            return Step::Pop(Done::Failed);
        };
        self.name = name;
        p.expect(Tok::Semi);
        let node = with_doc!(
            p,
            ModDeclNode {
                span: p.span_from(self.lo),
                modifiers: std::mem::take(&mut self.modifiers),
                name: crate::Name {
                    ident: self.name.id,
                    span: self.name.span,
                },
                doc: DocData::default(),
            }
        );
        Step::Pop(Done::Item(node.allocate(&mut p.ast)))
    }

    pub(crate) fn absorb(&mut self, _p: &mut Parser, _d: Done) -> Step {
        unreachable!("mod decl frame pushes no children")
    }
}

// ---- type alias ----

/// `pub(..)? type Name = Target;`: a transparent alias, or a
/// `Target` spelled `A | B` for the bound-only union. Aliases are
/// NON-GENERIC — `type Name<K, ..>` is a parse error.
pub(crate) struct TypeAliasFrame {
    modifiers: Vec<ModifierNode>,
    lo: u32,
    name: Name,
}

impl TypeAliasFrame {
    pub(crate) fn new(modifiers: Vec<ModifierNode>) -> Self {
        TypeAliasFrame {
            modifiers,
            lo: 0,
            name: Name {
                id: IdentId(0),
                span: Span::new(0, 0),
            },
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = item_lo(&self.modifiers, p.span().lo);
        p.bump(); // type
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
                        modifiers: std::mem::take(&mut self.modifiers),
                        name: crate::Name {
                            ident: self.name.id,
                            span: self.name.span,
                        },
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
    modifiers: Vec<ModifierNode>,
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
    pub(crate) fn new(modifiers: Vec<ModifierNode>) -> Self {
        ModuleLetFrame {
            modifiers,
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
        self.lo = item_lo(&self.modifiers, p.span().lo);
        p.bump(); // let
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
                        modifiers: std::mem::take(&mut self.modifiers),
                        name: crate::Name {
                            ident: self.name.id,
                            span: self.name.span,
                        },
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
    modifiers: Vec<ModifierNode>,
    lo: u32,
    name: Name,
    /// the body's left boundary — the `{` (hi recorded at the pop)
    body_lo: u32,
    members: Vec<MemberRef>,
}

impl EnumFrame {
    pub(crate) fn new(modifiers: Vec<ModifierNode>) -> Self {
        EnumFrame {
            modifiers,
            lo: 0,
            name: Name {
                id: IdentId(0),
                span: Span::new(0, 0),
            },
            body_lo: 0,
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
        self.lo = item_lo(&self.modifiers, p.span().lo);
        p.bump(); // enum
        let Some(name) = p.expect_ident("an enum name") else {
            return Step::Pop(Done::Failed);
        };
        self.name = name;
        // the body's left boundary — lo at the `{`, hi at the pop's `}`
        self.body_lo = p.expect(Tok::LBrace).map_or(p.prev_end, |s| s.lo);
        loop {
            if p.eat_punct(Tok::RBrace) {
                break;
            }
            let (before, dmark) = (p.pos, p.mark());
            match p.expect_ident("an enum member") {
                Some(m) => {
                    let node = with_doc!(
                        p,
                        crate::EnumMemberNode {
                            span: m.span,
                            name: crate::Name {
                                ident: m.id,
                                span: m.span,
                            },
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
                // braces included — the `{`..`}` region the loop just
                // consumed (a missing brace degenerates to the cut)
                body_span: p.span_from(self.body_lo),
                modifiers: std::mem::take(&mut self.modifiers),
                name: crate::Name {
                    ident: self.name.id,
                    span: self.name.span,
                },
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
    modifiers: Vec<ModifierNode>,
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
    pub(crate) fn new(is_class: bool, modifiers: Vec<ModifierNode>) -> Self {
        TyDeclFrame {
            is_class,
            modifiers,
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
        self.lo = item_lo(&self.modifiers, p.span().lo);
        p.bump(); // struct | class
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
                // own chain above) and no name token exists either: the
                // recorded name span degenerates empty (consumers fall
                // back to the field's own extent)
                let field = FieldDeclNode {
                    span: ty_span,
                    modifiers: Vec::new(),
                    name: crate::Name {
                        ident: inner,
                        span: Span::new(ty_span.lo, ty_span.lo),
                    },
                    ty: t,
                    init: None,
                    doc: DocData::default(),
                }
                .allocate(&mut p.ast);
                let node = with_doc!(
                    p,
                    ClassNode {
                        span: p.span_from(self.lo),
                        // no braces on the newtype spelling — the
                        // boundary degenerates to the terminator cut
                        body_span: p.span_from(self.lo),
                        modifiers: std::mem::take(&mut self.modifiers),
                        name: crate::Name {
                            ident: self.name.id,
                            span: self.name.span,
                        },
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
            (TdStage::Body, Done::Body(fields, methods, body_span)) => {
                let item = if self.is_class {
                    with_doc!(
                        p,
                        ClassNode {
                            span: p.span_from(self.lo),
                            body_span,
                            modifiers: std::mem::take(&mut self.modifiers),
                            name: crate::Name {
                                ident: self.name.id,
                                span: self.name.span,
                            },
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
                            body_span,
                            modifiers: std::mem::take(&mut self.modifiers),
                            name: crate::Name {
                                ident: self.name.id,
                                span: self.name.span,
                            },
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
    modifiers: Vec<ModifierNode>,
    lo: u32,
    name: Name,
    generics: Vec<IdentId>,
}

impl InterfaceFrame {
    pub(crate) fn new(modifiers: Vec<ModifierNode>) -> Self {
        InterfaceFrame {
            modifiers,
            lo: 0,
            name: Name {
                id: IdentId(0),
                span: Span::new(0, 0),
            },
            generics: Vec::new(),
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        self.lo = item_lo(&self.modifiers, p.span().lo);
        p.bump(); // interface
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
            Done::Body(_, methods, body_span) => {
                let node = with_doc!(
                    p,
                    InterfaceNode {
                        span: p.span_from(self.lo),
                        body_span,
                        modifiers: std::mem::take(&mut self.modifiers),
                        name: crate::Name {
                            ident: self.name.id,
                            span: self.name.span,
                        },
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
                "implementation in a declaration file — `impl` blocks live in `.rut`",
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
            (ImStage::Methods, Done::Body(_, methods, _body)) => {
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
    /// the body's left boundary — the `{` (hi recorded at the pop);
    /// the frame's own boundary bookkeeping, braces byte-exact
    body_lo: u32,
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
            body_lo: 0,
        }
    }

    fn mode_noun(&self) -> &'static str {
        match self.mode {
            BodyMode::Interface => "interfaces declare method signatures",
            BodyMode::Inherent => "inherent impl blocks declare methods",
            BodyMode::HostStruct => "host structs declare fields",
            BodyMode::Class { .. } => "type bodies declare fields — methods live in `impl` blocks",
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        let strict = matches!(
            self.mode,
            BodyMode::Class { .. } | BodyMode::HostStruct | BodyMode::Interface
        );
        let brace = p.expect(Tok::LBrace);
        if strict && brace.is_none() {
            return Step::Pop(Done::Failed);
        }
        // the body's left boundary — lo at the `{` (a missing brace in
        // the recovering mode degenerates to the last consumed token)
        self.body_lo = brace.map_or(p.prev_end, |s| s.lo);
        self.stage = TbStage::Members;
        self.members_top(p)
    }

    /// The bracket marker's PARSE half: `[word]` before a member's
    /// modifiers — any contextual ident; the CHECKER validates the
    /// closed marker set. The modifier's SPAN (brackets included)
    /// rides along — it is a ModifierNode's own span.
    fn parse_member_marker(p: &mut Parser) -> Option<crate::Name> {
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
        Some(crate::Name {
            ident: word,
            span: Span::new(lo, p.prev_end.max(lo)),
        })
    }

    fn members_top(&mut self, p: &mut Parser) -> Step {
        loop {
            if p.eat_punct(Tok::RBrace) || p.at_eof() {
                let fields = std::mem::take(&mut self.fields);
                let methods = std::mem::take(&mut self.methods);
                // hi at the `}` — braces included, byte-exact
                let body = p.span_from(self.body_lo);
                return Step::Pop(Done::Body(fields, methods, body));
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
                                    "host struct fields carry no visibility — all fields are public",
                                );
                            }
                            "static" | "async" => {
                                p.bump();
                                p.err(lo, format!("host struct fields carry no modifiers — `{m}` is not declarable here"));
                            }
                            _ => break,
                        }
                    }
                    match p.tok().clone() {
                        // parse (bodiless) to stay in sync; the methods are
                        // dropped — SurfaceStruct keeps fields only
                        Tok::Ident(kw) if kw == "fn" => {
                            p.err(lo, "host structs declare fields only — methods live in rut wrapper classes");
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
                                        "structs have no field visibility — all fields are public",
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
                                p.err(lo, "unexpected `async` — type bodies declare fields only");
                            }
                            _ => break,
                        }
                    }
                    match p.tok().clone() {
                        Tok::Ident(kw) if kw == "fn" => {
                            if p.mode != Mode::Decl {
                                p.err(
                                    lo,
                                    "methods live in `impl` blocks — type bodies declare fields only",
                                );
                            }
                            if mods.iter().any(|m| matches!(m.kind, ModKind::Static)) {
                                p.err(
                                    lo,
                                    "there is no `static fn` — a method without `self` IS a class method",
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
                                p.err(lo, "interface members carry no `pub` — the interface's visibility rules");
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
                    if let Some(word) = marker {
                        p.err(lo, format!(
                            "`[{}]` marks an inherent impl member — engine contracts are declared on `impl Type {{ .. }}` blocks, never an interface",
                            p.ast.name(word.ident)
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
                    p.err_here(format!("{what} — expected `fn`, found {found}"));
                    p.sync_stmt();
                }
                BodyMode::Inherent => {
                    // `impl T { .. }` — inherent methods: the bracket
                    // marker FIRST, then `pub`/`async`
                    let lo = p.span();
                    let mut mods: Vec<ModifierNode> = Vec::new();
                    if let Some(word) = Self::parse_member_marker(p) {
                        mods.push(ModifierNode {
                            span: word.span,
                            kind: ModKind::Marker(word.ident),
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
                    p.err_here(format!("{what} — expected `fn`, found {found}"));
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
                            "host struct fields have no initializers — the host constructs the record",
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

    fn field_done(&mut self, p: &mut Parser, init: Option<crate::ExprRef>) -> Step {
        let node = with_doc!(
            p,
            FieldDeclNode {
                span: Span::new(self.f_lo.lo, p.prev_end.max(self.f_lo.lo)),
                modifiers: std::mem::take(&mut self.f_mods),
                name: crate::Name {
                    ident: self.f_name.id,
                    span: self.f_name.span,
                },
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
        if self.with_body && p.mode == Mode::Impl {
            self.stage = MeStage::Body;
            Step::Push(Frame::Block(BlockFrame::lax(p, self.lo)))
        } else {
            p.expect(Tok::Semi);
            self.pop(p, None)
        }
    }

    fn pop(&mut self, p: &mut Parser, body: Option<Handle<BlockNode>>) -> Step {
        let d = with_doc!(
            p,
            MethodDeclNode {
                span: p.span_from(self.lo),
                modifiers: std::mem::take(&mut self.modifiers),
                name: crate::Name {
                    ident: self.name.id,
                    span: self.name.span,
                },
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
            // the incomplete rung: the return type is being typed —
            // the method survives with the ABSENT ret
            (MeStage::Ret, Done::Failed) => self.finish_sig(p),
            (MeStage::Body, Done::Block(b)) => self.pop(p, Some(b)),
            (MeStage::Body, Done::Failed) if matches!(p.tok(), Tok::Eof) => {
                // the incomplete rung at EOF: the body is being typed —
                // the bodiless method keeps a real member at the cursor
                self.pop(p, None)
            }
            (MeStage::Body, Done::Failed) => Step::Pop(Done::Failed),
            _ => unreachable!("method frame receives params, types, or a body"),
        }
    }
}

// ---- free functions ----

pub(crate) struct FnFrame {
    /// the item's first-token lo — the span law starts the decl there
    /// (the leading modifier when one is spelled, `fn` otherwise)
    lo: u32,
    /// the spelled modifiers, in source order — the node's `vis()`/
    /// `is_async()`/`entry()` derive from them
    modifiers: Vec<ModifierNode>,
    stage: FnSStage,
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
    pub(crate) fn new(modifiers: Vec<ModifierNode>) -> Self {
        FnFrame {
            lo: 0,
            modifiers,
            stage: FnSStage::Params,
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
        self.lo = item_lo(&self.modifiers, p.span().lo);
        p.bump(); // fn — the lo already covers a leading modifier
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

    /// The fn body. In a DECLARATION file a bodiless signature
    /// (`fn f(..);`) is the legal spelling; a braced body is
    /// implementation and diagnoses.
    fn body(&mut self, p: &mut Parser) -> Step {
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
            // the `;` closed the signature — the synthesized body is
            // not an open construct
            closed: true,
            stmts: Vec::new(),
            tail: None,
        }
        .alloc_plain(&mut p.ast);
        let f = with_doc!(
            p,
            FnNode {
                span: p.span_from(self.lo),
                modifiers: std::mem::take(&mut self.modifiers),
                name: crate::Name {
                    ident: self.name.id,
                    span: self.name.span,
                },
                generics: std::mem::take(&mut self.generics),
                params: self.params.take().expect("fn without params"),
                ret: self.ret.take(),
                bounds: std::mem::take(&mut self.bounds),
                body,
                closed: true,
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
            // the incomplete rung: the return type is being typed —
            // the fn survives with the ABSENT ret (a normal absent
            // slot); the body stage decides the rest
            (FnSStage::Ret, Done::Failed) => self.body(p),
            (FnSStage::Body, Done::Block(body)) => {
                let f = with_doc!(
                    p,
                    FnNode {
                        span: p.span_from(self.lo),
                        modifiers: std::mem::take(&mut self.modifiers),
                        name: crate::Name {
                            ident: self.name.id,
                            span: self.name.span,
                        },
                        generics: std::mem::take(&mut self.generics),
                        params: self.params.take().expect("fn without params"),
                        ret: self.ret.take(),
                        bounds: std::mem::take(&mut self.bounds),
                        body,
                        closed: body.get(&p.ast).closed,
                        doc: DocData::default(),
                    }
                );
                Step::Pop(Done::Item(f.allocate(&mut p.ast)))
            }
            (FnSStage::Body, Done::Failed) if matches!(p.tok(), Tok::Eof) => {
                // the incomplete rung at EOF: the body is being typed —
                // the synthesized bodiless fn (the decl-mode
                // precedent) keeps a real item at the cursor
                self.pop(p)
            }
            (FnSStage::Body, Done::Failed) => Step::Pop(Done::Failed),
            _ => unreachable!("fn frame receives params, types, or a body"),
        }
    }
}

// ---- host/builtin surface declarations (.d.rut) ----

pub(crate) struct SurfaceFrame {
    /// the dispatch linkage (the modifier chain's derived view stays
    /// the node's truth — the frame needs it before the members parse)
    linkage: Linkage,
    /// the spelled modifiers, in source order — the node's `vis()`/
    /// `linkage()`/`is_async()`/`ambient()` derive from them
    modifiers: Vec<ModifierNode>,
    lo: u32,
    stage: SuStage,
    is_impl: bool,
    is_primitive: bool,
    name: Name,
    generics: Vec<IdentId>,
    methods: Vec<MethodRef>,
    /// the brace body's left boundary — the `{` (hi recorded at the
    /// pop); the brace-bodied surface rows' `body_span`
    body_lo: u32,
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
    pub(crate) fn new(linkage: Linkage, modifiers: Vec<ModifierNode>) -> Self {
        SurfaceFrame {
            linkage,
            modifiers,
            lo: 0,
            stage: SuStage::Params,
            is_impl: false,
            is_primitive: false,
            name: Name {
                id: IdentId(0),
                span: Span::new(0, 0),
            },
            generics: Vec::new(),
            methods: Vec::new(),
            body_lo: 0,
            params: None,
            ret: None,
        }
    }

    /// the ambient gate of this frame's builtin linkage
    fn ambient(&self) -> bool {
        matches!(self.linkage, Linkage::Builtin { ambient: true })
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        let sp = p.span();
        if p.mode == Mode::Impl {
            p.err(
                sp,
                "declaration keyword in an implementation file — `host`/`builtin` belong in a `.d.rut`",
            );
        }
        // the linkage keyword — `host`, or the `builtin` of a builtin
        // spelling — is the item's own leading modifier. `prelude`'s
        // half is already recorded (classify bumped it): the modifier
        // WIDENS over the `builtin` token, one node per spelling; the
        // bare and `pub builtin` keywords are still ahead and land
        // here. Diagnostics aimed at the keyword keep its own span.
        match p.tok().clone() {
            Tok::Ident(k) if k == "host" => {
                self.modifiers.push(ModifierNode {
                    span: sp,
                    kind: ModKind::Host,
                });
                p.bump();
            }
            Tok::Ident(k) if k == "builtin" => {
                match self.modifiers.last_mut() {
                    // `prelude builtin` — one modifier covering both tokens
                    Some(m) if matches!(m.kind, ModKind::Builtin { .. }) => m.span.hi = sp.hi,
                    _ => self.modifiers.push(ModifierNode {
                        span: sp,
                        kind: ModKind::Builtin {
                            ambient: self.ambient(),
                        },
                    }),
                }
                p.bump();
            }
            _ => {}
        }
        self.lo = item_lo(&self.modifiers, sp.lo);
        match self.linkage {
            Linkage::Host => self.host_step(p),
            Linkage::Builtin { .. } => self.builtin_step(p),
        }
    }

    fn host_step(&mut self, p: &mut Parser) -> Step {
        match p.tok().clone() {
            Tok::Ident(k) if k == "async" => {
                let sp = p.span();
                p.bump();
                self.modifiers.push(ModifierNode {
                    span: sp,
                    kind: ModKind::Async,
                });
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
                "host fn signatures are concrete — generic parameters cannot cross the boundary",
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
            Tok::Ident(k) if k == "class" => {
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
                // the body's left boundary — lo at the `{`, hi at the pop
                self.body_lo = p.expect(Tok::LBrace).map_or(p.prev_end, |s| s.lo);
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
                    p.err(p.span(), "a primitive takes no type arguments");
                    while !matches!(p.tok(), Tok::LBrace | Tok::Semi | Tok::Eof) {
                        p.bump();
                    }
                }
                self.stage = SuStage::Members;
                self.body_lo = p.expect(Tok::LBrace).map_or(p.prev_end, |s| s.lo);
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
                self.body_lo = p.expect(Tok::LBrace).map_or(p.prev_end, |s| s.lo);
                self.members_top(p)
            }
            Tok::Ident(_) => {
                p.err_here(
                    "`builtin` spells its kind — `builtin primitive <name> { .. }`, `builtin class Name { .. }`, or `builtin impl <prim> { .. }`",
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
                let body = p.span_from(self.body_lo);
                let item = if self.is_impl {
                    BuiltinImplNode {
                        span: p.span_from(self.lo),
                        modifiers: std::mem::take(&mut self.modifiers),
                        prim: crate::Name {
                            ident: self.name.id,
                            span: self.name.span,
                        },
                        methods: std::mem::take(&mut self.methods),
                    }
                    .allocate(&mut p.ast)
                } else if self.is_primitive {
                    with_doc!(
                        p,
                        BuiltinPrimitiveNode {
                            span: p.span_from(self.lo),
                            body_span: body,
                            modifiers: std::mem::take(&mut self.modifiers),
                            name: crate::Name {
                                ident: self.name.id,
                                span: self.name.span,
                            },
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
                            body_span: body,
                            modifiers: std::mem::take(&mut self.modifiers),
                            name: crate::Name {
                                ident: self.name.id,
                                span: self.name.span,
                            },
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
            (SuStage::Members, Done::Body(fields, _methods, body_span)) => {
                let node = with_doc!(
                    p,
                    SurfaceStructNode {
                        span: p.span_from(self.lo),
                        body_span,
                        modifiers: std::mem::take(&mut self.modifiers),
                        name: crate::Name {
                            ident: self.name.id,
                            span: self.name.span,
                        },
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
                modifiers: std::mem::take(&mut self.modifiers),
                name: crate::Name {
                    ident: self.name.id,
                    span: self.name.span,
                },
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
            if p.at_kw("self") {
                let sp = p.bump().span;
                let node = SelfParamNode {
                    span: Span::new(lo.lo, sp.hi.max(lo.lo)),
                }
                .allocate(&mut p.ast);
                self.out.push(node);
                if !p.eat_punct(Tok::Comma) {
                    p.expect(Tok::RParen);
                    return Step::Pop(Done::Members(std::mem::take(&mut self.out)));
                }
                continue;
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
            self.p_name = name;
            if p.eat_punct(Tok::Colon) {
                self.stage = PsStage::Ty;
                return Step::Push(Frame::Type(TypeFrame::new(p)));
            }
            let node = ParamNode {
                span: Span::new(lo.lo, p.prev_end.max(lo.lo)),
                name: crate::Name {
                    ident: name.id,
                    span: name.span,
                },
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
                let (lo, name) = (self.p_lo, self.p_name);
                let node = ParamNode {
                    span: Span::new(lo.lo, p.prev_end.max(lo.lo)),
                    name: crate::Name {
                        ident: name.id,
                        span: name.span,
                    },
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
