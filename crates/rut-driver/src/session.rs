//! Module mounts & resolution — the compile-time half of RFC 0035 §1,
//! shaped by RFC 0029 §5 and RFC 0003 (§2).
//!
//! **One directory is one module.** Its `rut.toml` names the exact
//! package it answers to and how to reach its surface and body; the
//! manifest grammar itself — the parsed [`rut_bundle::Manifest`], its
//! dep tables and its error shapes — lives in `rut-bundle`
//! ([`rut_bundle::parse_manifest`]). This module is the mount table
//! that parsed manifest feeds:
//!
//! ```toml
//! [deps]
//! "pouch" = { path = "rut/pouch" }
//! ```
//!
//! Resolution is exact and single-step: a use path resolves only if a
//! module with that `name` is mounted — nothing is derived. Package
//! names are bare `[a-zA-Z0-9_]+` identifiers; a miss points at the
//! consumer manifest (`[deps]`).
//!
//! The dep kinds (RFC 0045): `[deps]` is today's transitively-mounted
//! table; `[peer-deps]` is REQUIRED by default (the consumer supplies
//! the peer) with `optional = true` marking the presence-mounted kind
//! whose integration group is the descriptor's `lib` file; `[dev-deps]`
//! mount only while building the pkg itself (the loader's law — the
//! Session never sees a dev table).
//!
//! The `Session` itself does no I/O (wasm hosts mount in memory); the
//! native file readers are the counterparts that read files (the
//! loader, over `rut-bundle`'s [`rut_bundle::Source`]).

use std::collections::BTreeMap;
use std::path::Path;

use rut_bundle::{parse_manifest, valid_spec, Entry, ManifestError};

/// What a mounted module's body IS. The graph dispatches on this:
/// a source body compiles (and may splice), a compiled body pushes as
/// decoded, a host body synthesizes its placeholder program.
#[derive(Clone, Debug)]
pub enum ModuleBody {
    /// a `.rut` body — compile it. `is_decl` marks a declaration-mode
    /// module (a `.d.rut` surface parsed as its own unit, RFC 0029):
    /// nothing to compile or run, but `rut dump` shows the AST.
    Source { text: String, is_decl: bool },
    /// a decoded `.rutc` program (a v5 compiled bundle's payload): the
    /// body already exists — the graph assigns it a fresh scope,
    /// rebases its packed ids, and pushes it. The loader's decode gate
    /// (version + surface verification) has passed.
    Compiled(rut_core::binary::Program),
    /// a native/host module (RFC 0022/0026): no rut body — bodyless
    /// functions the embedder binds at run time, exported constants,
    /// and the builtin rows (`core`'s prelude, `calc`'s `Math`). The
    /// graph synthesizes a placeholder program from these rows.
    Host {
        /// `(name, params, ret, is_async)` — `is_async` marks a
        /// `host async fn` (the host future lane)
        host_funcs: Vec<(String, Vec<rut_core::types::TypeId>, rut_core::types::TypeId, bool)>,
        /// exported constants: `(name, type, raw bits)` — `calc::PI`
        consts: Vec<(String, rut_core::types::TypeId, u64)>,
        /// Builtin containers published by name (`core` only), each
        /// with its ambient bit — `true` (`prelude builtin`) binds in
        /// every unit with no `use`, `false` (`pub builtin`) resolves
        /// only through `use`
        native_types: Vec<(String, rut_core::binary::NativeTy, bool)>,
        /// Builtin traits published by name (`core` only), same
        /// ambient-bit law as [`ModuleBody::Host`]'s `native_types`
        native_traits: Vec<(String, rut_core::binary::NativeTrait, bool)>,
        /// Compiler-lowered builtin function names (`core` only) — no
        /// bodies; rut-lir lowers them. Each row carries its ambient
        /// bit (same law)
        native_fns: Vec<(String, bool)>,
        /// Builtin-impl methods (`core` only, RFC 0032 §1.1 R2): the
        /// integer primitives' numeric methods — `(receiver prim, name,
        /// lowering id)`. Bodyless and hostless — rut-lir expands the
        /// method call inline, ambient on the primitive.
        native_impls: Vec<(rut_core::types::TypeId, String, rut_core::ops::Intrinsic)>,
    },
}

