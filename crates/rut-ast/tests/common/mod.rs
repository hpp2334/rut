//! The v1-shaped test shorthand over the parse entry — TEST-ONLY (the
//! production law is the Doc pipeline; this exists so the ported
//! grammar tests keep one spelling). Tests hold the `Doc` and borrow
//! `doc.ast()`.

use rut_ast::parser::{parse_with_recover, Mode};
use rut_ast::{Ast, BlockNode, Doc, ExprRef, Handle, ItemRef, StmtRef};
use rut_lexer::diag::Diag;

pub fn parse(src: &str, mode: Mode) -> (Doc, Vec<Diag>) {
    let doc = parse_with_recover(src.to_string(), mode);
    let diags = doc.ast().errors().to_vec();
    (doc, diags)
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
