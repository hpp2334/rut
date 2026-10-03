//! The compile pipeline over one module and the engine package
//! constructors: [`compile_module`] (the single-source lane — auto
//! core + `calc`), the program compiler ([`program`]), the seed types
//! ([`seeds`]), the engine pkg constructors ([`std`]), and the IR
//! dump renderer ([`dump`]).

pub mod dump;
pub mod program;
pub mod seeds;
pub mod std;

pub use dump::ir_dump_of;
pub use program::{compile_program, compile_program_resolved, ProgramOutput};
pub use seeds::{SeedGroup, Seeds};
pub use std::{calc_pkg, core_pkg, std_async_pkgs};

use rut_lexer::diag::Diag;
use rut_lexer::span::Span;
use rut_ast::dump as ast_dump;
use rut_parser::{parse, Mode};

use crate::run::{Compiled, RutRun, RunError};
pub use crate::session::Pkg;
use rut_core::binary::encode;

pub struct CompileOutput {
    pub diags: Vec<Diag>,
    pub ast_dump: String,
    /// the demo tree UI's structured AST (flattened tagged JSON)
    pub ast_json: String,
    pub ir_dump: String,
    pub binary: Option<Vec<u8>>,
}

/// Full pipeline over one module: the engine's packages ride the run
/// chain (auto core, explicit `calc`), the source offers as the root
/// pkg under `module_name`, and one `.compile()` runs the whole law.
/// The embedder-facing single-source lane (the wasm demo, `rut dump`).
pub fn compile_module(src: &str, mode: Mode, module_name: &str) -> CompileOutput {
    let (ast, diags) = parse(src, mode);
    let tree = ast_dump::to_dump_tree(&ast);
    let ast_dump = ast_dump::render_text(&tree, src);
    let ast_json = ast_dump::render_json(&tree);
    if !diags.is_empty() {
        return CompileOutput { diags, ast_dump, ast_json, ir_dump: String::new(), binary: None };
    }
    let compiled: Result<Compiled, RunError> = RutRun::new()
        .pkg(Pkg::source(module_name, src))
        .pkg(calc_pkg())
        .entrypoint(module_name)
        .compile();
    render(compiled, ast_dump, ast_json)
}

/// The chain product → the demo envelope: compile diags ride the
/// graph; shape/closure refusals come back as one span-0 diagnostic
/// (the envelope has no other channel).
fn render(compiled: Result<Compiled, RunError>, ast_dump: String, ast_json: String) -> CompileOutput {
    match compiled {
        Ok(c) => {
            let ir_dump = c
                .graph
                .program
                .as_ref()
                .map(|p| ir_dump_of(&p.funcs, &p.interner))
                .unwrap_or_default();
            let binary = c.graph.program.map(|p| encode(&p));
            CompileOutput { diags: c.graph.diags, ast_dump, ast_json, ir_dump, binary }
        }
        Err(e) => CompileOutput {
            diags: vec![Diag::new(Span::new(0, 0), e.msg)],
            ast_dump,
            ast_json,
            ir_dump: String::new(),
            binary: None,
        },
    }
}