impl Default for ModuleBody {
    /// An empty source body — the manifest-only mount (a module whose
    /// entries ride on other fields) parses to an empty unit.
    fn default() -> ModuleBody {
        ModuleBody::Source { text: String::new(), is_decl: false }
    }
}

/// One mounted module: the bare package name it answers to, its entry
/// files, and its body ([`ModuleBody`]).
#[derive(Clone, Debug, Default)]
pub struct Module {
    /// the exact package name — bare `[a-zA-Z0-9_]+`
    pub spec: String,
    /// The namespace head for qualified member access (`Math.sqrt`) —
    /// `None` when the module has no namespace form (RFC 0028).
    pub namespace: Option<String>,
    pub entry: Entry,
    /// the body: `.rut` source, a decoded `.rutc`, or the native rows
    pub body: ModuleBody,
    /// The host-fn registration scope — the `FuncCode` host-id prefix — when
    /// it must differ from the package name. `rt` stays the logger host
    /// module's use path while its internal registration naming remains
    /// `rt:log` (`rt:log::create_logger`), untouched since RFC 0022.
    pub host_scope: Option<String>,
    /// Force source-inlining into every consumer (`ink`): a module whose
    /// class methods must resolve at the call site cannot be linked.
    pub inline: bool,
}

/// One recorded `[peer-deps]` declaration (RFC 0045 §2): the declaring
/// pkg's claim about a peer. Peers are REQUIRED by default; `optional`
/// marks the presence-mounted kind. `lib` names the peer-gated
/// integration file — an impl-only `.rut` source, relative to the
/// declaring pkg's manifest. `path` is directory-time metadata: never
/// read for a dep's peer (presence is by NAME), read only at the
/// declaring pkg's own build, where a broken path is the loud D3
/// packaging-bug error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerDecl {
    pub optional: bool,
    pub lib: Option<String>,
    pub path: String,
}

impl PeerDecl {
    /// The declaration a descriptor table describes — the loader and
    /// [`Session::load_manifest`] share the reading.
    pub(crate) fn of(desc: &BTreeMap<String, String>) -> PeerDecl {
        PeerDecl {
            optional: desc.get("optional").map(|v| v == "true").unwrap_or(false),
            lib: desc.get("lib").cloned(),
            path: desc.get("path").cloned().unwrap_or_default(),
        }
    }
}

/// Why a use path did not resolve. A miss points at the consumer
/// manifest — the `[deps]` table (or the host) decides what exists —
/// unless the missed name is a declared optional peer of a mounted pkg
/// (RFC 0045 §4): then the dedicated missing-peer error answers, never
/// the bare text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveError {
    /// not `[a-zA-Z0-9_]+`
    BadSpec { spec: String },
    /// no module with that exact name is mounted
    NoModule { spec: String },
    /// D2 (RFC 0045 §4): the name is an OPTIONAL peer some mounted pkg
    /// declared, and the peer is absent — its integration group never
    /// mounted. Names the pkg, the peer, the integration it unlocks,
    /// and the fix. (A REQUIRED peer's absence is louder still: D1 at
    /// mount, so it never reaches resolve through the loader.)
    PeerMissing { spec: String, pkg: String },
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::BadSpec { spec } => write!(
                f,
                "malformed package name `{spec}` — package names are bare `[a-zA-Z0-9_]+` identifiers"
            ),
            ResolveError::NoModule { spec } => write!(
                f,
                "cannot resolve `{spec}` — no module with that name is mounted; declare it in your `rut.toml` `[deps]`"
            ),
            ResolveError::PeerMissing { spec, pkg } => write!(
                f,
                "cannot resolve `{spec}` — `{pkg}`'s {spec} integration is not mounted because the optional peer `{spec}` is absent from this program's closure; add `{spec} = {{ path = \"..\" }}` to your `rut.toml` `[deps]` (RFC 0045 §4)"
            ),
        }
    }
}
impl std::error::Error for ResolveError {}

