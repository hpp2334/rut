//! Cheap local type inference for a lowercase receiver: the enclosing
//! fn/method's params, then `let` bindings before the cursor.

use rut_ast::ast::*;
use rut_lexer::span::Span;

use super::lookup::contains;
use super::types::ty_head;

/// cheap local inference for a lowercase receiver: the enclosing
/// fn/method's params (`fn f(c: Circle)`), then `let` bindings before
/// `pos` (`let p = Circle.new(..)`, `let p: Point = ..`, `let s = ".."`)
pub(crate) fn infer_local(_src: &str, ast: &Ast, pos: u32, name: &str) -> Option<String> {
    // locate the enclosing fn/method body — smallest containing span
    let mut body: Option<(u32, NodeHandle<AnyExpr>)> = None;
    let mut see = |sp: Span, b: Option<NodeHandle<BlockNode>>| {
        if let Some(b) = b {
            if contains(sp, pos) && body.map(|(l, _)| sp.hi - sp.lo < l).unwrap_or(true) {
                body = Some((sp.hi - sp.lo, b.into()));
            }
        }
    };
    for h in ast.module_items(ast.root) {
        let sp = ast.span(h.id());
        match ast.item(*h) {
            ItemKind::Fn(d) => see(sp, Some(d.body)),
            ItemKind::Class { methods, .. }
            | ItemKind::Dataclass { methods, .. }
            | ItemKind::Trait { methods, .. }
            | ItemKind::Impl { methods, .. } => {
                for m in methods {
                    see(ast.span(m.id()), ast.method_decl(*m).body);
                }
            }
            _ => {}
        }
    }
    let (_, block) = body?;

    // params of that fn/method
    for h in ast.module_items(ast.root) {
        let sp = ast.span(h.id());
        if !contains(sp, pos) {
            continue;
        }
        match ast.item(*h) {
            ItemKind::Fn(d) => {
                if contains(sp, pos) {
                    if let Some(t) = param_ty(ast, &d.params, name) {
                        return Some(t);
                    }
                }
            }
            ItemKind::Class { methods, .. }
            | ItemKind::Dataclass { methods, .. }
            | ItemKind::Trait { methods, .. }
            | ItemKind::Impl { methods, .. } => {
                for m in methods {
                    let msp = ast.span(m.id());
                    if contains(msp, pos) {
                        let d = ast.method_decl(*m);
                        if let Some(t) = param_ty(ast, &d.params, name) {
                            return Some(t);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // lets before pos within the body (flat scan; heuristic by design)
    let mut found: Option<String> = None;
    walk_lets(ast, block, &mut |stmt| {
        let sp = ast.span(stmt.id());
        if sp.lo < pos {
            if let Kind::Stmt(StmtKind::LetStmt { name: n, ty, init, .. }) = ast.kind(stmt.id()) {
                if ast.name(*n) == name && found.is_none() {
                    if let Some(t) = ty {
                        found = Some(ty_head(ast, *t));
                    } else if let Some(inferred) = init_ty(ast, *init) {
                        found = Some(inferred);
                    }
                }
            }
        }
    });
    found
}

fn param_ty(ast: &Ast, params: &[NodeHandle<AnyParam>], name: &str) -> Option<String> {
    for p in params {
        if let MemberKind::Param(d) = ast.param(*p) {
            if ast.name(d.name) == name {
                if let Some(ty) = d.ty {
                    return Some(ty_head(ast, ty));
                }
            }
        }
    }
    None
}

/// the type an initializer suggests: `Circle.new(..)`, `Circle { .. }`,
/// string literals
fn init_ty(ast: &Ast, init: NodeHandle<AnyExpr>) -> Option<String> {
    match ast.expr(init) {
        ExprKind::Method { recv, name, .. } if ast.name(*name) == "new" => {
            if let ExprKind::Path { segs } = ast.expr(*recv) {
                segs.first().map(|s| ast.name(s.name).to_string())
            } else {
                None
            }
        }
        ExprKind::Struct { ty, .. } => Some(ty_head(ast, *ty)),
        ExprKind::Lit(Lit::Str(_)) => Some("str".to_string()),
        _ => None,
    }
}

/// visit every LetStmt in the block tree under `node`
fn walk_lets(ast: &Ast, node: NodeHandle<AnyExpr>, f: &mut dyn FnMut(NodeHandle<AnyStmt>)) {
    if let ExprKind::Block { stmts } = ast.expr(node) {
        for s in stmts {
            if let Kind::Stmt(StmtKind::LetStmt { .. }) = ast.kind(s.id()) {
                f(*s);
            }
            stmt_exprs(ast, *s, &mut |e| walk_lets(ast, e, f));
        }
    }
}

/// expressions held by a statement (conditions, inits, bodies)
fn stmt_exprs(ast: &Ast, s: NodeHandle<AnyStmt>, f: &mut dyn FnMut(NodeHandle<AnyExpr>)) {
    match ast.kind(s.id()) {
        Kind::Stmt(StmtKind::LetStmt { init, .. }) => f(*init),
        Kind::Stmt(StmtKind::If { cond, then, els }) => {
            f(*cond);
            f(NodeHandle::<AnyExpr>::from(*then));
            if let Some(ElseBranch::Block(b)) = els {
                f(NodeHandle::<AnyExpr>::from(*b));
            }
        }
        Kind::Stmt(StmtKind::While { cond, body, .. }) => {
            f(*cond);
            f(NodeHandle::<AnyExpr>::from(*body));
        }
        Kind::Stmt(StmtKind::ForOf { iter, body, .. }) => {
            f(*iter);
            f(NodeHandle::<AnyExpr>::from(*body));
        }
        Kind::Stmt(StmtKind::ForC { init, cond, update, body, .. }) => {
            f(*init);
            f(*cond);
            f(*update);
            f(NodeHandle::<AnyExpr>::from(*body));
        }
        Kind::Stmt(StmtKind::Return { value }) => {
            if let Some(v) = value {
                f(*v);
            }
        }
        Kind::Stmt(StmtKind::WhenStmt { scrut, arms }) => {
            f(*scrut);
            for a in arms {
                if let ArmKind::WhenArm { body, .. } = ast.arm(*a) {
                    f(*body);
                }
            }
        }
        Kind::Stmt(StmtKind::ExprStmt(e)) => f(*e),
        _ => {}
    }
}
