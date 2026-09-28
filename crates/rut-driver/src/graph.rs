//! Module-graph compilation — the driver half of RFC 0035 §1.
//!
//! Walks a root module's `use` statements through a [`Session`]. A module
//! that only exports concrete items is compiled under its own scope and
//! linked (its `Surface` is bound into each using module). A module that exports
//! a *generic* type (`Vec<T>`) cannot be linked — RFC 0013 monomorphizes at
//! compile time, and the instantiation must happen where the class body
//! lives — so its source is spliced into the consumer instead. There is
//! no include form to merge (one file is one module — loader.rs): the
//! splice composes each unit from its transitive leaf list, deduped by
//! origin spec, so two sibling uses sharing a transitive inline pkg
//! splice that pkg exactly once (the dep-kinds survey §2.4).
//!
//! The dispatch is on the mounted [`ModuleBody`]: a source body takes
//! the compile path above, a host body synthesizes its placeholder
//! program, and a compiled body (a v5 bundle's decoded `.rutc`) is
//! pushed after a fresh-scope rebase of its packed ids. The splice
//! verdict is [`linkable`] — one classifier, shared with the packer.
//!
//! Programs are pushed in post-order, so the link order registers every
//! scope before a dependent references it.

use std::collections::{HashMap, HashSet};

use rut_ast::ast::{Ast, ItemKind};
use rut_lexer::diag::Diag;
use rut_lexer::span::Span;
use rut_core::binary::Program;
use rut_parser::{parse, Mode};

use crate::session::{ModuleBody, Session};
use crate::compile_program_resolved;

/// A linked module graph.
pub struct GraphOutput {
    pub diags: Vec<Diag>,
    /// the flattened (dense-id) program — ready for `encode` and the VM
    pub program: Option<Program>,
}

/// Compile `root_spec` and its transitive uses from `session`.
pub fn compile_graph(session: &Session, root_spec: &str) -> GraphOutput {
    let units = compile_units(session, root_spec);
    if !units.ok {
        return GraphOutput { diags: units.diags, program: None };
    }
    match rut_core::link::link(units.programs) {
        Ok(p) => GraphOutput { diags: units.diags, program: Some(p) },
        Err(e) => {
            let mut diags = units.diags;
            diags.push(Diag::new(Span::new(0, 0), format!("link: {e}")));
            GraphOutput { diags, program: None }
        }
    }
}

/// Per-package compile results — [`compile_graph`]'s walk, exposed for
/// the packer: the pushed programs in post-order, each linked module's
/// own program and scope, and the spliced (inline) specs.
pub struct Units {
    pub diags: Vec<Diag>,
    /// every pushed program, post-order (compile_graph links these)
    pub programs: Vec<Program>,
    /// each ensured-and-linked module: spec → (index into `programs`, scope)
    pub linked: HashMap<String, (usize, rut_core::ScopeId)>,
    /// the ensured specs the splice law routed into their consumers
    pub inline: HashSet<String>,
    /// `true` when the root produced a program (the walk succeeded)
    pub ok: bool,
}

/// Compile (or mount, or splice) every module in `root_spec`'s use
/// closure — the packer's view of [`compile_graph`].
pub fn compile_units(session: &Session, root_spec: &str) -> Units {
    let mut c = GraphCompiler {
        session,
        next_scope: 1,
        programs: Vec::new(),
        done: HashMap::new(),
        visiting: HashSet::new(),
        diags: Vec::new(),
        inline: HashSet::new(),
    };
    let ok = c.ensure(root_spec, false).is_some();
    let (diags, programs, linked, inline) = c.finish();
    Units { diags, programs, linked, inline, ok }
}

/// Why a compiled module cannot link — THE splice law's verdict, the
/// one classifier the graph and the packer share: `inline`, a generic
/// export, or a trait-object parameter routes the module's SOURCE into
/// its consumers instead of a linked program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Linkability {
    /// concrete exports only — links
    Linkable,
    /// must splice source (inline, generic export, trait-object param)
    SpliceNeeded,
}