/// The business-owned mount table. The host decides what exists; the
/// resolver maps exact specifiers. Mirrors the runtime
/// `vm.register_module` (RFC 0035 §3).
#[derive(Clone, Debug, Default)]
pub struct Session {
    modules: BTreeMap<String, Module>,
    deps: BTreeMap<String, BTreeMap<String, String>>,
    /// The `[peer-deps]` declarations the loader recorded (RFC 0045):
    /// declaring pkg → peer spec → declaration. The loader's peer gate
    /// reads it post-closure; the reference-site missing-peer
    /// diagnostic (the D2 upgrade) resolves against it.
    peers: BTreeMap<String, BTreeMap<String, PeerDecl>>,
    /// The directory each mounted pkg came from (programmatic mounts —
    /// `mount_dir` and the dep walks). The loader's `assemble_peers`
    /// reads it to run the peer gate's append pass over a session built
    /// mount-by-mount, where no single walk owns the pkg→dir map.
    peer_dirs: BTreeMap<String, std::path::PathBuf>,
    /// Pkgs whose peer groups the gate already mounted — a second gate
    /// pass over the same session (the graph load ran one, an
    /// `assemble_peers` call adds another) never double-appends.
    groups_mounted: std::collections::BTreeSet<String>,
    /// A mounted v5 compiled bundle's pack-time scope ledger (scope →
    /// the spec that owned it when the closure was packed, engine
    /// mounts included). A decoded program's foreign ids spell these
    /// pack-time scopes; the graph's compiled-mount arm resolves each
    /// through this table to the module to ensure — a reference with no
    /// row is a load error (refuse, never guess).
    bundle_scopes: BTreeMap<rut_core::id::ScopeId, String>,
}

impl Session {
    pub fn new() -> Session {
        Session::default()
    }

    /// Mount a module. Its `spec` must be a bare package name.
    pub fn mount(&mut self, module: Module) -> Result<(), ManifestError> {
        if !valid_spec(&module.spec) {
            return Err(ManifestError(format!(
                "`{}` is not a package name — expected `[a-zA-Z0-9_]+`",
                module.spec
            )));
        }
        self.modules.insert(module.spec.clone(), module);
        Ok(())
    }

    /// Programmatic single-module mount (wasm/tests/plugins) — no file.
    pub fn register_module(&mut self, spec: &str, module: Module) -> Result<(), ManifestError> {
        let mut module = module;
        module.spec = spec.to_string();
        self.mount(module)
    }

    /// Exact resolution: the package name must be mounted as-is. A miss
    /// that is a declared optional peer of some mounted pkg answers D2
    /// (RFC 0045 §4) instead of the bare text — `resolve` is the ONE
    /// path every reference-site miss flows through, so the dedicated
    /// diagnostic holds by construction (item-level misses can never be
    /// peer-gated: groups are impl-only, RFC 0045 §3).
    pub fn resolve(&self, spec: &str) -> Result<&Module, ResolveError> {
        if !valid_spec(spec) {
            return Err(ResolveError::BadSpec { spec: spec.to_string() });
        }
        match self.modules.get(spec) {
            Some(m) => Ok(m),
            None => Err(self.peer_miss(spec)),
        }
    }

    /// The miss diagnostic for `spec`: when the name is a declared
    /// OPTIONAL peer of some mounted pkg, D2 — pkg + peer + the
    /// integration it unlocks + the fix; otherwise the bare NoModule
    /// text. Declaring pkgs scan in mount (BTreeMap) order, so the
    /// diagnostic is deterministic when several pkgs declare the same
    /// peer. A REQUIRED peer's absence never reaches here through the
    /// loader (the peer gate's D1 fires at mount); without the gate it
    /// stays the bare miss — D1's business, not D2's.
    fn peer_miss(&self, spec: &str) -> ResolveError {
        for (pkg, peers) in &self.peers {
            if peers.get(spec).is_some_and(|d| d.optional) {
                return ResolveError::PeerMissing { spec: spec.to_string(), pkg: pkg.clone() };
            }
        }
        ResolveError::NoModule { spec: spec.to_string() }
    }

