//! Inlay hints — the inference display, inline (the lsp-features survey
//! §4.3). The client renders; the server supplies position + label + the
//! colon. Two kinds:
//!
//! * **TYPE** (`InlayHintKind::TYPE`) — unannotated `let`s, for-of /
//!   for-c loop variables, and module lets: the inferred type rendered
//!   at the binding's end (`let c⌊: Circle⌋ = Circle::new(..)`). An
//!   explicit annotation is never restated. `Binding::inferred` IS the
//!   unannotated-and-derived law for fn-locals; for module lets the
//!   AST's `ty: None` decides and `LetDef::ty` supplies the text.
//! * **PARAMETER** (`InlayHintKind::PARAMETER`) — the callee's
//!   parameter names before the args at call sites, only on the
//!   exact-arity match (mismatch → no hints, never wrong hints). The
//!   callee resolves by the same rules phase 1 prices: free fns by
//!   unique match (the `call_ty` law), methods through the receiver's
//!   type head (self / capitalized / primitive / binding / module let /
//!   the display-side expression inference). Own-surface methods (interface
//!   bodies, builtin/primitive surfaces) carry no recorded params — a
//!   hit there is FINAL (the known-receiver-is-final law) and yields no
//!   hints.
//!
//! The consistency law: a type hint's text is the SAME string phase 1's
//! hover shows for the binding — one source (`Binding::ty` /
//! `LetDef::ty`, literal defaults included) — and the tooltip
//! IS the binding's hover markdown. Divergence would be a bug.

use ls_types::{
    InlayHint, InlayHintKind, InlayHintLabel, InlayHintTooltip, MarkupContent, MarkupKind, Position,
};

use rut_ast::ast::*;
use rut_lexer::token::Token;

use crate::hover::lookup::{self, recv_type};
use crate::hover::types::{head_of_ty, DefIndex, FnDef};
use crate::hover::{bindings, infer, BindKind, Binding};
use crate::line_index::LineIndex;

/// everything one inlay query needs — the same `doc_ctx` pipeline the
/// other queries ride, plus the line index (byte spans ⇄ LSP positions
/// over the normalized source)
pub struct Ctx<'a> {
    pub ast: &'a Ast,
    pub toks: &'a [Token],
    /// the document's own index — module-let hints and their renders
    pub doc: &'a DefIndex,
    /// the lookup chain: the open document first, then std + workspace
    pub idxs: &'a [&'a DefIndex],
    pub index: &'a LineIndex,
    pub src: &'a str,
}

/// one hint before position/label conversion — byte-based so the range
/// filter runs in the spans' currency
struct Raw {
    byte: u32,
    label: String,
    kind: InlayHintKind,
    tooltip: Option<String>,
}

/// `textDocument/inlayHint` for `[lo, hi]` (byte offsets into the
/// normalized source — the full document when the face passes 0..len).
/// Sorted by position; a hint outside the range is dropped.
pub fn hints(ctx: &Ctx, lo: u32, hi: u32) -> Vec<InlayHint> {
    let mut raw: Vec<Raw> = Vec::new();
    let binds = bindings::collect(ctx.ast, ctx.toks, ctx.idxs);
    type_hints(ctx, &binds, &mut raw);
    call_hints(ctx, &binds, &mut raw);
    raw.sort_by_key(|h| ctx.index.position(ctx.src, h.byte));
    raw.into_iter()
        .filter(|h| h.byte >= lo && h.byte <= hi)
        .map(|h| {
            let (line, character) = ctx.index.position(ctx.src, h.byte);
            InlayHint {
                position: Position { line, character },
                label: InlayHintLabel::String(h.label),
                kind: Some(h.kind),
                tooltip: h.tooltip.map(|value| {
                    InlayHintTooltip::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value,
                    })
                }),
                text_edits: None,
                padding_left: None,
                padding_right: None,
                data: None,
            }
        })
        .collect()
}

