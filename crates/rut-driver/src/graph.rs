//! Module-graph compilation — the driver half.
//!
//! Walks a root module's `use` statements through a [`Session`]. Every
//! module compiles under its own scope and LINKS: its `Surface` binds
//! into each using module, class methods cross on the surface's
//! inherent rows (the linkable-classes phase), and a generic export's
//! instantiations are requested from the declaring package
//! (owner-anchored generics). No source crosses a boundary — there is
//! no splice, no second compilation model.
//!
//! The dispatch is on the mounted [`ModuleBody`]: a source body takes
//! the compile path (its peer-integration groups, presence-gated, ride
//! the same unit), a host body synthesizes its placeholder program,
//! and a compiled body (a v5 bundle's decoded `.rutc`) is pushed after
//! a fresh-scope rebase of its packed ids.
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
use crate::Seeds;

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
/// the packer: the pushed programs in post-order, and each linked
/// module's own program and scope.
pub struct Units {
    pub diags: Vec<Diag>,
    /// every pushed program, post-order (compile_graph links these)
    pub programs: Vec<Program>,
    /// each ensured-and-linked module: spec → (index into `programs`, scope)
    pub linked: HashMap<String, (usize, rut_core::ScopeId)>,
    /// `true` when the root produced a program (the walk succeeded)
    pub ok: bool,
}

/// Compile (or mount) every module in `root_spec`'s use closure — the
/// packer's view of [`compile_graph`].
pub fn compile_units(session: &Session, root_spec: &str) -> Units {
    let mut c = GraphCompiler {
        session,
        next_scope: 1,
        programs: Vec::new(),
        done: HashMap::new(),
        visiting: HashSet::new(),
        diags: Vec::new(),
        unit_src: HashMap::new(),
        in_flight: HashMap::new(),
        body_kind: HashMap::new(),
        requests: Vec::new(),
        seed_pool: HashMap::new(),
        seeded_keys: HashSet::new(),
        fresh_owners: HashSet::new(),
    };
    let ok = c.ensure(root_spec).is_some();
    c.resolve_requests();
    let (diags, programs, linked) = c.finish();
    Units { diags, programs, linked, ok }
}

/// A resolved module: the claim ticket into the walk's tables — the
/// pushed program's index and the unit's scope. One compilation model:
/// every module links.
#[derive(Clone)]
struct Unit {
    idx: usize,
    scope: rut_core::ScopeId,
}

struct GraphCompiler<'a> {
    session: &'a Session,
    next_scope: rut_core::ScopeId,
    /// post-order: dependencies precede their users
    programs: Vec<Program>,
    done: HashMap<String, Unit>,
    visiting: HashSet<String>,
    diags: Vec<Diag>,
    /// linked SOURCE units' exact compile inputs — the recompile a
    /// request-heavy owner gets replays them verbatim, seeds aside
    unit_src: HashMap<String, UnitSrc>,
    /// specs mid-`ensure` (a chain can nest several) → the scope their
    /// unit will carry. A compiled module mounted while its CONSUMER is
    /// still being compiled maps the consumer's packed block forward
    /// through this — the requester's rows travel inside the owner's
    /// binary (the seeded instantiations reference them).
    in_flight: HashMap<String, rut_core::ScopeId>,
    /// how each ensured spec is mounted (source / compiled / host) —
    /// the request resolution reads it to pick the owner arm
    body_kind: HashMap<String, u8>,
    /// instantiation requests routed to declaring packages, with the
    /// requesting unit's spec (its program holds the argument rows the
    /// seed blocks copy)
    requests: Vec<(String, rut_lir::check::InstRequest)>,
    /// accumulated seeds per owner across resolution rounds (a second
    /// recompile must keep the first round's seeds)
    seed_pool: HashMap<String, Vec<(String, rut_lir::check::InstRequest)>>,
    /// request keys already routed to a seed pool: a re-seeded owner's
    /// own mirrors re-emit the same rows every recompile, and feeding
    /// them back would loop the resolution forever
    seeded_keys: HashSet<String>,
    /// owners that received at least one NEW seed this round — the only
    /// units a resolution round re-resolves
    fresh_owners: HashSet<String>,
}

