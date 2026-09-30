//! rut-lexer — the frontend base: the mode-stack
//! tokenizer over the flat token enum, plus the
//! vocabulary shared by every frontend stage — `Span`/`NEST_MAX` and
//! `Diag` + renderer.

pub mod diag;
pub mod lexer;
pub mod span;
pub mod token;