/// Classify a compiled program (with its manifest's `inline` flag).
/// The graph's splice decision and the packer's compiled-group
/// decision both call this — there is no second implementation.
pub fn linkable(program: &Program, inline: bool) -> Linkability {
    let has_generic = program.surface.type_exports.iter().any(|t| t.is_generic);
    // a fn whose exported signature takes trait-typed PARAMETERS cannot
    // be linked either: a trait parameter is an implicit generic bound
    // (RFC 0012 §5) — it specializes per concrete argument, one clone
    // per argument type (finite, terminating via the Inst cache), and
    // that must happen where the arguments are. Splice its source in.
    let has_trait_param = program.surface.funcs.iter().any(|f| {
        f.params
            .iter()
            .any(|&p| matches!(program.types.kind(p), rut_core::types::TyKind::TraitObj { .. }))
    });
    if inline || has_generic || has_trait_param {
        Linkability::SpliceNeeded
    } else {
        Linkability::Linkable
    }
}

/// A resolved module: a linked scope, or a leaf list to splice.
#[derive(Clone)]
enum Unit {
    Linked { idx: usize, scope: rut_core::ScopeId },
    /// the ordered `(origin spec, own source)` leaves spliced into the
    /// consumer — post-order (deps before users), deduplicated by
    /// origin spec (first position wins), ending with the unit's own
    /// leaf — plus the uses its own source names (so the consumer binds
    /// them too). A second splice of the same origin can only duplicate
    /// definitions (items are order-independent) and never contributes
    /// a name the first splice did not, so skipping it is
    /// semantics-preserving (dep-kinds survey §2.4).
    Inline {
        leaves: Vec<(String, String)>,
        bound: Vec<(rut_core::ScopeId, rut_core::binary::Surface, String)>,
    },
}

struct GraphCompiler<'a> {
    session: &'a Session,
    next_scope: rut_core::ScopeId,
    /// post-order: dependencies precede their users
    programs: Vec<Program>,
    done: HashMap<String, Unit>,
    visiting: HashSet<String>,
    diags: Vec<Diag>,
    /// the ensured specs the splice law routed into their consumers
    inline: HashSet<String>,
}

