//! rut-lexer — the frontend base (RFC 0041 §2 layout): the mode-stack
//! tokenizer (RFC 0030 §1) over the flat token enum (0002 §4), plus the
//! vocabulary shared by every frontend stage — `Span`/`NEST_MAX` (0030
//! OQ-3) and `Diag` + renderer (0030 §6).

pub mod diag;
pub mod lexer;
pub mod span;
pub mod token;
