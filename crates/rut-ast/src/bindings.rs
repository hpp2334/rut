//! The local binding pass — `Doc::bindings()`, lazy per version. One
//! walk over fn/method/lambda bodies records every local binding:
//!
//! ```text
//! { name, decl_ident_span, scope_span, kind, is_const, ty_span, init }
//! ```
//!
//! TREE+TEXT FACTS ONLY: scopes come from block/loop NODE spans, the
//! declaring idents from the nodes' recorded `name_span`s (the
//! destructured let carries its per-member spans since the parser
//! change), and `ty_span` is the WRITTEN annotation. Inference is NOT
//! this pass — an un-annotated binding's type consults the resolver
//! (`init` rides along for it); the rows carry declared annotations
//! only. Resolution is the shadow-correct rule: the LATEST declaration
//! at-or-before the cursor among bindings whose scope still contains
//! it.
//!
//! One display corner keeps a token read (inside the Doc — this IS the
//! token store's crate): a document that ends inside an incomplete
//! statement leaves its enclosing block unclosed, and the cursor typing
//! at the source end sits ON the block span's hi — which the exclusive
//! containment drops. Such a scope extends one past the source end.

use rut_core::IdentId;
use rut_lexer::span::Span;
use rut_lexer::token::Tok;

use crate::{
    BlockNode, ElseBranch, ExprRef, FnNode, Handle, ItemRef, LetStmtNode, MethodRef, ParamRef,
    StmtRef, TyRef, TyTupleNode,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindKind {
    Let,
    Param,
    /// `for (let v of xs)` / `for (let i = 0; ..)` loop variables
    IterVar,
}

impl BindKind {
    /// the hover's descriptor line
    pub fn label(self) -> &'static str {
        match self {
            BindKind::Let => "let binding",
            BindKind::Param => "parameter",
            BindKind::IterVar => "loop variable",
        }
    }
}

/// one local binding — the row the pass records
#[derive(Debug, Clone)]
pub struct Binding {
    pub name: IdentId,
    /// byte span of the declaring identifier token — the definition
    /// target
    pub decl_ident_span: Span,
    /// where the binding is visible: from the declaring ident to the
    /// end of its introducing block (fn/lambda body, loop, the whole
    /// loop statement for a for-c counter)
    pub scope_span: Span,
    pub kind: BindKind,
    /// the head spelled `const` — the shallow-const law's flag
    pub is_const: bool,
    /// the WRITTEN annotation's own span — `None` when the binding
    /// carries none. Text is the `Doc::text` join; inference for the
    /// un-annotated rows is the resolver's lane (`init`).
    pub ty_span: Option<Span>,
    /// the initializer expression — the resolver's inference handle
    /// (`None` for params)
    pub init: Option<ExprRef>,
    /// the row's index within a destructured let (`let (a, b) = ..` —
    /// the positional key the resolver splits a tuple initializer by)
    pub destructure_index: Option<usize>,
}

/// the shadow-correct, scope-exit-correct resolve: the LATEST declaring
/// ident at-or-before `pos` among bindings whose scope contains `pos`
pub fn resolve(bs: &[Binding], pos: u32, name: IdentId) -> Option<&Binding> {
    bs.iter()
        .filter(|b| b.name == name && b.decl_ident_span.lo <= pos && contains(b.scope_span, pos))
        .max_by_key(|b| b.decl_ident_span.lo)
}

/// half-open span containment — the scopes' containment test
pub fn contains(sp: Span, pos: u32) -> bool {
    sp.lo <= pos && pos < sp.hi
}

/// every fn-local binding in the document, in declaration order. The
/// walk is pure tree+text: scopes from node spans, annotations from
/// written spans, names from recorded name spans.
pub(crate) fn compute(doc: &crate::Doc) -> Vec<Binding> {
    let ast = doc.ast();
    let mut cx = Cx {
        doc,
        out: Vec::new(),
    };
    for item in ast.root().items(ast) {
        match item {
            ItemRef::Fn(h) => {
                let d: &FnNode = h.get(ast);
                let scope = cx.scope_of(d.body.span(ast));
                cx.push_params(&d.params, scope);
                cx.walk_block(d.body);
            }
            ItemRef::Class(h) => cx.walk_methods(&h.get(ast).methods),
            ItemRef::Struct(h) => cx.walk_methods(&h.get(ast).methods),
            ItemRef::Interface(h) => cx.walk_methods(&h.get(ast).methods),
            ItemRef::Impl(h) => cx.walk_methods(&h.get(ast).methods),
            _ => {}
        }
    }
    cx.out
}

struct Cx<'a> {
    doc: &'a crate::Doc,
    out: Vec<Binding>,
}

