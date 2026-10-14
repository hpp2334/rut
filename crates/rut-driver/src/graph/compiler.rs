//! The graph walk itself: [`GraphCompiler`] ensures every module in
//! the closure, compiles each under its own scope, and routes the
//! owner-anchored instantiation requests.

use std::collections::{HashMap, HashSet};

use rut_lexer::diag::Diag;
use rut_lexer::span::Span;
use rut_core::binary::Program;
use rut_parser::{parse, Mode};

use rut_ast::ast::{Ast, ItemKind, Vis};

use super::build_seed_groups;
use crate::session::{PkgBody, Session};
use crate::compile::compile_program_resolved;
use crate::Seeds;

/// A resolved module: the claim ticket into the walk's tables — the
/// pushed program's index and the unit's scope. One compilation model:
/// every module links.

#[derive(Clone)]
pub(super) struct Unit {
    pub(super) idx: usize,
    pub(super) scope: rut_core::ScopeId,
}

pub(super) struct GraphCompiler<'a> {
    pub(super) session: &'a Session,
    pub(super) next_scope: rut_core::ScopeId,
    /// post-order: dependencies precede their users
    pub(super) programs: Vec<Program>,
    pub(super) done: HashMap<String, Unit>,
    pub(super) visiting: HashSet<String>,
    pub(super) diags: Vec<Diag>,
    /// linked SOURCE units' exact compile inputs — the recompile a
    /// request-heavy owner gets replays them verbatim, seeds aside
    pub(super) unit_src: HashMap<String, UnitSrc>,
    /// specs mid-`ensure` (a chain can nest several) → the scope their
    /// unit will carry. A compiled module mounted while its CONSUMER is
    /// still being compiled maps the consumer's packed block forward
    /// through this — the requester's rows travel inside the owner's
    /// binary (the seeded instantiations reference them).
    pub(super) in_flight: HashMap<String, rut_core::ScopeId>,
    /// how each ensured spec is mounted (source / compiled / host) —
    /// the request resolution reads it to pick the owner arm
    pub(super) body_kind: HashMap<String, u8>,
    /// instantiation requests routed to declaring packages, with the
    /// requesting unit's spec (its program holds the argument rows the
    /// seed blocks copy)
    pub(super) requests: Vec<(String, rut_lir::check::InstRequest)>,
    /// accumulated seeds per owner across resolution rounds (a second
    /// recompile must keep the first round's seeds)
    pub(super) seed_pool: HashMap<String, Vec<(String, rut_lir::check::InstRequest)>>,
    /// request keys already routed to a seed pool: a re-seeded owner's
    /// own mirrors re-emit the same rows every recompile, and feeding
    /// them back would loop the resolution forever
    pub(super) seeded_keys: HashSet<String>,
    /// owners that received at least one NEW seed this round — the only
    /// units a resolution round re-resolves
    pub(super) fresh_owners: HashSet<String>,
    /// deps' per-module decl rows (phase 3): the use validation's
    /// table, parsed once per spec and shared across consumers
    pub(super) dep_decls: HashMap<String, std::collections::BTreeMap<String, Vec<crate::mods::DeclRow>>>,
    /// The program root's spec — the dev arm's key: only the ROOT may
    /// `use` its `dev-deps` heads (D5's allowlist reads it). From
    /// `compile_graph`/`compile_units`' `root_spec`.
    pub(super) root: String,
}

/// A linked source unit's compile inputs, recorded for the owner-side
/// recompile (`ensure_source` replays them verbatim, seeds aside).
pub(super) struct UnitSrc {
    text: String,
    is_decl: bool,
    bound: Vec<(rut_core::ScopeId, rut_core::binary::Surface, String)>,
    /// the file-module layout the unit compiled with (phase 3) — the
    /// replay must reassemble the same module tree
    layout: rut_lir::check::ModInputs,
    /// the own-package use bindings the unit validated with — the
    /// replay re-binds the same bare names to their target modules
    own_uses: Vec<(String, String, String)>,
}

impl<'a> GraphCompiler<'a> {
    /// Decompose the finished walk: diags, pushed programs, and the
    /// linked-units index (spec → program + scope).
    pub(super) fn finish(
        self,
    ) -> (
        Vec<Diag>,
        Vec<Program>,
        HashMap<String, (usize, rut_core::ScopeId)>,
    ) {
        let GraphCompiler { diags, programs, done, .. } = self;
        let mut linked = HashMap::new();
        for (spec, unit) in done {
            linked.insert(spec, (unit.idx, unit.scope));
        }
        (diags, programs, linked)
    }