    /// The host-fn table the mounted host pkgs declare (RFC 0025):
    /// `<scope>::<name>` → signature. The scope is the module's
    /// `host_scope` override when set (`rt` → `rt:log`, RFC 0022), else
    /// the package name. The table feeds `Vm::verify_host_fns` — the
    /// load-time half of the `.d.rut` ↔ host-impl contract (a mismatch
    /// panics before any rut code runs).
    ///
    /// An `async` host row expands into its row family: the decl spells
    /// one name but the embedder registers five bodies (the base name —
    /// a trap, the weave never dispatches it — plus
    /// `__start`/`__yield`/`__take`/`__cancel`), so the RFC 0025
    /// "bound but undeclared" direction stays total for
    /// `register_async!` registrations.
    pub fn expected_host_fns(
        &self,
    ) -> std::collections::BTreeMap<String, (Vec<rut_core::types::TypeId>, rut_core::types::TypeId)> {
        use rut_core::types::{TY_I32, TY_NIL, TY_OPAQUE};
        let mut out = std::collections::BTreeMap::new();
        for (spec, m) in &self.modules {
            let ModuleBody::Host { host_funcs, .. } = &m.body else {
                continue; // only host bodies declare host rows
            };
            let scope = m.host_scope.as_deref().unwrap_or(spec);
            for (name, params, ret, is_async) in host_funcs {
                out.insert(format!("{scope}::{name}"), (params.clone(), *ret));
                if !*is_async {
                    continue;
                }
                // the row family (the weave's wire, phase 4): start
                // answers the state cell (the Completer box, `opaque`),
                // yield is the resumption probe, take marshals the
                // answer through the decl's own return type, cancel is
                // the best-effort abort arm
                let base = format!("{scope}::{name}");
                out.insert(format!("{base}__start"), (params.clone(), TY_OPAQUE));
                out.insert(
                    format!("{base}__yield"),
                    (vec![TY_OPAQUE, TY_OPAQUE], TY_I32),
                );
                out.insert(format!("{base}__take"), (vec![TY_OPAQUE], *ret));
                out.insert(format!("{base}__cancel"), (vec![TY_OPAQUE], TY_NIL));
            }
        }
        out
    }

    /// Record a `[peer-deps]` declaration for `pkg` (RFC 0045). The
    /// loader calls this while walking — it reads every mounted pkg's
    /// manifest anyway, so the registry costs no extra I/O.
    pub fn record_peer(&mut self, pkg: &str, peer: &str, decl: PeerDecl) {
        self.peers.entry(pkg.to_string()).or_default().insert(peer.to_string(), decl);
    }

    /// The peer declarations the loader recorded: declaring pkg →
    /// (peer spec → declaration). Phase 1's peer gate reads it
    /// post-closure; phase 2's D2 upgrade reads it at resolve time.
    pub fn peer_decls(&self) -> &BTreeMap<String, BTreeMap<String, PeerDecl>> {
        &self.peers
    }

    /// Record the directory a pkg was mounted from (programmatic
    /// mounts). `assemble_peers` reads the map to run the gate's group
    /// reads over a session built mount-by-mount.
    pub fn record_peer_dir(&mut self, pkg: &str, dir: &Path) {
        self.peer_dirs.entry(pkg.to_string()).or_insert_with(|| dir.to_path_buf());
    }

    /// The mounted pkgs' directories, first mount wins.
    pub fn peer_dirs(&self) -> &BTreeMap<String, std::path::PathBuf> {
        &self.peer_dirs
    }

