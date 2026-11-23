//! Inlay hints — the inference display, inline (the lsp-features survey
//! §4.3). A DOCUMENT PASS, not a position query: one walk over the
//! document's lets and calls produces every hint; the viewport filter
//! at the ABI edge is presentation. The client renders; the server
//! supplies position + label + the colon. Two kinds:
//!
//! * **TYPE** (`InlayHintKind::TYPE`) — the bindings pass's
//!   unannotated rows (`LetStmtNode { ty: None }` — a normal absent
//!   slot, the trigger) and loop variables, plus the module lets whose
//!   annotation is absent: the resolver's declared-or-inferred type
//!   rendered at the binding's end (`let c⌊: Circle⌋ =
//!   Circle::new(..)`). An explicit annotation is never restated.
//! * **PARAMETER** (`InlayHintKind::PARAMETER`) — the callee's
//!   parameter names before the args at call sites, only on the
//!   exact-arity match (mismatch → no hints, never wrong hints). The
//!   callee resolves through the resolver family's doors —
//!   [`Chain::expr_ty`] (literals → primitives, paths → the bindings
//!   read internally and module lets, calls → the callee's written
//!   ret, member chains → the member lanes) for receivers and
//!   [`Chain::impl_method_fn`] for the method rule (own-surface hits
//!   are FINAL — no hints, never wrong names).
//!
//! The consistency law: a type hint's text is the SAME string hover
//! shows for the binding — one source (the declared annotation's span
//! join, else the resolver's answer), literal defaults included — and
//! the tooltip IS the binding's hover markdown. Divergence would be a
//! bug. Inlay text is the sanctioned source-less rendering (inferred);
//! tooltips render the written decl verbatim via the views.

use ls_types::{
    InlayHint, InlayHintKind, InlayHintLabel, InlayHintTooltip, MarkupContent, MarkupKind, Position,
};

use rut_ast::{ElseBranch, ExprRef, ItemRef, StmtRef};

use crate::world::collection::{Chain, MountedFile};
use crate::world::types::head_of_ty;

/// everything one inlay query needs — the chain the collection handed
/// out (the open document at the head; its kept document owns the
/// tree, the line index, and the text)
pub struct Ctx<'a> {
    /// the document's own mounted file — module-let hints and their
    /// renders
    pub doc: &'a MountedFile,
    /// the lookup chain: the open document first, then the prelude + workspace
    pub chain: &'a Chain<'a>,
    /// the doc's file-module context (the stamped twin's pair) — the
    /// receiver resolution's own-lane key
    pub doc_mods: &'a crate::world::mods::DocMods,
}

impl Ctx<'_> {
    /// the open document's tree (the chain head's kept document)
    fn ast(&self) -> &rut_ast::Ast {
        self.doc.doc.ast()
    }
}

/// one hint before position/label conversion — byte-based so the range
/// filter runs in the spans' currency. The PER-VERSION lazy view's row
/// (the shell caches the computed pass beside its other lazies and
/// drops it on sync)
pub(crate) struct RawInlay {
    pub byte: u32,
    pub label: String,
    pub kind: InlayHintKind,
    pub tooltip: Option<String>,
}

