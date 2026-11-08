//! The local binding pass (the lsp-features survey §3.2 design) —
//! span-first records of every fn-local binding:
//!
//! ```text
//! { name, decl_ident_span, scope_span, kind: Let|Param|IterVar, ty }
//! ```
//!
//! built in one walk over fn/method bodies (lambdas included). Resolution
//! is the rule `infer_local`'s flat scan lacked: **the latest declaration
//! before the cursor among bindings whose scope still contains it** —
//! shadow-correct (a re-declared name wins inside its scope) and
//! scope-exit correct (a binding's scope ends with its introducing
//! block, so it cannot leak past it).
//!
//! Spans are the contract: within-file definition is a LOOKUP
//! against these records, so `decl_ident_span` must be byte-exact. The
//! decl nodes record their `name_span`s (v2's parse verdict); a
//! destructured let's EXTRA binding names still recover from the token
//! stream (`ident_span` — the one recovery left, the whole span trick
//! this pass pioneered). `ty` is the display text (declared annotation
//! as written, else the `infer::expr_ty` heuristic);
//! `Binding::ty_head` cuts it to the lookup head.

use rut_ast::{
    Ast, BlockNode, ElseBranch, ExprRef, FnNode, Handle, ItemRef, LetStmtNode, MethodRef, ParamRef,
    StmtRef, TyRef, TyTupleNode,
};
use rut_semantic::Span;
use rut_semantic::{Tok, Token};

use super::infer;
use super::lookup::contains;
use super::types::{ty_src, DefIndex};

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

#[derive(Debug, Clone)]
pub struct Binding {
    pub name: String,
    /// byte span of the declaring identifier token — phase 2's
    /// definition target
    pub decl_ident_span: Span,
    /// where the binding is visible: from the declaring ident to the
    /// end of its introducing block (fn/lambda body, loop, the whole
    /// loop statement for a for-c counter). A block the document ends
    /// inside extends through the source end (`Cx::scope_of`) — the
    /// cursor typing at the EOF stays inside its block's scope.
    pub scope_span: Span,
    pub kind: BindKind,
    /// the header spelled `mut` — the mut-binding law's flag, carried so
    /// the hover renders the binding as it binds (`let mut x`)
    pub is_mut: bool,
    /// display text of the declared or inferred type (`Circle`,
    /// `?Circle`, `Vec<i32>`, `f64`); `None` when nothing derives
    pub ty: Option<String>,
    /// true when `ty` came from inference rather than a written
    /// annotation — the hover says so
    pub inferred: bool,
}

impl Binding {
    /// the survey record's `ty_head` — the lookup head a member query
    /// keys on (the `?` strips, generics cut at `<`)
    pub fn ty_head(&self) -> Option<String> {
        self.ty.as_deref().map(super::types::head_of_ty)
    }
}

/// the shadow-correct, scope-exit-correct resolve: the LATEST declaring
/// ident at-or-before `pos` among bindings whose scope contains `pos`
pub fn resolve<'b>(bs: &'b [Binding], pos: u32, name: &str) -> Option<&'b Binding> {
    bs.iter()
        .filter(|b| b.name == name && b.decl_ident_span.lo <= pos && contains(b.scope_span, pos))
        .max_by_key(|b| b.decl_ident_span.lo)
}

/// every fn-local binding in the document, in declaration order
pub fn collect(ast: &Ast, toks: &[Token], idxs: &[&DefIndex]) -> Vec<Binding> {
    // the source end — the lexer rides an EOF token last, its hi the
    // normalized length; `scope_of` extends unterminated blocks past it
    let src_end = toks.last().map(|t| t.span.hi).unwrap_or(0);
    let mut cx = Cx {
        ast,
        toks,
        idxs,
        src_end,
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
    ast: &'a Ast,
    toks: &'a [Token],
    idxs: &'a [&'a DefIndex],
    /// the source end (the EOF token's hi) — `scope_of`'s extension target
    src_end: u32,
    out: Vec<Binding>,
}

