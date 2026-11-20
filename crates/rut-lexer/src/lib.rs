//! rut-lexer — the frontend base: the mode-stack
//! tokenizer over the flat token enum, plus the
//! vocabulary shared by every frontend stage — `Span`/`NEST_MAX` and
//! `Diag` + renderer. The text layer (`Edit`/`Source`) and the token
//! splice live here too — the edit pipeline's base, consumed by the
//! `rut-ast` `Doc`.

pub mod diag;
pub mod lexer;
pub mod source;
pub mod span;
pub mod splice;
pub mod token;