impl Cx<'_> {
    #[allow(clippy::too_many_arguments)] // a binding row IS this many fields
    fn push(
        &mut self,
        name: IdentId,
        decl: Span,
        scope: Span,
        kind: BindKind,
        is_const: bool,
        ty_span: Option<Span>,
        init: Option<ExprRef>,
        destructure_index: Option<usize>,
    ) {
        self.out.push(Binding {
            name,
            decl_ident_span: decl,
            scope_span: scope,
            kind,
            is_const,
            ty_span,
            init,
            destructure_index,
        });
    }

    /// the visibility span for a binding introduced in `span`'s region —
    /// the span's own hi, EXCEPT when the document ends inside the
    /// region (the source-end hole, see the module docs): such a scope
    /// extends one past the source end, so the EOF position stays
    /// inside; a block that closed keeps its exact span — nothing leaks
    /// past a real `}`.
    fn scope_of(&self, span: Span) -> Span {
        let src_end = self.doc.len();
        let hi = if span.hi >= src_end && self.open_brace_at_end(span) {
            src_end + 1
        } else {
            span.hi
        };
        Span::new(span.lo, hi)
    }

    /// whether `span` ends unclosed: the depth over the tokens inside
    /// the span never returns to zero after the first opener. (Inside
    /// the Doc — the token store's own crate. Comments never lex,
    /// strings and f-strings ride as single tokens so their braces stay
    /// out, and a closed block's own `}` IS inside the span.)
    fn open_brace_at_end(&self, span: Span) -> bool {
        let mut depth = 0i32;
        for t in self
            .doc
            .tokens_in(span)
            .iter()
            .filter(|t| t.span.lo >= span.lo && t.span.hi <= span.hi)
        {
            match t.tok {
                Tok::LBrace => depth += 1,
                Tok::RBrace => {
                    depth -= 1;
                    if depth <= 0 {
                        return false; // the block's own closer — closed
                    }
                }
                _ => {}
            }
        }
        depth > 0
    }

    /// params of a fn/method/lambda — visible through the whole body
    fn push_params(&mut self, params: &[ParamRef], scope: Span) {
        let ast = self.doc.ast();
        for p in params {
            let ParamRef::Param(h) = p else {
                continue; // `self` — a keyword, never a binding
            };
            let d = h.get(ast);
            let ty_span = d.ty.map(|t| t.span(ast));
            self.push(
                d.name.ident,
                d.name.span,
                scope,
                BindKind::Param,
                false,
                ty_span,
                None,
                None,
            );
        }
    }

    fn walk_methods(&mut self, methods: &[MethodRef]) {
        let ast = self.doc.ast();
        for m in methods {
            let MethodRef::Method(h) = m else {
                continue;
            };
            let d = h.get(ast);
            if let Some(body) = d.body {
                let scope = self.scope_of(body.span(ast));
                self.push_params(&d.params, scope);
                self.walk_block(body);
            }
        }
    }

    fn walk_block(&mut self, block: Handle<BlockNode>) {
        let ast = self.doc.ast();
        let span = self.scope_of(block.span(ast));
        for s in block.get(ast).stmts.iter().copied() {
            self.walk_stmt(s, span);
        }
        // the block's tail value is in the same scope
        if let Some(t) = block.get(ast).tail {
            self.walk_expr(t);
        }
    }

    fn walk_stmt(&mut self, s: StmtRef, block: Span) {
        let ast = self.doc.ast();
        match s {
            StmtRef::Let(h) => self.walk_let(h, block),
            StmtRef::ForOf(h) => {
                let d = h.get(ast);
                let body_span = self.scope_of(d.body.span(ast));
                self.push(
                    d.var.ident,
                    d.var.span,
                    Span::new(d.var.span.lo, body_span.hi),
                    BindKind::IterVar,
                    d.is_const,
                    None,
                    Some(d.iter),
                    None,
                );
                self.walk_expr(d.iter);
                self.walk_block(d.body);
            }
            StmtRef::ForC(h) => {
                let d = h.get(ast);
                // the counter is visible through cond + update too —
                // the scope is the whole loop statement (extended
                // through an end-of-buffer truncation, `scope_of`)
                let scope = self.scope_of(d.span);
                self.push(
                    d.var.ident,
                    d.var.span,
                    Span::new(d.var.span.lo, scope.hi),
                    BindKind::IterVar,
                    false,
                    None,
                    Some(d.init),
                    None,
                );
                self.walk_expr(d.init);
                self.walk_expr(d.cond);
                self.walk_expr(d.update);
                self.walk_block(d.body);
            }
            StmtRef::If(h) => {
                let d = h.get(ast);
                self.walk_expr(d.cond);
                self.walk_block(d.then);
                match d.els {
                    Some(ElseBranch::If(ih)) => self.walk_stmt(StmtRef::If(ih), block),
                    Some(ElseBranch::Block(b)) => self.walk_block(b),
                    None => {}
                }
            }
            StmtRef::While(h) => {
                let d = h.get(ast);
                self.walk_expr(d.cond);
                self.walk_block(d.body);
            }
            StmtRef::When(h) => {
                let d = h.get(ast);
                self.walk_expr(d.scrut);
                for a in &d.arms {
                    self.walk_expr(a.get(ast).body);
                }
            }
            StmtRef::Return(h) => {
                if let Some(v) = h.get(ast).value {
                    self.walk_expr(v);
                }
            }
            StmtRef::Expr(e) => self.walk_expr(e),
            StmtRef::Break(_) | StmtRef::Continue(_) | StmtRef::Error(_) => {}
        }
    }

    fn walk_let(&mut self, h: Handle<LetStmtNode>, block: Span) {
        let ast = self.doc.ast();
        let d = h.get(ast);
        let n_names = d.destructure.as_ref().map(|ns| ns.len()).unwrap_or(1);
        // per-binding annotation SPAN: a tuple type destructures
        // positionally; a single name takes the annotation whole
        let anns: Vec<Option<Span>> = match (&d.destructure, d.ty) {
            (Some(_), Some(TyRef::Tuple(th))) => {
                let tup: &TyTupleNode = th.get(ast);
                (0..n_names)
                    .map(|i| tup.elems.get(i).map(|&e| e.span(ast)))
                    .collect()
            }
            (_, Some(t)) => vec![Some(t.span(ast)); n_names],
            _ => vec![None; n_names],
        };
        let names: &[crate::Name] = match &d.destructure {
            Some(ns) => ns,
            None => &[d.name],
        };
        for (i, decl) in names.iter().enumerate() {
            self.push(
                decl.ident,
                decl.span,
                Span::new(decl.span.lo, block.hi),
                BindKind::Let,
                d.is_const,
                anns.get(i).copied().flatten(),
                Some(d.init),
                d.destructure.as_ref().map(|_| i),
            );
        }
        self.walk_expr(d.init); // lambdas in the initializer
    }

    fn walk_expr(&mut self, e: ExprRef) {
        let ast = self.doc.ast();
        match e {
            ExprRef::Block(bh) => self.walk_block(bh),
            ExprRef::Lambda(h) => {
                let d = h.get(ast);
                let span = self.scope_of(d.body.span(ast));
                self.push_params(&d.params, span);
                self.walk_expr(d.body);
            }
            _ => {
                for c in expr_children(ast, e) {
                    self.walk_expr(c);
                }
            }
        }
    }
}