/// A linked source unit's compile inputs, recorded for the owner-side
/// recompile (`ensure_source` replays them verbatim, seeds aside).
struct UnitSrc {
    text: String,
    is_decl: bool,
    bound: Vec<(rut_core::ScopeId, rut_core::binary::Surface, String)>,
}

impl<'a> GraphCompiler<'a> {
    /// Decompose the finished walk: diags, pushed programs, and the
    /// linked-units index (spec → program + scope).
    fn finish(
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
    fn ensure(&mut self, spec: &str) -> Option<Unit> {
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
            ModuleBody::Compiled(prog) => {
                self.body_kind.insert(spec.to_string(), 1);
                self.ensure_compiled(spec, prog)
            }
            ModuleBody::Host { .. } => {
                self.body_kind.insert(spec.to_string(), 2);
                self.ensure_host(spec)
            }
            ModuleBody::Source { text, is_decl } => {
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

    /// The native-module path: no rut body — synthesize
    /// a placeholder program whose bodyless funcs the embedder
    /// implements. Intrinsics (compiler-lowered) and constants ride the
    /// same surface. `core` rides it too: no funcs, just the native
    /// type/trait/fn names of the prelude.
    fn ensure_host(&mut self, spec: &str) -> Option<Unit> {
        let module = self.session.resolve(spec).ok()?;
        let ModuleBody::Host { host_funcs, consts, native_types, native_traits, native_fns, native_impls } =
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
            for (name, params, ret, _is_async) in host_funcs {
                let bad = params.iter().any(|p| !boot_tt.crosses_boundary(*p))
                    || !boot_tt.crosses_boundary(*ret);
                if bad {
                    self.diags.push(Diag::new(
                        Span::new(0, 0),
                        format!(
                            "host function `{host_scope}::{name}`: only primitives, `str`, `bytes`, `opaque`, and `Option`/`Result` over those cross the host boundary"
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
        // the prelude ride, exactly as the source walk spells it: every
        // unit binds `core`'s ambient names whether or not its ids
        // reference core's scope
        if spec != "core" && self.session.resolve("core").is_ok() {
            self.ensure("core")?;
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
        module: &crate::session::Module,
        src: &str,
        is_decl: bool,
        scope: rut_core::ScopeId,
    ) -> Option<Unit> {
        // the unit's text: this module's source, then the presence-gated
        // peer groups (the "\n" seams are the old append shape). The
        // parse runs over the COMBINED text — the groups' `use`
        // statements join the unit's use list through the ordinary scan.
        let mut combined = src.to_string();
        if !is_decl {
            for group in self.session.peer_groups_of(spec) {
                combined.push('\n');
                combined.push_str(&group);
            }
        }
        let (ast, d) = parse(&combined, if is_decl { Mode::Decl } else { Mode::Impl });
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

        // every dep links; its surface binds into this unit
        let mut bound: Vec<(rut_core::ScopeId, rut_core::binary::Surface, String)> = Vec::new();
        let mut bound_scopes = HashSet::new();
        for dep in &uses {
            let dep_unit = self.ensure(dep)?;
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
        );
        for r in &out.requests {
            self.requests.push((spec.to_string(), r.clone()));
        }
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
        // the exact compile inputs, for the owner-side recompile a
        // consumer request triggers (seeds aside, the replay is verbatim
        // — same surface bindings, same text)
        self.unit_src.insert(
            spec.to_string(),
            UnitSrc { text: combined, is_decl, bound },
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
        let kind = if r.is_impl {
            "i"
        } else if r.is_fn {
            "f"
        } else {
            "t"
        };
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
    fn desc_closure(
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
    /// the instantiations to materialize before its compile roots); a
    /// PACKAGED owner's binary must already carry the row, or the
    /// consumer refuses loudly (re-pack with the consumer in the
    /// closure). Seeds cascade — a seeded owner's compile may request
    /// from further owners — until the queue drains; every round
    /// processes in canonical (sorted) order.
    fn resolve_requests(&mut self) {
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
                    Some(1) => self.check_compiled_owner(&owner, &seeds),
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
        // one seed group per requester: the descriptor closure of every
        // request's arguments, sparse at the requester's locals — the
        // attach side merges the groups into one padded run per scope
        let mut per_requester: std::collections::BTreeMap<
            String,
            std::collections::BTreeMap<(rut_core::ScopeId, u32), rut_core::types::RutType>,
        > = std::collections::BTreeMap::new();
        for (requester, r) in seeds {
            let Some((ridx, _)) = self.done.get(requester).map(|u| (u.idx, ())) else {
                continue;
            };
            let entry = per_requester.entry(requester.clone()).or_default();
            let owner_scope = self.done.get(owner).map(|u| u.scope).unwrap_or(0);
            let closure = Self::desc_closure(&self.programs[ridx], &r.args, owner_scope);
            for ((s, l), row) in closure {
                entry.insert((s, l), row);
            }
        }
        let mut groups: Vec<crate::SeedGroup> = Vec::new();
        for (requester, rows) in &per_requester {
            let Some((ridx, requester_scope)) = self.done.get(requester).map(|u| (u.idx, u.scope)) else { continue };
            let prog = &self.programs[ridx];
            let rows: Vec<((rut_core::ScopeId, u32), rut_core::types::RutType)> =
                rows.iter().map(|(k, v)| (*k, v.clone())).collect();
            let mine: Vec<_> = seeds.iter().filter(|(rq, _)| rq == requester).collect();
            let insts = mine
                .iter()
                .filter(|(_, r)| !r.is_fn)
                .map(|(_, r)| {
                    (
                        prog.interner.name(r.decl).to_string(),
                        r.args.clone(),
                        r.methods
                            .iter()
                            .map(|&m| prog.interner.name(m).to_string())
                            .collect::<Vec<_>>(),
                    )
                })
                .collect();
            let fns = mine
                .iter()
                .filter(|(_, r)| r.is_fn)
                .map(|(_, r)| (prog.interner.name(r.decl).to_string(), r.args.clone()))
                .collect();
            
            let impl_methods = mine
                .iter()
                .filter(|(_, r)| r.is_impl)
                .map(|(_, r)| {
                    (
                        prog.interner.name(r.decl).to_string(),
                        r.impl_target,
                        prog.interner.name(r.methods[0]).to_string(),
                    )
                })
                .collect();
            // the requester's own impls, fn ids scope-qualified into the
            // requester's block. A target spelled under a THIRD scope
            // (the owner's own block as this requester saw it, another
            // pkg's) skips with the row drop in desc_closure: the owner
            // re-derives its own types, and a foreign block it never
            // bound has no locals to answer.
            let impls: Vec<crate::SeedImpl> = prog
                .surface
                .impls
                .iter()
                .filter(|im| {
                    let s = rut_core::id::scope_of(im.target);
                    s == rut_core::id::BOOT_SCOPE || s == requester_scope
                })
                .map(|im| crate::SeedImpl {
                    trait_name: prog.interner.name(im.trait_name).to_string(),
                    target: im.target,
                    methods: im
                        .methods
                        .iter()
                        .map(|(n, f)| (prog.interner.name(*n).to_string(), rut_core::pack(requester_scope, *f)))
                        .collect(),
                    methods_concrete: im
                        .methods_concrete
                        .iter()
                        .map(|(n, f)| (prog.interner.name(*n).to_string(), rut_core::pack(requester_scope, *f)))
                        .collect(),
                })
                .collect();
            
            
            groups.push(crate::SeedGroup {
                rows,
                names: &prog.interner,
                insts,
                fns,
                impl_methods,
                impls,
            });
        }
        let out = compile_program_resolved(
            &text,
            if is_decl { Mode::Decl } else { Mode::Impl },
            owner,
            scope,
            &bound,
            true,
            &Seeds { groups: &groups },
        );
        for r in &out.requests {
            self.requests.push((owner.to_string(), r.clone()));
        }
        if !out.diags.is_empty() || out.program.is_none() {
            self.diags.extend(out.diags);
            return;
        }
        self.programs[idx] = out.program.expect("checked above");
    }

    /// A packaged owner cannot grow: its binary carries exactly the
    /// instantiations the pack-time closure requested. Each consumer
    /// request must already have a ledger row — link unifies the
    /// requester's mirror onto it — or the closure was packed without
    /// this consumer, which is a loud refusal, never a guess.
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
                        "instantiation `{}` was not compiled into `{owner}`'s binary — re-pack with the consumer in the closure",
                        key
                    ),
                ));
                return;
            }
        }
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
