//! The run chain — the fluent composition of ONE run:
//! **`Offer` → `RutRun` → `Compiled` → `Vm`**.
//!
//! The offered world is bundles + host bodies (L6 — the pipeline
//! stages stay internal). Offers ride ONE verb, [`RutRun::pkg`], over
//! the [`Offer`] classification — the same lib/host pair the manifest
//! `type` field spells:
//!
//! ```ignore
//! let bundle = rut_driver::Bundle::parse(&bytes)?;   // decode + verify once
//! let compiled = RutRun::new()
//!     .pkg(&bundle)                                  // Offer::Lib — relocate + mount per world
//!     .pkg(rut_std::math::pkg())                     // Offer::Host — the host bodies
//!     .entrypoint("app")
//!     .compile()?;
//! let mut vm = Vm::builder().compiled(compiled).build()?;
//! ```
//!
//! `&Bundle` and `HostPkg` coerce into [`Offer`] (`impl Into`), so the
//! call site spells the verb, not the wrapping. Ephemeral source rides
//! a source bundle: `pack_inline_pkg(name, text)` assembles one —
//! assembly only, never compiles; the run compiles the member when it
//! is offered. The walked pkg data — a dir/srczip walk's yield, a hand
//! mount's builder shape — mounts through [`RutRun::mount`], the data
//! door beside the classification. First offer wins: a pkg whose name
//! is already offered is ignored.
//!
//! The `.compile()` law, in order:
//! 1. any deferred offer refusal surfaces (an archive that decoded at
//!    `Bundle::parse` but failed its per-world mount — a scope-ledger
//!    mismatch, an ambiguous identity rebind);
//! 2. auto-offer the core prelude UNLESS a pkg named `core` was offered
//!    (first-pkg-wins override; the §0.14 law made literal);
//! 3. close the world, mount rows — the offered pkgs mount into the
//!    crate-internal table;
//! 4. the host registry: every `Offer::Host` installs against the
//!    rows snapshot (the context is internal now — a wiring drift is
//!    the loud panic, exactly as before);
//! 5. the closure check — every `use` name in every offered source
//!    resolves AND is declared by its consumer (the D5 nameability
//!    law: mounted-but-undeclared refuses, unmounted keeps NoModule);
//!    a miss is [`RunError`], carrying the resolver's own diagnostic;
//! 6. parse → check → LIR → instantiate → link. The graph walk
//!    instantiates every consumer request against its owner (template
//!    monomorphization, ledger lookups, repr refolds), then the merge
//!    links — scope resolution, relocation, arithmetic only. Verify is
//!    the load gate: the final pass AFTER link (`rut_vm::verify`), run
//!    by the embedder's load path. Shape/closure failures are
//!    `Err(RunError)`; compile diagnostics ride INSIDE
//!    [`Compiled::graph`] exactly as they always did.

mod archive;

use std::collections::BTreeMap;

use rut_ast::parser::Mode;

use crate::graph::{compile_graph, GraphOutput};
pub use crate::session::{Mounted, MountedBody};
pub use archive::Bundle;

pub use crate::session::HostRow;

/// A walk's yield: the pkgs in the closure (root included) and the
/// root's name — mount them to a run through [`RutRun::mount`], one
/// pkg at a time (or offer a decoded archive whole with
/// [`RutRun::pkg`]).
#[derive(Clone, Debug)]
pub struct Walk {
    pub pkgs: Vec<Mounted>,
    pub root: String,
}

impl Walk {
    /// The pkg mounted under `name` — `None` when the walk did not
    /// carry it (first-mount-wins decided).
    pub fn pkg(&self, name: &str) -> Option<&Mounted> {
        self.pkgs.iter().find(|p| p.spec == name)
    }
}

/// What one [`RutRun::pkg`] offer IS — the classification of the
/// offered package, the same lib/host pair the manifest `type` field
/// spells. Kinds as variants, not methods.
#[allow(clippy::large_enum_variant)]
// the payloads ARE the enum: decoded bundles move as values
// through the graph — boxing would churn every construction/match
// for a size win the graph never sees
pub enum Offer<'a> {
    /// A rut package: a compiled archive (`Bundle::parse` — decoded
    /// once, relocated + mounted per world) or an in-memory source
    /// member (`pack_inline_pkg` — assembled, never compiled; the run
    /// compiles it when offered).
    Lib(&'a Bundle),
    /// Host bodies — the embedding Rust's own rows
    /// (`rut_vm::HostPkg`), installed against the rows snapshot at
    /// `.compile()`.
    Host(rut_vm::HostPkg),
}

