//! The run chain — the fluent composition of ONE run:
//! **`Pkg` → `RutRun` → `Compiled` → `Vm`**.
//!
//! Offers are `.pkg(..)` (first-pkg-wins), host bodies are
//! `.host_pkg(..)`, the symbol restore is `.symbols(..)`, the root is
//! `.entrypoint(..)`, and ONE terminal step validates + compiles:
//! [`RutRun::compile`]. The product, [`Compiled`], hands its parts to
//! the VM through the [`rut_vm::IntoVmParts`] seam:
//!
//! ```ignore
//! let compiled = RutRun::new()
//!     .pkg(Pkg::source("app", src))
//!     .host_pkg(rut_std::math::pkg())
//!     .entrypoint("app")
//!     .compile()?;
//! let mut vm = Vm::builder().compiled(compiled).build()?;
//! ```
//!
//! The `.compile()` law, in order:
//! 1. auto-offer the core prelude UNLESS a pkg named `core` was offered
//!    (first-pkg-wins override; the §0.14 law made literal);
//! 2. close the world, mount rows — the offered pkgs mount into the
//!    crate-internal table, and the mount-by-mount world runs its ONE
//!    peer-gate append pass (a walk collector already ran it over its
//!    own yield; the gate is idempotent per pkg);
//! 3. the host registry: every `.host_pkg(..)` installs against the
//!    rows snapshot (the context is internal now — a wiring drift is
//!    the loud panic, exactly as before);
//! 4. the closure check — every `use` name in every offered source
//!    resolves; a miss is [`RunError`], carrying the resolver's own
//!    diagnostic (the D2 peer text included);
//! 5. parse → check → LIR → link. Shape/closure failures are
//!    `Err(RunError)`; compile diagnostics ride INSIDE
//!    [`Compiled::graph`] exactly as they always did.

use std::collections::BTreeMap;

use rut_parser::Mode;

use crate::graph::{compile_graph, GraphOutput};
pub use crate::session::{Pkg, PkgBody};

pub use crate::session::{GenSource, HostRow, PeerDecl};

/// A walk's yield: the pkgs in the closure (root included) and the
/// root's name — offer them to a run with
/// [`RutRun::pkg`](RutRun::pkg) / [`RutRun::pkgs`](RutRun::pkgs).
#[derive(Clone, Debug)]
pub struct Loaded {
    pub pkgs: Vec<Pkg>,
    pub root: String,
}

impl Loaded {
    /// The pkg mounted under `name` — `None` when the walk did not
    /// carry it (first-mount-wins decided).
    pub fn pkg(&self, name: &str) -> Option<&Pkg> {
        self.pkgs.iter().find(|p| p.spec == name)
    }
}

/// Why a run refused to compose or compile: a shape failure (a bad
/// package name, a missing entrypoint) or a closure failure (a `use`
/// name nothing offered resolves). The message IS the diagnostic —
/// the resolver's and the manifest grammar's own texts ride inside.
/// Compile diagnostics are NOT here: they live on
/// [`Compiled::graph`] as always.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunError {
    pub msg: String,
}

impl RunError {
    pub fn law(msg: impl Into<String>) -> RunError {
        RunError { msg: msg.into() }
    }
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for RunError {}

/// A composed, compiled run: the linked graph (diags inside — check
/// [`GraphOutput::diags`] before trusting [`GraphOutput::program`]) and
/// the host registry every `.host_pkg(..)` installed. Feeds the VM:
/// `Vm::builder().compiled(compiled).build()`.
pub struct Compiled {
    pub graph: GraphOutput,
    pub hosts: rut_vm::interp::HostRegistry,
}

impl std::fmt::Debug for Compiled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // the registry's rows are raw code pointers — Debug spells the
        // shape, never the bytes
        f.debug_struct("Compiled")
            .field("diags", &self.graph.diags.len())
            .field("program", &self.graph.program.is_some())
            .field("hosts", &true)
            .finish()
    }
}