/// direct expression children — the scope walk's recursion edges
/// (`Block`/`Lambda` are handled by the walker itself). Shared with the
/// inlay hints' call-site walk (one recursion rule, two consumers).
pub fn expr_children(ast: &crate::Ast, e: ExprRef) -> Vec<ExprRef> {
    let mut out: Vec<ExprRef> = Vec::new();
    match e {
        ExprRef::Call(h) => {
            let d = h.get(ast);
            out.push(d.callee);
            out.extend(d.args.iter().copied());
        }
        ExprRef::Method(h) => {
            let d = h.get(ast);
            out.push(d.recv);
            out.extend(d.args.iter().copied());
        }
        ExprRef::Field(h) => out.push(h.get(ast).recv),
        ExprRef::Index(h) => {
            let d = h.get(ast);
            out.push(d.recv);
            out.push(d.idx);
        }
        ExprRef::Unary(h) => out.push(h.get(ast).expr),
        ExprRef::Await(h) => out.push(h.get(ast).expr),
        ExprRef::Binary(h) => {
            let d = h.get(ast);
            out.push(d.lhs);
            out.push(d.rhs);
        }
        ExprRef::Assign(h) => {
            let d = h.get(ast);
            out.push(d.target);
            out.push(d.value);
        }
        ExprRef::Struct(h) => {
            let d = h.get(ast);
            for fi in &d.fields {
                out.push(fi.value);
            }
        }
        ExprRef::Tuple(h) => out.extend(h.get(ast).elems.iter().copied()),
        ExprRef::ArrayLit(h) => out.extend(h.get(ast).elems.iter().copied()),
        ExprRef::ArrayRepeat(h) => {
            let d = h.get(ast);
            out.push(d.value);
            out.push(d.count);
        }
        ExprRef::WhenExpr(h) => {
            let d = h.get(ast);
            out.push(d.scrut);
            for a in &d.arms {
                out.push(a.get(ast).body);
            }
        }
        ExprRef::If(h) => {
            let d = h.get(ast);
            out.push(d.cond);
            out.push(ExprRef::Block(d.then));
            // `else if` links hold if STATEMENTS (the shared ElseBranch):
            // flattened — each link's condition and block join the child
            // list so the walk descends uniformly (blocks are expression
            // nodes; the walker's Block arm resumes through them)
            let mut link = d.els;
            while let Some(e) = link {
                match e {
                    ElseBranch::Block(b) => {
                        out.push(ExprRef::Block(b));
                        break;
                    }
                    ElseBranch::If(ih) => {
                        let nd = ih.get(ast);
                        out.push(nd.cond);
                        out.push(ExprRef::Block(nd.then));
                        link = nd.els;
                    }
                }
            }
        }
        ExprRef::Is(h) => out.push(h.get(ast).expr),
        ExprRef::Cast(h) => out.push(h.get(ast).expr),
        // `Block`/`Lambda` never reach here (the walker handles them);
        // literals and paths have no children. FStr holes are raw token
        // streams as far as this pass is concerned — not walked (a
        // lambda inside an f-string hole keeps its bindings unrecorded;
        // a miss, never wrong text)
        _ => {}
    }
    out
}