impl<'a> From<&'a Bundle> for Offer<'a> {
    fn from(bundle: &'a Bundle) -> Offer<'a> {
        Offer::Lib(bundle)
    }
}

impl<'a> From<rut_vm::HostPkg> for Offer<'a> {
    fn from(pkg: rut_vm::HostPkg) -> Offer<'a> {
        Offer::Host(pkg)
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
/// the host registry every `Offer::Host` installed. Feeds the VM:
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
        let program = self.graph.program.ok_or_else(|| {
            rut_vm::VmError::new(
                "no program — the compile failed; the diagnostics are on Compiled::graph",
            )
        })?;
        Ok(rut_vm::VmParts {
            program: std::rc::Rc::new(program),
            limits: rut_vm::interp::Limits::default(),
            hosts: self.hosts,
        })
    }
}

/// The fluent run builder — `RutRun::new()`. An unnameable impl detail:
/// compose with the chain methods and end at `.compile()`.
#[derive(Default)]
pub struct RutRun {
    pkgs: BTreeMap<String, Mounted>,
    /// Offer refusals deferred from `pkg(..)` — an archive that
    /// decoded at `Bundle::parse` but failed its per-world mount.
    /// `pkg` stays infallible (the chain stays fluent); the first
    /// refusal surfaces at `.compile()`.
    deferred: Vec<RunError>,
    host_pkgs: Vec<rut_vm::HostPkg>,
    entrypoint: Option<String>,
}

/// The `core` prelude pkg — the erasure primitive (`opaque`), the
/// engine's builtin surface (`Iterable`, the disposal pair), and the
/// compiler-lowered functions (`panic`, the `str`/`bytes` natives).
/// v1.1 removed `Option`/`Result`/`own` — a use site diagnoses
/// through the ordinary unknown-name ladder. A native pkg with no
/// body: its surface is
/// [`rut_core::binary::Surface::core`], the single source of truth
/// (`rut/core/core.d.rut` mirrors it for the LSP). Builtin names are
/// ambient except the `pub builtin` spellings (the disposal pair) —
/// those resolve only through `use core::{ .. }`; each row
/// keeps its ambient bit so the binding loops see the split.
/// `core` needs no `[deps]` declaration: `.compile()` auto-offers it
/// unconditionally (§0.14), while every other package resolves through
/// `[deps]` or host registration. Crate-internal: `core` is the one
/// engine-mounted pkg — there is no public constructor to reach for.
pub(crate) fn core_pkg() -> Mounted {
    // the surface is symbol-id based; the host-facing mount table is
    // string-based — `sym::text` bridges at this boundary only
    let core = rut_core::binary::Surface::core();
    let txt = |id: rut_core::IdentId| -> String {
        rut_core::sym::text(id).unwrap_or_default().to_string()
    };
    Mounted {
        spec: "core".to_string(),
        // each row keeps its ambient bit — the binding loops read
        // it off the synthesized surface, so the `pub builtin`
        // spellings stay import-gated end to end
        body: MountedBody::Host {
            native_types: core
                .native_types
                .iter()
                .map(|(n, k, a)| (txt(*n), *k, *a))
                .collect(),
            native_fns: core.native_fns.iter().map(|(n, a)| (txt(*n), *a)).collect(),
            consts: core
                .consts
                .iter()
                .map(|c| (txt(c.name), c.ty, c.bits))
                .collect(),
            native_impls: core
                .native_impls
                .iter()
                .map(|(t, n, i)| (*t, txt(*n), *i))
                .collect(),
            host_funcs: vec![],
            root_surface: None,
        },
        ..Default::default()
    }
}

