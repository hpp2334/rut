//! The v1-shaped test shorthand over the entry pair — TEST-ONLY (the
//! production law is the entry pair itself; this exists so the
//! ported grammar tests keep one spelling).

use rut_ast::parser::{parse_with_recover, Mode, ParseOpts};
use rut_ast::{Ast, BlockNode, ExprRef, Handle, ItemRef, StmtRef};
use rut_lexer::diag::Diag;

pub fn parse(src: &str, mode: Mode) -> (Ast, Vec<Diag>) {
    let p = parse_with_recover(src.to_string(), mode, ParseOpts::default());
    let diags = p.ast().errors().to_vec();
    let (ast, _) = p.into_parts();
    (ast, diags)
}

/// the FIRST fn item's body block (clean parses — the grammar tests'
/// favorite door)
// each test binary compiles this module fresh; not every file walks
// every door — shared vocabulary, not per-file dead code
#[allow(dead_code)]
pub fn first_fn_body(ast: &Ast) -> Handle<BlockNode> {
    for i in ast.root().items(ast) {
        if let ItemRef::Fn(h) = *i {
            return h.get(ast).body;
        }
    }
    panic!("no fn item in the module");
}

/// the LAST fn item's body block
#[allow(dead_code)]
pub fn last_fn_body(ast: &Ast) -> Handle<BlockNode> {
    for i in ast.root().items(ast).iter().rev() {
        if let ItemRef::Fn(h) = *i {
            return h.get(ast).body;
        }
    }
    panic!("no fn item in the module");
}

/// the first statement's initializer — the `let x = INIT;` shape
#[allow(dead_code)]
pub fn let_init(ast: &Ast, body: Handle<BlockNode>) -> ExprRef {
    match body.get(ast).stmts[0] {
        StmtRef::Let(h) => h.get(ast).init,
        ref s => panic!("expected a let stmt, got {s:?}"),
    }
}
