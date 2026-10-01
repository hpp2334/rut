//! The compile pipeline over one module and the engine package
//! mounts: [`compile_module`] (fresh session) /
//! [`compile_module_in`] (a caller-built session), the program
//! compiler ([`program`]), the seed types ([`seeds`]), the engine
//! mounts ([`std`]), and the IR dump renderer ([`dump`]).

pub mod dump;
pub mod program;
pub mod seeds;
pub mod std;

pub use dump::ir_dump_of;
pub use program::{compile_program, compile_program_resolved, ProgramOutput};
pub use seeds::{SeedGroup, SeedImpl, Seeds};
pub use std::{mount_calc, mount_std, mount_std_async, mount_std_core};

use rut_lexer::diag::Diag;
use rut_ast::dump as ast_dump;
use rut_parser::{parse, Mode};

use crate::graph::compile_graph;
use crate::session::{Module, ModuleBody, Session};
use rut_core::binary::encode;

pub struct CompileOutput {
    pub diags: Vec<Diag>,
    pub ast_dump: String,
    /// the demo tree UI's structured AST (flattened tagged JSON)
    pub ast_json: String,
    pub ir_dump: String,
    pub binary: Option<Vec<u8>>,
}

/// Full pipeline over one module: resolve its `use` statements against a
/// session with the engine's packages mounted (`core`, `calc`), then
/// link, flatten, encode.
pub fn compile_module(src: &str, mode: Mode, module_name: &str) -> CompileOutput {
    let mut session = Session::new();
    mount_std(&mut session);
    compile_module_in(&mut session, src, mode, module_name)
}

/// [`compile_module`] against a caller-built session — hosts and tests
/// that mount their own libraries (the third-party pkgs are NOT in
/// [`mount_std`]; mount them with [`crate::mount_dir`]).
pub fn compile_module_in(
    session: &mut Session,
    src: &str,
    mode: Mode,
    module_name: &str,
) -> CompileOutput {
    let (ast, mut diags) = parse(src, mode);
    let tree = ast_dump::to_dump_tree(&ast);
    let ast_dump = ast_dump::render_text(&tree, src);
    let ast_json = ast_dump::render_json(&tree);
    if !diags.is_empty() {
        return CompileOutput { diags, ast_dump, ast_json, ir_dump: String::new(), binary: None };
    }
    let spec = module_name.to_string();
    if let Err(e) = session.register_module(
        &spec,
        Module {
            body: ModuleBody::Source { text: src.to_string(), is_decl: mode == Mode::Decl },
            ..Default::default()
        },
    ) {
        diags.push(Diag::new(rut_lexer::span::Span::new(0, 0), e.to_string()));
        return CompileOutput { diags, ast_dump, ast_json, ir_dump: String::new(), binary: None };
    }
    let g = compile_graph(session, &spec);
    let ir_dump = g.program.as_ref().map(|p| ir_dump_of(&p.funcs, &p.interner)).unwrap_or_default();
    let binary = g.program.map(|p| encode(&p));
    CompileOutput { diags: g.diags, ast_dump, ast_json, ir_dump, binary }
}