/// The seam: a compiled run hands its parts to the VM. Unset limits on
/// the builder side mean `Limits::default()` — uncapped, the mechanism
/// law — so a compiled run carries no budget opinion of its own.
impl rut_vm::IntoVmParts for Compiled {
    fn into_vm_parts(self) -> Result<rut_vm::VmParts, rut_vm::VmError> {
        let program = self
            .graph
            .program
            .ok_or_else(|| rut_vm::VmError::new(
                "no program — the compile failed; the diagnostics are on Compiled::graph",
            ))?;
        Ok(rut_vm::VmParts {
            program: std::rc::Rc::new(program),
            limits: rut_vm::interp::Limits::default(),
            hooks: rut_vm::interp::HostHooks::default(),
            hosts: self.hosts,
        })
    }
}

/// The fluent run builder — `RutRun::new()`. An unnameable impl detail:
/// compose with the chain methods and end at `.compile()`.
#[derive(Default)]
pub struct RutRun {
    pkgs: BTreeMap<String, Pkg>,
    host_pkgs: Vec<rut_vm::HostPkg>,
    entrypoint: Option<String>,
}

/// The `core` prelude pkg — the erasure primitive (`opaque`), the
/// engine's builtin surface (`Iterable`, the disposal pair), and the
/// compiler-lowered functions (`panic`, the `str`/`bytes` natives).
/// v1.1 removed `Option`/`Result`/`own` — use sites diagnose with the
/// removal. A native pkg with no body: its surface is
/// [`rut_core::binary::Surface::core`], the single source of truth
/// (`rut/core/core.d.rut` mirrors it for the LSP). Builtin names are
/// ambient except the `pub builtin` spellings (the disposal pair) —
/// those resolve only through `use core::{ .. }`; each row
/// keeps its ambient bit so the binding loops see the split.
/// `core` needs no `[deps]` declaration: `.compile()` auto-offers it
/// unconditionally (§0.14), while every other package resolves through
/// `[deps]` or host registration. Crate-internal: `core` is the one
/// engine-mounted pkg — there is no public constructor to reach for.
pub(crate) fn core_pkg() -> Pkg {
    // the surface is symbol-id based; the host-facing mount table is
    // string-based — `sym::text` bridges at this boundary only
    let core = rut_core::binary::Surface::core();
    let txt = |id: rut_core::IdentId| -> String {
        rut_core::sym::text(id).unwrap_or_default().to_string()
    };
    Pkg {
        spec: "core".to_string(),
        // each row keeps its ambient bit — the binding loops read
        // it off the synthesized surface, so the `pub builtin`
        // spellings stay import-gated end to end
        body: PkgBody::Host {
            native_types: core.native_types.iter().map(|(n, k, a)| (txt(*n), *k, *a)).collect(),
            native_fns: core.native_fns.iter().map(|(n, a)| (txt(*n), *a)).collect(),
            consts: core.consts.iter().map(|c| (txt(c.name), c.ty, c.bits)).collect(),
            native_impls: core
                .native_impls
                .iter()
                .map(|(t, n, i)| (*t, txt(*n), *i))
                .collect(),
            host_funcs: vec![],
        },
        ..Default::default()
    }
}

/// The declared host rows of a pkg set, async-expanded — the same
/// snapshot `.compile()` installs against, exposed for hosts that bind
/// RAW rows (full `scope::name` registrations on the registry) and want
/// the `.d.rut` ↔ binding contract checked both ways
/// (`HostRegistry::verify_against`). Pure: data in, table out.
pub fn declared_host_fns(pkgs: &[Pkg]) -> rut_vm::interp::ExpectedHostFns {
    host_pkg_ctx(pkgs).flatten()
}

/// The mounted host pkgs' declared rows, partitioned by registration
/// scope — the snapshot `.compile()` installs against, for hosts that
/// build registries OUTSIDE the chain (a bench probe's fresh `Vm` per
/// iteration, a raw-row host's `verify_against`). Pure: data in,
/// snapshot out.
pub fn host_pkg_ctx(pkgs: &[Pkg]) -> rut_vm::interp::HostPkgContext {
    let mut session = crate::session::Session::new();
    for pkg in pkgs {
        let _ = session.mount(pkg.clone());
    }
    session.host_pkg_context()
}

impl RutRun {
    /// The chain's head.
    pub fn new() -> RutRun {
        RutRun::default()
    }

    /// Offer a pkg. FIRST OFFER WINS: a pkg whose name is already
    /// offered is ignored — the embedder's (or the chain's) earlier
    /// mount outranks the later one, exactly the walk's law.
    pub fn pkg(mut self, pkg: Pkg) -> Self {
        self.pkgs.entry(pkg.spec.clone()).or_insert(pkg);
        self
    }