impl Cx<'_> {
    #[allow(clippy::too_many_arguments)] // a binding row IS this many fields
    fn push(
        &mut self,
        name: &str,
        decl: Span,
        scope: Span,
        kind: BindKind,
        is_mut: bool,
        ty: Option<String>,
        inferred: bool,
    ) {
        self.out.push(Binding {
            name: name.to_string(),
            decl_ident_span: decl,
            scope_span: scope,
            kind,
            is_mut,
            ty,
            inferred,
        });
    }

    /// the visibility span for a binding introduced in `span`'s region —
    /// the span's own hi, EXCEPT when the span still holds an open `{` at
    /// its end. That is the end-of-buffer hole: a document that ends
    /// inside an incomplete statement leaves its enclosing block
    /// unclosed, the parser ends the block's span at the EOF token, and
    /// the cursor typing at the source end sits exactly ON that hi —
    /// which `contains`' exclusive test drops, so every binding of the
    /// block (and with it member completion on the receiver, the locals
    /// tier, go-to-def) resolves to nothing. Such a scope extends one
    /// past the source end, so the EOF position stays inside; a block
    /// that closed keeps its exact span — nothing leaks past a real `}`.
    fn scope_of(&self, span: Span) -> Span {
        let hi = if self.open_brace_at_end(span) {
            self.src_end + 1
        } else {
            span.hi
        };
        Span::new(span.lo, hi)
    }

    /// whether `span` ends with an unclosed `{`: counting the braces of
    /// the token slice inside the span, the depth never returns to zero
    /// after the first opener. Token-backed (like `ident_span`'s
    /// recovery — no parser changes): comments never lex, strings and
    /// f-strings ride as single tokens so their braces stay out, and a
    /// closed block's own `}` IS inside the span (block spans end at
    /// their own closer — the exact-span law). A span with no braces at
    /// all (an expression lambda's body) is never "open" — depth zero
    /// means keep the span.
    fn open_brace_at_end(&self, span: Span) -> bool {
        let mut depth = 0i32;
        for t in self
            .toks
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
        for p in params {
            let ParamRef::Param(h) = p else {
                continue; // `self` / `mut self` — a keyword, never a binding
            };
            let d = h.get(self.ast);
            let name = self.ast.name(d.name).to_string();
            let Some(decl) = ident_span(self.toks, d.span, &name) else {
                continue;
            };
            let ty = d.ty.map(|t| ty_src(self.ast, t));
            self.push(&name, decl, scope, BindKind::Param, d.is_mut, ty, false);
        }
    }

    fn walk_methods(&mut self, methods: &[MethodRef]) {
        for m in methods {
            let MethodRef::Method(h) = m else {
                continue;
            };
            let d = h.get(self.ast);
            if let Some(body) = d.body {
                let scope = self.scope_of(body.span(self.ast));
                self.push_params(&d.params, scope);
                self.walk_block(body);
            }
        }
    }

    fn walk_block(&mut self, block: Handle<BlockNode>) {
        let span = self.scope_of(block.span(self.ast));
        for s in block.get(self.ast).stmts.iter().copied() {
            self.walk_stmt(s, span);
        }
        // the block's tail value is in the same scope
        if let Some(t) = block.get(self.ast).tail {
            self.walk_expr(t);
        }
    }

    fn walk_stmt(&mut self, s: StmtRef, block: Span) {
        match s {
            StmtRef::Let(h) => self.walk_let(h, block),
            StmtRef::ForOf(h) => {
                let d = h.get(self.ast);
                let sp = d.span;
                let body_span = self.scope_of(d.body.span(self.ast));
                let text = self.ast.name(d.var);
                if let Some(decl) = ident_span(self.toks, sp, text) {
                    let raw = infer::expr_ty(self.ast, &self.out, self.idxs, d.iter)
                        .and_then(|t| elem_ty(&t));
                    let inferred = raw.is_some();
                    self.push(
                        text,
                        decl,
                        Span::new(decl.lo, body_span.hi),
                        BindKind::IterVar,
                        d.is_mut,
                        raw,
                        inferred,
                    );
                }
                self.walk_expr(d.iter);
                self.walk_block(d.body);
            }
            StmtRef::ForC(h) => {
                let d = h.get(self.ast);
                let sp = d.span;
                let text = self.ast.name(d.var);
                if let Some(decl) = ident_span(self.toks, sp, text) {
                    let raw = infer::expr_ty(self.ast, &self.out, self.idxs, d.init);
                    let inferred = raw.is_some();
                    // the counter is visible through cond + update too —
                    // the scope is the whole loop statement (extended
                    // through an end-of-buffer truncation, `scope_of`)
                    let scope = self.scope_of(sp);
                    self.push(
                        text,
                        decl,
                        Span::new(decl.lo, scope.hi),
                        BindKind::IterVar,
                        d.is_mut,
                        raw,
                        inferred,
                    );
                }
                self.walk_expr(d.init);
                self.walk_expr(d.cond);
                self.walk_expr(d.update);
                self.walk_block(d.body);
            }
            StmtRef::If(h) => {
                let d = h.get(self.ast);
                self.walk_expr(d.cond);
                self.walk_block(d.then);
                match d.els {
                    Some(ElseBranch::If(ih)) => self.walk_stmt(StmtRef::If(ih), block),
                    Some(ElseBranch::Block(b)) => self.walk_block(b),
                    None => {}
                }
            }
            StmtRef::While(h) => {
                let d = h.get(self.ast);
                self.walk_expr(d.cond);
                self.walk_block(d.body);
            }
            StmtRef::When(h) => {
                let d = h.get(self.ast);
                self.walk_expr(d.scrut);
                for a in &d.arms {
                    self.walk_expr(a.get(self.ast).body);
                }
            }
            StmtRef::Return(h) => {
                if let Some(v) = h.get(self.ast).value {
                    self.walk_expr(v);
                }
            }
            StmtRef::Expr(e) => self.walk_expr(e),
            StmtRef::Break(_) | StmtRef::Continue(_) | StmtRef::Error(_) => {}
        }
    }

    fn walk_let(&mut self, h: Handle<LetStmtNode>, block: Span) {
        let d = h.get(self.ast);
        let sp = d.span;
        let names: Vec<rut_ast::IdentId> = match &d.destructure {
            Some(ns) => ns.clone(),
            None => vec![d.name],
        };
        // per-binding annotation: a tuple type destructures
        // positionally; a single name takes the annotation whole
        let anns: Vec<Option<String>> = match (&d.destructure, d.ty) {
            (Some(_), Some(TyRef::Tuple(th))) => {
                let tup: &TyTupleNode = th.get(self.ast);
                names
                    .iter()
                    .enumerate()
                    .map(|(i, _)| tup.elems.get(i).map(|&e| ty_src(self.ast, e)))
                    .collect()
            }
            (_, Some(t)) => vec![Some(ty_src(self.ast, t)); names.len()],
            _ => vec![None; names.len()],
        };
        for (i, n) in names.iter().enumerate() {
            let text = self.ast.name(*n);
            // first match = the declaration: the declaring ident
            // precedes every use of the name inside the stmt
            let Some(decl) = ident_span(self.toks, sp, text) else {
                continue;
            };
            let mut inferred = false;
            let ty_text = anns[i].clone().or_else(|| {
                let raw = match &d.destructure {
                    Some(_) => match d.init {
                        ExprRef::Tuple(th) => th
                            .get(self.ast)
                            .elems
                            .get(i)
                            .and_then(|&e| infer::expr_ty(self.ast, &self.out, self.idxs, e)),
                        _ => None,
                    },
                    None => infer::expr_ty(self.ast, &self.out, self.idxs, d.init),
                };
                if raw.is_some() {
                    inferred = true;
                }
                raw
            });
            self.push(
                text,
                decl,
                Span::new(decl.lo, block.hi),
                BindKind::Let,
                d.is_mut,
                ty_text,
                inferred,
            );
        }
        self.walk_expr(d.init); // lambdas in the initializer
    }

    fn walk_expr(&mut self, e: ExprRef) {
        match e {
            ExprRef::Block(bh) => self.walk_block(bh),
            ExprRef::Lambda(h) => {
                let d = h.get(self.ast);
                let span = self.scope_of(d.body.span(self.ast));
                self.push_params(&d.params, span);
                self.walk_expr(d.body);
            }
            _ => {
                for c in expr_children(self.ast, e) {
                    self.walk_expr(c);
                }
            }
        }
    }
}

