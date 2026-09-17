//! Module-graph compilation — the driver half of RFC 0035 §1.
//!
//! Walks a root module's `use` statements through a [`Session`]. A module
//! that only exports concrete items is compiled under its own scope and
//! linked (its `Surface` is bound into each using module). A module that exports
//! a *generic* type (`Vec<T>`) cannot be linked — RFC 0013 monomorphizes at
//! compile time, and the instantiation must happen where the class body
//! lives — so its source is inlined into the consumer instead (its own
//! relative includes are already merged by the loader).
//!
//! Programs are pushed in post-order, so the link order registers every
//! scope before a dependent references it.

use std::collections::{HashMap, HashSet};

use rut_ast::ast::{Ast, ItemKind};
use rut_lexer::diag::Diag;
use rut_lexer::span::Span;
use rut_core::binary::Program;
use rut_parser::{parse, Mode};

use crate::session::Session;
use crate::compile_program_resolved;

/// A linked module graph.
pub struct GraphOutput {
    pub diags: Vec<Diag>,
    /// the flattened (dense-id) program — ready for `encode` and the VM
    pub program: Option<Program>,
}

/// Compile `root_spec` and its transitive uses from `session`.
pub fn compile_graph(session: &Session, root_spec: &str) -> GraphOutput {
    let mut c = GraphCompiler {
        session,
        next_scope: 1,
        programs: Vec::new(),
        done: HashMap::new(),
        visiting: HashSet::new(),
        diags: Vec::new(),
    };
    if c.ensure(root_spec, false).is_none() {
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

/// A resolved module: a linked scope, or source to inline.
#[derive(Clone)]
enum Unit {
    Linked { idx: usize, scope: rut_core::ScopeId },
    /// source to splice into the consumer, plus the uses its own source
    /// names (so the consumer binds them too)
    Inline { source: String, bound: Vec<(rut_core::ScopeId, rut_core::binary::Surface)> },
}

struct GraphCompiler<'a> {
    session: &'a Session,
    next_scope: rut_core::ScopeId,
    /// post-order: dependencies precede their users
    programs: Vec<Program>,
    done: HashMap<String, Unit>,
    visiting: HashSet<String>,
    diags: Vec<Diag>,
}

impl<'a> GraphCompiler<'a> {
    /// Compile (or inline) `spec` if needed. `as_dep` allows the inline path;
    /// the root is always compiled and linked.
    fn ensure(&mut self, spec: &str, as_dep: bool) -> Option<Unit> {
        if let Some(u) = self.done.get(spec) {
            return Some(u.clone());
        }
        if !self.visiting.insert(spec.to_string()) {
            self.diags.push(Diag::new(
                Span::new(0, 0),
                format!("cyclic use: `{spec}` is already being compiled"),
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
        // a native module (RFC 0022/0026): no rut body — synthesize a
        // placeholder program whose bodyless funcs the embedder implements.
        // Intrinsics (compiler-lowered) and constants ride the same surface.
        // `core` rides it too: no funcs, just the native type/trait/fn
        // names of the prelude (RFC 0028).
        if module.source.is_none()
            && (!module.host_funcs.is_empty()
                || !module.intrinsics.is_empty()
                || !module.consts.is_empty()
                || !module.native_types.is_empty()
                || !module.native_traits.is_empty()
                || !module.native_fns.is_empty())
        {
            use rut_core::binary::{FuncCode, Program};
            // the host-fn registration scope defaults to the package
            // name; `rt` overrides it to keep its internal `rt:log`
            // registration naming (RFC 0022)
            let host_scope = module.host_scope.as_deref().unwrap_or(spec);
            // host functions obey the same crossing rule as `entry fn`
            // (RFC 0023 §2 / RFC 0035 §3)
            let boot_tt = rut_core::types::TypeTable::boot();
            for (name, params, ret) in &module.host_funcs {
                let bad = params.iter().any(|p| !boot_tt.crosses_boundary(*p))
                    || !boot_tt.crosses_boundary(*ret);
                if bad {
                    self.diags.push(Diag::new(
                        Span::new(0, 0),
                        format!(
                            "host function `{host_scope}::{name}`: only primitives, `str`, `bytes`, `Opaque`, and `Option`/`Result` over those cross the host boundary (RFC 0023 §2)"
                        ),
                    ));
                    return None;
                }
            }
            let scope = self.next_scope;
            self.next_scope += 1;
            let mut surface = rut_core::binary::Surface::default();
            let mut funcs = Vec::new();
            for (i, (name, params, ret)) in module.host_funcs.iter().enumerate() {
                surface.funcs.push(rut_core::binary::SurfaceFn {
                    name: surface.names.intern(name),
                    params: params.clone(),
                    ret: *ret,
                    local: i as u32,
                    intrinsic: None,
                });
                funcs.push(FuncCode {
                    name: surface.names.intern(name),
                    params: params.clone(),
                    ret: *ret,
                    is_method: false,
                    n_captures: 0,
                    regs: vec![],
                    argv: vec![],
                    labels: vec![],
                    code: vec![],
                    spans: vec![],
                    host: Some(format!("{host_scope}::{name}")),
                });
            }
            for (name, id, arity) in &module.intrinsics {
                // the declared signature is a placeholder: the LIR types the
                // call from its arguments (RFC 0032 §1.1 R2)
                surface.funcs.push(rut_core::binary::SurfaceFn {
                    name: surface.names.intern(name),
                    params: vec![rut_core::types::TY_I32; *arity],
                    ret: rut_core::types::TY_I32,
                    local: 0,
                    intrinsic: Some(*id),
                });
            }
            for (name, ty, bits) in &module.consts {
                surface.consts.push(rut_core::binary::SurfaceConst {
                    name: surface.names.intern(name),
                    ty: *ty,
                    bits: *bits,
                });
            }
            surface.namespace = module.namespace.as_deref().map(|n| surface.names.intern(n));
            surface.native_types = module
                .native_types
                .iter()
                .map(|(n, k)| (surface.names.intern(n), *k))
                .collect();
            surface.native_traits = module
                .native_traits
                .iter()
                .map(|(n, k)| (surface.names.intern(n), *k))
                .collect();
            surface.native_fns = module
                .native_fns
                .iter()
                .map(|n| surface.names.intern(n))
                .collect();
            let interner = surface.names.clone();
            let program = Program { name: spec.to_string(), scope, interner, surface, funcs, ..Default::default() };
            let idx = self.programs.len();
            self.programs.push(program);
            let unit = Unit::Linked { idx, scope };
            self.done.insert(spec.to_string(), unit.clone());
            return Some(unit);
        }
        let Some(src) = module.source.clone() else {
            self.diags.push(Diag::new(
                Span::new(0, 0),
                format!("module `{spec}` has no body source to compile"),
            ));
            return None;
        };

        let (ast, d) = parse(&src, if module.is_decl { Mode::Decl } else { Mode::Impl });
        if !d.is_empty() {
            self.diags.extend(d);
            return None;
        }
        let uses = uses_of(&ast);

        let mut extra = String::new();
        let mut bound: Vec<(rut_core::ScopeId, rut_core::binary::Surface)> = Vec::new();
        let mut bound_scopes = HashSet::new();
        for dep in &uses {
            match self.ensure(dep, true)? {
                Unit::Inline { source, bound: b } => {
                    extra.push_str(&source);
                    extra.push('\n');
                    for (sc, surf) in b {
                        if bound_scopes.insert(sc) {
                            bound.push((sc, surf));
                        }
                    }
                }
                Unit::Linked { idx, scope } => {
                    if bound_scopes.insert(scope) {
                        bound.push((scope, self.programs[idx].surface.clone()));
                    }
                }
            }
        }

        // the compilation unit: inlined generic deps, then this module.
        // A declaration unit (`.d.rut`) takes no spliced bodies — its
        // use statements bind surfaces only; a decl file is pure surface
        // (RFC 0029), and Decl mode rejects implementations.
        let combined = if extra.is_empty() || module.is_decl {
            src
        } else {
            format!("{extra}\n{src}")
        };
        let scope = self.next_scope;
        self.next_scope += 1;
        let out = compile_program_resolved(
            &combined,
            if module.is_decl { Mode::Decl } else { Mode::Impl },
            spec,
            scope,
            &bound,
            true,
        );
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
        let program = out.program.unwrap();
        let has_generic = program.surface.type_exports.iter().any(|t| t.is_generic);
        // an explicitly-inlined module (e.g. `ink`) or a generic export
        // cannot be linked — splice its source (and its uses) in
        if as_dep && (module.inline || has_generic) {
            let unit = Unit::Inline { source: combined, bound };
            self.done.insert(spec.to_string(), unit.clone());
            return Some(unit);
        }
        let idx = self.programs.len();
        self.programs.push(program);
        let unit = Unit::Linked { idx, scope };
        self.done.insert(spec.to_string(), unit.clone());
        Some(unit)
    }
}

/// The exact package names a module uses, in source order, deduped.
fn uses_of(ast: &Ast) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for it in ast.module_items(ast.root).to_vec() {
        if let ItemKind::Use { pkg, .. } = ast.item(it) {
            let name = ast.name(*pkg).to_string();
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}
