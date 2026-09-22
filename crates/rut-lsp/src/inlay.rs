//! Inlay hints — the inference display, inline (the lsp-features survey
//! §4.3). The client renders; the server supplies position + label + the
//! colon. Two kinds:
//!
//! * **TYPE** (`InlayHintKind::TYPE`) — unannotated `let`s, for-of /
//!   for-c loop variables, and module lets: the inferred type rendered
//!   at the binding's end (`let c⌊: Circle⌋ = Circle.new(..)`). An
//!   explicit annotation is never restated. `Binding::inferred` IS the
//!   unannotated-and-derived law for fn-locals; for module lets the
//!   AST's `ty: None` decides and `LetDef::ty` supplies the text.
//! * **PARAMETER** (`InlayHintKind::PARAMETER`) — the callee's
//!   parameter names before the args at call sites, only on the
//!   exact-arity match (mismatch → no hints, never wrong hints). The
//!   callee resolves by the same rules phase 1 prices: free fns by
//!   unique match (the `call_ty` law), methods through the receiver's
//!   type head (self / capitalized / primitive / binding / module let /
//!   the display-side expression inference). Own-surface methods (trait
//!   bodies, builtin/primitive surfaces) carry no recorded params — a
//!   hit there is FINAL (the known-receiver-is-final law) and yields no
//!   hints.
//!
//! The consistency law: a type hint's text is the SAME string phase 1's
//! hover shows for the binding — one source (`Binding::ty` /
//! `LetDef::ty`, RFC 0007 literal defaults included) — and the tooltip
//! IS the binding's hover markdown. Divergence would be a bug.

