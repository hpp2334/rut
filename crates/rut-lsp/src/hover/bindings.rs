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
//! Spans are the contract: phase 2's within-file definition is a LOOKUP
//! against these records, so `decl_ident_span` must be byte-exact. Names
//! are `IdentId`s — the AST carries no name spans — so the declaring
//! identifier is recovered from the token stream (the recovery
//! `build.rs` pioneered; no parser changes). `ty` is the display text
//! (declared annotation as written, else the `infer::expr_ty`
//! heuristic); `Binding::ty_head` cuts it to the lookup head.

use rut_ast::ast::*;
use rut_lexer::span::Span;
use rut_lexer::token::{Tok, Token};

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
    /// loop statement for a for-c counter)
    pub scope_span: Span,
    pub kind: BindKind,
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
        .filter(|b| {
            b.name == name
                && b.decl_ident_span.lo <= pos
                && contains(b.scope_span, pos)
        })
        .max_by_key(|b| b.decl_ident_span.lo)
}

/// every fn-local binding in the document, in declaration order
pub fn collect(ast: &Ast, toks: &[Token], idxs: &[&DefIndex]) -> Vec<Binding> {
    let mut cx = Cx { ast, toks, idxs, out: Vec::new() };
    for h in ast.module_items(ast.root) {
        match ast.item(*h) {
            ItemKind::Fn(d) => {
                let scope = ast.span(d.body.id());
                cx.push_params(&d.params, scope);
                cx.walk_block(d.body);
            }
            ItemKind::Class { methods, .. }
            | ItemKind::Dataclass { methods, .. }
            | ItemKind::Trait { methods, .. }
            | ItemKind::Impl { methods, .. } => {
                for m in methods {
                    let d = ast.method_decl(*m);
                    if let Some(body) = d.body {
                        let scope = ast.span(body.id());
                        cx.push_params(&d.params, scope);
                        cx.walk_block(body);
                    }
                }
            }
            _ => {}
        }
    }
    cx.out
}

struct Cx<'a> {
    ast: &'a Ast,
    toks: &'a [Token],
    idxs: &'a [&'a DefIndex],
    out: Vec<Binding>,
}