    /// Mark `pkg`'s peer groups as already mounted — a second gate pass
    /// over the same session skips them (no double-append).
    pub fn mark_groups_mounted(&mut self, pkg: &str) {
        self.groups_mounted.insert(pkg.to_string());
    }

    /// Was `pkg`'s peer group already appended by an earlier gate pass?
    pub fn groups_mounted(&self, pkg: &str) -> bool {
        self.groups_mounted.contains(pkg)
    }

    /// Record one row of a v5 bundle's pack-time scope ledger (the
    /// loader reads every row of `rut.scopes` at mount).
    pub fn record_bundle_scope(&mut self, scope: rut_core::id::ScopeId, spec: &str) {
        self.bundle_scopes.insert(scope, spec.to_string());
    }

    /// The spec a pack-time scope belonged to, per the mounted ledger.
    pub fn bundle_scope(&self, scope: rut_core::id::ScopeId) -> Option<&str> {
        self.bundle_scopes.get(&scope).map(String::as_str)
    }

    /// Append peer-group source to a mounted module's body (RFC 0045
    /// §3, presence-based group assembly): the combined text stays ONE
    /// source string, so every consumer of a source body — the graph
    /// splice, the wasm mounts — is untouched. A module without a
    /// source body (a host pkg, a compiled `.rutc`) refuses: appended
    /// groups are a source-shape law.
    pub fn append_source(&mut self, spec: &str, text: &str) -> Result<(), ManifestError> {
        let Some(m) = self.modules.get_mut(spec) else {
            return Err(ManifestError(format!(
                "cannot append a peer group to `{spec}` — no such module is mounted"
            )));
        };
        match &mut m.body {
            ModuleBody::Source { text: src, .. } => {
                src.push('\n');
                src.push_str(text);
                Ok(())
            }
            _ => Err(ManifestError(format!(
                "cannot append a peer group to `{spec}` — the module has no rut source body; a `.d.rut` decl surface does not gate (RFC 0045 §3)"
            ))),
        }
    }

    /// Parse and mount a module manifest; a consumer manifest's `[deps]`
    /// are recorded for the host. Use [`parse_manifest`] directly when the
    /// parsed `Manifest` itself is needed.
    pub fn load_manifest(&mut self, text: &str) -> Result<(), ManifestError> {
        let manifest = parse_manifest(text)?;
        if let Some(name) = &manifest.name {
            self.mount(Module {
                spec: name.clone(),
                entry: manifest.entry.clone(),
                ..Default::default()
            })?;
            for (peer, desc) in &manifest.peer_deps {
                self.record_peer(name, peer, PeerDecl::of(desc));
            }
        }
        for (spec, dep) in &manifest.deps {
            if !valid_spec(spec) {
                return Err(ManifestError(format!(
                    "dep `{spec}` is not a package name — expected `[a-zA-Z0-9_]+`"
                )));
            }
            self.deps.insert(spec.clone(), dep.clone());
        }
        Ok(())
    }

    pub fn modules(&self) -> impl Iterator<Item = (&String, &Module)> {
        self.modules.iter()
    }

