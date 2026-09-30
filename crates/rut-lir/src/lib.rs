//! rut-lir — the fused middle-end (layout): resolve +
//! typecheck fused with body compilation (milestone M1) over
//! the flat-arena AST, producing LIR bytecode in
//! `rut-core`'s `FuncCode` form.
//!
//! `check/` and `lir/` are one crate on purpose: the typechecker's
//! monomorphization queue (`check/inst.rs`) drives the body compiler
//! (`lir::FnCompiler`) and the body compiler reports back through
//! `check::Ctx` — the fusion is the M1 design, not an accident.

pub mod check;
pub mod lir;