impl Cx<'_> {
    fn push(&mut self, name: &str, decl: Span, scope: Span, kind: BindKind, ty: Option<String>, inferred: bool) {
        self.out.push(Binding {
            name: name.to_string(),
            decl_ident_span: decl,
            scope_span: scope,
            kind,
            ty,
            inferred,
        });
    }

    /// params of a fn/method/lambda — visible through the whole body
    fn push_params(&mut self, params: &[NodeHandle<AnyParam>], scope: Span) {
        for p in params {
            if let MemberKind::Param(d) = self.ast.param(*p) {
                let name = self.ast.name(d.name).to_string();
                let Some(decl) = ident_span(self.toks, self.ast.span(p.id()), &name) else {
                    continue;
                };
                let ty = d.ty.map(|t| ty_src(self.ast, t));
                self.push(&name, decl, scope, BindKind::Param, ty, false);
            }
        }
    }

    fn walk_block(&mut self, block: NodeHandle<BlockNode>) {
        let span = self.ast.span(block.id());
        for s in self.ast.block(block).to_vec() {
            self.walk_stmt(s, span);
        }
    }

    fn walk_stmt(&mut self, s: NodeHandle<AnyStmt>, block: Span) {
        let sp = self.ast.span(s.id());
        match self.ast.kind(s.id()) {
            Kind::Stmt(StmtKind::LetStmt { name, destructure, ty, init, .. }) => {
                let names: Vec<IdentId> = match destructure {
                    Some(ns) => ns.clone(),
                    None => vec![*name],
                };
                // per-binding annotation: a tuple type destructures
                // positionally; a single name takes the annotation whole
                let anns: Vec<Option<String>> = match (destructure, ty) {
                    (Some(_), Some(t)) => match self.ast.ty(*t) {
                        TypeKind::TyTuple { elems } => names
                            .iter()
                            .enumerate()
                            .map(|(i, _)| elems.get(i).map(|&e| ty_src(self.ast, e)))
                            .collect(),
                        _ => vec![None; names.len()],
                    },
                    (_, Some(t)) => vec![Some(ty_src(self.ast, *t)); names.len()],
                    _ => vec![None; names.len()],
                };
                for (i, n) in names.iter().enumerate() {
                    let text = self.ast.name(*n);
                    // first match = the declaration: the declaring ident
                    // precedes every use of the name inside the stmt
                    let Some(decl) = ident_span(self.toks, sp, text) else { continue };
                    let mut inferred = false;
                    let ty_text = anns[i].clone().or_else(|| {
                        let raw = match destructure {
                            Some(_) => match self.ast.expr(*init) {
                                ExprKind::Tuple { elems } => {
                                    elems.get(i).and_then(|&e| infer::expr_ty(self.ast, &self.out, self.idxs, e))
                                }
                                _ => None,
                            },
                            None => infer::expr_ty(self.ast, &self.out, self.idxs, *init),
                        };
                        if raw.is_some() {
                            inferred = true;
                        }
                        raw
                    });
                    self.push(text, decl, Span::new(decl.lo, block.hi), BindKind::Let, ty_text, inferred);
                }
                self.walk_expr(*init); // lambdas in the initializer
            }
            Kind::Stmt(StmtKind::ForOf { var, iter, body }) => {
                let body_span = self.ast.span(body.id());
                let text = self.ast.name(*var);
                if let Some(decl) = ident_span(self.toks, sp, text) {
                    let raw = infer::expr_ty(self.ast, &self.out, self.idxs, *iter).and_then(|t| elem_ty(&t));
                    let inferred = raw.is_some();
                    self.push(text, decl, Span::new(decl.lo, body_span.hi), BindKind::IterVar, raw, inferred);
                }
                self.walk_expr(*iter);
                self.walk_block(*body);
            }
            Kind::Stmt(StmtKind::ForC { var, init, cond, update, body }) => {
                let text = self.ast.name(*var);
                if let Some(decl) = ident_span(self.toks, sp, text) {
                    let raw = infer::expr_ty(self.ast, &self.out, self.idxs, *init);
                    let inferred = raw.is_some();
                    // the counter is visible through cond + update too —
                    // the scope is the whole loop statement
                    self.push(text, decl, Span::new(decl.lo, sp.hi), BindKind::IterVar, raw, inferred);
                }
                self.walk_expr(*init);
                self.walk_expr(*cond);
                self.walk_expr(*update);
                self.walk_block(*body);
            }
            Kind::Stmt(StmtKind::If { cond, then, els }) => {
                self.walk_expr(*cond);
                self.walk_block(*then);
                match els {
                    Some(ElseBranch::If(h)) => self.walk_stmt((*h).into(), block),
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
                    match self.ast.arm(*a) {
                        ArmKind::WhenArm { body, .. } => self.walk_expr(*body),
                        ArmKind::SelectArm { fut, bind, body } => {
                            // `fut as name -> body` binds `name` over the arm
                            if let Some(b) = bind {
                                let body_span = self.ast.span(body.id());
                                let text = self.ast.name(*b);
                                if let Some(decl) = ident_span(self.toks, self.ast.span(a.id()), text) {
                                    self.push(text, decl, Span::new(decl.lo, body_span.hi), BindKind::Let, None, false);
                                }
                            }
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
        match self.ast.expr(e) {
            ExprKind::Block { stmts } => {
                let span = self.ast.span(e.id());
                for s in stmts {
                    self.walk_stmt(*s, span);
                }
            }
            ExprKind::Lambda { params, body, ret: _ } => {
                let span = self.ast.span(body.id());
                self.push_params(params, span);
                self.walk_expr(*body);
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
pub(crate) fn expr_children(ast: &Ast, e: NodeHandle<AnyExpr>) -> Vec<NodeHandle<AnyExpr>> {
    fn all(out: &mut Vec<NodeHandle<AnyExpr>>, hs: &[NodeHandle<AnyExpr>]) {
        out.extend_from_slice(hs);
    }
    fn arm_bodies(ast: &Ast, arms: &[NodeHandle<AnyArm>], out: &mut Vec<NodeHandle<AnyExpr>>) {
        for a in arms {
            match ast.arm(*a) {
                ArmKind::WhenArm { body, .. } => out.push(*body),
                ArmKind::SelectArm { fut, bind: _, body } => {
                    out.push(*fut);
                    out.push(*body);
                }
            }
        }
    }
    let mut out: Vec<NodeHandle<AnyExpr>> = Vec::new();
    match ast.expr(e) {
        ExprKind::Call { callee, args } => {
            out.push(*callee);
            all(&mut out, args);
        }
        ExprKind::Method { recv, args, .. } => {
            out.push(*recv);
            all(&mut out, args);
        }
        ExprKind::Field { recv, .. } => out.push(*recv),
        ExprKind::Index { recv, idx } => {
            out.push(*recv);
            out.push(*idx);
        }
        ExprKind::Unary { expr, .. } | ExprKind::Try { expr } | ExprKind::Await { expr } => out.push(*expr),
        ExprKind::Binary { lhs, rhs, .. } => {
            out.push(*lhs);
            out.push(*rhs);
        }
        ExprKind::Assign { target, value, .. } => {
            out.push(*target);
            out.push(*value);
        }
        ExprKind::Struct { fields, .. } => {
            for (_, v) in fields {
                out.push(*v);
            }
        }
        ExprKind::Tuple { elems } | ExprKind::ArrayLit { elems } => all(&mut out, elems),
        ExprKind::ArrayRepeat { value, count } => {
            out.push(*value);
            out.push(*count);
        }
        ExprKind::WhenExpr { scrut, arms } => {
            out.push(*scrut);
            arm_bodies(ast, arms, &mut out);
        }
        ExprKind::Select { arms } => arm_bodies(ast, arms, &mut out),
        ExprKind::Is { expr, .. } | ExprKind::Cast { expr, .. } => out.push(*expr),
        // `Block`/`Lambda` never reach here (the walker handles them);
        // literals and paths have no children. FStr holes are raw token
        // streams — not walked (a lambda inside an f-string hole keeps
        // its bindings unrecorded; a miss, never wrong text)
        _ => {}
    }
    out
}

/// the byte span of `name`'s declaring identifier inside `span` — the
/// first non-keyword `Ident` token matching the name. The declaring
/// ident precedes every use of the name inside the decl node's own
/// span, so first-match IS the declaration. This token recovery is the
/// whole span trick (names are `IdentId`s — no parser changes).
pub(crate) fn ident_span(toks: &[Token], span: Span, name: &str) -> Option<Span> {
    toks.iter()
        .filter(|t| t.span.lo >= span.lo && t.span.hi <= span.hi)
        .find_map(|t| match &t.tok {
            Tok::Ident(s) if s == name && !rut_parser::is_reserved_kw(s) => Some(t.span),
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