    /// Offer every pkg of a walk's yield, first-pkg-wins.
    pub fn pkgs(mut self, loaded: &Loaded) -> Self {
        for p in &loaded.pkgs {
            self = self.pkg(p.clone());
        }
        self
    }

    /// Hand over one host pkg's bodies. Installed at `.compile()`
    /// against the rows snapshot: a mounted scope whose declared rows
    /// the installer does not bind is the loud panic (the .d.rut ↔
    /// host-impl contract); an unmounted scope's rows merge inert (the
    /// blanket-install law).
    pub fn host_pkg(mut self, pkg: rut_vm::HostPkg) -> Self {
        self.host_pkgs.push(pkg);
        self
    }

    /// Restore a symbol table's names and positions into the offered
    /// compiled pkgs — the load half of compile-time symbol stripping,
    /// applied immediately. Sections whose pkg is absent (or not a
    /// compiled body) match nothing, which is tolerated, not an error.
    pub fn symbols(mut self, map: &rut_core::strip::SymbolMap) -> Self {
        apply_symbols(&mut self.pkgs, map);
        self
    }

    /// The root pkg — the graph compiles it and its transitive uses.
    pub fn entrypoint(mut self, name: &str) -> Self {
        self.entrypoint = Some(name.to_string());
        self
    }

    /// The terminal step: validate + compile (the law in the module
    /// docs). `Ok` always carries a [`Compiled`]; check
    /// `Compiled::graph.diags` for the compile's own diagnostics.
    pub fn compile(self) -> Result<Compiled, RunError> {
        let RutRun { pkgs, host_pkgs, entrypoint } = self;
        // 1. auto-offer the core prelude unless a pkg named `core` was
        //    offered (first-pkg-wins override)
        let mut pkgs = pkgs;
        if !pkgs.contains_key("core") {
            pkgs.insert("core".to_string(), core_pkg());
        }
        // 2. close the world, mount rows — plus the mount-by-mount
        //    world's ONE peer-gate append pass (idempotent per pkg: a
        //    walk collector's yield already carries its groups)
        let mut session = crate::session::Session::new();
        // the compiled walks' ledgers, namespaced PER ARCHIVE: an
        // archive's rows shift above everything recorded so far (boot
        // passes through) and EVERY program of that archive — its root
        // AND its groups — rebases with the same map before mounting.
        // The walk's own law, applied at the close of the world, so two
        // independently packed bundles never argue about a number (each
        // binary's ids resolve through its OWN archive's rows). The
        // archive's identity is its ROOT's row vector: the pack ledger
        // names the archive's whole closure, so the members are the
        // rows' specs — pure data, no walk bookkeeping on the pkg (a
        // member rebases only when its body is a decoded program; a
        // shadowed name keeps its own, earlier-won body).
        let mut roots: BTreeMap<String, Vec<(rut_core::id::ScopeId, String)>> =
            pkgs.iter()
                .filter(|(_, p)| !p.bundle_scopes.is_empty())
                .map(|(n, p)| (n.clone(), p.bundle_scopes.clone()))
                .collect();
        for (root, rows) in &roots {
            // one shift per archive: the fit check + the map, applied to
            // the root and every decoded member
            let base = session.bundle_scope_next_base().ok_or_else(|| {
                RunError::law("the bundle scope numbering space is exhausted")
            })?;
            for &(s, _) in rows {
                if s != rut_core::id::BOOT_SCOPE
                    && base as u32 + s as u32 > rut_core::id::MAX_SCOPE
                {
                    return Err(RunError::law(format!(
                        "the bundle's scope ledger reaches scope {s}, which does not fit above this program's {base} — the numbering space is exhausted"
                    )));
                }
            }
            for (s, spec) in rows {
                let shifted = if *s == rut_core::id::BOOT_SCOPE {
                    *s
                } else {
                    base + s
                };
                session.record_bundle_scope(shifted, spec);
            }
            let shift = |s: rut_core::id::ScopeId| {
                if s == rut_core::id::BOOT_SCOPE {
                    s
                } else {
                    base + s
                }
            };
            if let Some(mut pkg) = pkgs.remove(root) {
                if let PkgBody::Compiled(prog) = &mut pkg.body {
                    *prog = rut_core::link::rebase(prog.clone(), &shift);
                }
                pkg.bundle_scopes =
                    pkg.bundle_scopes.iter().map(|&(s, ref spec)| (shift(s), spec.clone())).collect();
                session
                    .register_module(root, pkg)
                    .map_err(|e| RunError::law(e.to_string()))?;
            }
            for (_, member) in rows {
                if member == root || member == "core" {
                    continue;
                }
                let Some(mut pkg) = pkgs.remove(member) else { continue };
                if let PkgBody::Compiled(prog) = &mut pkg.body {
                    *prog = rut_core::link::rebase(prog.clone(), &shift);
                }
                session
                    .register_module(member, pkg)
                    .map_err(|e| RunError::law(e.to_string()))?;
            }
        }
        for (name, pkg) in pkgs {
            session
                .register_module(&name, pkg)
                .map_err(|e| RunError::law(e.to_string()))?;
        }
        crate::loader::peer_gate(session.table_mut())?;
        // 3. HostRegistry + install every .host_pkg against the rows
        //    snapshot (a wiring drift panics — the contract, unchanged)
        let ctx = session.host_pkg_context();
        let mut hosts = rut_vm::interp::HostRegistry::new();
        for hp in host_pkgs {
            hosts.install_host_pkg(&ctx, hp);
        }
        // 4. the closure check — every `use` name resolves (the
        //    load-lane gate 7, now the run's law). A source that does
        //    not parse stays out of this check; the compile reports it.
        let mounted: Vec<(String, Pkg)> =
            session.modules().map(|(s, m)| (s.clone(), m.clone())).collect();
        for (_, pkg) in &mounted {
            let (text, mode) = match &pkg.body {
                PkgBody::Source { text, is_decl } => (
                    text,
                    if *is_decl { Mode::Decl } else { Mode::Impl },
                ),
                _ => continue,
            };
            let (ast, diags) = rut_parser::parse(text, mode);
            if !diags.is_empty() {
                continue;
            }
            for name in crate::graph::uses_of(&ast) {
                if let Err(e) = session.resolve(&name) {
                    return Err(RunError::law(e.to_string()));
                }
            }
        }
        // 5. parse → check → LIR → link
        let root = entrypoint.ok_or_else(|| {
            RunError::law("no entrypoint — the run needs .entrypoint(..) to name the root pkg")
        })?;
        if session.resolve(&root).is_err() {
            return Err(RunError::law(format!(
                "entrypoint `{root}` is not offered — .pkg(..) it (or offer the walk's root) before .compile()"
            )));
        }
        let graph = compile_graph(&session, &root);
        Ok(Compiled { graph, hosts })
    }
}

