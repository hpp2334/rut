//! The toolchain's std surface — embedded source, indexed once and
//! consulted by hover/completion after the open document (one directory
//! per module: `rut/pouch/rut.toml` names `"pouch"`). Shared by both
//! faces: the stdio server (`server`, native) and the wasm shim
//! (`rut-lsp-wasm`) — one index, no drift.

use crate::hover::{self, DefIndex};

pub const CORE: &str = include_str!("../../../rut/core/core.d.rut");
pub const CALC: &str = include_str!("../../../rut/calc/calc.d.rut");
pub const POUCH: &str = include_str!("../../../rut/pouch/pouch.rut");

const CORE_LABEL: &str = "core";

/// the std surface as definition indexes, in lookup order
pub fn indexes() -> Vec<DefIndex> {
    [
        (CORE_LABEL, CORE, rut_parser::Mode::Decl),
        ("calc", CALC, rut_parser::Mode::Decl),
        ("pouch", POUCH, rut_parser::Mode::Impl),
    ]
    .into_iter()
    .map(|(origin, src, mode)| {
        let src = rut_lexer::lexer::normalize(src);
        let (ast, _) = rut_parser::parse(&src, mode);
        let mut idx = hover::index(&src, &ast);
        idx.origin = origin.to_string();
        idx
    })
    .collect()
}