/// direct expression children — the scope walk's recursion edges
/// (`Block`/`Lambda` are handled by the walker itself). Shared with the
/// inlay hints' call-site walk (one recursion rule, two consumers —
/// the `member_target` precedent)
pub(crate) fn expr_children(ast: &Ast, e: ExprRef) -> Vec<ExprRef> {
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
        ExprRef::Try(h) => out.push(h.get(ast).expr),
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
            for (_, v) in &d.fields {
                out.push(*v);
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

/// the byte span of `name`'s declaring identifier inside `span` — the
/// first non-keyword `Ident` token matching the name. The declaring
/// ident precedes every use of the name inside the decl node's own
/// span, so first-match IS the declaration. The decl nodes record
/// their own `name_span`s; this recovery remains for the names the
/// nodes do not carry (a destructured let's extra bindings).
pub(crate) fn ident_span(toks: &[Token], span: Span, name: &str) -> Option<Span> {
    toks.iter()
        .filter(|t| t.span.lo >= span.lo && t.span.hi <= span.hi)
        .find_map(|t| match &t.tok {
            Tok::Ident(s) if s == name && !rut_semantic::is_reserved_kw(s) => Some(t.span),
            _ => None,
        })
}

/// the element type of a sequence type text — the survey's for-of rule:
/// `[T]` → `T`, `Vec<T>` → `T` (first generic argument). `None` when the
/// text isn't a sequence spelling (a bare head iterates nothing we can
/// name — a miss, never wrong members)
pub(crate) fn elem_ty(text: &str) -> Option<String> {
    let t = text.trim_start_matches('?');
    if t.starts_with('[') && t.ends_with(']') {
        return Some(t[1..t.len() - 1].trim().to_string());
    }
    let open = t.find('<')?;
    if !t.ends_with('>') {
        return None;
    }
    let inner = &t[open + 1..t.len() - 1];
    let mut depth = 0i32;
    for (i, c) in inner.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => return Some(inner[..i].trim().to_string()),
            _ => {}
        }
    }
    Some(inner.trim().to_string())
}
