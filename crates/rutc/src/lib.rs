//! rutc — the rut compiler (RFC 0041 §2 layout): syntax/ (RFC 0030),
//! resolve+check (RFC 0031), lir (RFC 0032), emit (RFC 0033).

pub mod ast;
pub mod diag;
pub mod dump;
pub mod emit;
pub mod lexer;
pub mod parser;
pub mod span;
pub mod token;

pub mod check;
pub mod lir;

pub use diag::Diag;
pub use emit::{compile_module, ir_dump_of, CompileOutput};
pub use parser::Mode;
pub use span::Span;
pub use dump::{render_json, render_text, to_dump_tree, DumpNode, DumpVal};
