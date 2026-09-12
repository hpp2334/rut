//! Module-graph compilation — the driver half of RFC 0035 §1.
//!
//! Walks a root module's `import` statements through a [`Session`], compiles
//! every dependency first (post-order), binds each module's surface into its
//! importers, and links the lot into one dense [`Program`]. The order the
//! programs are pushed is the link order, so every scope is registered before
//! a dependent references it.

use std::collections::{HashMap, HashSet};

use rut_ast::ast::{Ast, ItemKind};
use rut_lexer::diag::Diag;
use rut_lexer::span::Span;
use rut_core::binary::Program;
use rut_parser::{parse, Mode};

use crate::session::Session;
use crate::compile_program;

/// A linked module graph.
pub struct GraphOutput {
    pub diags: Vec<Diag>,
    /// the flattened (dense-id) program — ready for `encode` and the VM
    pub program: Option<Program>,
}

/// Compile `root_spec` and its transitive imports from `session`.
pub fn compile_graph(session: &Session, root_spec: &str) -> GraphOutput {
    let mut c = GraphCompiler {
        session,
        next_scope: 1,
        programs: Vec::new(),
        done: HashMap::new(),
        visiting: HashSet::new(),
        diags: Vec::new(),
    };
    if c.ensure(root_spec).is_none() {
        return GraphOutput { diags: c.diags, program: None };
    }
    match rut_core::link::link(c.programs) {
        Ok(p) => GraphOutput { diags: c.diags, program: Some(p) },
        Err(e) => {
            c.diags.push(Diag::new(Span::new(0, 0), format!("link: {e}")));
            GraphOutput { diags: c.diags, program: None }
        }
    }
}

struct GraphCompiler<'a> {
    session: &'a Session,
    next_scope: rut_core::ScopeId,
    /// post-order: dependencies precede importers
    programs: Vec<Program>,
    done: HashMap<String, usize>,
    visiting: HashSet<String>,
    diags: Vec<Diag>,
}

impl<'a> GraphCompiler<'a> {
    /// Compile `spec` if needed, returning its program index and scope.
    fn ensure(&mut self, spec: &str) -> Option<(usize, rut_core::ScopeId)> {
        if let Some(&i) = self.done.get(spec) {
            let scope = self.programs[i].scope;
            return Some((i, scope));
        }
        if !self.visiting.insert(spec.to_string()) {
            self.diags.push(Diag::new(
                Span::new(0, 0),
                format!("cyclic import: `{spec}` is already being compiled"),
            ));
            return None;
        }
        let module = match self.session.resolve(spec) {
            Ok(m) => m,
            Err(e) => {
                self.diags.push(Diag::new(Span::new(0, 0), e.to_string()));
                return None;
            }
        };
        let Some(src) = module.source.as_deref() else {
            self.diags.push(Diag::new(
                Span::new(0, 0),
                format!("module `{spec}` has no body source to compile"),
            ));
            return None;
        };

        // discover this module's imports (parse once up front)
        let (ast, d) = parse(src, Mode::Impl);
        if !d.is_empty() {
            self.diags.extend(d);
            return None;
        }
        let imports = imports_of(&ast, src);

        let mut bound: Vec<(rut_core::ScopeId, rut_core::binary::Surface)> = Vec::new();
        let mut bound_scopes = HashSet::new();
        for dep in &imports {
            let Some((idx, dep_scope)) = self.ensure(dep) else {
                return None;
            };
            if bound_scopes.insert(dep_scope) {
                bound.push((dep_scope, self.programs[idx].surface.clone()));
            }
        }

        let scope = self.next_scope;
        self.next_scope += 1;
        let out = compile_program(src, Mode::Impl, spec, scope, &bound);
        if !out.diags.is_empty() || out.program.is_none() {
            self.diags.extend(out.diags);
            if out.program.is_none() && self.diags.is_empty() {
                self.diags.push(Diag::new(
                    Span::new(0, 0),
                    format!("module `{spec}` produced no program"),
                ));
            }
            return None;
        }
        let idx = self.programs.len();
        self.programs.push(out.program.unwrap());
        self.done.insert(spec.to_string(), idx);
        Some((idx, scope))
    }
}

/// The exact specifiers a module imports, in source order, deduped.
fn imports_of(ast: &Ast, _src: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for it in ast.module_items(ast.root).to_vec() {
        if let ItemKind::Import { from, .. } = ast.item(it) {
            if !out.contains(from) {
                out.push(from.clone());
            }
        }
    }
    out
}
