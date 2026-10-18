//! The graph walk itself: [`GraphCompiler`] ensures every module in
//! the closure, compiles each under its own scope, and routes the
//! owner-anchored instantiation requests.

use std::collections::{HashMap, HashSet};

use rut_lexer::diag::Diag;
use rut_lexer::span::Span;
use rut_core::binary::Program;
use rut_core::ops::Op;
use rut_parser::{parse, Mode};

use rut_ast::ast::{Ast, ItemKind, Vis};

use super::build_seed_groups;
use crate::session::{PkgBody, Session};
use crate::compile::compile_program_resolved;
use crate::compile::material::split_top_commas;
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
    /// the same collection's LOUD half (mount/coherence problems), per
    /// spec — rides the dep-rows cache so `ensure_source` and the use
    /// validation share ONE parse of the pkg
    pub(super) dep_problems: HashMap<String, Vec<String>>,
    /// the exported generics whose templates could not lower as
    /// instantiation-independent bodies — `(owner, decl, construct)`.
    /// FATAL at pack (the pinned stability law).
    pub(super) template_failures: Vec<(String, String, String)>,
    /// the IR lane's staged scope blocks, per (owner spec, requester
    /// scope): the requester scope number the seed args spell → the
    /// FRESH scope the staged block lives under in the owner's table.
    /// Fresh per staging: the requester's number may be a ghost (a
    /// carried block from a replaced mount), may BE the owner's own
    /// scope, and one requester's rows stage into several owners — one
    /// coordinate system per staged block, mapped at link by the
    /// staged-scope sweep.
    pub(super) staged_scopes: HashMap<(String, rut_core::ScopeId), rut_core::ScopeId>,
    /// The program root's spec — the dev arm's key: only the ROOT may
    /// `use` its `dev-deps` heads (D5's allowlist reads it). From
    /// `compile_graph`/`compile_units`' `root_spec`.
    pub(super) root: String,
}

/// A linked source unit's compile inputs, recorded for the owner-side
/// recompile (`ensure_source` replays them verbatim, seeds aside).
/// Each bound surface rides BOTH names: `origin` the exporter's mount
/// key (the owner-anchor spelling — a bundle group's content
/// identity), `head` the head text the consumer's `use` spelled (the
/// per-file binding key — the use row that binds a row of this
/// surface).
pub(super) struct UnitSrc {
    text: String,
    is_decl: bool,
    bound: Vec<(rut_core::ScopeId, rut_core::binary::Surface, String, String)>,
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