impl<'a> GraphCompiler<'a> {
    /// Decompose the finished walk: diags, pushed programs, the
    /// linked-units index (spec → program + scope), the inline set.
    fn finish(
        self,
    ) -> (
        Vec<Diag>,
        Vec<Program>,
        HashMap<String, (usize, rut_core::ScopeId)>,
        HashSet<String>,
    ) {
        let GraphCompiler { diags, programs, done, inline, .. } = self;
        let mut linked = HashMap::new();
        for (spec, unit) in done {
            if let Unit::Linked { idx, scope } = unit {
                linked.insert(spec, (idx, scope));
            }
        }
        (diags, programs, linked, inline)
    }

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
        match &module.body {
            // a decoded `.rutc` (a v5 compiled bundle's payload): no
            // compilation, no binding — ensure its deps (the pack-time
            // scope ledger names them), assign a fresh scope, rebase
            // the packed ids, push. `inline` is a source-shape flag and
            // does not reach here: a compiled module already linked.
            ModuleBody::Compiled(prog) => self.ensure_compiled(spec, prog),
            ModuleBody::Host { .. } => self.ensure_host(spec),
            ModuleBody::Source { text, is_decl } => self.ensure_source(spec, module, text, *is_decl, as_dep),
        }
    }

    /// The native-module path (RFC 0022/0026): no rut body — synthesize
    /// a placeholder program whose bodyless funcs the embedder
    /// implements. Intrinsics (compiler-lowered) and constants ride the
    /// same surface. `core` rides it too: no funcs, just the native
    /// type/trait/fn names of the prelude (RFC 0028).
    fn ensure_host(&mut self, spec: &str) -> Option<Unit> {
        let module = self.session.resolve(spec).ok()?;
        let ModuleBody::Host { host_funcs, consts, native_types, native_traits, native_fns, native_impls } =
            &module.body
        else {
            return None;
        };
        {
            use rut_core::binary::{FuncCode, Program};
            // the host-fn registration scope defaults to the package
            // name; `rt` overrides it to keep its internal `rt:log`
            // registration naming (RFC 0022)
            let host_scope = module.host_scope.as_deref().unwrap_or(spec);
            // host functions obey the same crossing rule as `entry fn`
            // (RFC 0023 §2 / RFC 0035 §3)
            let boot_tt = rut_core::types::TypeTable::boot();
            for (name, params, ret, _is_async) in host_funcs {
                let bad = params.iter().any(|p| !boot_tt.crosses_boundary(*p))
                    || !boot_tt.crosses_boundary(*ret);
                if bad {
                    self.diags.push(Diag::new(
                        Span::new(0, 0),
                        format!(
                            "host function `{host_scope}::{name}`: only primitives, `str`, `bytes`, `opaque`, and `Option`/`Result` over those cross the host boundary (RFC 0023 §2)"
                        ),
                    ));
                    return None;
                }
            }
            let scope = self.next_scope;
            self.next_scope += 1;
            let mut surface = rut_core::binary::Surface::default();
            let mut funcs = Vec::new();
            for (i, (name, params, ret, is_async)) in host_funcs.iter().enumerate() {
                surface.funcs.push(rut_core::binary::SurfaceFn {
                    name: surface.names.intern(name),
                    params: params.clone(),
                    ret: *ret,
                    local: i as u32,
                    is_async: *is_async,
                    // the registration name rides the surface: the
                    // importer derives the minted row family's host ids
                    // from it, so `register_async!`'s one literal stays
                    // the source of truth on both sides
                    host: Some(surface.names.intern(&format!("{host_scope}::{name}"))),
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
                    pos: vec![],
                    host_id: Some(surface.names.intern(&format!("{host_scope}::{name}"))),
                });
            }
            for (name, ty, bits) in consts {
                surface.consts.push(rut_core::binary::SurfaceConst {
                    name: surface.names.intern(name),
                    ty: *ty,
                    bits: *bits,
                });
            }
            surface.namespace = module.namespace.as_deref().map(|n| surface.names.intern(n));
            // each row keeps the ambient bit it was mounted with —
            // host rows default `true`, and core's mount copies the
            // decl spellings verbatim (the `pub builtin` disposal pair
            // stays import-gated through registration)
            surface.native_types = native_types
                .iter()
                .map(|(n, k, a)| (surface.names.intern(n), *k, *a))
                .collect();
            surface.native_traits = native_traits
                .iter()
                .map(|(n, k, a)| (surface.names.intern(n), *k, *a))
                .collect();
            surface.native_fns = native_fns
                .iter()
                .map(|(n, a)| (surface.names.intern(n), *a))
                .collect();
            // the integer prims' numeric methods (RFC 0032 §1.1 R2) —
            // bound ambient on the receiver primitive, no use gate
            surface.native_impls = native_impls
                .iter()
                .map(|(t, n, i)| (*t, surface.names.intern(n), *i))
                .collect();
            let interner = surface.names.clone();
            let program = Program { name: spec.to_string(), scope, interner, surface, funcs, ..Default::default() };
            let idx = self.programs.len();
            self.programs.push(program);
            let unit = Unit::Linked { idx, scope };
            self.done.insert(spec.to_string(), unit.clone());
            Some(unit)
        }
    }

    /// The compiled-bundle path: a decoded program's ids spell its
    /// pack-time scopes — its own, and the scopes of every module its
    /// compiled units bound (the engine host mounts it called through,
    /// and the linked deps whose surfaces it bound). The mount must not
    /// assume pack-time and load-time numbering agree: ensure each
    /// referenced module (in ascending pack-scope order, which replays
    /// the pack walk's post-order), map every scope half to its
    /// load-time value, rebase, push.
    fn ensure_compiled(&mut self, spec: &str, prog: &Program) -> Option<Unit> {
        let Some(own_pack) = rut_core::link::own_scope(prog) else {
            self.diags.push(Diag::new(
                Span::new(0, 0),
                format!("module `{spec}` carries no scope blocks — not a compiled module binary"),
            ));
            return None;
        };
        let mut foreign = rut_core::link::foreign_scopes(prog);
        foreign.remove(&own_pack);
        // ascending pack-scope order: pack-time scopes were assigned in
        // the walk's post-order, so this replays the pack walk's
        // use order, dep before user
        let mut mapped: Vec<(rut_core::ScopeId, rut_core::ScopeId)> = Vec::new();
        for &pack_scope in &foreign {
            let Some(dep_spec) = self.session.bundle_scope(pack_scope).map(str::to_string) else {
                self.diags.push(Diag::new(
                    Span::new(0, 0),
                    format!(
                        "module `{spec}` references scope {pack_scope}, which the bundle's scope ledger does not name — the bundle is incomplete or corrupt"
                    ),
                ));
                return None;
            };
            match self.ensure(&dep_spec, true)? {
                Unit::Linked { scope, .. } => mapped.push((pack_scope, scope)),
                Unit::Inline { .. } => {
                    self.diags.push(Diag::new(
                        Span::new(0, 0),
                        format!(
                            "module `{spec}` was compiled against `{dep_spec}`, which spliced instead of linking — the bundle mixes incompatible group kinds"
                        ),
                    ));
                    return None;
                }
            }
        }
        // the prelude ride, exactly as the source walk spells it: every
        // unit binds `core`'s ambient names whether or not its ids
        // reference core's scope
        if spec != "core" && self.session.resolve("core").is_ok() {
            self.ensure("core", true)?;
        }
        let scope = self.next_scope;
        self.next_scope += 1;
        let map = |s: rut_core::ScopeId| -> rut_core::ScopeId {
            if s == own_pack {
                scope
            } else {
                mapped.iter().find(|&&(p, _)| p == s).map(|&(_, l)| l).unwrap_or(s)
            }
        };
        let program = rut_core::link::rebase(prog.clone(), &map);
        let idx = self.programs.len();
        self.programs.push(program);
        let unit = Unit::Linked { idx, scope };
        self.done.insert(spec.to_string(), unit.clone());
        Some(unit)
    }

    /// The compile path: parse the module's source, resolve its uses,
    /// splice what the splice law demands, compile the unit — and
    /// route it to a linked program or an inline leaf list.
    fn ensure_source(
        &mut self,
        spec: &str,
        module: &crate::session::Module,
        src: &str,
        is_decl: bool,
        as_dep: bool,
    ) -> Option<Unit> {
        let (ast, d) = parse(src, if is_decl { Mode::Decl } else { Mode::Impl });
        if !d.is_empty() {
            self.diags.extend(d);
            return None;
        }
        let uses = uses_of(&ast);
        // the mounted prelude rides every compilation unit: its AMBIENT
        // names need no `use` (a `use core::{ .. }` statement stays
        // legal but redundant for them), while the `pub builtin`
        // spellings bind only when the module's use named them — the
        // binding loops read each row's ambient bit.
        let mut uses = uses;
        if spec != "core" && self.session.resolve("core").is_ok() && !uses.iter().any(|u| u == "core")
        {
            uses.push("core".to_string());
        }

        // The splice composition (dep-kinds survey §2.4): every dep's
        // leaf list extends ours SKIPPING specs already present — first
        // position wins, so the order stays topological and a shared
        // transitive inline pkg splices exactly once, no matter how many
        // sibling uses ride it. A dep's accepted leaves join with the
        // recursive "\n\n" seam (the old combined-text shape), each
        // dep's subtree closes with the loop's "\n", and the unit's own
        // source follows the final "\n" — byte-identical to the old
        // per-dep `extra + "\n" + src` shape whenever nothing is
        // deduped (T13).
        let mut extra = String::new();
        let mut leaves: Vec<(String, String)> = Vec::new();
        // the origin map (RFC 0012 §2a): each accepted leaf's byte range
        // in the combined text + the pkg whose source it is — pure
        // metadata, the text's layout is untouched (T13's byte-identity
        // holds). The unit's own leaf closes it below.
        let mut origins: Vec<rut_lir::check::OriginLeaf> = Vec::new();
        let mut spliced: HashSet<String> = HashSet::new();
        let mut bound: Vec<(rut_core::ScopeId, rut_core::binary::Surface, String)> = Vec::new();
        let mut bound_scopes = HashSet::new();
        for dep in &uses {
            match self.ensure(dep, true)? {
                Unit::Inline { leaves: dep_leaves, bound: b } => {
                    let mut accepted_here = 0usize;
                    for (dep_spec, src) in dep_leaves {
                        if spliced.insert(dep_spec.clone()) {
                            if accepted_here > 0 {
                                extra.push_str("\n\n");
                            }
                            let lo = extra.len() as u32;
                            extra.push_str(&src);
                            origins.push(rut_lir::check::OriginLeaf {
                                lo,
                                hi: extra.len() as u32,
                                spec: dep_spec.clone(),
                            });
                            leaves.push((dep_spec, src));
                            accepted_here += 1;
                        }
                    }
                    if accepted_here > 0 {
                        extra.push('\n');
                    }
                    for (sc, surf, spc) in b {
                        if bound_scopes.insert(sc) {
                            bound.push((sc, surf, spc));
                        }
                    }
                }
                Unit::Linked { idx, scope } => {
                    if bound_scopes.insert(scope) {
                        // the exporter's spec rides the binding (RFC 0012
                        // §2a) — a linked dep contributes no text, so its
                        // names' origins travel here, not on the map
                        bound.push((scope, self.programs[idx].surface.clone(), dep.clone()));
                    }
                }
            }
        }

        // the compilation unit: inlined generic deps, then this module.
        // A declaration unit (`.d.rut`) takes no spliced bodies — its
        // use statements bind surfaces only; a decl file is pure surface
        // (RFC 0029), and Decl mode rejects implementations.
        let own_leaf = (spec.to_string(), src.to_string());
        let src_len = src.len() as u32;
        let combined = if extra.is_empty() || is_decl {
            src.to_string()
        } else {
            format!("{extra}\n{src}")
        };
        // the own leaf closes the origin map (RFC 0012 §2a): after the
        // final seam when spliced text precedes it, the whole text
        // otherwise. A decl unit compiles its own source only — the
        // spliced leaves never entered the text, so they leave no ranges.
        if is_decl {
            origins.clear();
        }
        let own_lo = if extra.is_empty() || is_decl { 0 } else { extra.len() as u32 + 1 };
        origins.push(rut_lir::check::OriginLeaf {
            lo: own_lo,
            hi: own_lo + src_len,
            spec: spec.to_string(),
        });
        let scope = self.next_scope;
        self.next_scope += 1;
        let out = compile_program_resolved(
            &combined,
            if is_decl { Mode::Decl } else { Mode::Impl },
            spec,
            scope,
            &bound,
            true,
            &origins,
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
        // NOTE (deferred): a MIXED module — a compiled body plus a host
        // surface (calc.rut helpers alongside `sqrt`..`fma`) — is
        // deliberately NOT built in this phase; `Math` stays wholly on
        // the Rust side (rut-std bodies + mount_calc). It lands with the
        // host-pkgs plan, where calc becomes a declared package.
        // an explicitly-inlined module (e.g. `ink`), a generic export,
        // or a trait-param export cannot be linked — splice the leaves
        // (and the uses) in. A decl unit's combined text is its own
        // source only (no spliced bodies), so its leaf list is its own
        // leaf alone — exactly what a consumer of the old combined text
        // used to splice.
        if as_dep && linkable(&program, module.inline) == Linkability::SpliceNeeded {
            self.inline.insert(spec.to_string());
            let mut leaves = leaves;
            if is_decl {
                // the combined text is the decl source alone
                leaves = vec![own_leaf];
            } else {
                // post-order: the unit's own leaf closes the list
                leaves.push(own_leaf);
            }
            let unit = Unit::Inline { leaves, bound };
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