/// TYPE hints — the binding pass's unannotated lets and loop variables,
/// then the decl layer's module lets. Position: the end of the binding's
/// declaring ident (VS Code's dotted-decoration spot). Label: `: Type` —
/// the server supplies the colon. Tooltip: the hover markdown.
fn type_hints(ctx: &Ctx, binds: &[Binding], out: &mut Vec<Raw>) {
    for b in binds {
        // `inferred` = no written annotation AND something derived — an
        // annotated binding is never restated, a miss never guessed
        if !b.inferred {
            continue;
        }
        if !matches!(b.kind, BindKind::Let | BindKind::IterVar) {
            continue;
        }
        let Some(ty) = b.ty.as_deref().filter(|t| !t.trim().is_empty()) else {
            continue;
        };
        out.push(Raw {
            byte: b.decl_ident_span.hi,
            label: format!(": {ty}"),
            kind: InlayHintKind::TYPE,
            tooltip: Some(crate::hover::binding_markdown(b)),
        });
    }
    // module lets — the AST's `ty: None` is the unannotated law;
    // `LetDef::ty` (annotation-or-inferred, the hover render's source)
    // supplies the text
    for h in ctx.ast.module_items(ctx.ast.root) {
        let ItemKind::ModuleLet { ty, .. } = ctx.ast.item(*h) else {
            continue;
        };
        if ty.is_some() {
            continue;
        }
        let span = ctx.ast.span(h.id());
        let Some(l) = ctx.doc.lets.iter().find(|l| l.span == span) else {
            continue;
        };
        let Some(text) = l.ty.as_deref().filter(|t| !t.trim().is_empty()) else {
            continue;
        };
        let Some(name) = l.name_span else { continue };
        out.push(Raw {
            byte: name.hi,
            label: format!(": {text}"),
            kind: InlayHintKind::TYPE,
            tooltip: Some(crate::hover::render_let(ctx.doc, l)),
        });
    }
}

/// PARAMETER hints — one walk over every body (fns, methods, lambdas,
/// module-let initializers), a hint per arg where the callee's recorded
/// params match the arity exactly.
fn call_hints(ctx: &Ctx, binds: &[Binding], out: &mut Vec<Raw>) {
    let mut cx = CallCx { ctx, binds, out };
    for h in ctx.ast.module_items(ctx.ast.root) {
        match ctx.ast.item(*h) {
            ItemKind::Fn(d) => cx.walk_block(d.body),
            ItemKind::Class { methods, .. }
            | ItemKind::Struct { methods, .. }
            | ItemKind::Interface { methods, .. }
            | ItemKind::Impl { methods, .. } => {
                for m in methods {
                    if let Some(body) = ctx.ast.method_decl(*m).body {
                        cx.walk_block(body);
                    }
                }
            }
            ItemKind::ModuleLet { init, .. } => cx.walk_expr(*init),
            _ => {}
        }
    }
}

struct CallCx<'a> {
    ctx: &'a Ctx<'a>,
    binds: &'a [Binding],
    out: &'a mut Vec<Raw>,
}