    /// Compile (or mount) `spec` if needed — every module links. The
    /// bookkeeping keys on the pkg's MOUNT KEY (the canonical
    /// registration name): a bare use head bridging to an
    /// identity-keyed group and a ledger row spelling that identity
    /// converge on one unit, never two compilations of one pkg.
    pub(super) fn ensure(&mut self, spec: &str) -> Option<Unit> {
        let spec = match self.session.resolve(spec) {
            Ok(m) => m.spec.clone(),
            Err(e) => {
                self.diags.push(Diag::new(Span::new(0, 0), e.to_string()));
                return None;
            }
        };
        if let Some(u) = self.done.get(&spec) {
            return Some(u.clone());
        }
        if !self.visiting.insert(spec.clone()) {
            self.diags.push(Diag::new(
                Span::new(0, 0),
                format!("cyclic use: `{spec}` is already being compiled"),
            ));
            return None;
        }
        let module = match self.session.resolve(&spec) {
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
                self.body_kind.insert(spec.clone(), 1);
                // the unit's scope is assigned BEFORE its deps ensure,
                // same law as the source arm: a dep's carried seed rows
                // may reference THIS unit's packed block (a requester's
                // rows riding the owner's binary — `Vec<Shape>` with the
                // element interface the requester declared), and the
                // mid-compile bind needs the number now
                let scope = self.next_scope;
                self.next_scope += 1;
                self.in_flight.insert(spec.clone(), scope);
                let out = self.ensure_compiled(&spec, prog, scope, &module.bundle_scopes);
                self.in_flight.remove(&spec);
                out
            }
            PkgBody::Host { .. } => {
                self.body_kind.insert(spec.clone(), 2);
                self.ensure_host(&spec)
            }
            PkgBody::Source { text, is_decl } => {
                self.body_kind.insert(spec.clone(), 0);
                // the unit's scope is assigned BEFORE its deps ensure: a
                // compiled dep's binary may carry this unit's packed rows
                // (the seeds it was packed with reference its consumer),
                // and the forward mapping needs the number now
                let scope = self.next_scope;
                self.next_scope += 1;
                self.in_flight.insert(spec.clone(), scope);
                let out = self.ensure_source(&spec, module, text, *is_decl, scope);
                self.in_flight.remove(&spec);
                out
            }
        }
    }

    /// The declaration-gated ensure — the use law at the graph (D5's
    /// compile-side half). Consumer `spec` may ensure and bind only
    /// the heads its manifest declares (`deps` plus the
    /// program root's `dev-deps`; `core` rides unconditionally): a
    /// mounted-but-undeclared head is D5, diagnosed in place, the unit
    /// fails; an unmounted one is `ensure`'s own resolve error
    /// (NoModule — the same resolver text either way, so the miss
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
            // the host-fn registration scope IS the package NAME (the
            // registration naming has no override) — the bare name
            // half of a mount key, so an identity-keyed group (a
            // `deps/` group) registers under the same rows the
            // embedder's registry spells
            let host_scope = crate::bundle::mount_name(spec);
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
        let mut program = rut_core::link::rebase(prog.clone(), &map);
        // the decoded table interns with the loader's block-scan spelling
        // (`id_for` attributes the tail to the greatest registered base ≤
        // dense) — the IR materialization appends staged blocks and mints
        // ABOVE the own block, so every tail row would re-attribute to the
        // staged scope with unbounded locals and map past the merged
        // segment. Own attribution pins the tail to this module's own
        // scope: the mints spell (own, local), which link maps through
        // the own segment — in bounds, and stable under later stagings.
        program.types.own_attribution = true;
        let idx = self.programs.len();
        self.programs.push(program);
        let unit = Unit { idx, scope };
        self.done.insert(spec.to_string(), unit.clone());
        Some(unit)
    }

    /// The compile path: parse the module's source, resolve its uses,
    /// compile the unit, link it. The unit's text is its own source —
    /// every decl's origin IS this module, so no origin map exists.
    /// `scope` was assigned by the dispatcher
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
        // ranges attribute every decl to its module).
        let mut combined = src.to_string();
        let mut files: Vec<(u32, String)> = vec![(0, String::new())];
        for (path, m) in &module.mods {
            combined.push('\n');
            files.push((combined.len() as u32, path.clone()));
            combined.push_str(&m.text);
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
        // never guessed around. The rows AND problems ride the shared
        // dep cache (`dep_mod_table`'s lane): a consumer's earlier use
        // validation may already have paid the parse.
        if !module.mods.is_empty() {
            if let Some(problems) = self.dep_mount_problems(spec) {
                for p in problems {
                    self.diags.push(Diag::new(Span::new(0, 0), p));
                }
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
        // law gates each head first (declared, not merely mounted).
        // Each surface rides its mount key (`origin`, the owner
        // anchor) AND the spelled head (the per-file binding key).
        let mut bound: Vec<(rut_core::ScopeId, rut_core::binary::Surface, String, String)> =
            Vec::new();
        let mut bound_scopes = HashSet::new();
        for dep in &uses {
            let dep_unit = self.ensure_use(spec, dep)?;
            // the binding's origin spells the exporter's MOUNT KEY (the
            // canonical registration name — a bare head bridging to an
            // identity-keyed group anchors its instantiation requests
            // where the owner unit actually lives); bare mounts spell
            // themselves, so directory worlds bind unchanged
            let origin = self
                .session
                .resolve(dep)
                .map(|m| m.spec.clone())
                .unwrap_or_else(|_| dep.clone());
            if bound_scopes.insert(dep_unit.scope) {
                // the exporter's spec rides the binding
                bound.push((
                    dep_unit.scope,
                    self.programs[dep_unit.idx].surface.clone(),
                    origin,
                    dep.clone(),
                ));
            }
        }

        // the deps' decl rows, keyed by the spelled head (the
        // validated-must-bind classification input): the use-line error
        // reads the dep's vis/kind row to name the fix — a private row
        // its `pub`, a `pub async fn` the async gap. The tables ride the
        // shared cache (validate filled them); a compiled dep's decls
        // ride its binary and classify generically.
        let dep_tables: Vec<
            (String, std::collections::BTreeMap<String, Vec<crate::mods::DeclRow>>),
        > = uses
            .iter()
            .filter_map(|h| self.dep_mod_table(h).cloned().map(|t| (h.clone(), t)))
            .collect();

        // the validation parse hands straight to the compile — one
        // parse per unit, not one per pass over it
        let out = crate::compile::compile_program_pre(
                (ast, Vec::new()),
                &combined,
                if is_decl { Mode::Decl } else { Mode::Impl },
                spec,
                scope,
                &bound,
                true,
                &Seeds::none(),
                &layout,
                &own_uses,
                &dep_tables,
        );
        for r in &out.requests {
            self.requests.push((spec.to_string(), r.clone()));
        }
        for (decl, construct) in &out.template_failures {
            self.template_failures.push((spec.to_string(), decl.clone(), construct.clone()));
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
    /// PACKAGED owner serves the request from its own binary: the
    /// pack-time ledger first (the fast path — the closure already
    /// compiled the shape), then its TEMPLATES (v22 — the body
    /// monomorphizes in place by id substitution); a miss of both is a
    /// torn or doctored bundle, refused loudly. Seeds cascade — a
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
                    Some(1) => self.reseed_ir_owner(&owner, &seeds),
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
                // seeds re-materialize nothing (the materialization memo
                // dedups).
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
        let out = crate::compile::compile_program_pre(
            parse(&text, if is_decl { Mode::Decl } else { Mode::Impl }),
            &text,
            if is_decl { Mode::Decl } else { Mode::Impl },
            owner,
            scope,
            &bound,
            true,
            &Seeds { groups: &groups },
            &layout,
            &own_uses,
            // the replay of a unit that already passed the must-bind
            // gate (same text, same bindings) — no classification input
            // to carry
            &[],
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

    /// A packaged owner serves a missed request from its TEMPLATES (v22
    /// — the delivery law's second lane): the owner program's
    /// placeholder-compiled bodies monomorphize IN PLACE, by id
    /// substitution. The owner's types grow the requester's argument
    /// rows (staged at the requester's own locals — the seed law), the
    /// template body substitutes, the repr bytes rewrite, the
    /// repr-keyed folds re-run, and the result is appended with its
    /// owner-anchored ledger rows — byte-equal to the directory-compiled
    /// instantiation (the pinned law). Bound-member stubs resolve
    /// against the concrete argument's own surface: an owner-local
    /// member binds directly, a member template cascades to its own
    /// materialization, a foreign generic routes a fresh InstRequest
    /// (the cascade), and a consumer-local member mints the mirror stub
    /// whose ledger row names the consumer (link unifies it with the
    /// real body there — the carried-member law's direction).
    fn reseed_ir_owner(&mut self, owner: &str, seeds: &[(String, rut_lir::check::InstRequest)]) {
        let Some(unit) = self.done.get(owner) else { return };
        let idx = unit.idx;
        // the live seeds (the pack-time ledger answers the rest — link
        // unifies the mirror, zero new work)
        let live: Vec<(String, rut_lir::check::InstRequest)> = seeds
            .iter()
            .filter(|(requester, r)| !self.ir_ledger_hit(idx, requester, r))
            .map(|(requester, r)| (requester.clone(), r.clone()))
            .collect();
        // the seed law's staging, ONCE per requester: every live seed's
        // argument rows share the requester's scope, so the union of
        // their closures stages as ONE block at one base — a per-request
        // staging would re-base the scope per shape and strand the
        // earlier requests' ids (locals beyond the first block's extent
        // densify into unrelated rows)
        let mut requesters: Vec<String> = live.iter().map(|(re, _)| re.clone()).collect();
        requesters.sort();
        requesters.dedup();
        for requester in &requesters {
            let args: Vec<Vec<rut_core::types::TypeId>> = live
                .iter()
                .filter(|(re, _)| re == requester)
                .map(|(_, r)| r.args.clone())
                .collect();
            if let Err(msg) = self.stage_ir_seeds(idx, owner, requester, &args) {
                self.diags.push(Diag::new(Span::new(0, 0), msg));
                return;
            }
        }
        for (requester, r) in &live {
            if r.is_fn {
                if let Err(msg) =
                    self.materialize_request(idx, owner, requester, true, r.decl, &r.args, None)
                {
                    self.diags.push(Diag::new(Span::new(0, 0), msg));
                    return;
                }
            } else if r.methods.is_empty() {
                // a type-only request: the instantiation's identity row —
                // the staged substitution mints it (no body exists)
                if let Err(msg) =
                    self.materialize_request(idx, owner, requester, false, r.decl, &r.args, None)
                {
                    self.diags.push(Diag::new(Span::new(0, 0), msg));
                    return;
                }
            } else {
                for &m in &r.methods {
                    if let Err(msg) =
                        self.materialize_request(idx, owner, requester, false, r.decl, &r.args, Some(m))
                    {
                        self.diags.push(Diag::new(Span::new(0, 0), msg));
                        return;
                    }
                }
            }
        }
    }

    /// The seed law's staging half, per requester: the union closure of
    /// every live seed's arguments appends as ONE block under a FRESH
    /// scope allocated for this staging — never the requester's own
    /// number, which may be a ghost (a carried block from a replaced
    /// mount), may BE the owner's own scope, and may stage into several
    /// owners (one coordinate system per staged block; link's
    /// staged-scope sweep maps each block's base). The staged rows'
    /// inner ids spelling the requester scope rewrite to the fresh
    /// scope (a recursive kind walk); ids in other scopes pass (the
    /// owner carries them, or a later arg stages them). Every row's
    /// name re-interns by text (ids are per-program).
    /// `materialize_request` spells each staged request's arguments
    /// through the fresh scope.
    fn stage_ir_seeds(
        &mut self,
        idx: usize,
        owner: &str,
        requester: &str,
        args_per_request: &[Vec<rut_core::types::TypeId>],
    ) -> Result<(), String> {
        let r_idx = self.done.get(requester).map(|u| u.idx);
        let Some(r_idx) = r_idx else { return Ok(()) }; // hand-routed: nothing to stage
        // the requester scopes that need staging: non-boot, not already
        // carried by the owner, and not staged for this (owner,
        // requester) pair yet
        let mut to_stage: Vec<rut_core::id::ScopeId> = Vec::new();
        {
            let owner_prog = &self.programs[idx];
            for args in args_per_request {
                for &a in args {
                    let s = rut_core::scope_of(a);
                    if s == rut_core::BOOT_SCOPE
                        || owner_prog
                            .types
                            .scope_base
                            .get(s as usize)
                            .copied()
                            .unwrap_or(0)
                            != 0
                        || self.staged_scopes.contains_key(&(owner.to_string(), s))
                    {
                        continue;
                    }
                    if !to_stage.contains(&s) {
                        to_stage.push(s);
                    }
                }
            }
        }
        if to_stage.is_empty() {
            return Ok(());
        }
        // read the requester's closures immutably: per staged scope, the
        // union closure of every arg spelling it, padded 0..=max_local
        let mut plan: Vec<(rut_core::types::RutType, rut_core::id::ScopeId, u32)> = Vec::new();
        {
            let r_prog = &self.programs[r_idx];
            let mut cursor = self.programs[idx].types.types.len() as u32;
            for &s in &to_stage {
                let mut closure: std::collections::HashMap<
                    (rut_core::id::ScopeId, u32),
                    rut_core::types::RutType,
                > = std::collections::HashMap::new();
                for args in args_per_request {
                    for &a in args {
                        if rut_core::scope_of(a) != s {
                            continue;
                        }
                        for ((s2, l2), row) in
                            Self::desc_closure(r_prog, &[a], rut_core::id::ScopeId::MAX)
                        {
                            if s2 == s {
                                closure.insert((s2, l2), row);
                            }
                        }
                    }
                }
                if closure.is_empty() {
                    continue;
                }
                let mut max_local = 0u32;
                for (s2, l) in closure.keys() {
                    if *s2 == s {
                        max_local = max_local.max(*l);
                    }
                }
                let mut filled = 0u32;
                while filled <= max_local {
                    match closure.get(&(s, filled)) {
                        Some(row) => plan.push((row.clone(), s, cursor)),
                        None => plan.push((
                            rut_core::types::RutType {
                                name: rut_core::sym::NIL,
                                kind: rut_core::types::TyKind::Nil,
                            },
                            s,
                            cursor,
                        )),
                    }
                    cursor += 1;
                    filled += 1;
                }
            }
        }
        if plan.is_empty() {
            return Ok(());
        }
        // the staged rows' IDENT ids spell the REQUESTER's interner —
        // ids are per-program, so every name re-interns here by text
        // (the surface's own law: the text crosses, never the id). A
        // name left as the requester's raw ident reads as an arbitrary
        // word in the owner's interner — the mint's structural dedup
        // then matches on garbage and the merged table spells nonsense.
        let rename_texts: Vec<(rut_core::sym::IdentId, String)> = {
            let req_interner = &self.programs[r_idx].interner;
            plan.iter()
                .flat_map(|(row, _, _)| {
                    let mut out = vec![row.name];
                    if let rut_core::types::TyKind::Data { fields } = &row.kind {
                        out.extend(fields.iter().map(|f| f.name));
                    }
                    out
                })
                .map(|id| (id, req_interner.name(id).to_string()))
                .collect()
        };
        // the fresh staging scope: allocated from the graph's counter so
        // it collides with no mount scope in any program of this graph
        let stage_scope = {
            let fresh = self.next_scope;
            self.next_scope += 1;
            fresh
        };
        for &s in &to_stage {
            self.staged_scopes
                .insert((owner.to_string(), s), stage_scope);
        }
        let own = &mut self.programs[idx];
        let renames: Vec<(rut_core::sym::IdentId, rut_core::sym::IdentId)> = rename_texts
            .iter()
            .map(|&(id, ref text)| (id, own.interner.intern(text)))
            .filter(|&(from, to)| from != to)
            .collect();
        for (row, _, _) in plan.iter_mut() {
            if let Some(&(_, to)) = renames.iter().find(|(from, _)| *from == row.name) {
                row.name = to;
            }
            if let rut_core::types::TyKind::Data { fields } = &mut row.kind {
                for f in fields.iter_mut() {
                    if let Some(&(_, to)) = renames.iter().find(|(from, _)| *from == f.name) {
                        f.name = to;
                    }
                }
            }
            // the requester-scope inner ids rewrite to the fresh scope
            for &s in &to_stage {
                remap_kind_scope(&mut row.kind, s, stage_scope);
            }
        }
        let tail = own.types.types.len() as u32;
        for (row, s_old, _) in plan.iter_mut() {
            *s_old = stage_scope;
            own.types.types.push(row.clone());
        }
        // the staged block's base: the block's first appended row
        let need = stage_scope as usize + 1;
        if own.types.scope_base.len() < need {
            own.types.scope_base.resize(need, 0);
        }
        own.types.scope_base[stage_scope as usize] = tail;
        Ok(())
    }

    /// The pack-time ledger's answer for one request — the same
    /// name-text canon the request side speaks (boot rows by id,
    /// everything else by the row's name text).
    fn ir_ledger_hit(&self, idx: usize, requester: &str, r: &rut_lir::check::InstRequest) -> bool {
        let Some(unit) = self.done.get(requester) else { return false };
        let pidx = unit.idx;
        let req = &self.programs[pidx];
        let own = &self.programs[idx];
        // non-boot rows canon by their name text; an id that densifies
        // OUT of its table (the pack's template-pass ledger rows carry
        // subst ids spelling the template window's rows, which the
        // carried blocks of a consumer's mount do not include) can
        // never match an in-bounds request canon — it canons to a
        // unique per-id sentinel and the row misses
        let canon_of = |prog: &Program, t: rut_core::types::TypeId| -> String {
            if rut_core::scope_of(t) == rut_core::BOOT_SCOPE {
                format!("b{}", rut_core::local_of(t))
            } else {
                let d = prog.types.dense(t) as usize;
                match prog.types.types.get(d) {
                    Some(row) => prog.interner.name(row.name).to_string(),
                    None => format!("#oob({t})"),
                }
            }
        };
        let args = r.args.iter().map(|&a| canon_of(req, a)).collect::<Vec<_>>().join(",");
        let owner_text = r.owner.as_str();
        let decl_text = req.interner.name(r.decl).to_string();
        if r.is_fn {
            own.inst_fns.iter().any(|row| {
                own.interner.name(row.owner) == owner_text
                    && match &row.kind {
                        rut_core::binary::InstFnKind::Free { name, subst, .. } => {
                            own.interner.name(*name) == decl_text
                                && subst.iter().map(|&t| canon_of(own, t)).collect::<Vec<_>>().join(",") == args
                        }
                        _ => false,
                    }
            })
        } else if r.methods.is_empty() {
            own.inst_types.iter().any(|row| {
                own.interner.name(row.owner) == owner_text
                    && own.interner.name(row.decl) == decl_text
                    && row.args.iter().map(|&t| canon_of(own, t)).collect::<Vec<_>>().join(",") == args
            })
        } else {
            r.methods.iter().all(|&m| {
                let mtext = req.interner.name(m).to_string();
                own.inst_fns.iter().any(|row| {
                    own.interner.name(row.owner) == owner_text
                        && match &row.kind {
                            rut_core::binary::InstFnKind::Method { data, name, subst, .. } => {
                                own.interner.name(*data) == decl_text
                                    && own.interner.name(*name) == mtext
                                    && subst.iter().map(|&t| canon_of(own, t)).collect::<Vec<_>>().join(",") == args
                            }
                            _ => false,
                        }
                })
            })
        }
    }


    /// Materialize one request against owner program `idx`: spell the
    /// arguments through the staged block (`stage_ir_seeds` laid it —
    /// the ids carry over untouched), find the template, substitute,
    /// append. `method` is `Some` for a method-body request, `None` for
    /// a free fn or a type-only request.
    #[allow(clippy::too_many_arguments)]
    fn materialize_request(
        &mut self,
        idx: usize,
        owner: &str,
        requester: &str,
        is_fn: bool,
        decl: rut_core::sym::IdentId,
        args: &[rut_core::types::TypeId],
        method: Option<rut_core::sym::IdentId>,
    ) -> Result<(), String> {
        // the seed law: rows land in the staged block under the fresh
        // staging scope (`stage_ir_seeds` laid it and recorded the
        // mapping), so the owner's substitution spells the staged
        // world. Boot ids are global; a scope the owner already carries
        // (its own world, or a previously staged block) passes through.
        let r_idx = self.done.get(requester).map(|u| u.idx);
        let mut arg_map: Vec<(rut_core::types::TypeId, rut_core::types::TypeId)> = Vec::new();
        for &a in args {
            let s = rut_core::scope_of(a);
            let staged = if s == rut_core::BOOT_SCOPE {
                None
            } else {
                self.staged_scopes.get(&(owner.to_string(), s)).copied()
            };
            let out = staged
                .map(|fresh| rut_core::pack(fresh, rut_core::local_of(a)))
                .unwrap_or(a);
            arg_map.push((a, out));
        }
        // the consumer-local members' index (case (c) of the stub
        // resolution): each program's inherent rows, keyed by scope —
        // computed once, so the materialization never holds a second
        // program borrow while growing the owner
        let mut home = Homes::default();
        for p in self.programs.iter() {
            if p.scope == rut_core::BOOT_SCOPE {
                continue;
            }
            home.name.insert(p.scope, p.name.clone());
            let entry = home.members.entry(p.scope).or_default();
            for ih in &p.surface.inherents {
                let set = entry.entry(ih.target).or_default();
                for m in &ih.methods {
                    set.insert(p.interner.name(m.name).to_string());
                }
            }
        }
        // the request's name text spells through the REQUESTER's
        // interner (the ids are per-program; the text is what crosses)
        let (decl_text, method_text) = match r_idx {
            Some(ri) => {
                let req = &self.programs[ri];
                (
                    req.interner.name(decl).to_string(),
                    method.map(|m| req.interner.name(m).to_string()),
                )
            }
            None => (String::new(), None),
        };
        // ---- phase B: substitute + materialize, inside the owner
        // program's borrow
        let Self { programs, requests, .. } = self;
        let own = &mut programs[idx];
        // the template row, matched by (kind, name text, arity) — a
        // BODY request only; a type-only request needs no template
        // (its identity row mints structurally below)
        let arity = args.len();
        let row = if is_fn || method.is_some() {
            own.templates.iter().enumerate().find(|(_, t)| {
                let name_ok = match &t.kind {
                    rut_core::binary::TemplateKind::Free { name } => {
                        is_fn && own.interner.name(*name) == decl_text
                    }
                    rut_core::binary::TemplateKind::Method { data, name } => {
                        !is_fn
                            && own.interner.name(*data) == decl_text
                            && method_text.as_deref() == Some(own.interner.name(*name))
                    }
                };
                name_ok && t.param_rows.len() == arity
            })
        } else {
            None
        };
        // THE DELIVERY LAW's loud refusal: a BODY request with no
        // template row is a torn or doctored bundle — the pack could
        // not have produced it (a body request is never answered by the
        // type-only identity mint below, which would fabricate
        // `Method { .., fid: 0 }` and mislink at runtime)
        if (is_fn || method.is_some()) && row.is_none() {
            let what = if is_fn {
                decl_text.clone()
            } else {
                format!(
                    "{}${}",
                    decl_text,
                    method_text.as_deref().unwrap_or("<method>")
                )
            };
            return Err(format!(
                "instantiation `{owner}#{what}<..>` is served by neither `{owner}`'s \
                 pack-time ledger nor its template bodies — corrupt or doctored, re-pack \
                 the directory"
            ));
        }
        let fid = if let Some((_, trow)) = row {
            let params = trow.param_rows.clone();
            let first_fid = own.template_first_fid;
            let stubs = own.template_stubs.clone();
            // the memo: (template fid, argument values) → the
            // materialized fid; a body reserves its slot BEFORE cloning,
            // so recursion lands on the reserved fid (the ensure_inst
            // pattern)
            let mut memo: HashMap<(u32, Vec<rut_core::types::TypeId>), u32> = HashMap::new();
            materialize_body(
                own,
                &home,
                &stubs,
                first_fid,
                trow.fid,
                &params,
                &arg_map.iter().map(|(_, o)| *o).collect::<Vec<_>>(),
                owner,
                &mut memo,
                requests,
                0,
            )?
        } else {
            // a type-only request: the instantiation's identity row —
            // the class's template export substitutes structurally and
            // the owner-anchored row mints (no body exists)
            let base = own
                .surface
                .type_exports
                .iter()
                .find(|t| t.is_generic && own.interner.name(t.name) == decl_text);
            let Some(base) = base else {
                return Err(format!(
                    "instantiation `{owner}#{decl_text}<..>` is served by neither `{owner}`'s \
                     pack-time ledger nor its template bodies — corrupt or doctored, re-pack \
                     the directory"
                ));
            };
            let own_scope = rut_core::link::own_scope(own).unwrap_or(0);
            let tmpl = rut_core::pack(base.scope.unwrap_or(own_scope), base.local);
            let mut pairs = Vec::new();
            for (p, &a) in base.params.iter().zip(args.iter()) {
                let ptext = own.interner.name(*p).to_string();
                let Some(row) = own
                    .interner
                    .lookup(&format!("#{ptext}"))
                    .and_then(|id| own.types.dense_id_of_name(id))
                else {
                    return Err(format!(
                        "`{owner}`'s template row for `{decl_text}` carries no `#{ptext}` placeholder — \
                         corrupt or doctored, re-pack the directory"
                    ));
                };
                pairs.push((row, a));
            }
            let ledger_rows = own.inst_fns.clone();
            let mut sub = crate::compile::material::Substitution::new(
                &mut own.types,
                &mut own.interner,
                pairs,
                owner.to_string(),
            )
            .with_ledger_owners(&ledger_rows);
            let _ = sub.run(tmpl);
            own.inst_types.extend(std::mem::take(&mut sub.minted_inst_tys));
            0 // no body — the kind arm below returns early
        };
        // the request's own ledger row: the owner-anchored identity the
        // consumer's mirror unifies with at link. The NAME TEXTS intern
        // in the OWNER's interner — the request's idents spell the
        // requester's table and mean nothing here.
        let subst_owner = own.interner.intern(owner);
        let decl_ident = own.interner.intern(&decl_text);
        let kind = if is_fn {
            rut_core::binary::InstFnKind::Free {
                name: decl_ident,
                subst: arg_map.iter().map(|(_, o)| *o).collect(),
                origins: vec![],
            }
        } else if method.is_some() {
            rut_core::binary::InstFnKind::Method {
                data: decl_ident,
                name: own.interner.intern(&method_text.unwrap_or_default()),
                subst: arg_map.iter().map(|(_, o)| *o).collect(),
                origins: vec![],
            }
        } else {
            return Ok(()); // a type-only request: the identity row minted above
        };
        own.inst_fns.push(rut_core::binary::InstFn { owner: subst_owner, kind, fid });
        Ok(())
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
/// spans index the COMBINED text — the pkg's root file and its mod
/// children concatenated — so each diag reads its
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
    /// row validates against the path-carrying rows.
    fn dep_mod_table(
        &mut self,
        spec: &str,
    ) -> Option<&std::collections::BTreeMap<String, Vec<crate::mods::DeclRow>>> {
        if !self.dep_decls.contains_key(spec) {
            let pkg = self.session.resolve(spec).ok()?;
            let (set, problems) = crate::mods::collect_module_set(spec, pkg);
            let table: std::collections::BTreeMap<String, Vec<crate::mods::DeclRow>> =
                set.units.into_iter().map(|u| (u.path, u.decls)).collect();
            self.dep_problems.insert(spec.to_string(), problems);
            self.dep_decls.insert(spec.to_string(), table);
        }
        self.dep_decls.get(spec)
    }

    /// The dep collection's loud half for `spec` — the same single
    /// parse `dep_mod_table` pays for (the table rides the same cache),
    /// so an ensured unit and its consumers never parse the pkg twice
    /// for the same rows.
    fn dep_mount_problems(&mut self, spec: &str) -> Option<Vec<String>> {
        if !self.dep_decls.contains_key(spec) {
            // fill both halves in one collection (the quiet-table path
            // re-derives nothing)
            let pkg = self.session.resolve(spec).ok()?;
            let (set, problems) = crate::mods::collect_module_set(spec, pkg);
            let table: std::collections::BTreeMap<String, Vec<crate::mods::DeclRow>> =
                set.units.into_iter().map(|u| (u.path, u.decls)).collect();
            self.dep_problems.insert(spec.to_string(), problems.clone());
            self.dep_decls.insert(spec.to_string(), table);
            return Some(problems);
        }
        self.dep_problems.get(spec).cloned()
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

/// The consumer-local member index (the stub resolution's case (c)):
/// every program's inherent rows, keyed by its own scope — the
/// materialization reads it instead of holding a second program
/// borrow while growing the owner.
#[derive(Default)]
struct Homes {
    name: HashMap<rut_core::id::ScopeId, String>,
    members: HashMap<rut_core::id::ScopeId, HashMap<u32, std::collections::HashSet<String>>>,
}

/// Materialize one template body into the owner program: reserve the
/// fn slot, clone the template's `FuncCode` through the substitution
/// (`TypeId`s rebuilt structurally, repr bytes recomputed, the
/// repr-keyed folds re-run), then fix every callee operand — a
/// bound-member stub resolves against the concrete argument's own
/// surface, a template-window callee materializes recursively, a
/// foreign mirror stub re-mints with the substituted ledger row and
/// routes a fresh `InstRequest`. Returns the materialized fid.
#[allow(clippy::too_many_arguments)]
fn materialize_body(
    own: &mut Program,
    home: &Homes,
    stubs: &[rut_core::binary::TemplateStub],
    first_fid: u32,
    template_fid: u32,
    parent_params: &[rut_core::types::TypeId],
    parent_values: &[rut_core::types::TypeId],
    owner: &str,
    memo: &mut HashMap<(u32, Vec<rut_core::types::TypeId>), u32>,
    requests: &mut Vec<(String, rut_lir::check::InstRequest)>,
    depth: u32,
) -> Result<u32, String> {
    if depth > 100 {
        return Err(format!(
            "template materialization for `{owner}` recursed past 100 frames — a generic cycle the templates cannot close"
        ));
    }
    // the callee's own substitution context: a ledger row naming the
    // fid spells its template-side arguments (the row's subst) — those
    // run through the parent's substitution; a rowless callee (a
    // lambda / for-of closure minted inside the template) inherits the
    // parent's context wholesale
    let (params, values) = match own.inst_fns.iter().find(|r| r.fid == template_fid) {
        Some(row) => {
            let row_subst: Vec<rut_core::types::TypeId> = match &row.kind {
                rut_core::binary::InstFnKind::Free { subst, .. }
                | rut_core::binary::InstFnKind::Method { subst, .. } => subst.clone(),
                rut_core::binary::InstFnKind::HostThunk { .. } => vec![],
            };
            let ledger_rows = own.inst_fns.clone();
            let mut sub = crate::compile::material::Substitution::new(
                &mut own.types,
                &mut own.interner,
                parent_params.iter().cloned().zip(parent_values.iter().cloned()).collect(),
                owner.to_string(),
            )
            .with_ledger_owners(&ledger_rows);
            let vals = row_subst.iter().map(|&t| sub.run(t)).collect();
            (row_subst, vals)
        }
        None => (parent_params.to_vec(), parent_values.to_vec()),
    };
    let key = (template_fid, values.clone());
    if let Some(&f) = memo.get(&key) {
        return Ok(f);
    }
    let source = own
        .funcs
        .get(template_fid as usize)
        .ok_or_else(|| format!("template body #{template_fid} out of range in `{owner}`"))?
        .clone();
    let ledger_row = own.inst_fns.iter().find(|r| r.fid == template_fid).cloned();
    if source.code.is_empty() && source.host_id.is_none() {
        // a MIRROR STUB (the template window's request to a foreign
        // generic): never clone an empty body — re-mint the stub with
        // the substituted ledger row and route the request to the
        // foreign owner (the cascade)
        let row = ledger_row
            .ok_or_else(|| format!("`{owner}`: bodyless fn #{template_fid} carries no ledger row"))?;
        let owner_text = own.interner.name(row.owner).to_string();
        let (is_fn, data_ident, name_ident): (bool, _, _) = match &row.kind {
            rut_core::binary::InstFnKind::Free { name, .. } => {
                (true, *name, *name)
            }
            rut_core::binary::InstFnKind::Method { data, name, .. } => {
                (false, *data, *name)
            }
            rut_core::binary::InstFnKind::HostThunk { .. } => {
                return Err(format!(
                    "`{owner}`: a host thunk (#{template_fid}) rode a template call — doctored"
                ));
            }
        };
        let args: Vec<rut_core::types::TypeId> = values.clone();
        let methods: Vec<rut_core::sym::IdentId> = if is_fn { vec![] } else { vec![name_ident] };
        // the new stub: bodyless, its ledger row spells the substituted
        // key — link binds the call to the foreign owner's served body
        let data_text = own.interner.name(data_ident).to_string();
        let name_text = own.interner.name(name_ident).to_string();
        let stub_name = own.interner.intern(&format!("mirror#{owner_text}${data_text}${name_text}"));
        let new_fid = own.funcs.len() as u32;
        own.funcs.push(rut_core::binary::FuncCode {
            name: stub_name,
            params: vec![],
            ret: rut_core::types::TY_NIL,
            is_method: !is_fn,
            n_captures: 0,
            regs: vec![],
            argv: vec![],
            labels: vec![],
            code: vec![],
            spans: vec![],
            pos: vec![],
            host_id: None,
        });
        let k = if is_fn {
            rut_core::binary::InstFnKind::Free {
                name: name_ident,
                subst: args.clone(),
                origins: vec![],
            }
        } else {
            rut_core::binary::InstFnKind::Method {
                data: data_ident,
                name: name_ident,
                subst: args.clone(),
                origins: vec![],
            }
        };
        own.inst_fns.push(rut_core::binary::InstFn {
            owner: own.interner.intern(&owner_text),
            kind: k,
            fid: new_fid,
        });
        requests.push((
            owner.to_string(),
            rut_lir::check::InstRequest {
                owner: owner_text,
                decl: data_ident,
                args,
                methods,
                is_fn,
                is_impl: false,
                impl_target: 0,
            },
        ));
        memo.insert(key, new_fid);
        return Ok(new_fid);
    }
    // reserve the slot, then substitute
    let new_fid = own.funcs.len() as u32;
    memo.insert(key.clone(), new_fid);
    own.funcs.push(source.clone());
    // the template row's `data` text, read before the substitution
    // borrows the table — the receiver test's expected spelling
    let receiver_data_text = ledger_row.as_ref().and_then(|r| match &r.kind {
        rut_core::binary::InstFnKind::Method { data, .. } => {
            Some(own.interner.name(*data).to_string())
        }
        _ => None,
    });
    let (mut body, minted) = {
        let ledger_rows = own.inst_fns.clone();
        let mut sub = crate::compile::material::Substitution::new(
            &mut own.types,
            &mut own.interner,
            params.iter().cloned().zip(values.iter().cloned()).collect(),
            owner.to_string(),
        )
        .with_ledger_owners(&ledger_rows);
        // the method template's RECEIVER: the body spells the class's
        // OWN bare row (`HashMap` — the record shape), whose tie to the
        // type parameters rides the template row (`data` +
        // `param_rows`), not any argument spell of its own — no name
        // rebuild can derive it. Pin bare row → instantiation row
        // (`HashMap<i64, str>` — the exact row a fresh compile mints
        // for the consumer's shape), so the served body, its
        // MakeRecord/field ops and the boundary checks all speak the
        // consumer's spelling. The receiver test is textual: the first
        // parameter names the template row's `data` (a static method
        // like `with_capacity(n: i32)` has no receiver — its first
        // parameter names something else).
        if let Some(&self_row) = source.params.first() {
            let base_text = sub.name_of(self_row).to_string();
            let is_receiver = receiver_data_text.as_deref() == Some(base_text.as_str());
            if is_receiver && !base_text.contains('<') {
                let kind = sub.tt.kind(self_row).clone();
                let row = sub.mint_instantiation(&base_text, &values, kind);
                sub.force(self_row, row);
            }
        }
        let mut body = crate::compile::material::materialize_fn(&mut sub, &source, &mut |f| f);
        // the CONST half of the template-stability law: an op may pin a
        // TYPE ID as a literal (`opaque::downcast<V>`'s want — the
        // emitter bakes `Const(TypeId(#V))`). Every type OPERAND
        // already re-spelled through the substitution; the baked
        // literals take the same walk, each landing on a (possibly
        // fresh) pool entry carrying the SUBSTITUTED id — a stale
        // placeholder id makes every runtime downcast of the
        // substituted type answer nil.
        {
            let mut seen: HashMap<u32, u32> = HashMap::new();
            for op in body.code.iter_mut() {
                let rut_core::ops::Op::Const { k, .. } = op else { continue };
                if let Some(&to) = seen.get(k) {
                    *k = to;
                    continue;
                }
                let new_k = match own.consts.get(*k as usize) {
                    Some(rut_core::binary::ConstVal::TypeId(t)) => {
                        let nt = sub.run(*t);
                        if nt == *t {
                            *k
                        } else {
                            match own
                                .consts
                                .iter()
                                .position(|c| matches!(c, rut_core::binary::ConstVal::TypeId(x) if *x == nt))
                            {
                                Some(p) => p as u32,
                                None => {
                                    own.consts.push(rut_core::binary::ConstVal::TypeId(nt));
                                    (own.consts.len() - 1) as u32
                                }
                            }
                        }
                    }
                    _ => *k,
                };
                seen.insert(*k, new_k);
                *k = new_k;
            }
        }
        let minted = std::mem::take(&mut sub.minted_inst_tys);
        (body, minted)
    };
    // fix the callee operands against the substituted world
    let member_stubs: HashMap<u32, (rut_core::types::TypeId, rut_core::sym::IdentId)> = stubs
        .iter()
        .map(|s| (s.fid, (s.param, s.member)))
        .collect();
    for op in body.code.iter_mut() {
        let needs_fix = matches!(op, Op::Call { .. } | Op::CallM { .. } | Op::MakeClosure { .. });
        if !needs_fix {
            continue;
        }
        let f = match op {
            Op::Call { func, .. } | Op::CallM { func, .. } | Op::MakeClosure { func, .. } => *func,
            _ => unreachable!("guarded above"),
        };
        let fixed = if let Some((param, member)) = member_stubs.get(&f).copied() {
            let mut sub2 = crate::compile::material::Substitution::new(
                &mut own.types,
                &mut own.interner,
                params.iter().cloned().zip(values.iter().cloned()).collect(),
                owner.to_string(),
            );
            let concrete = sub2.run(param);
            resolve_member_stub(
                own, home, stubs, first_fid, concrete, member, owner, &params, &values, memo,
                requests, depth,
            )?
        } else if rut_core::scope_of(f) == rut_core::BOOT_SCOPE && f >= first_fid {
            // a plain operand at or above the window start names THIS
            // program's template window (own fn refs compile to plain
            // boot-scope ids; a scope-qualified operand is a FOREIGN
            // concrete fn reference — link maps it through the module
            // bases, exactly as a directory-compiled instantiation's
            // would, so it passes through untouched)
            materialize_body(
                own, home, stubs, first_fid, f, &params, &values, owner, memo, requests, depth + 1,
            )?
        } else {
            f
        };
        match op {
            Op::Call { func, .. } => *func = fixed,
            Op::CallM { func, .. } => *func = fixed,
            Op::MakeClosure { func, .. } => *func = fixed,
            _ => {}
        }
    }
    own.inst_types.extend(minted);
    own.funcs[new_fid as usize] = body;
    Ok(new_fid)
}

/// Resolve one bound-member stub: the concrete argument's own surface
/// answers. An owner-local class binds its compiled fn directly; an
/// instantiation of an OWN generic class materializes the member
/// template recursively; a consumer-local class mints the mirror stub
/// whose ledger row names the consumer (link unifies it with the real
/// body there — the carried-member law's direction).
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn resolve_member_stub(
    own: &mut Program,
    home: &Homes,
    stubs: &[rut_core::binary::TemplateStub],
    first_fid: u32,
    concrete: rut_core::types::TypeId,
    member: rut_core::sym::IdentId,
    owner: &str,
    _params: &[rut_core::types::TypeId],
    _values: &[rut_core::types::TypeId],
    memo: &mut HashMap<(u32, Vec<rut_core::types::TypeId>), u32>,
    requests: &mut Vec<(String, rut_lir::check::InstRequest)>,
    depth: u32,
) -> Result<u32, String> {
    let member_text = own.interner.name(member).to_string();
    // (a) the owner's own compiled member
    let local = own.surface.inherents.iter().find_map(|ih| {
        if ih.target != concrete {
            return None;
        }
        ih.methods
            .iter()
            .find(|m| own.interner.name(m.name) == member_text && m.local != 0)
            .map(|m| m.local)
    });
    if let Some(local) = local {
        return Ok(local);
    }
    // (b) an instantiation of an OWN generic class: the member template
    // materializes at the concrete's arguments
    let concrete_text = own.type_name(concrete).to_string();
    if let Some((base, args_text)) = concrete_text.split_once('<') {
        let args_text = args_text.strip_suffix('>').unwrap_or(args_text);
        let arg_rows: Vec<rut_core::types::TypeId> = split_top_commas(args_text)
            .iter()
            .filter_map(|t| own.interner.lookup(t).and_then(|id| own.types.dense_id_of_name(id)))
            .collect();
        let trow = own.templates.iter().find(|t| {
            t.param_rows.len() == arg_rows.len()
                && match &t.kind {
                    rut_core::binary::TemplateKind::Method { data, name } => {
                        own.interner.name(*data) == base
                            && own.interner.name(*name) == member_text
                    }
                    rut_core::binary::TemplateKind::Free { name } => {
                        own.interner.name(*name) == base
                    }
                }
        });
        if let Some(trow) = trow {
            let (f, p) = (trow.fid, trow.param_rows.clone());
            return materialize_body(
                own, home, stubs, first_fid, f, &p, &arg_rows, owner, memo, requests, depth + 1,
            );
        }
    }
    // (c) a consumer-local class: the mirror stub — a bodyless fn whose
    // ledger row names the concrete's HOME unit; link unifies it with
    // the real body there
    let home_scope = rut_core::scope_of(concrete);
    if home_scope != rut_core::BOOT_SCOPE {
        if let Some(home_name) = home.name.get(&home_scope).cloned() {
            let served = home
                .members
                .get(&home_scope)
                .and_then(|m| m.get(&concrete))
                .map(|set| set.contains(&member_text))
                .unwrap_or(false);
            if !served {
                return Err(format!(
                    "`{owner}`'s template calls `{concrete_text}.{member_text}`, but the type's \
                     own surface does not carry that member — the shape the consumer spelled is \
                     not satisfied; re-pack after fixing the caller"
                ));
            }
            let data_ident = own
                .interner
                .lookup(&concrete_text)
                .unwrap_or_else(|| own.interner.intern(&concrete_text));
            let stub_name = own
                .interner
                .intern(&format!("mirror#{}${}${}", home_name, concrete_text, member_text));
            let fid = own.funcs.len() as u32;
            own.funcs.push(rut_core::binary::FuncCode {
                name: stub_name,
                params: vec![],
                ret: rut_core::types::TY_NIL,
                is_method: true,
                n_captures: 0,
                regs: vec![],
                argv: vec![],
                labels: vec![],
                code: vec![],
                spans: vec![],
                pos: vec![],
                host_id: None,
            });
            own.inst_fns.push(rut_core::binary::InstFn {
                owner: own.interner.intern(&home_name),
                kind: rut_core::binary::InstFnKind::Method {
                    data: data_ident,
                    name: member,
                    subst: vec![],
                    origins: vec![],
                },
                fid,
            });
            return Ok(fid);
        }
    }
    Err(format!(
        "`{owner}`'s template calls `{concrete_text}.{member_text}`, but no surface carries that \
         member — corrupt or doctored, re-pack the directory"
    ))
}

/// Rewrite every `(from, local)` type id inside `kind` to `(to, local)` —
/// the staging half's scope rebase: the staged rows' inner ids spelling
/// the requester's scope move into the fresh staging scope's coordinate
/// system. Structural recursion over the type-carrying variants; the
/// leaves and non-type payloads pass.
fn remap_kind_scope(kind: &mut rut_core::types::TyKind, from: rut_core::id::ScopeId, to: rut_core::id::ScopeId) {
    if from == to {
        return;
    }
    let rb = |t: &mut rut_core::types::TypeId| {
        if rut_core::scope_of(*t) == from && *t != u32::MAX {
            *t = rut_core::pack(to, rut_core::local_of(*t));
        }
    };
    match kind {
        rut_core::types::TyKind::Nil
        | rut_core::types::TyKind::Prim(_)
        | rut_core::types::TyKind::Str
        | rut_core::types::TyKind::Bytes
        | rut_core::types::TyKind::Enum { .. }
        | rut_core::types::TyKind::IfaceObj { .. }
        | rut_core::types::TyKind::Opaque
        | rut_core::types::TyKind::Trace
        | rut_core::types::TyKind::DisposalContext => {}
        rut_core::types::TyKind::Array { elem }
        | rut_core::types::TyKind::Opt { elem }
        | rut_core::types::TyKind::Weak { elem }
        | rut_core::types::TyKind::Future { elem } => rb(elem),
        rut_core::types::TyKind::Data { fields } => {
            for f in fields.iter_mut() {
                rb(&mut f.ty);
            }
        }
        rut_core::types::TyKind::Fn { params, ret } => {
            for p in params.iter_mut() {
                rb(p);
            }
            rb(ret);
        }
    }
}