    pub fn deps(&self) -> impl Iterator<Item = (&String, &BTreeMap<String, String>)> {
        self.deps.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POUCH: &str = r#"
# rut/pouch/rut.toml
name = "pouch"
entry.type = "./pouch.d.rut"
"#;

    #[test]
    fn mount_and_resolve() {
        let mut s = Session::new();
        s.load_manifest(POUCH).unwrap();
        let m = s.resolve("pouch").unwrap();
        assert_eq!(m.entry.type_path.as_deref(), Some("./pouch.d.rut"));
    }

    #[test]
    fn missing_module_names_the_manifest() {
        let s = Session::new();
        let err = s.resolve("missing").unwrap_err();
        assert_eq!(err, ResolveError::NoModule { spec: "missing".into() });
        assert!(err.to_string().contains("`rut.toml` `[deps]`"), "{}", err);
    }

    /// The pinned grammar (survey §0), plus the §2.3 `lib` keys.
    const JSON: &str = r#"
name = "json"
entry.lib = "./json.rut"

[peer-deps]
pouch   = { path = "../pouch",   optional = true, lib = "./serde_pouch.rut" }
nmapset = { path = "../nmapset", optional = true, lib = "./serde_nmapset.rut" }

[dev-deps]
pouch   = { path = "../pouch" }
nmapset = { path = "../nmapset" }
"#;

    #[test]
    fn d2_optional_peer_miss_names_pkg_peer_and_fix() {
        // RFC 0045 §4 (D2): with json's registry entry recorded, a miss
        // on the peer's name is the DEDICATED diagnostic — pkg + peer +
        // the integration it unlocks + the fix — never the bare
        // NoModule text. The pinned survey text, verbatim.
        let mut s = Session::new();
        s.load_manifest(JSON).unwrap();
        let err = s.resolve("pouch").unwrap_err();
        assert_eq!(
            err.to_string(),
            "cannot resolve `pouch` — `json`'s pouch integration is not mounted because the optional peer `pouch` is absent from this program's closure; add `pouch = { path = \"..\" }` to your `rut.toml` `[deps]` (RFC 0045 §4)"
        );
        // a name NO pkg declares as a peer stays the bare miss
        let err = s.resolve("stranger").unwrap_err();
        assert_eq!(err, ResolveError::NoModule { spec: "stranger".into() });
        // a REQUIRED peer's absence is D1's business (loud at mount);
        // reached gate-less it stays the bare miss, not a false D2
        let mut s = Session::new();
        s.load_manifest(
            "name = \"j\"\nentry.lib = \"./j.rut\"\n[peer-deps]\nnmapset = { path = \"../nmapset\" }\n",
        )
        .unwrap();
        let err = s.resolve("nmapset").unwrap_err();
        assert_eq!(err, ResolveError::NoModule { spec: "nmapset".into() });
    }

    #[test]
    fn malformed_specifier() {
        let s = Session::new();
        assert!(matches!(s.resolve("just-a-name"), Err(ResolveError::BadSpec { .. })));
        assert!(matches!(s.resolve("rt:log"), Err(ResolveError::BadSpec { .. })));
    }

    #[test]
    fn programmatic_mount_sets_spec() {
        let mut s = Session::new();
        s.register_module(
            "my_map",
            Module { body: ModuleBody::Source { text: "...".into(), is_decl: false }, ..Default::default() },
        )
        .unwrap();
        let m = s.resolve("my_map").unwrap();
        assert!(matches!(&m.body, ModuleBody::Source { text, .. } if text == "..."));
    }

    #[test]
    fn session_records_peer_declarations() {
        let mut s = Session::new();
        s.load_manifest(JSON).unwrap();
        let decl = s.peer_decls().get("json").and_then(|p| p.get("pouch")).unwrap();
        assert_eq!(
            *decl,
            PeerDecl {
                optional: true,
                lib: Some("./serde_pouch.rut".into()),
                path: "../pouch".into()
            }
        );
        // append_source keeps ONE source string; a sourceless module refuses
        let mut s = Session::new();
        s.register_module(
            "m",
            Module { body: ModuleBody::Source { text: "fn a() {}".into(), is_decl: false }, ..Default::default() },
        )
        .unwrap();
        s.append_source("m", "fn b() {}").unwrap();
        let m = s.resolve("m").unwrap();
        assert!(
            matches!(&m.body, ModuleBody::Source { text, .. } if text == "fn a() {}\nfn b() {}"),
            "{:?}",
            m.body
        );
        // a host body (no rut source) refuses the append
        s.register_module(
            "d",
            Module {
                body: ModuleBody::Host {
                    host_funcs: vec![],
                    consts: vec![],
                    native_types: vec![],
                    native_traits: vec![],
                    native_fns: vec![],
                    native_impls: vec![],
                },
                ..Default::default()
            },
        )
        .unwrap();
        assert!(s.append_source("d", "fn c() {}").is_err());
    }
}