/// The declared host rows of a pkg set, async-expanded — the same
/// snapshot `.compile()` installs against, exposed for hosts that bind
/// RAW rows (full `scope::name` registrations on the registry) and want
/// the `.d.rut` ↔ binding contract checked both ways
/// (`HostRegistry::verify_against`). Pure: data in, table out.
pub fn declared_host_fns(pkgs: &[Mounted]) -> rut_vm::interp::ExpectedHostFns {
    host_pkg_ctx(pkgs).flatten()
}

/// The mounted host pkgs' declared rows, partitioned by registration
/// scope — the snapshot `.compile()` installs against, for hosts that
/// build registries OUTSIDE the chain (a bench probe's fresh `Vm` per
/// iteration, a raw-row host's `verify_against`). Pure: data in,
/// snapshot out.
pub fn host_pkg_ctx(pkgs: &[Mounted]) -> rut_vm::interp::HostPkgContext {
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

    /// Offer a pkg — the ONE offering verb, over the [`Offer`]
    /// classification: `&Bundle` (a decoded archive — relocated +
    /// mounted per world; or a source member from `pack_inline_pkg`)
    /// coerces to [`Offer::Lib`], a `rut_vm::HostPkg` to
    /// [`Offer::Host`]. Walked pkg data rides the data door,
    /// [`RutRun::mount`].
    ///
    /// FIRST OFFER WINS: a pkg whose name is already offered is
    /// ignored — the embedder's (or the chain's) earlier mount
    /// outranks the later one, exactly the walk's law.
    ///
    /// Infallible by law (the chain stays fluent): an archive whose
    /// per-world mount refuses defers the error to `.compile()`.
    pub fn pkg<'a>(mut self, pkg: impl Into<Offer<'a>>) -> Self {
        match pkg.into() {
            Offer::Lib(bundle) => match bundle.walk() {
                Ok(walk) => {
                    for p in walk.pkgs {
                        self.pkgs.entry(p.spec.clone()).or_insert(p);
                    }
                }
                Err(e) => self.deferred.push(e),
            },
            Offer::Host(hp) => self.host_pkgs.push(hp),
        }
        self
    }

    /// Mount one walked pkg — the DATA door beside the [`Offer`]
    /// classification: the dir/srczip walks' yield
    /// ([`rut_native`](https://docs.rs/rut-native) `load_dir*`,
    /// [`crate::run::Walk`]) and the hand mounts' builder shape
    /// (`Mounted::source(..).manifest(..)`, `Mounted::host(..)`)
    /// enter the run here — byte carriers (a decoded archive, an
    /// assembled source member) offer through [`RutRun::pkg`]
    /// instead. First offer wins, exactly like the classification.
    pub fn mount(mut self, pkg: Mounted) -> Self {
        self.pkgs.entry(pkg.spec.clone()).or_insert(pkg);
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
        let RutRun {
            pkgs,
            mut deferred,
            host_pkgs,
            entrypoint,
        } = self;
        // 1. a deferred offer refusal — an archive that decoded at
        //    `Bundle::parse` but failed its per-world mount — surfaces
        //    here, the chain's first fallible step
        if let Some(e) = deferred.pop() {
            return Err(e);
        }
        // 2. auto-offer the core prelude unless a pkg named `core` was
        //    offered (first-pkg-wins override)
        let mut pkgs = pkgs;
        if !pkgs.contains_key("core") {
            pkgs.insert("core".to_string(), core_pkg());
        }
        // 3. close the world, mount rows
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
        let roots: BTreeMap<String, Vec<(rut_core::id::ScopeId, String)>> = pkgs
            .iter()
            .filter(|(_, p)| !p.bundle_scopes.is_empty())
            .map(|(n, p)| (n.clone(), p.bundle_scopes.clone()))
            .collect();
        for (root, rows) in &roots {
            // one shift per archive: the fit check + the map, applied to
            // the root and every decoded member
            let base = session
                .bundle_scope_next_base()
                .ok_or_else(|| RunError::law("the bundle scope numbering space is exhausted"))?;
            for &(s, _) in rows {
                if s != rut_core::id::BOOT_SCOPE && base as u32 + s as u32 > rut_core::id::MAX_SCOPE
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
                if let MountedBody::Compiled(prog) = &mut pkg.body {
                    *prog = rut_core::link::rebase(prog.clone(), &shift);
                }
                pkg.bundle_scopes = pkg
                    .bundle_scopes
                    .iter()
                    .map(|&(s, ref spec)| (shift(s), spec.clone()))
                    .collect();
                session
                    .register_module(root, pkg)
                    .map_err(|e| RunError::law(e.to_string()))?;
            }
            for (_, member) in rows {
                if member == root || member == "core" {
                    continue;
                }
                let Some(mut pkg) = pkgs.remove(member) else {
                    continue;
                };
                if let MountedBody::Compiled(prog) = &mut pkg.body {
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
        // 4. HostRegistry + install every Offer::Host against the rows
        //    snapshot (a wiring drift panics — the contract, unchanged)
        let ctx = session.host_pkg_context();
        let mut hosts = rut_vm::interp::HostRegistry::new();
        for hp in host_pkgs {
            hosts.install_host_pkg(&ctx, hp);
        }
        // 5. the closure check — every `use` name resolves AND is
        //    declared (the load-lane gate 7, now the run's law; the
        //    nameability half is D5's). A source that does not parse
        //    stays out of this check; the compile reports it. The
        //    entrypoint read is hoisted above the check: the root spec
        //    is the dev arm's key (only the ROOT may name its
        //    `dev-deps`), so it must be known here.
        let root = entrypoint.ok_or_else(|| {
            RunError::law("no entrypoint — the run needs .entrypoint(..) to name the root pkg")
        })?;
        if session.resolve(&root).is_err() {
            return Err(RunError::law(format!(
                "entrypoint `{root}` is not offered — .pkg(..)/.mount(..) it (or offer the walk's root) before .compile()"
            )));
        }
        let mounted: Vec<(String, Mounted)> = session
            .modules()
            .map(|(s, m)| (s.clone(), m.clone()))
            .collect();
        for (spec, pkg) in &mounted {
            // a per-file layout for the use split: files = [(0, this
            // file's mod path)], the scopes tree the pkg mounted — the
            // own-package lane's walk starts at the file the use is
            // written in
            let layout_for =
                |from: &str| crate::graph::mod_inputs_of(pkg, vec![(0, from.to_string())]);
            // the root's own uses — a source body's parse (compiled
            // bodies carry no source; the flat shape is exactly
            // today's check)
            if let MountedBody::Source { text, is_decl } = &pkg.body {
                let mode = if *is_decl { Mode::Decl } else { Mode::Impl };
                // the recovering entry: the closure check wants the
                // whole file's `use` heads even beside syntax diags
                // the body stays mounted (the match borrows) — clone to
                // move into the closure check's parse
                let p = rut_ast::parser::parse_with_recover(
                    text.to_string(),
                    mode,
                    rut_ast::parser::ParseOpts::default(),
                );
                if p.ast().is_clean() {
                    let layout = layout_for("");
                    for name in crate::graph::dep_use_heads(&session, spec, &root, &layout, p.ast())
                    {
                        if let Err(e) = session.resolve_use(spec, &name, &root) {
                            return Err(RunError::law(e.to_string()));
                        }
                    }
                }
            }
            // the mounted mod tree's files are units too: their `use`
            // statements join the same closure check — whatever the
            // body kind (a compiled root's children still ride source)
            for m in pkg.mods.values() {
                // recovering: a file beside its syntax diags still
                // joins the closure check
                let p = rut_ast::parser::parse_with_recover(
                    m.text.clone(),
                    Mode::Impl,
                    rut_ast::parser::ParseOpts::default(),
                );
                if !p.ast().is_clean() {
                    continue; // the compile reports the syntax
                }
                let layout = layout_for(&m.path);
                for name in crate::graph::dep_use_heads(&session, spec, &root, &layout, p.ast()) {
                    if let Err(e) = session.resolve_use(spec, &name, &root) {
                        return Err(RunError::law(format!("`{}` ({}): {e}", pkg.spec, m.path)));
                    }
                }
            }
        }
        // 6. parse → check → LIR → instantiate → link (verify is the
        //    load gate's final pass AFTER link)
        let graph = compile_graph(&session, &root);
        Ok(Compiled { graph, hosts })
    }
}