/// `textDocument/inlayHint` for `[lo, hi]` (byte offsets into the
/// normalized source — the full document when the face passes 0..len).
/// THE PASS is the shell's lazy per-version view: computed at first
/// read over the whole document, cached beside the doc's other lazies
/// (dropped by sync — a new version recomputes), and the range filter
/// rides the cache (presentation). Sorted by position.
pub fn hints(ctx: &Ctx, lo: u32, hi: u32) -> Vec<InlayHint> {
    let (index, src) = (ctx.doc.doc.lines(), ctx.doc.doc.src());
    let raw = ctx.doc.doc.inlays_view(|| {
        let mut raw: Vec<RawInlay> = Vec::new();
        type_hints(ctx, &mut raw);
        call_hints(ctx, &mut raw);
        raw.sort_by_key(|h| index.position(src, h.byte));
        raw
    });
    raw.iter()
        .filter(|h| h.byte >= lo && h.byte <= hi)
        .map(|h| {
            let (line, character) = index.position(src, h.byte);
            InlayHint {
                position: Position { line, character },
                label: InlayHintLabel::String(h.label.clone()),
                kind: Some(h.kind),
                tooltip: h.tooltip.clone().map(|value| {
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

/// TYPE hints — the bindings pass's unannotated rows (the doc's own
/// lazy pass read internally) and the decl layer's module lets whose
/// annotation is absent. Position: the end of the binding's declaring
/// ident (VS Code's dotted-decoration spot). Label: `: Type` — the
/// server supplies the colon. Tooltip: the hover markdown.
fn type_hints(ctx: &Ctx, out: &mut Vec<RawInlay>) {
    let doc = ctx.doc;
    for b in doc.doc.bindings() {
        // an unannotated row IS the trigger (`ty_span` absent — a
        // normal absent slot); the label only lands when the resolver
        // derives something (a miss never guesses)
        if b.ty_span.is_some() {
            continue;
        }
        if !matches!(b.kind, rut_ast::BindKind::Let | rut_ast::BindKind::IterVar) {
            continue;
        }
        let Some(ty) = ctx.chain.binding_ty(b).filter(|t| !t.trim().is_empty()) else {
            continue;
        };
        let name = doc.doc.ast().name(b.name);
        let tooltip = crate::query::hover::binding_markdown(b, name, Some(ty.clone()));
        out.push(RawInlay {
            byte: b.decl_ident_span.hi,
            label: format!(": {ty}"),
            kind: InlayHintKind::TYPE,
            tooltip: Some(tooltip),
        });
    }
    // module lets — the AST's `ty: None` is the unannotated law; the
    // view's annotation-or-inferred type text (the hover render's own
    // source) supplies the label
    for l in doc.lets() {
        if l.h
            .downcast::<rut_ast::ModuleLetNode>()
            .map(|n| n.get(ctx.ast()).ty.is_some())
            == Some(true)
        {
            continue;
        }
        let Some(text) = l.ty().filter(|t| !t.trim().is_empty()) else {
            continue;
        };
        let Some(name) = l.name_span() else { continue };
        out.push(RawInlay {
            byte: name.hi,
            label: format!(": {text}"),
            kind: InlayHintKind::TYPE,
            tooltip: Some(crate::query::hover::render_let(&l)),
        });
    }
}

/// PARAMETER hints — one walk over every body (fns, methods, lambdas,
/// module-let initializers), a hint per arg where the callee's recorded
/// params match the arity exactly.
fn call_hints(ctx: &Ctx, out: &mut Vec<RawInlay>) {
    let mut cx = CallCx { ctx, out };
    for item in ctx.ast().root().items(ctx.ast()) {
        match item {
            ItemRef::Fn(h) => cx.walk_block(h.get(ctx.ast()).body),
            ItemRef::Class(h) => cx.walk_methods(&h.get(ctx.ast()).methods),
            ItemRef::Struct(h) => cx.walk_methods(&h.get(ctx.ast()).methods),
            ItemRef::Interface(h) => cx.walk_methods(&h.get(ctx.ast()).methods),
            ItemRef::Impl(h) => cx.walk_methods(&h.get(ctx.ast()).methods),
            ItemRef::ModuleLet(h) => cx.walk_expr(h.get(ctx.ast()).init),
            _ => {}
        }
    }
}

struct CallCx<'a> {
    ctx: &'a Ctx<'a>,
    out: &'a mut Vec<RawInlay>,
}

impl CallCx<'_> {
    fn walk_methods(&mut self, methods: &[rut_ast::MethodRef]) {
        for m in methods {
            let rut_ast::MethodRef::Method(h) = m else {
                continue;
            };
            if let Some(body) = h.get(self.ctx.ast()).body {
                self.walk_block(body);
            }
        }
    }

    fn walk_block(&mut self, block: rut_ast::Handle<rut_ast::BlockNode>) {
        let d = block.get(self.ctx.ast());
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
            StmtRef::Let(h) => self.walk_expr(h.get(self.ctx.ast()).init),
            StmtRef::ForOf(h) => {
                let d = h.get(self.ctx.ast());
                self.walk_expr(d.iter);
                self.walk_block(d.body);
            }
            StmtRef::ForC(h) => {
                let d = h.get(self.ctx.ast());
                self.walk_expr(d.init);
                self.walk_expr(d.cond);
                self.walk_expr(d.update);
                self.walk_block(d.body);
            }
            StmtRef::If(h) => {
                let d = h.get(self.ctx.ast());
                self.walk_expr(d.cond);
                self.walk_block(d.then);
                match d.els {
                    Some(ElseBranch::If(ih)) => self.walk_stmt(StmtRef::If(ih)),
                    Some(ElseBranch::Block(b)) => self.walk_block(b),
                    None => {}
                }
            }
            StmtRef::While(h) => {
                let d = h.get(self.ctx.ast());
                self.walk_expr(d.cond);
                self.walk_block(d.body);
            }
            StmtRef::When(h) => {
                let d = h.get(self.ctx.ast());
                self.walk_expr(d.scrut);
                for a in &d.arms {
                    self.walk_expr(a.get(self.ctx.ast()).body);
                }
            }
            StmtRef::Return(h) => {
                if let Some(v) = h.get(self.ast()).value {
                    self.walk_expr(v);
                }
            }
            StmtRef::Expr(e) => self.walk_expr(e),
            StmtRef::Break(_) | StmtRef::Continue(_) | StmtRef::Error(_) => {}
        }
    }

    fn walk_expr(&mut self, e: ExprRef) {
        let ast = self.ctx.ast();
        match e {
            ExprRef::Call(h) => {
                let (callee, args) = {
                    let d = h.get(ast);
                    (d.callee, d.args.clone())
                };
                self.free_call(callee, &args);
                self.walk_expr(callee);
                for a in args {
                    self.walk_expr(a);
                }
            }
            ExprRef::Method(h) => {
                let (member, recv, args) = {
                    let d = h.get(ast);
                    (d.member, d.recv, d.args.clone())
                };
                if let Some(n) = member {
                    self.method_call(e, recv, n.ident, &args);
                }
                self.walk_expr(recv);
                for a in args {
                    self.walk_expr(a);
                }
            }
            ExprRef::Block(bh) => self.walk_block(bh),
            ExprRef::Lambda(h) => self.walk_expr(h.get(self.ast()).body),
            _ => {
                for c in rut_ast::expr_children(self.ast(), e) {
                    self.walk_expr(c);
                }
            }
        }
    }

    fn ast(&self) -> &rut_ast::Ast {
        self.ctx.ast()
    }

    /// `f(..)` — a free fn by unique match (the `call_ty` law:
    /// ambiguity is a miss, never a wrong name)
    fn free_call(&mut self, callee: ExprRef, args: &[ExprRef]) {
        let ExprRef::Path(h) = callee else {
            return;
        };
        let segs = &h.get(self.ast()).segs;
        if segs.len() != 1 {
            return;
        }
        let name = self.ast().name(segs[0].name.ident);
        let hits: Vec<_> = self
            .ctx
            .chain
            .iter()
            .flat_map(|f| f.fns())
            .filter(|f| f.name() == name && f.owner().is_none())
            .collect();
        let [one] = hits.as_slice() else { return };
        let params = one.params();
        self.arg_hints(&params, args);
    }

    /// `recv.m(..)` — the receiver's type head, then the inherent impl
    /// fn's recorded params. A named receiver resolves through the
    /// resolver family (self, capitalized, primitive, the bindings —
    /// read internally — module let); anything else (chained calls,
    /// field paths) through the expression door
    fn method_call(
        &mut self,
        call: ExprRef,
        recv: ExprRef,
        name: rut_ast::IdentId,
        args: &[ExprRef],
    ) {
        let ast = self.ctx.ast();
        let pos = call.span(ast).lo;
        let head = match recv {
            ExprRef::Path(ph) if ph.get(ast).segs.len() == 1 => {
                let id = ph.get(ast).segs[0].name.ident;
                self.ctx.chain.named_recv(self.ctx.doc_mods, pos, id)
            }
            _ => self
                .ctx
                .chain
                .expr_ty(self.ctx.doc, recv)
                .map(|t| head_of_ty(&t)),
        };
        let Some(ty_name) = head else { return };
        let Some(params) = self.method_params(&ty_name, ast.name(name)) else {
            return;
        };
        self.arg_hints(&params, args);
    }

    /// the callee's parameter names, `self` excluded — inherent
    /// impl-block methods only, via the shared door (own-surface
    /// methods are FINAL with no hints, never wrong names)
    fn method_params(&self, ty_name: &str, member: &str) -> Option<Vec<String>> {
        self.ctx
            .chain
            .impl_method_fn(ty_name, member)
            .map(|f| f.params())
    }

    /// one PARAMETER hint per arg, at the arg's first byte — only on
    /// the exact-arity match (the survey's "mismatch → no hints, never
    /// wrong hints" law)
    fn arg_hints(&mut self, params: &[String], args: &[ExprRef]) {
        if params.len() != args.len() || args.is_empty() {
            return;
        }
        for (p, a) in params.iter().zip(args) {
            self.out.push(RawInlay {
                byte: a.span(self.ast()).lo,
                label: format!("{p}:"),
                kind: InlayHintKind::PARAMETER,
                tooltip: None,
            });
        }
    }
}