use ls_types::{InlayHint, InlayHintKind, InlayHintLabel, InlayHintTooltip, MarkupContent, MarkupKind, Position};

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
        let ItemKind::ModuleLet { ty, .. } = ctx.ast.item(*h) else { continue };
        if ty.is_some() {
            continue;
        }
        let span = ctx.ast.span(h.id());
        let Some(l) = ctx.doc.lets.iter().find(|l| l.span == span) else { continue };
        let Some(text) = l.ty.as_deref().filter(|t| !t.trim().is_empty()) else { continue };
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
            | ItemKind::Dataclass { methods, .. }
            | ItemKind::Trait { methods, .. }
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
        for s in self.ctx.ast.block(block) {
            self.walk_stmt(*s);
        }
    }

    fn walk_stmt(&mut self, s: NodeHandle<AnyStmt>) {
        match self.ctx.ast.kind(s.id()) {
            Kind::Stmt(StmtKind::LetStmt { init, .. }) => self.walk_expr(*init),
            Kind::Stmt(StmtKind::ForOf { iter, body, .. }) => {
                self.walk_expr(*iter);
                self.walk_block(*body);
            }
            Kind::Stmt(StmtKind::ForC { init, cond, update, body, .. }) => {
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
                        ArmKind::SelectArm { fut, bind: _, body } => {
                            self.walk_expr(*fut);
                            self.walk_expr(*body);
                        }
                    }
                }
            }
            Kind::Stmt(StmtKind::Return { value }) => {
                if let Some(v) = value {
                    self.walk_expr(*v);
                }
            }
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
            ExprKind::Method { recv, name, args, .. } => {
                self.method_call(e, *recv, *name, args);
                self.walk_expr(*recv);
                for a in args.clone() {
                    self.walk_expr(a);
                }
            }
            ExprKind::Block { stmts } => {
                for s in stmts.clone() {
                    self.walk_stmt(s);
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
        let ExprKind::Path { segs } = self.ctx.ast.expr(callee) else { return };
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
    fn method_call(&mut self, call: NodeHandle<AnyExpr>, recv: NodeHandle<AnyExpr>, name: IdentId, args: &[NodeHandle<AnyExpr>]) {
        let pos = self.ctx.ast.span(call.id()).lo;
        // a named receiver resolves through the phase-1 recv rule
        // (`self`, capitalized, primitive, binding, module let);
        // anything else (chained calls, field paths) through the
        // display-side expression inference
        let head = match self.ctx.ast.expr(recv) {
            ExprKind::Path { segs } if segs.len() == 1 => {
                recv_type(self.ctx.idxs, self.binds, pos, self.ctx.ast.name(segs[0].name))
            }
            _ => infer::expr_ty(self.ctx.ast, self.binds, self.ctx.idxs, recv).map(|t| head_of_ty(&t)),
        };
        let Some(ty_name) = head else { return };
        let Some(params) = self.method_params(&ty_name, self.ctx.ast.name(name)) else { return };
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

#[cfg(test)]
mod tests {
    use super::*;
    use rut_lexer::lexer::{lex, normalize};
    use rut_parser::{parse, Mode};

    const DOC: &str = "file:///t/main.rut";

    struct Doc {
        src: String,
        toks: Vec<Token>,
        ast: Ast,
        index: DefIndex,
        lines: LineIndex,
    }

    fn doc(src: &str) -> Doc {
        let src = normalize(src);
        let (toks, _) = lex(&src);
        let (ast, _) = parse(&src, Mode::Impl);
        let mut index = crate::hover::index(&src, &ast, &toks);
        index.origin = DOC.to_string();
        let lines = LineIndex::new(&src);
        Doc { src, toks, ast, index, lines }
    }

    /// the query context + its chain, both held by the caller (the
    /// `Ctx` borrows the slice)
    struct W<'a> {
        d: &'a Doc,
        idxs: Vec<&'a DefIndex>,
    }

    fn w<'a>(d: &'a Doc, extra: &'a [DefIndex]) -> W<'a> {
        W { d, idxs: std::iter::once(&d.index).chain(extra.iter()).collect() }
    }

    impl W<'_> {
        fn cx(&self) -> Ctx<'_> {
            Ctx {
                ast: &self.d.ast,
                toks: &self.d.toks,
                doc: &self.d.index,
                idxs: &self.idxs,
                index: &self.d.lines,
                src: &self.d.src,
            }
        }

        fn all(&self) -> Vec<InlayHint> {
            hints(&self.cx(), 0, self.d.src.len() as u32)
        }
    }

    /// (line, char) of the `n`th (1-based) occurrence of `needle`
    fn at(src: &str, needle: &str, occurrence: usize) -> (u32, u32) {
        let mut from = 0;
        for _ in 0..occurrence {
            match src[from..].find(needle) {
                Some(i) => from += i + needle.len(),
                None => panic!("needle {needle:?} not found (occurrence {occurrence})"),
            }
        }
        let start = from - needle.len();
        let line = src[..start].matches('\n').count() as u32;
        let ch = start - src[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
        (line, ch as u32)
    }

    /// (line, char) one past the `n`th occurrence — the end of the
    /// ident a TYPE hint hangs from (same line)
    fn end_of(src: &str, needle: &str, occurrence: usize) -> (u32, u32) {
        let (line, ch) = at(src, needle, occurrence);
        (line, ch + needle.len() as u32)
    }

    fn pos(h: &InlayHint) -> (u32, u32) {
        (h.position.line, h.position.character)
    }

    fn label(h: &InlayHint) -> &str {
        match &h.label {
            InlayHintLabel::String(s) => s,
            _ => panic!("label parts not produced"),
        }
    }

    fn tooltip(h: &InlayHint) -> &str {
        match &h.tooltip {
            Some(InlayHintTooltip::MarkupContent(m)) => &m.value,
            _ => "",
        }
    }

    fn labels(hs: &[InlayHint]) -> Vec<&str> {
        hs.iter().map(label).collect()
    }

    // ---- TYPE hints ----

    #[test]
    fn unannotated_let_gets_a_type_hint() {
        let src = [
            "class Circle {",
            "    r: f64;",
            "}",
            "fn go() -> f64 {",
            "    let c = Circle.new(1.0);",
            "    return c.r;",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        let hs = w(&d, &[]).all();
        assert_eq!(hs.len(), 1, "{hs:?}");
        assert_eq!(label(&hs[0]), ": Circle");
        assert_eq!(hs[0].kind, Some(InlayHintKind::TYPE));
        // right after the declaring ident: `let c⌊: Circle⌋ = ..`
        assert_eq!(pos(&hs[0]), end_of(&src, "let c", 1));
        // the tooltip IS the hover markdown
        assert!(tooltip(&hs[0]).contains("let c: Circle"), "{:?}", tooltip(&hs[0]));
        assert!(tooltip(&hs[0]).contains("type inferred"), "{:?}", tooltip(&hs[0]));
    }

    #[test]
    fn annotated_lets_are_never_restated() {
        let src = "class Circle {\n    r: f64;\n}\nfn go() -> nil {\n    let c: Circle = Circle.new(1.0);\n}\n";
        let d = doc(&src);
        assert!(w(&d, &[]).all().is_empty());
    }

    #[test]
    fn inference_miss_stays_silent() {
        let src = "fn main() -> nil {\n    let x = mystery();\n}\n";
        let d = doc(src);
        assert!(w(&d, &[]).all().is_empty());
    }

    #[test]
    fn for_of_and_for_c_vars_get_hints() {
        let src = [
            "struct Point {",
            "    x: f64;",
            "}",
            "fn sum(points: [Point]) -> f64 {",
            "    let total = 0.0;",
            "    for (let p of points) {",
            "        total += p.x;",
            "    }",
            "    for (let i = 0; i < 3; i += 1) {",
            "        total += total;",
            "    }",
            "    return total;",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        let hs = w(&d, &[]).all();
        // the RFC 0007 float default types `total`, the [Point] element
        // types `p`, the literal default types `i`
        assert_eq!(labels(&hs), [": f32", ": Point", ": i32"], "{hs:?}");
        assert_eq!(pos(&hs[1]), end_of(&src, "for (let p", 1));
        assert_eq!(hs[1].kind, Some(InlayHintKind::TYPE));
        assert!(tooltip(&hs[1]).contains("p: Point"), "{:?}", tooltip(&hs[1]));
        assert!(tooltip(&hs[1]).contains("loop variable"), "{:?}", tooltip(&hs[1]));
    }

    #[test]
    fn hint_and_hover_agree_on_the_type_text() {
        // the consistency law: the hint's type is the hover's type —
        // one source (the binding pass), RFC 0007 defaults included
        let src = [
            "class Circle {",
            "    r: f64;",
            "}",
            "fn go() -> f64 {",
            "    let c = Circle.new(1.0);",
            "    return c.r;",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        let hs = w(&d, &[]).all();
        assert_eq!(hs.len(), 1);
        let idxs: Vec<&DefIndex> = vec![&d.index];
        let (hl, hc) = at(&src, "let c", 1);
        let byte = d.lines.byte(&d.src, hl, hc) + 4; // the `c` ident
        let hov = crate::hover::hover(&idxs, &d.toks, &d.ast, byte);
        let md = hov.expect("hover at the binding").markdown;
        assert!(md.contains("let c: Circle"), "{md}");
        let hover_ty = md.split("let c: ").nth(1).unwrap().split('\n').next().unwrap();
        assert_eq!(label(&hs[0]), format!(": {hover_ty}"));
    }

    #[test]
    fn module_let_gets_a_hint_at_its_decl() {
        let src = "let TOTAL = 41;\nfn bump() -> i32 {\n    return TOTAL + 1;\n}\n";
        let d = doc(&src);
        let hs = w(&d, &[]).all();
        assert_eq!(hs.len(), 1, "{hs:?}");
        assert_eq!(label(&hs[0]), ": i32");
        assert_eq!(pos(&hs[0]), end_of(&src, "let TOTAL", 1));
        assert!(tooltip(&hs[0]).contains("let TOTAL = 41;"), "{:?}", tooltip(&hs[0]));
        // an annotated module let restates nothing
        let src2 = "let TOTAL: i32 = 41;\nfn bump() -> i32 {\n    return TOTAL + 1;\n}\n";
        let d2 = doc(src2);
        assert!(w(&d2, &[]).all().is_empty());
    }

    #[test]
    fn destructured_literal_tuple_hints_each_name() {
        // the binding pass infers destructured names from a literal
        // tuple (the shape its records price)
        let src = "fn go() -> f64 {\n    let (a, b) = (1, 2.0);\n    return b;\n}\n";
        let d = doc(&src);
        let hs = w(&d, &[]).all();
        assert_eq!(labels(&hs), [": i32", ": f32"], "{hs:?}");
        assert_eq!(pos(&hs[0]), end_of(&src, "(a", 1));
        let (bl, bc) = at(&src, "b)", 1); // end of the `b` ident
        assert_eq!(pos(&hs[1]), (bl, bc + 1));
    }

    #[test]
    fn tuple_types_render_no_empty_hints() {
        // a tuple ret used to render empty (`ty_src` lacked the arm),
        // which leaked a bare `: ` hint — now the type renders as
        // written, in the hint AND the hover (one source)
        let src = [
            "fn parse(s: str) -> (opaque, str) {",
            "    return (opaque(s), \"\");",
            "}",
            "fn main() -> str {",
            "    let v = parse(\"x\");",
            "    return \"\";",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        let hs = w(&d, &[]).all();
        // the type hint renders the tuple as written, and the call
        // site's param hint rides along
        assert_eq!(labels(&hs), [": (opaque, str)", "s:"], "{hs:?}");
        // ...and the hover shows the same text (the consistency law)
        let idxs: Vec<&DefIndex> = vec![&d.index];
        let (hl, hc) = at(&src, "let v", 1);
        let byte = d.lines.byte(&d.src, hl, hc) + 4; // the `v` ident
        let md = crate::hover::hover(&idxs, &d.toks, &d.ast, byte)
            .expect("hover at the binding")
            .markdown;
        assert!(md.contains("let v: (opaque, str)"), "{md}");
    }

    // ---- PARAMETER hints ----

    #[test]
    fn param_hints_at_a_free_call() {
        let src = [
            "fn hex_val(c: str, k: i32) -> i32 {",
            "    return k;",
            "}",
            "fn main() -> i32 {",
            "    return hex_val(\"a\", 2);",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        let hs = w(&d, &[]).all();
        assert_eq!(labels(&hs), ["c:", "k:"], "{hs:?}");
        assert_eq!(hs[0].kind, Some(InlayHintKind::PARAMETER));
        // each hint sits at the arg's first byte
        assert_eq!(pos(&hs[0]), at(&src, "\"a\"", 1));
        assert_eq!(pos(&hs[1]), at(&src, "2);", 1));
    }

    #[test]
    fn param_hints_at_a_method_call_skip_self() {
        let src = [
            "class Circle {",
            "    r: f64;",
            "}",
            "impl Circle {",
            "    fn grown(self, k: f64) -> Circle { return self; }",
            "}",
            "fn go() -> Circle {",
            "    let c = Circle.new(1.0);",
            "    return c.grown(2.0);",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        let hs = w(&d, &[]).all();
        // the let's TYPE hint + the call's PARAMETER hint (`self` never
        // surfaces — `new` has no recorded impl fn here, `grown` does)
        assert_eq!(labels(&hs), [": Circle", "k:"], "{hs:?}");
        assert_eq!(pos(&hs[1]), at(&src, "2.0)", 1));
    }

    #[test]
    fn arity_mismatch_means_no_hints() {
        // the lets are annotated (type hints would otherwise fire —
        // inference doesn't arity-check); only the PARAMETER hints are
        // under test here: mismatch → none, never wrong names
        let src = [
            "fn f(a: i32, b: i32) -> i32 {",
            "    return a;",
            "}",
            "fn main() -> i32 {",
            "    let x: i32 = f(1);",
            "    let y: i32 = f(1, 2, 3);",
            "    let z: i32 = f();",
            "    return x + y + z;",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        assert!(w(&d, &[]).all().is_empty(), "{:?}", labels(&w(&d, &[]).all()));
    }

    #[test]
    fn ambiguous_callee_stays_silent() {
        // the doc's `f` plus a same-named free fn in another index:
        // no unique match, no hints (the call_ty law)
        let src = "fn f(a: i32) -> i32 {\n    return a;\n}\nfn main() -> i32 {\n    let x = f(1);\n    return x;\n}\n";
        let d = doc(src);
        let mut foreign = doc("fn f(b: str) -> str {\n    return b;\n}\n").index;
        foreign.origin = "file:///ws/other/lib.rut".to_string();
        let extra = [foreign];
        assert!(w(&d, &extra).all().is_empty());
    }

    #[test]
    fn own_surface_method_beats_a_same_named_impl_fn() {
        // a trait-body `m` is the receiver's own surface: the FINAL
        // answer carries no recorded params, so no hints — the impl
        // fn's params must not leak through (never wrong hints)
        let src = [
            "struct P {",
            "    x: i32;",
            "}",
            "trait Draw {",
            "    fn m(self, k: i32) -> i32;",
            "}",
            "impl Draw for P {",
            "    fn m(self, k: i32) -> i32 { return k; }",
            "}",
            "fn go(d: Draw) -> i32 {",
            "    return d.m(1);",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        assert!(w(&d, &[]).all().is_empty(), "{:?}", labels(&w(&d, &[]).all()));
    }

    #[test]
    fn nested_calls_hint_at_their_own_args() {
        let src = [
            "fn inner(a: i32) -> i32 {",
            "    return a;",
            "}",
            "fn outer(b: i32, c: i32) -> i32 {",
            "    return b + c;",
            "}",
            "fn main() -> i32 {",
            "    return outer(inner(1), 2);",
            "}",
            "",
        ]
        .join("\n");
        let d = doc(&src);
        let hs = w(&d, &[]).all();
        // by position: outer's first arg STARTS at `inner`, inner's own
        // arg sits inside it
        assert_eq!(labels(&hs), ["b:", "a:", "c:"], "{hs:?}");
        assert_eq!(pos(&hs[0]), at(&src, "inner(1)", 1));
        assert_eq!(pos(&hs[1]), at(&src, "1)", 1));
        assert_eq!(pos(&hs[2]), at(&src, "2);", 1));
    }

    #[test]
    fn module_let_initializer_calls_get_param_hints() {
        let src = "fn mk(n: i32) -> i32 {\n    return n;\n}\nlet N = mk(3);\n";
        let d = doc(&src);
        let hs = w(&d, &[]).all();
        assert_eq!(labels(&hs), [": i32", "n:"], "{hs:?}");
    }

    // ---- the range filter ----

    #[test]
    fn range_filters_hints() {
        let src = "let A = 1;\nlet B = 2;\n";
        let d = doc(src);
        let w1 = w(&d, &[]);
        assert_eq!(labels(&w1.all()), [": i32", ": i32"]);
        // only the second line's hint stays inside [line 1, end]
        let lo = d.lines.byte(&d.src, 1, 0);
        let hi = d.src.len() as u32;
        let hs = hints(&w1.cx(), lo, hi);
        assert_eq!(hs.len(), 1, "{hs:?}");
        assert_eq!(pos(&hs[0]).0, 1);
        // a range before everything is empty, never wrong
        assert!(hints(&w1.cx(), 0, 0).is_empty());
    }
}
