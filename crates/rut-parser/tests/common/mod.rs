//! The v1-shaped test shorthand over the entry pair — TEST-ONLY (the
//! production law is the entry pair itself; this exists so the
//! port-pending grammar tests keep one spelling).

use rut_ast::Ast;
use rut_lexer::diag::Diag;
use rut_parser::{parse_with_recover, Mode, ParseOpts};

pub fn parse(src: &str, mode: Mode) -> (Ast, Vec<Diag>) {
    let p = parse_with_recover(src, mode, ParseOpts::default());
    let diags = p.ast.errors().to_vec();
    (p.ast, diags)
}