impl CallCx<'_> {
    fn walk_block(&mut self, block: NodeHandle<BlockNode>) {
        for s in self.ctx.ast.block(block).to_vec() {
            self.walk_stmt(s);
        }
        // the block's tail value is in the same scope
        if let Some(t) = self.ctx.ast.block_tail(block) {
            self.walk_expr(t);
        }
    }

    fn walk_stmt(&mut self, s: NodeHandle<AnyStmt>) {
        match self.ctx.ast.kind(s.id()) {
            Kind::Stmt(StmtKind::LetStmt { init, .. }) => self.walk_expr(*init),
            Kind::Stmt(StmtKind::ForOf { iter, body, .. }) => {
                self.walk_expr(*iter);
                self.walk_block(*body);
            }
            Kind::Stmt(StmtKind::ForC {
                init,
                cond,
                update,
                body,
                ..
            }) => {
                self.walk_expr(*init);
                self.walk_expr(*cond);
                self.walk_expr(*update);
                self.walk_block(*body);
            }
            Kind::Stmt(StmtKind::If { cond, then, els }) => {
                self.walk_expr(*cond);
                self.walk_block(*then);
                match els {
                    Some(ElseBranch::If(h)) => self.walk_stmt((*h).into()),
                    Some(ElseBranch::Block(b)) => self.walk_block(*b),
                    None => {}
                }
            }
            Kind::Stmt(StmtKind::While { cond, body }) => {
                self.walk_expr(*cond);
                self.walk_block(*body);
            }
            Kind::Stmt(StmtKind::WhenStmt { scrut, arms }) => {
                self.walk_expr(*scrut);
                for a in arms {
                    match self.ctx.ast.arm(*a) {
                        ArmKind::WhenArm { body, .. } => self.walk_expr(*body),
                    }
                }
            }
            Kind::Stmt(StmtKind::Return { value: Some(v) }) => self.walk_expr(*v),
            Kind::Stmt(StmtKind::Return { value: None }) => {}
            Kind::Stmt(StmtKind::ExprStmt(e)) => self.walk_expr(*e),
            _ => {}
        }
    }

    fn walk_expr(&mut self, e: NodeHandle<AnyExpr>) {
        match self.ctx.ast.expr(e) {
            ExprKind::Call { callee, args } => {
                self.free_call(*callee, args);
                self.walk_expr(*callee);
                for a in args.clone() {
                    self.walk_expr(a);
                }
            }
            ExprKind::Method {
                recv, name, args, ..
            } => {
                self.method_call(e, *recv, *name, args);
                self.walk_expr(*recv);
                for a in args.clone() {
                    self.walk_expr(a);
                }
            }
            ExprKind::Block { .. } => {
                let b = NodeHandle::<BlockNode>::new(e.id());
                for s in self.ctx.ast.block(b).to_vec() {
                    self.walk_stmt(s);
                }
                if let Some(t) = self.ctx.ast.block_tail(b) {
                    self.walk_expr(t);
                }
            }
            ExprKind::Lambda { body, .. } => self.walk_expr(*body),
            _ => {
                for c in bindings::expr_children(self.ctx.ast, e) {
                    self.walk_expr(c);
                }
            }
        }
    }

    /// `f(..)` — a free fn by unique match (the `call_ty` law:
    /// ambiguity is a miss, never a wrong name)
    fn free_call(&mut self, callee: NodeHandle<AnyExpr>, args: &[NodeHandle<AnyExpr>]) {
        let ExprKind::Path { segs } = self.ctx.ast.expr(callee) else {
            return;
        };
        if segs.len() != 1 {
            return;
        }
        let name = self.ctx.ast.name(segs[0].name);
        let hits: Vec<&FnDef> = self
            .ctx
            .idxs
            .iter()
            .flat_map(|i| i.fns.iter().filter(|f| f.name == name && f.owner.is_none()))
            .collect();
        let [one] = hits.as_slice() else { return };
        let params = one.params.clone();
        self.arg_hints(&params, args);
    }

    /// `recv.m(..)` — the receiver's type head, then the inherent impl
    /// fn's recorded params
    fn method_call(
        &mut self,
        call: NodeHandle<AnyExpr>,
        recv: NodeHandle<AnyExpr>,
        name: IdentId,
        args: &[NodeHandle<AnyExpr>],
    ) {
        let pos = self.ctx.ast.span(call.id()).lo;
        // a named receiver resolves through the phase-1 recv rule
        // (`self`, capitalized, primitive, binding, module let);
        // anything else (chained calls, field paths) through the
        // display-side expression inference
        let head = match self.ctx.ast.expr(recv) {
            ExprKind::Path { segs } if segs.len() == 1 => recv_type(
                self.ctx.idxs,
                self.binds,
                pos,
                self.ctx.ast.name(segs[0].name),
            ),
            _ => infer::expr_ty(self.ctx.ast, self.binds, self.ctx.idxs, recv)
                .map(|t| head_of_ty(&t)),
        };
        let Some(ty_name) = head else { return };
        let Some(params) = self.method_params(&ty_name, self.ctx.ast.name(name)) else {
            return;
        };
        self.arg_hints(&params, args);
    }

    /// the callee's parameter names, `self` excluded — inherent
    /// impl-block methods only, via the shared `impl_method_fn` rule
    /// (own-surface methods are FINAL with no hints, never wrong names)
    fn method_params(&self, ty_name: &str, member: &str) -> Option<Vec<String>> {
        lookup::impl_method_fn(self.ctx.idxs, ty_name, member).map(|f| f.params.clone())
    }

    /// one PARAMETER hint per arg, at the arg's first byte — only on
    /// the exact-arity match (the survey's "mismatch → no hints, never
    /// wrong hints" law)
    fn arg_hints(&mut self, params: &[String], args: &[NodeHandle<AnyExpr>]) {
        if params.len() != args.len() || args.is_empty() {
            return;
        }
        for (p, a) in params.iter().zip(args) {
            self.out.push(Raw {
                byte: self.ctx.ast.span(a.id()).lo,
                label: format!("{p}:"),
                kind: InlayHintKind::PARAMETER,
                tooltip: None,
            });
        }
    }
}