/// Restore a symbol table's names and positions into the offered
/// compiled pkgs — names first (linking and the VM both see the real
/// thing), then the span/pos sections. Sections whose pkg is absent
/// (or not a compiled body) are skipped; a map from a different build
/// matches nothing, tolerated.
fn apply_symbols(pkgs: &mut BTreeMap<String, Pkg>, map: &rut_core::strip::SymbolMap) {
    // names first: exact-key restore over every offered compiled pkg
    // (kept strings and non-keys pass through)
    let restore: std::collections::HashMap<&str, &str> = map
        .names
        .iter()
        .map(|(m, o)| (m.as_str(), o.as_str()))
        .collect();
    for (_, pkg) in pkgs.iter_mut() {
        if let PkgBody::Compiled(prog) = &mut pkg.body {
            prog.interner.remap_tail(|s| {
                restore.get(s).copied().unwrap_or(s).to_string()
            });
        }
    }
    // then the span/pos sections, spec-resolved like the names
    for section in &map.sections {
        if let Some(pkg) = pkgs.get_mut(&section.spec) {
            if let PkgBody::Compiled(prog) = &mut pkg.body {
                // a fn-count mismatch means the section was taken
                // from a different build — matches nothing,
                // tolerated silently (the same law as the name rows)
                if prog.funcs.len() == section.fns.len() {
                    for (f, sy) in prog.funcs.iter_mut().zip(&section.fns) {
                        f.spans = sy.spans.clone();
                        f.pos = sy.pos.clone();
                    }
                }
            }
        }
    }
}