    /// Compile (or mount) `spec` if needed — every module links.
    pub(super) fn ensure(&mut self, spec: &str) -> Option<Unit> {
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
            PkgBody::Compiled(prog) => {
                self.body_kind.insert(spec.to_string(), 1);
                // the unit's scope is assigned BEFORE its deps ensure,
                // same law as the source arm: a dep's carried seed rows
                // may reference THIS unit's packed block (a requester's
                // rows riding the owner's binary — `Vec<Shape>` with the
                // element interface the requester declared), and the
                // mid-compile bind needs the number now
                let scope = self.next_scope;
                self.next_scope += 1;
                self.in_flight.insert(spec.to_string(), scope);
                let out = self.ensure_compiled(spec, prog, scope, &module.bundle_scopes);
                self.in_flight.remove(spec);
                out
            }
            PkgBody::Host { .. } => {
                self.body_kind.insert(spec.to_string(), 2);
                self.ensure_host(spec)
            }
            PkgBody::Source { text, is_decl } => {
                self.body_kind.insert(spec.to_string(), 0);
                // the unit's scope is assigned BEFORE its deps ensure: a
                // compiled dep's binary may carry this unit's packed rows
                // (the seeds it was packed with reference its consumer),
                // and the forward mapping needs the number now
                let scope = self.next_scope;
                self.next_scope += 1;
                self.in_flight.insert(spec.to_string(), scope);
                let out = self.ensure_source(spec, module, text, *is_decl, scope);
                self.in_flight.remove(spec);
                out
            }
        }
    }

    /// The declaration-gated ensure — the use law at the graph (D5's
    /// compile-side half). Consumer `spec` may ensure and bind only the
    /// heads its manifest declares (`deps` ∪ `peer-deps` ∪ the
    /// program root's `dev-deps`; `core` rides unconditionally): a
    /// mounted-but-undeclared head is D5, diagnosed in place, the unit
    /// fails; an unmounted one is `ensure`'s own resolve error
    /// (D2/NoModule — the same resolver text either way, so the miss
    /// diagnoses exactly once).
    fn ensure_use(&mut self, spec: &str, dep: &str) -> Option<Unit> {
        if let Err(e) = self.session.resolve_use(spec, dep, &self.root) {
            self.diags.push(Diag::new(Span::new(0, 0), e.to_string()));
            return None;
        }
        self.ensure(dep)
    }

    /// The native-module path: no rut body — synthesize
    /// a placeholder program whose bodyless funcs the embedder
    /// implements. Intrinsics (compiler-lowered) and constants ride the
    /// same surface. `core` rides it too: no funcs, just the native
    /// type/trait/fn names of the prelude.
    fn ensure_host(&mut self, spec: &str) -> Option<Unit> {
        let module = self.session.resolve(spec).ok()?;
        let PkgBody::Host { host_funcs, consts, native_types, native_fns, native_impls, .. } =
            &module.body
        else {
            return None;
        };
        {
            use rut_core::binary::{FuncCode, Program};
            // the host-fn registration scope IS the package name (the
            // registration naming has no override)
            let host_scope = spec;
            // host functions obey the same crossing rule as `entry fn`
            //
            let boot_tt = rut_core::types::TypeTable::boot();
            // a fresh interner answers the row names: every boot row
            // carries a well-known symbol, and the named-vs-tuple
            // question only arises for `Data` rows — a boot signature
            // spells the fixed crossing table, never a record
            let names = rut_core::Interner::default();
            for row in host_funcs {
                let bad = row.params.iter().any(|p| !boot_tt.crosses_boundary(&names, *p))
                    || !boot_tt.crosses_boundary(&names, row.ret);
                if bad {
                    self.diags.push(Diag::new(
                        Span::new(0, 0),
                        format!(
                            "host function `{}`: only primitives, `str`, `nil`, `bytes`, `?T` over those, anonymous tuples of crossable types, and `opaque` cross the host boundary",
                            row.full_name(host_scope),
                        ),
                    ));
                    return None;
                }
            }
            let scope = self.next_scope;
            self.next_scope += 1;
            let mut surface = rut_core::binary::Surface::default();
            let mut funcs = Vec::new();
            for (i, row) in host_funcs.iter().enumerate() {
                // the registration name: `pkg::path::name` through the
                // row's own full_name — the ONE source (flat rows are
                // the `path = ""` case, spelling today's shape)
                let host = row.full_name(host_scope);
                // the surface name: the row's mod-path-qualified
                // spelling (`net::connect`) — the qualified-use binding
                // composes the consumer's use path against it; root
                // rows stay bare (`connect`), the flat byte-identical
                // shape
                let qual = row.rel_name();
                surface.funcs.push(rut_core::binary::SurfaceFn {
                    name: surface.names.intern(&qual),
                    params: row.params.clone(),
                    ret: row.ret,
                    local: i as u32,
                    is_async: row.is_async,
                    // the registration name rides the surface: the
                    // importer derives the minted row family's host ids
                    // from it, so `register_async!`'s one literal stays
                    // the source of truth on both sides
                    host: Some(surface.names.intern(&host)),
                });
                funcs.push(FuncCode {
                    name: surface.names.intern(&qual),
                    params: row.params.clone(),
                    ret: row.ret,
                    is_method: false,
                    n_captures: 0,
                    regs: vec![],
                    argv: vec![],
                    labels: vec![],
                    code: vec![],
                    spans: vec![],
                    pos: vec![],
                    host_id: Some(surface.names.intern(&host)),
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
            surface.native_fns = native_fns
                .iter()
                .map(|(n, a)| (surface.names.intern(n), *a))
                .collect();
            // the integer prims' numeric methods —
            // bound ambient on the receiver primitive, no use gate
            surface.native_impls = native_impls
                .iter()
                .map(|(t, n, i)| (*t, surface.names.intern(n), *i))
                .collect();
            let interner = surface.names.clone();
            let program = Program { name: spec.to_string(), scope, interner, surface, funcs, ..Default::default() };
            let idx = self.programs.len();
            self.programs.push(program);
            let unit = Unit { idx, scope };
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
    /// load-time value, rebase, push. A scope naming a unit that is
    /// being compiled RIGHT NOW (the requester whose seeded rows travel
    /// in this binary) maps forward through `in_flight`.
    fn ensure_compiled(
        &mut self,
        spec: &str,
        prog: &Program,
        scope: rut_core::ScopeId,
        ledger: &[(rut_core::id::ScopeId, String)],
    ) -> Option<Unit> {
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
            let dep_spec = match self.session.bundle_scope(pack_scope).map(str::to_string) {
                Some(s) => s,
                None => {
                    self.diags.push(Diag::new(
                        Span::new(0, 0),
                        format!(
                            "module `{spec}` references scope {pack_scope}, which the bundle's scope ledger does not name — the bundle is incomplete or corrupt"
                        ),
                    ));
                    return None;
                }
            };
            // the requester mid-compile: its scope exists (assigned at
            // ensure entry); bind the pack-time block to it directly
            if let Some(&live) = self.in_flight.get(&dep_spec) {
                mapped.push((pack_scope, live));
                continue;
            }
            let dep = self.ensure(&dep_spec)?;
            mapped.push((pack_scope, dep.scope));
        }
        // the bundle's OWN ledger: the pack-time module set (this root
        // plus its groups), ascending — the same post-order replay, dep
        // before user. v21's split-pack root carries its bodies as
        // riding source, so the binary's foreign scan sees none of
        // these; the ledger is the mounted bundle's own word on the
        // closure it ships. A row mid-compile (the requester whose rows
        // this closure carries) rides `in_flight` — nothing to ensure.
        let mut ledger_rows: Vec<(rut_core::id::ScopeId, &str)> =
            ledger.iter().map(|(s, sp)| (*s, sp.as_str())).collect();
        ledger_rows.sort_by_key(|(s, _)| *s);
        for (_, dep_spec) in &ledger_rows {
            // the ambient prelude rides no manifest row (the source
            // walk's own law — the prelude ride below re-ensures it when
            // mounted); a packed graph's ledger carries its scope row
            if *dep_spec == "core" {
                continue;
            }
            if *dep_spec == spec || self.in_flight.contains_key(*dep_spec) {
                continue;
            }
            self.ensure(dep_spec)?;
        }
        // the prelude ride, exactly as the source walk spells it: every
        // unit binds `core`'s ambient names whether or not its ids
        // reference core's scope
        if spec != "core" && self.session.resolve("core").is_ok() {
            self.ensure("core")?;
        }
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
        let unit = Unit { idx, scope };
        self.done.insert(spec.to_string(), unit.clone());
        Some(unit)
    }

    /// The compile path: parse the module's source, resolve its uses,
    /// compile the unit, link it. The unit's text is its own source
    /// plus its peer-integration groups (presence-gated:
    /// the session recorded the group texts whose optional peers are
    /// in this closure) — every decl's origin IS this module, so no
    /// origin map exists. `scope` was assigned by the dispatcher
    /// before the deps ensured (the forward mapping a mounted owner
    /// binary needs).
    fn ensure_source(
        &mut self,
        spec: &str,
        module: &crate::session::Pkg,
        src: &str,
        is_decl: bool,
        scope: rut_core::ScopeId,
    ) -> Option<Unit> {
        // the unit's text: this module's source, then the mounted mod
        // files (phase 3 — each file keeps its own module scope; the
        // concatenation is the compilation's byte layout, the file
        // ranges attribute every decl to its module), then the
        // presence-gated peer groups (the "\n" seams are the old append
        // shape; peers splice into the ROOT module — integration text).
        let mut combined = src.to_string();
        let mut files: Vec<(u32, String)> = vec![(0, String::new())];
        for (path, m) in &module.mods {
            combined.push('\n');
            files.push((combined.len() as u32, path.clone()));
            combined.push_str(&m.text);
        }
        if !is_decl {
            for group in self.session.resolve(spec).map(|m| m.peer_groups.as_slice()).unwrap_or_default() {
                combined.push('\n');
                // the peer splices attribute to the root module: a
                // `""` sentinel row past the last child
                files.push((combined.len() as u32, String::new()));
                combined.push_str(&group);
            }
        }
        // a declaration-mode pkg (a `.d.rut` surface) is body-free — a
        // mod tree under it is incoherent (the children are bodies);
        // refuse loudly, compile flat
        if is_decl && !module.mods.is_empty() {
            self.diags.push(Diag::new(
                Span::new(0, 0),
                format!(
                    "`{spec}` is a declaration-mode pkg (`.d.rut`) and mounts {} file module(s) — \
                     surface files are body-free; move the `mod` tree under an implementation pkg",
                    module.mods.len()
                ),
            ));
            return None;
        }
        // the file layout BEFORE the parse: a failed parse's diags
        // attribute the same way (the file ranges are byte
        // arithmetic, text aside)
        let layout = mod_inputs_of(module, files);
        let (ast, d) = parse(&combined, if is_decl { Mode::Decl } else { Mode::Impl });
        if !d.is_empty() {
            self.diags.extend(attribute_diags(d, spec, &layout));
            return None;
        }
        // phase-2 plumbing: collect the pkg's mounted module set —
        // every file (root + children) parses to its own
        // `ItemKind::Module` root, declarations gathered with the mod
        // path attached. Here the collection runs for its LOUD half: a
        // declared child with no mounted module, a mounted module
        // nothing declares, a child that fails to parse — each named,
        // never guessed around.
        if !module.mods.is_empty() {
            let (_, problems) = crate::mods::collect_module_set(spec, module);
            for p in problems {
                self.diags.push(Diag::new(Span::new(0, 0), p));
            }
        }
        // phase 3: the use statements validate against the DEPS' mounted
        // trees, plus the own-package lane (the intra-package import).
        // Uses are the cross-package door — each dep segment must be a
        // `pub mod` of the dep, the leaf a `pub` decl of the walked
        // module; the own lane walks the consumer's own mod tree and
        // gates its leaves under the crossing predicate. The validated
        // dep heads ensure; the own bindings ride the compile.
        let (uses, own_uses) = match self.validate_qualified_uses(spec, module, &ast, &layout) {
            Some(uses) => uses,
            None => return None,
        };
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

        // every dep links; its surface binds into this unit — the use
        // law gates each head first (declared, not merely mounted)
        let mut bound: Vec<(rut_core::ScopeId, rut_core::binary::Surface, String)> = Vec::new();
        let mut bound_scopes = HashSet::new();
        for dep in &uses {
            let dep_unit = self.ensure_use(spec, dep)?;
            if bound_scopes.insert(dep_unit.scope) {
                // the exporter's spec rides the binding
                bound.push((dep_unit.scope, self.programs[dep_unit.idx].surface.clone(), dep.clone()));
            }
        }

        let out = compile_program_resolved(
            &combined,
            if is_decl { Mode::Decl } else { Mode::Impl },
            spec,
            scope,
            &bound,
            true,
            &Seeds::none(),
            &layout,
            &own_uses,
        );
        for r in &out.requests {
            self.requests.push((spec.to_string(), r.clone()));
        }
        if !out.diags.is_empty() || out.program.is_none() {
            self.diags.extend(attribute_diags(out.diags, spec, &layout));
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
        // the exact compile inputs, for the owner-side recompile a
        // consumer request triggers (seeds aside, the replay is verbatim
        // — same surface bindings, same text, same module layout)
        self.unit_src.insert(
            spec.to_string(),
            UnitSrc { text: combined, is_decl, bound, layout, own_uses },
        );
        let idx = self.programs.len();
        self.programs.push(program);
        let unit = Unit { idx, scope };
        self.done.insert(spec.to_string(), unit.clone());
        Some(unit)
    }

    /// A request's canonical key — owner, declaration text, and the
    /// argument spellings through the requester's table. The sort order
    /// of these keys is the resolution order: deterministic for the same
    /// source, byte-stable for the same pack.
    fn request_key(&self, requester: &str, r: &rut_lir::check::InstRequest) -> String {
        let Some((idx, _)) = self.done.get(requester).map(|u| (u.idx, ())) else {
            return format!("{}#?", r.owner);
        };
        let prog = &self.programs[idx];
        let decl = prog.interner.name(r.decl).to_string();
        let args = r
            .args
            .iter()
            .map(|&a| {
                // the semantic spelling: boot rows by id, everything else
                // by the row's own name text. A reseeded owner's own-block
                // locals shift between rounds (its table grows with every
                // materialized body), and a scope-packed canon would read
                // the same request as a new one forever. The text — not
                // the interner id — crosses compiles.
                if rut_core::scope_of(a) == rut_core::BOOT_SCOPE {
                    format!("b{}", rut_core::local_of(a))
                } else {
                    prog.interner.name(prog.types.type_at(a).name).to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        // the kind + methods ride the key: one (decl, args) pair can carry
        // several method-body requests, and a fn body request is not a
        // type instantiation
        let kind = if r.is_fn { "f" } else { "t" };
        let methods = r
            .methods
            .iter()
            .map(|&m| prog.interner.name(m).to_string())
            .collect::<Vec<_>>()
            .join(",");
        format!("{}#{}#{}<{}>#{}", r.owner, kind, decl, args, methods)
    }

    /// The reachable descriptor closure of `args` in `prog`'s table, as
    /// one sparse block per scope: rows at their exact locals (padding
    /// shells fill gaps), so the owner registering the block reproduces
    /// the requester's id space — the two sides spell every argument the
    /// same way. Boot rows pass through and carry no descriptor. Rows
    /// under `owner_scope` skip entirely: those are the owner's OWN
    /// types as the requester spelled them — the owner's recompile
    /// re-derives them from its own source at the same locals, and a
    /// seed copy would collide with the fresh rows (same scope number).
    pub(super) fn desc_closure(
        prog: &Program,
        args: &[rut_core::types::TypeId],
        owner_scope: rut_core::ScopeId,
    ) -> std::collections::BTreeMap<(rut_core::ScopeId, u32), rut_core::types::RutType> {
        let mut out = std::collections::BTreeMap::new();
        let mut stack: Vec<rut_core::types::TypeId> = args.to_vec();
        while let Some(id) = stack.pop() {
            if id == u32::MAX {
                continue;
            }
            let s = rut_core::scope_of(id);
            if s == owner_scope {
                continue;
            }
            if s == rut_core::BOOT_SCOPE {
                continue;
            }
            let l = rut_core::local_of(id);
            if out.contains_key(&(s, l)) {
                continue;
            }
            let dense = prog.types.dense(id);
            let Some(row) = prog.types.types.get(dense as usize) else { continue };
            
            out.insert((s, l), row.clone());
            match &row.kind {
                rut_core::types::TyKind::Array { elem }
                | rut_core::types::TyKind::Opt { elem }
                | rut_core::types::TyKind::Weak { elem } => stack.push(*elem),
                rut_core::types::TyKind::Data { fields } => {
                    for f in fields {
                        stack.push(f.ty);
                    }
                }
                rut_core::types::TyKind::Fn { params, ret } => {
                    stack.extend_from_slice(params);
                    stack.push(*ret);
                }
                _ => {}
            }
        }
        out
    }

    /// Resolve the consumer requests: instantiation is owned by the
    /// declaring package, and each request routes to its owner. A SOURCE
    /// owner recompiles with the seeds (the requesters' descriptors as
    /// sparse blocks — the locals match the requester's layout — plus
    /// the instantiations to materialize before its compile roots). A
    /// PACKAGED owner grows the same way when its pkg RIDES generic
    /// source: the ridden text lowers in this session and the
    /// monomorphized bodies compile at the link, owner = the pkg's spec.
    /// A packaged owner that rides no source — non-generic by the
    /// packer's riding law — must already carry the row, or the
    /// consumer refuses loudly (re-pack it). Seeds cascade — a
    /// seeded owner's compile may request from further owners — until
    /// the queue drains; every round processes in canonical (sorted)
    /// order.
    pub(super) fn resolve_requests(&mut self) {
        for _round in 0..32 {
            if self.requests.is_empty() {
                return;
            }
            let mut requests = std::mem::take(&mut self.requests);
            requests.sort_by_cached_key(|(requester, r)| self.request_key(requester, r));
            for (requester, r) in requests {
                let key = self.request_key(&requester, &r);
                if self.seeded_keys.contains(&key) {
                    continue;
                }
                self.seeded_keys.insert(key);
                self.fresh_owners.insert(r.owner.clone());
                self.seed_pool.entry(r.owner.clone()).or_default().push((requester, r));
            }
            // only owners that received NEW seeds this round re-resolve:
            // a reseed re-emits its own mirrors' requests, and re-running
            // an unchanged owner would feed the loop forever
            let mut owners: Vec<String> = self.fresh_owners.drain().collect();
            owners.sort();
            for owner in owners {
                let Some(seeds) = self.seed_pool.get(&owner).cloned() else { continue };
                match self.body_kind.get(&owner).copied() {
                    Some(0) => self.reseed_source_owner(&owner, &seeds),
                    Some(1) => self.reseed_compiled_owner(&owner, &seeds),
                    _ => {
                        let first = &seeds[0].1;
                        let decl = first
                            .decl
                            .0
                            .to_string();
                        self.diags.push(Diag::new(
                            Span::new(0, 0),
                            format!(
                                "instantiation request for `{owner}` (decl {decl}) — the pkg is mounted as a host module and declares no generics"
                            ),
                        ));
                    }
                }
                // the pool ACCUMULATES: a later round's reseed must carry
                // every seed this owner has seen, or the replacement
                // program drops the bodies an earlier round already
                // materialized (a consumer's mirror then claims nothing).
                // The loop ends when the request queue drains; identical
                // seeds re-materialize nothing (ensure_inst dedups).
                if !self.diags.is_empty() {
                    return;
                }
            }
        }
        if !self.requests.is_empty() {
            self.diags.push(Diag::new(
                Span::new(0, 0),
                "instantiation requests did not settle in 32 resolution rounds — a generic cycle the request law cannot close",
            ));
        }
    }

    /// One seed's argument list in the OWNER's id space: the seed's args
    /// name the requester's registered block rows, which the recompile
    /// registers verbatim — the ids carry over untouched.
    fn reseed_source_owner(&mut self, owner: &str, seeds: &[(String, rut_lir::check::InstRequest)]) {

        let Some((idx, scope)) = self.done.get(owner).map(|u| (u.idx, u.scope)) else {

            return;
        };
        let Some(src) = self.unit_src.get(owner) else {

            return;
        };
        let (text, is_decl, bound) = (src.text.clone(), src.is_decl, src.bound.clone());
        let layout = src.layout.clone();
        let own_uses = src.own_uses.clone();
        let groups = build_seed_groups(&self.programs, &self.done, owner, seeds);
        let out = compile_program_resolved(
            &text,
            if is_decl { Mode::Decl } else { Mode::Impl },
            owner,
            scope,
            &bound,
            true,
            &Seeds { groups: &groups },
            &layout,
            &own_uses,
        );
        for r in &out.requests {
            self.requests.push((owner.to_string(), r.clone()));
        }
        if !out.diags.is_empty() || out.program.is_none() {
            self.diags.extend(attribute_diags(out.diags, owner, &layout));
            return;
        }
        self.programs[idx] = out.program.expect("checked above");
    }

    /// A packaged owner whose pkg RIDES generic source grows at the
    /// consumer's link: the request missed the pack-time ledger, so the
    /// ridden source lowers in THIS session (the consumer's) — same
    /// scope as the mounted unit, the same seeds a directory owner
    /// would get — and the recompiled program replaces the mounted one
    /// in place. Identity is the owner's spec (`compile_program_resolved`
    /// under the owner's name), so the materialized rows are
    /// indistinguishable in the ledger from pack-time ones: ONE row
    /// program-wide, link unifies the mirrors onto it. Nothing persists:
    /// this is per-load compilation, the `.rutc` caches stay pack-time.
    /// A compiled owner that rides no source (non-generic by the
    /// packer's riding law) still cannot grow: the loud refusal names
    /// the fix.
    fn reseed_compiled_owner(&mut self, owner: &str, seeds: &[(String, rut_lir::check::InstRequest)]) {
        let Some((idx, scope)) = self.done.get(owner).map(|u| (u.idx, u.scope)) else {
            return;
        };
        let Some(gen) = self
            .session
            .resolve(owner)
            .ok()
            .and_then(|m| m.gen_source.clone())
        else {
            // the no-riding law: the ledger lacks the shape AND no
            // source rides — the binary must already carry the shape
            self.check_compiled_owner(owner, seeds);
            return;
        };
        // the unit's text: the ridden source, then the mounted mod
        // files (phase 3 — a mod-rooted pkg's children ride the rows and
        // mount beside it; the recompile must reassemble the SAME tree
        // the pack-time unit compiled), then the peers' group files
        // whose peers are in this program's closure (the presence law,
        // peer-name order — the exact splice a directory owner's gate
        // performs)
        let mut text = gen.text.clone();
        let mut files: Vec<(u32, String)> = vec![(0, String::new())];
        let module = self.session.resolve(owner).ok();
        for (path, m) in module.iter().flat_map(|m| m.mods.iter()) {
            text.push('\n');
            files.push((text.len() as u32, path.clone()));
            text.push_str(&m.text);
        }
        for (peer, group) in &gen.peers {
            if self.session.resolve(peer).is_ok() {
                text.push('\n');
                files.push((text.len() as u32, String::new()));
                text.push_str(group);
            }
        }
        // parse + bind: the ridden source's uses resolve at their
        // load-time scopes — every foreign scope the mounted binary
        // references is already ensured (the compiled-mount arm ran
        // them in ledger order before pushing this unit). The layout
        // exists BEFORE the parse: a failed parse's diags attribute
        // the same way (byte arithmetic, text aside)
        let layout = module
            .as_ref()
            .map(|m| mod_inputs_of(m, files))
            .unwrap_or_else(|| rut_lir::check::ModInputs::flat());
        let (ast, d) = parse(&text, Mode::Impl);
        if !d.is_empty() {
            self.diags.extend(attribute_diags(d, owner, &layout));
            return;
        }
        // the dep heads only: a head the owner's own package claims is
        // the intra-package lane — the compile binds it through the
        // layout (the walk validated at the pack-time compile)
        let mut uses = dep_use_heads(self.session, owner, &self.root, &layout, &ast);
        // the prelude ride, exactly as the source walk spells it
        if owner != "core" && self.session.resolve("core").is_ok() && !uses.iter().any(|u| u == "core") {
            uses.push("core".to_string());
        }
        // the own-package bindings re-derive silently: the text is
        // byte-identical, the layout the same tree
        let own_uses = own_use_bindings(self.session, owner, &self.root, &layout, &ast);
        let mut bound: Vec<(rut_core::ScopeId, rut_core::binary::Surface, String)> = Vec::new();
        let mut bound_scopes = HashSet::new();
        for dep in &uses {
            // the use law gates here too: the ridden source's heads
            // are the OWNER pkg's uses (D5's consumer is the owner)
            let Some(dep_unit) = self.ensure_use(owner, dep) else { return };
            if bound_scopes.insert(dep_unit.scope) {
                bound.push((dep_unit.scope, self.programs[dep_unit.idx].surface.clone(), dep.clone()));
            }
        }
        let groups = build_seed_groups(&self.programs, &self.done, owner, seeds);
        let out = compile_program_resolved(
            &text,
            Mode::Impl,
            owner,
            scope,
            &bound,
            true,
            &Seeds { groups: &groups },
            &layout,
            &own_uses,
        );
        for r in &out.requests {
            self.requests.push((owner.to_string(), r.clone()));
        }
        if !out.diags.is_empty() || out.program.is_none() {
            self.diags.extend(attribute_diags(out.diags, owner, &layout));
            return;
        }
        self.programs[idx] = out.program.expect("checked above");
    }

    /// A compiled owner without riding source cannot grow: its binary
    /// carries exactly the instantiations the pack-time closure
    /// requested, and no source rides to serve anything else. Each
    /// consumer request must
    /// already have a ledger row — link unifies the requester's mirror
    /// onto it — or the closure was packed without this consumer,
    /// which is a loud refusal naming the re-pack fix, never a guess.
    fn check_compiled_owner(&mut self, owner: &str, seeds: &[(String, rut_lir::check::InstRequest)]) {
        let Some((idx, _)) = self.done.get(owner).map(|u| (u.idx, ())) else { return };
        for (requester, r) in seeds {
            let key = self.request_key(requester, r);
            let prog = &self.programs[idx];
            // the ledger side spells the same key: a type instantiation
            // row (kind `t`, no methods), its arguments through the
            // owner's own table — boot rows by id, everything else by
            // the row's name text (the same semantic canon the request
            // side speaks)
            let hit = prog.inst_types.iter().any(|row| {
                let args = row
                    .args
                    .iter()
                    .map(|&a| {
                        if rut_core::scope_of(a) == rut_core::BOOT_SCOPE {
                            format!("b{}", rut_core::local_of(a))
                        } else {
                            prog.interner.name(prog.types.type_at(a).name).to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                key == format!("{}#t#{}<{}>#", prog.interner.name(row.owner), prog.interner.name(row.decl), args)
            });
            if !hit {
                self.diags.push(Diag::new(
                    Span::new(0, 0),
                    format!(
                        "instantiation `{}` was not compiled into `{owner}`'s binary and no generic source rides — \
                         this bundle predates generic-source riding — re-pack it",
                        key
                    ),
                ));
                return;
            }
        }
    }
}


/// The dep heads one parsed unit's `use` statements name, in source
/// order, deduped. `path[0]` is the package — the dep the graph
/// ensures and binds; the mod-path segments behind it are the
/// qualified-use lanes' (validation walks the dep's tree, the binding
/// composes them onto the host mod rows). The OWN-PACKAGE lane is
/// excluded: a head the consumer's own package claims (its spec, or a
/// `mod` edge visible-walked from the use's file) is the
/// intra-package import — no package to ensure or declare; the
/// compile binds it through the layout. Silent: the compile's
/// validation owns the diagnostics, this is the ensure-list /
/// closure-check split. `layout` maps each use statement's span to
/// its file (the own walk's start).
pub(crate) fn dep_use_heads(
    session: &Session,
    spec: &str,
    root: &str,
    layout: &rut_lir::check::ModInputs,
    ast: &Ast,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for it in ast.module_items(ast.root).to_vec() {
        let ItemKind::Use { path, .. } = ast.item(it) else {
            continue;
        };
        let head = ast.name(path[0]).to_string();
        let from = layout.file_of(ast.span(it.id()).lo).to_string();
        if session.resolve_use(spec, &head, root).is_err()
            && session.own_use_head(spec, &head, &from).is_some()
        {
            continue; // the intra-package lane — no package behind it
        }
        if !out.contains(&head) {
            out.push(head);
        }
    }
    out
}

/// The file a mod path lives in, spelled for the own-lane texts: the
/// loader's physical record (`imp.rut` single-file, `imp/mod.rut`
/// directory-form), the root's body `mod.rut` — the exact file the
/// user sees, never the directory-form guess.
fn file_display(module: &crate::session::Pkg, path: &str) -> String {
    match module.mods.get(path) {
        Some(m) => m.display(),
        None if path.is_empty() => "mod.rut".to_string(),
        None => format!("{path}/mod.rut"),
    }
}

/// The own-package use bindings one parsed unit's statements carry,
/// derived silently — the reseed replay's twin of the validation's
/// own lane (the text is byte-identical, the validation already ran).
/// Each statement's head resolves through the session's own-package
/// answer (a dep-lane head joins nothing); the interior segments walk
/// the layout's mod tree; every leaf binds `(importing file, name)` →
/// target module.
pub(crate) fn own_use_bindings(
    session: &Session,
    spec: &str,
    root: &str,
    layout: &rut_lir::check::ModInputs,
    ast: &Ast,
) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    for it in ast.module_items(ast.root).to_vec() {
        let ItemKind::Use { path, names } = ast.item(it) else {
            continue;
        };
        let head = ast.name(path[0]).to_string();
        let from = layout.file_of(ast.span(it.id()).lo).to_string();
        if session.resolve_use(spec, &head, root).is_ok() {
            continue; // the dep lane — not ours
        }
        let Some(mut at) = session.own_use_head(spec, &head, &from) else {
            continue;
        };
        let mut walked = true;
        for &seg in &path[1..] {
            match layout
                .scopes
                .get(&at)
                .and_then(|s| s.children.get(ast.name(seg)))
            {
                Some((child, _vis)) => at = child.clone(),
                None => {
                    walked = false;
                    break;
                }
            }
        }
        if !walked {
            continue;
        }
        for &n in names {
            out.push((from.clone(), ast.name(n).to_string(), at.clone()));
        }
    }
    out
}

/// The file-module inputs a compilation resolves against (phase 3):
/// the concatenated text's file ranges plus the scope tree built from
/// the pkg's mounted module set (each child's parent segment implies
/// the edge; `mod` vs `pub mod` rides the edge's vis).
pub(crate) fn mod_inputs_of(
    module: &crate::session::Pkg,
    files: Vec<(u32, String)>,
) -> rut_lir::check::ModInputs {
    let mut inputs =
        rut_lir::check::ModInputs { files, scopes: std::collections::BTreeMap::new() };
    for (path, m) in &module.mods {
        let (parent, name) = match path.rsplit_once('/') {
            Some((p, n)) => (p.to_string(), n.to_string()),
            None => (String::new(), path.clone()),
        };
        inputs
            .scopes
            .entry(parent.clone())
            .or_default()
            .children
            .insert(name, (path.clone(), m.vis));
        inputs.scopes.entry(path.clone()).or_default().parent = parent;
    }
    inputs
}

/// Tag + rebase a compile's diags at the unit-append boundary: the
/// spans index the COMBINED text — the pkg's root file, its mod
/// children, the peer groups concatenated — so each diag reads its
/// file module off the layout (`ModInputs::file_of`: root file → the
/// spec, a child → `spec/path`) and rebases its span and label spans
/// to file-local bytes. A span-0 diag has no position to show and
/// stays unattributed (the graph-level refusals' shape).
fn attribute_diags(
    diags: impl IntoIterator<Item = Diag>,
    spec: &str,
    layout: &rut_lir::check::ModInputs,
) -> Vec<Diag> {
    diags
        .into_iter()
        .map(|mut d| {
            if d.span.lo == 0 && d.span.hi == 0 {
                return d;
            }
            let path = layout.file_of(d.span.lo);
            let start = layout
                .files
                .iter()
                .rev()
                .find(|(s, _)| *s <= d.span.lo)
                .map(|(s, _)| *s)
                .unwrap_or(0);
            let rebase = |s: Span| Span::new(s.lo.saturating_sub(start), s.hi.saturating_sub(start));
            d.span = rebase(d.span);
            d.labels = std::mem::take(&mut d.labels)
                .into_iter()
                .map(|(s, m)| (rebase(s), m))
                .collect();
            if path.is_empty() {
                d.in_file(spec)
            } else {
                d.in_file(format!("{spec}/{path}"))
            }
        })
        .collect()
}

impl GraphCompiler<'_> {
    /// The use statements validate against the DEPS' mounted trees
    /// (phase 3): `use dep::a::b::{C}` walks `a::b` — each segment must
    /// be a `pub mod` of the dep (a plain `mod` is invisible from
    /// outside the package), and each leaf name must be a `pub` decl of
    /// the walked module. Flat uses (`use dep::{A,B}` / `use dep::A;`)
    /// validate each leaf against the dep's ROOT module — the surface
    /// the flat binding walks — at the use line: a typo'd import is
    /// rejected where it is written, not at a call site that may never
    /// be written. The OWN-PACKAGE lane rides beside them: a head that
    /// is no declared dep may name the consumer's own package — its
    /// spec naming the root module, or a `mod` edge walked from the
    /// importing file's mod path (`use imp::{TodoList}` beside
    /// `mod imp;`); each leaf must be a decl of the walked module,
    /// visible from the importing file under the crossing predicate
    /// (import is no visibility bypass), and a module name is not an
    /// import — walk it. The leaf texts spell the fix at the walked
    /// module: a miss is the dep lanes' unknown-import shape (plus the
    /// module the name does live in, when it lives in the package), a
    /// visibility refusal names the importing file and the declaring
    /// file's PHYSICAL name, a module name names the walk. A head that
    /// names BOTH a declared dep and an own mod edge is the loud
    /// ambiguity (mounts and mod edges are different namespaces; silence
    /// here would be a trap). Answers the dep heads to ensure plus the
    /// validated own bindings (`(importing file, name)` → target
    /// module) — `None` = diagnosed, the unit fails.
    fn validate_qualified_uses(
        &mut self,
        spec: &str,
        module: &crate::session::Pkg,
        ast: &Ast,
        layout: &rut_lir::check::ModInputs,
    ) -> Option<(Vec<String>, Vec<(String, String, String)>)> {
        let dep_heads = dep_use_heads(self.session, spec, &self.root, layout, ast);
        let mut ok = true;
        let mut own_uses: Vec<(String, String, String)> = Vec::new();
        let session = self.session;
        for it in ast.module_items(ast.root).to_vec() {
            let ItemKind::Use { path, names } = ast.item(it) else {
                continue;
            };
            let head_text = ast.name(path[0]).to_string();
            let span = ast.span(it.id());
            // the importing file's mod path — the own walk starts here
            let from = layout.file_of(span.lo).to_string();
            // the head's two namespaces: the mounted+declared dep
            // (today's law) and the own package. Neither answers →
            // today's resolve shapes; both → the loud ambiguity.
            let dep = session.resolve_use(spec, &head_text, &self.root).ok();
            let own_head = session.own_use_head(spec, &head_text, &from);
            if dep.is_none() && own_head.is_none() {
                let e = session
                    .resolve_use(spec, &head_text, &self.root)
                    .unwrap_err();
                self.diags.push(Diag::new(Span::new(0, 0), e.to_string()));
                ok = false;
                continue;
            }
            if let (Some(_), Some(target)) = (&dep, &own_head) {
                let what = if target.is_empty() {
                    "this package's own name (the root module)".to_string()
                } else {
                    format!(
                        "this package's own module `{head_text}` ({})",
                        rut_lir::check::ModInputs::display(target),
                    )
                };
                // the use statement's span indexes the combined text —
                // attribute it like every compile diag
                self.diags.extend(attribute_diags(
                    vec![Diag::new(
                        span,
                        format!(
                            "`{head_text}` names both the declared dependency `{head_text}` and {what} — \
                             the use path is ambiguous; rename one of the two",
                        ),
                    )],
                    spec,
                    layout,
                ));
                ok = false;
                continue;
            }
            let Some(pkg) = dep else {
                // ---- the own-package lane ----
                let mut at = own_head.expect("own lane checked above");
                // the interior segments walk child mods (`walk_mod_path`
                // semantics — edges are intra-package namespaces; the
                // LEAF visibility-gates)
                let mut walked = true;
                for &seg in &path[1..] {
                    let seg_text = ast.name(seg);
                    match layout
                        .scopes
                        .get(&at)
                        .and_then(|s| s.children.get(seg_text))
                    {
                        Some((child, _vis)) => at = child.clone(),
                        None => {
                            // attributed like every compile diag: the
                            // arrow names the use line
                            self.diags.extend(attribute_diags(
                                vec![Diag::new(
                                    span,
                                    format!(
                                        "`{seg_text}` is not a module under `{}` — only `mod` children \
                                         extend a qualified path",
                                        if at.is_empty() { head_text.as_str() } else { at.as_str() },
                                    ),
                                )],
                                spec,
                                layout,
                            ));
                            walked = false;
                            break;
                        }
                    }
                }
                if !walked {
                    ok = false;
                    continue;
                }
                // the leaves: decls of the walked module, visible from
                // the importing file under the crossing predicate. The
                // texts spell the fix: a miss reads the walked module
                // the way the use spelled it (plus where the name does
                // live, when it lives in this package); a visibility
                // refusal names the importing file and the declaring
                // file's physical name; a module name names the walk.
                let walked_name = |at: &str| -> String {
                    if at.is_empty() {
                        spec.to_string()
                    } else {
                        at.replace('/', "::")
                    }
                };
                let mut bad: Vec<Diag> = Vec::new();
                {
                    let Some(table) = self.dep_mod_table(spec) else { continue };
                    let Some(rows) = table.get(&at) else { continue };
                    for &n in names {
                        let text = ast.name(n);
                        match rows.iter().find(|d| d.name == text) {
                            None => {
                                // the name may still live in a sibling
                                // module of this package — the hint
                                // names it (first hit, mod-path order)
                                let home = table
                                    .iter()
                                    .filter(|(m, _)| m.as_str() != at.as_str())
                                    .find(|(_, rows)| rows.iter().any(|d| d.name == text))
                                    .map(|(m, _)| walked_name(m));
                                let hint = match home {
                                    Some(h) => format!(" (declared in module `{h}`)"),
                                    None => String::new(),
                                };
                                bad.push(Diag::new(
                                    span,
                                    format!(
                                        "`{}` has no `{text}` — unknown import{hint}",
                                        walked_name(&at),
                                    ),
                                ));
                            }
                            Some(d) if d.kind == crate::mods::DeclKind::Mod => {
                                let mut fix = String::new();
                                for &seg in path {
                                    fix.push_str(ast.name(seg));
                                    fix.push_str("::");
                                }
                                bad.push(Diag::new(
                                    span,
                                    format!(
                                        "`{text}` is a module of this package — a module is not \
                                         imported; walk it: `use {fix}{text}::{{ .. }}`"
                                    ),
                                ));
                            }
                            Some(d) if !rut_lir::check::vis_crosses(&from, &at, d.vis) => {
                                bad.push(Diag::new(
                                    span,
                                    format!(
                                        "`{text}` is not visible from `{}` — mark it `pub` \
                                         (or `pub(pkg)`) in `{}`",
                                        file_display(module, &from),
                                        file_display(module, &at),
                                    ),
                                ));
                            }
                            Some(_) => {
                                own_uses.push((from.clone(), text.to_string(), at.clone()));
                            }
                        }
                    }
                }
                if !bad.is_empty() {
                    // the use statement's span indexes the combined text
                    // — attribute it like every compile diag (the
                    // renderer spells `--> file:line:col` off it)
                    self.diags.extend(attribute_diags(bad, spec, layout));
                    ok = false;
                }
                continue;
            };
            // the dep lane: the head is a mounted, declared dep — the
            // nameability gate already answered above
            let dep_name = head_text;
            if path.len() < 2 {
                // the flat spelling — each leaf must be a decl of the
                // dep's root module (or the dep's namespace head). The
                // gate names only what the dep never declared, plus the
                // mod names no surface carries: visibility stays the
                // binding's business exactly as today — a private row
                // keeps importing, and what the surface never exported
                // keeps failing at its use site. A compiled dep's decls
                // ride its binary (the binding gate answers), and an
                // undeclared head keeps its D5 shape at the binding.
                if !self.dep_rows_readable(&dep_name) {
                    continue;
                }
                // the namespace head (`use calc::{Math}`) binds like a
                // leaf — the manifest rides it, never a decl row
                let ns = session
                    .resolve(&dep_name)
                    .ok()
                    .and_then(|m| m.namespace.clone());
                let bad: Vec<Diag> = {
                    let Some(table) = self.dep_mod_table(&dep_name) else { continue };
                    // no root rows to read (a hand-built host surface
                    // like `core` collects no root unit) — nothing
                    // validates here
                    let Some(rows) = table.get("") else { continue };
                    names
                        .iter()
                        .filter_map(|&n| {
                            let text = ast.name(n);
                            if Some(text) == ns.as_deref() {
                                return None;
                            }
                            match rows.iter().find(|d| d.name == text) {
                                None => Some(Diag::new(
                                    span,
                                    format!("`{dep_name}` has no `{text}` — unknown import"),
                                )),
                                Some(d) if d.kind == crate::mods::DeclKind::Mod => Some(Diag::new(
                                    span,
                                    format!(
                                        "`{text}` is a module of `{dep_name}` — a module is not \
                                         imported; walk it: `use {dep_name}::{text}::{{ .. }}`"
                                    ),
                                )),
                                Some(_) => None,
                            }
                        })
                        .collect()
                };
                if !bad.is_empty() {
                    // the use statement's span indexes the combined text
                    // — attribute it like every compile diag (the
                    // renderer spells `--> file:line:col` off it)
                    self.diags.extend(attribute_diags(bad, spec, layout));
                    ok = false;
                }
                continue;
            }
            // the nameability gate already answered: `pkg` is the
            // declared dep the head resolved to. Mod-path validation
            // walks only declared heads.
            // the dep's tree flavor names the declaring files: a host
            // surface's children are `.d.rut` files (`mod.d.rut`), a
            // lib body's are `mod.rut` — the diagnostics spell the one
            // that exists
            let child_file = match &pkg.body {
                PkgBody::Host { .. } => crate::decl::DECL_CHILD_FILE,
                _ => "mod.rut",
            };
            let at_file = |at: &str| -> String {
                if at.is_empty() {
                    child_file.to_string()
                } else {
                    format!("{at}/{child_file}")
                }
            };
            // walk the mod path, one `pub mod` edge per segment
            let mut at = String::new();
            let mut walked = true;
            for &seg in &path[1..] {
                let seg_text = ast.name(seg);
                let child = if at.is_empty() {
                    seg_text.to_string()
                } else {
                    format!("{at}/{seg_text}")
                };
                let Some(edge) = pkg.mods.get(&child) else {
                    if pkg.mods.is_empty() {
                        self.diags.push(Diag::new(
                            Span::new(0, 0),
                            format!(
                                "`{seg_text}` is not a module of `{dep_name}` — `{dep_name}` is a flat package; \
                                 its names bind flat: `use {dep_name}::{{ .. }}`"
                            ),
                        ));
                    } else {
                        self.diags.push(Diag::new(
                            Span::new(0, 0),
                            format!(
                                "`{seg_text}` is not a module of `{dep_name}` — no `mod {seg_text};` under {} \
                                 (`use {dep_name}::..` walks the dep's mounted `mod` tree)",
                                at_file(&at),
                            ),
                        ));
                    }
                    walked = false;
                    break;
                };
                if edge.vis != Vis::Pub {
                    self.diags.push(Diag::new(
                        Span::new(0, 0),
                        format!(
                            "`{seg_text}` is not visible from outside the package (`mod`, not `pub mod`) — \
                             spell `pub mod {seg_text};` in `{dep_name}`'s {} to widen it",
                            at_file(&at),
                        ),
                    ));
                    walked = false;
                    break;
                }
                at = child;
            }
            if !walked {
                ok = false;
                continue;
            }
            // the leaf names: each must be a `pub` decl of the walked
            // module (when the dep's rows are readable — a source body
            // and a host surface both carry their text; a compiled
            // unit's decls ride its binary, the binding gate answers)
            let rows_readable = self.dep_rows_readable(&dep_name);
            // the verdicts compute under the table borrow, then diagnose
            let verdicts: Vec<Option<String>> = {
                let Some(table) = self.dep_mod_table(&dep_name) else { continue };
                let Some(rows) = table.get(&at) else { continue };
                if rows.is_empty() && !rows_readable {
                    continue; // a compiled unit's decls are its binary's
                }
                names
                    .iter()
                    .map(|&n| {
                        let text = ast.name(n);
                        match rows.iter().find(|d| d.name == text) {
                            None => Some(format!(
                                "`{text}` is not declared in `{dep_name}`'s module `{at}` ({})",
                                at_file(&at),
                            )),
                            Some(d) if d.vis != Vis::Pub => Some(format!(
                                "`{text}` in `{dep_name}::{}` is not `pub` — only `pub` names cross packages",
                                at.replace('/', "::"),
                            )),
                            _ => None,
                        }
                    })
                    .collect()
            };
            for msg in verdicts.into_iter().flatten() {
                self.diags.push(Diag::new(Span::new(0, 0), msg));
                ok = false;
            }
        }
        if !ok {
            return None;
        }
        Some((dep_heads, own_uses))
    }

    /// The dep's per-module decl rows (parsed once per spec — the
    /// consumers share it). A `PkgBody::Host` dep collects from its
    /// root surface + mounted tree (the body carries `root_surface`,
    /// the pkg carries `mods`), so a qualified-use leaf naming a host
    /// row validates against the path-carrying rows. The PEER GROUPS'
    /// decls join the root module's rows: a group splices into the
    /// root at compile time (the `""` sentinel file), so its `pub`
    /// decls are the unit's exports too — the leaf gates read the
    /// surface the unit actually compiles, group text included.
    fn dep_mod_table(
        &mut self,
        spec: &str,
    ) -> Option<&std::collections::BTreeMap<String, Vec<crate::mods::DeclRow>>> {
        if !self.dep_decls.contains_key(spec) {
            let pkg = self.session.resolve(spec).ok()?;
            let (set, _) = crate::mods::collect_module_set(spec, pkg);
            let mut table: std::collections::BTreeMap<String, Vec<crate::mods::DeclRow>> =
                set.units.into_iter().map(|u| (u.path, u.decls)).collect();
            if !pkg.peer_groups.is_empty() {
                let root = table.entry(String::new()).or_default();
                for group in &pkg.peer_groups {
                    let (ast, d) = rut_parser::parse(group, Mode::Impl);
                    if d.is_empty() {
                        root.extend(crate::mods::decl_rows(&ast));
                    }
                }
            }
            self.dep_decls.insert(spec.to_string(), table);
        }
        self.dep_decls.get(spec)
    }

    /// Does the dep's decl text ride the pkg (a source body or a host
    /// surface)? Its per-module rows are then collectible — the leaf
    /// gate runs. A compiled unit's rows ride its binary instead.
    fn dep_rows_readable(&self, spec: &str) -> bool {
        self.session
            .resolve(spec)
            .map(|m| matches!(m.body, crate::session::PkgBody::Source { .. } | crate::session::PkgBody::Host { .. }))
            .unwrap_or(false)
    }
}
