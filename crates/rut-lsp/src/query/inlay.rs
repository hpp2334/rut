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

use rut_ast::{Ast, ElseBranch, ExprRef, ItemRef, StmtRef};
use rut_semantic::LineIndex;
use rut_semantic::Token;

use crate::hover::lookup::{self, recv_type};
use crate::hover::types::{head_of_ty, DefIndex, FnDef};
use crate::hover::{bindings, infer, BindKind, Binding};

/// everything one inlay query needs — the same `doc_ctx` pipeline the
/// other queries ride, plus the line index (byte spans ⇄ LSP positions
/// over the normalized source)
pub struct Ctx<'a> {
    pub ast: &'a Ast,
    pub toks: &'a [Token],
    /// the document's own index — module-let hints and their renders
    pub doc: &'a DefIndex,
    /// the lookup chain: the open document first, then the prelude + workspace
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
    for item in ctx.ast.root().items(ctx.ast) {
        let ItemRef::ModuleLet(h) = item else {
            continue;
        };
        if h.get(ctx.ast).ty.is_some() {
            continue;
        }
        let span = h.span(ctx.ast);
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
    for item in ctx.ast.root().items(ctx.ast) {
        match item {
            ItemRef::Fn(h) => cx.walk_block(h.get(ctx.ast).body),
            ItemRef::Class(h) => cx.walk_methods(&h.get(ctx.ast).methods),
            ItemRef::Struct(h) => cx.walk_methods(&h.get(ctx.ast).methods),
            ItemRef::Interface(h) => cx.walk_methods(&h.get(ctx.ast).methods),
            ItemRef::Impl(h) => cx.walk_methods(&h.get(ctx.ast).methods),
            ItemRef::ModuleLet(h) => cx.walk_expr(h.get(ctx.ast).init),
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
    fn walk_methods(&mut self, methods: &[rut_ast::MethodRef]) {
        for m in methods {
            let rut_ast::MethodRef::Method(h) = m else {
                continue;
            };
            if let Some(body) = h.get(self.ctx.ast).body {
                self.walk_block(body);
            }
        }
    }

    fn walk_block(&mut self, block: rut_ast::Handle<rut_ast::BlockNode>) {
        let d = block.get(self.ctx.ast);
        for s in d.stmts.iter().copied() {
            self.walk_stmt(s);
        }
        // the block's tail value is in the same scope
        if let Some(t) = d.tail {
            self.walk_expr(t);
        }
    }

    fn walk_stmt(&mut self, s: StmtRef) {
        match s {
            StmtRef::Let(h) => self.walk_expr(h.get(self.ctx.ast).init),
            StmtRef::ForOf(h) => {
                let d = h.get(self.ctx.ast);
                self.walk_expr(d.iter);
                self.walk_block(d.body);
            }
            StmtRef::ForC(h) => {
                let d = h.get(self.ctx.ast);
                self.walk_expr(d.init);
                self.walk_expr(d.cond);
                self.walk_expr(d.update);
                self.walk_block(d.body);
            }
            StmtRef::If(h) => {
                let d = h.get(self.ctx.ast);
                self.walk_expr(d.cond);
                self.walk_block(d.then);
                match d.els {
                    Some(ElseBranch::If(ih)) => self.walk_stmt(StmtRef::If(ih)),
                    Some(ElseBranch::Block(b)) => self.walk_block(b),
                    None => {}
                }
            }
            StmtRef::While(h) => {
                let d = h.get(self.ctx.ast);
                self.walk_expr(d.cond);
                self.walk_block(d.body);
            }
            StmtRef::When(h) => {
                let d = h.get(self.ctx.ast);
                self.walk_expr(d.scrut);
                for a in &d.arms {
                    self.walk_expr(a.get(self.ctx.ast).body);
                }
            }
            StmtRef::Return(h) => {
                if let Some(v) = h.get(self.ctx.ast).value {
                    self.walk_expr(v);
                }
            }
            StmtRef::Expr(e) => self.walk_expr(e),
            StmtRef::Break(_) | StmtRef::Continue(_) | StmtRef::Error(_) => {}
        }
    }

    fn walk_expr(&mut self, e: ExprRef) {
        match e {
            ExprRef::Call(h) => {
                let d = h.get(self.ctx.ast);
                self.free_call(d.callee, &d.args);
                self.walk_expr(d.callee);
                for a in d.args.clone() {
                    self.walk_expr(a);
                }
            }
            ExprRef::Method(h) => {
                let d = h.get(self.ctx.ast);
                self.method_call(e, d.recv, d.name, &d.args);
                self.walk_expr(d.recv);
                for a in d.args.clone() {
                    self.walk_expr(a);
                }
            }
            ExprRef::Block(bh) => self.walk_block(bh),
            ExprRef::Lambda(h) => self.walk_expr(h.get(self.ctx.ast).body),
            _ => {
                for c in bindings::expr_children(self.ctx.ast, e) {
                    self.walk_expr(c);
                }
            }
        }
    }

    /// `f(..)` — a free fn by unique match (the `call_ty` law:
    /// ambiguity is a miss, never a wrong name)
    fn free_call(&mut self, callee: ExprRef, args: &[ExprRef]) {
        let ExprRef::Path(h) = callee else {
            return;
        };
        let segs = &h.get(self.ctx.ast).segs;
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
        call: ExprRef,
        recv: ExprRef,
        name: rut_ast::IdentId,
        args: &[ExprRef],
    ) {
        let pos = call.span(self.ctx.ast).lo;
        // a named receiver resolves through the phase-1 recv rule
        // (`self`, capitalized, primitive, binding, module let);
        // anything else (chained calls, field paths) through the
        // display-side expression inference
        let head = match recv {
            ExprRef::Path(ph) if ph.get(self.ctx.ast).segs.len() == 1 => recv_type(
                self.ctx.idxs,
                self.binds,
                pos,
                self.ctx.ast.name(ph.get(self.ctx.ast).segs[0].name),
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
    fn arg_hints(&mut self, params: &[String], args: &[ExprRef]) {
        if params.len() != args.len() || args.is_empty() {
            return;
        }
        for (p, a) in params.iter().zip(args) {
            self.out.push(Raw {
                byte: a.span(self.ctx.ast).lo,
                label: format!("{p}:"),
                kind: InlayHintKind::PARAMETER,
                tooltip: None,
            });
        }
    }
}
