//! The compile-time mount table — crate-internal since the run chain
//! landed (`RutRun::new()..compile()` is the only public door). The
//! loading model is documented at `docs/src/reference/loading.md`.
//!
//! **One directory is one module.** Its `rut.jsonc` names the exact
//! package it answers to and how to reach its surface and body; the
//! manifest grammar itself — the parsed [`crate::bundle::Manifest`], its
//! dep tables and its error shapes — lives in the [`crate::bundle`]
//! module ([`crate::bundle::parse_manifest`]). This module is the mount table
//! that parsed manifest feeds:
//!
//! ```toml
//! [deps]
//! "pouch" = { path = "rut/pouch" }
//! ```
//!
//! Resolution is exact and single-step: a use path resolves only if a
//! pkg with that `name` is mounted — nothing is derived. Package
//! names are bare `[a-zA-Z0-9_]+` identifiers; a miss points at the
//! consumer manifest (`[deps]`).
//!
//! The dep kinds: `[deps]` is today's transitively-mounted
//! table; `[peer-deps]` is REQUIRED by default (the consumer supplies
//! the peer) with `optional = true` marking the presence-mounted kind
//! whose integration group is the descriptor's `lib` file; `[dev-deps]`
//! mount only while building the pkg itself (the loader's law — the
//! Session never sees a dev table).
//!
//! The `Session` itself does no I/O (wasm hosts mount in memory); the
//! native file readers are the counterparts that read files (the
//! loader, over the [`crate::bundle::Source`] trait).


mod error;
mod module;

pub(crate) use error::ResolveError;
pub use module::{GenSource, HostRow, Pkg, PkgBody, PeerDecl};

use std::collections::BTreeMap;

use crate::bundle::{parse_manifest, valid_spec, ManifestError};

/// The business-owned mount table. The host decides what exists; the
/// resolver maps exact specifiers. Crate-internal: the run chain
/// (`RutRun`) is the public face — this is where `.compile()` mounts
/// the offered pkgs and walks the graph.
#[derive(Clone, Debug, Default)]
pub(crate) struct Session {
    modules: BTreeMap<String, Pkg>,
    /// A mounted v7 compiled bundle's pack-time scope ledger (scope →
    /// the spec that owned it when the closure was packed, engine
    /// mounts included). A decoded program's foreign ids spell these
    /// pack-time scopes; the graph's compiled-mount arm resolves each
    /// through this table to the module to ensure — a reference with no
    /// row is a load error (refuse, never guess).
    bundle_scopes: BTreeMap<rut_core::id::ScopeId, String>,
}

impl Session {
    pub(crate) fn new() -> Session {
        Session::default()
    }

    /// Mount a pkg. Its `spec` must be a bare package name.
    pub(crate) fn mount(&mut self, pkg: Pkg) -> Result<(), ManifestError> {
        if !valid_spec(&pkg.spec) {
            return Err(ManifestError(format!(
                "`{}` is not a package name — expected `[a-zA-Z0-9_]+`",
                pkg.spec
            )));
        }
        self.modules.insert(pkg.spec.clone(), pkg);
        Ok(())
    }

    /// Programmatic single-pkg mount (tests/plugins) — no file. First
    /// mount wins is the CALLER's law; this table keeps the last.
    pub(crate) fn register_module(&mut self, spec: &str, pkg: Pkg) -> Result<(), ManifestError> {
        let mut pkg = pkg;
        pkg.spec = spec.to_string();
        self.mount(pkg)
    }

    /// Exact resolution: the package name must be mounted as-is. A miss
    /// that is a declared optional peer of some mounted pkg answers D2
    /// instead of the bare text — `resolve` is the ONE
    /// path every reference-site miss flows through, so the dedicated
    /// diagnostic holds by construction (item-level misses can never be
    /// peer-gated: groups are impl-only).
    pub(crate) fn resolve(&self, spec: &str) -> Result<&Pkg, ResolveError> {
        if !valid_spec(spec) {
            return Err(ResolveError::BadSpec { spec: spec.to_string() });
        }
        match self.modules.get(spec) {
            Some(m) => Ok(m),
            None => Err(self.peer_miss(spec)),
        }
    }

    /// Mutable exact resolution — the symbol-table apply lane's
    /// take/replace access to a mounted pkg's body (a stripped
    /// bundle's sidecar restores names + positions in place, before the
    /// graph compiles). Same miss diagnostics as [`Session::resolve`].
    pub(crate) fn resolve_mut(&mut self, spec: &str) -> Result<&mut Pkg, ResolveError> {
        if !valid_spec(spec) {
            return Err(ResolveError::BadSpec { spec: spec.to_string() });
        }
        if self.modules.contains_key(spec) {
            return Ok(self.modules.get_mut(spec).unwrap());
        }
        Err(self.peer_miss(spec))
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
        for (_, m) in &self.modules {
            if m.peers.get(spec).is_some_and(|d| d.optional) {
                return ResolveError::PeerMissing { spec: spec.to_string(), pkg: m.spec.clone() };
            }
        }
        ResolveError::NoModule { spec: spec.to_string() }
    }

    /// The mounted host pkgs' declared rows, partitioned by
    /// registration scope — the driver's distillation of its mount
    /// knowledge into the vm-side value `HostRegistry::install_host_pkg`
    /// checks against. One walk replaces the whole helper family:
    /// scope = spec (the registration naming has no override), each row
    /// carries its `(params, ret)`, and async rows expand into their
    /// family (`__start` params→opaque, `__yield` opaque,opaque→i32,
    /// `__take` opaque→ret, `__cancel` opaque→nil) exactly as
    /// `register_async!`'s emitter spells them. Snapshot semantics:
    /// build once per boot lane; rebuild if mounts change after.
    pub(crate) fn host_pkg_context(&self) -> rut_vm::interp::HostPkgContext {
        use rut_core::types::{TY_I32, TY_NIL, TY_OPAQUE};
        let mut ctx = rut_vm::interp::HostPkgContext::default();
        for (spec, m) in &self.modules {
            let PkgBody::Host { host_funcs, .. } = &m.body else {
                continue; // only host bodies declare host rows
            };
            for (name, params, ret, is_async) in host_funcs {
                ctx.declare(spec, name, params.clone(), *ret);
                if !*is_async {
                    continue;
                }
                // the row family (the weave's wire, phase 4): start
                // answers the state cell (the Completer box, `opaque`),
                // yield is the resumption probe, take marshals the
                // answer through the decl's own return type, cancel is
                // the best-effort abort arm
                ctx.declare(spec, &format!("{name}__start"), params.clone(), TY_OPAQUE);
                ctx.declare(spec, &format!("{name}__yield"), vec![TY_OPAQUE, TY_OPAQUE], TY_I32);
                ctx.declare(spec, &format!("{name}__take"), vec![TY_OPAQUE], *ret);
                ctx.declare(spec, &format!("{name}__cancel"), vec![TY_OPAQUE], TY_NIL);
            }
        }
        ctx
    }

    /// Record one row of a v7 bundle's pack-time scope ledger (the
    /// loader reads every row of `rut.scopes` at mount).
    pub(crate) fn record_bundle_scope(&mut self, scope: rut_core::id::ScopeId, spec: &str) {
        self.bundle_scopes.insert(scope, spec.to_string());
    }

    /// The spec a pack-time scope belonged to, per the mounted ledger.
    pub(crate) fn bundle_scope(&self, scope: rut_core::id::ScopeId) -> Option<&str> {
        self.bundle_scopes.get(&scope).map(String::as_str)
    }

    /// The first free scope number above every recorded ledger row — a
    /// freshly mounted archive's namespace base (`None` when the
    /// numbering space is exhausted). Boot owns 0 and passes through
    /// every map; each archive's rows shift above everything recorded
    /// so far, so two independently packed bundles can share one
    /// session without their pack-time numberings ever arguing.
    pub(crate) fn bundle_scope_next_base(&self) -> Option<rut_core::id::ScopeId> {
        let max = self
            .bundle_scopes
            .keys()
            .copied()
            .max()
            .unwrap_or(rut_core::id::BOOT_SCOPE);
        let next = max.checked_add(1)?;
        if next as u32 > rut_core::id::MAX_SCOPE {
            None
        } else {
            Some(next)
        }
    }

    /// The mounted pkgs, spec order.
    pub(crate) fn modules(&self) -> impl Iterator<Item = (&String, &Pkg)> {
        self.modules.iter()
    }

    /// Take every mounted pkg out — the walk collectors' yield
    /// (`Loaded`): the session dies, the pkgs travel.
    pub(crate) fn drain_pkgs(&mut self) -> Vec<Pkg> {
        self.modules.values().cloned().collect()
    }

    /// Parse and mount a pkg manifest; a consumer manifest's `[deps]`
    /// are folded into the mounted pkg's own table. Crate-internal
    /// today (the wasm wall it served is a chain now) — the manifest
    /// grammar's tests pin through it. Dies with the walk in Phase B.
    #[allow(dead_code)]
    pub(crate) fn load_manifest(&mut self, text: &str) -> Result<(), ManifestError> {
        let manifest = parse_manifest(text)?;
        if let Some(name) = &manifest.name {
            self.mount(Pkg {
                spec: name.clone(),
                entry: manifest.entry.clone(),
                peers: manifest
                    .peer_deps
                    .iter()
                    .map(|(peer, desc)| (peer.clone(), PeerDecl::of(desc)))
                    .collect(),
                ..Default::default()
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POUCH: &str = r#"
// rut/pouch manifest — the surface + the body
{
  "name": "pouch",
  "entry": { "type": "./pouch.d.rut", "lib": "./pouch.rut" }
}
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
        assert!(err.to_string().contains("`rut.jsonc` `deps`"), "{}", err);
    }

    /// The pinned grammar (survey §0), plus the §2.3 `lib` keys.
    const JSON: &str = r#"
{
  "name": "json",
  "entry": { "lib": "./json.rut" },

  "peer-deps": {
    "pouch":   { "path": "../pouch",   "optional": true, "lib": "./serde_pouch.rut" },
    "nmapset": { "path": "../nmapset", "optional": true, "lib": "./serde_nmapset.rut" }
  },

  "dev-deps": {
    "pouch":   { "path": "../pouch" },
    "nmapset": { "path": "../nmapset" }
  }
}
"#;

    #[test]
    fn d2_optional_peer_miss_names_pkg_peer_and_fix() {
        // (D2): with json's registry entry recorded, a miss
        // on the peer's name is the DEDICATED diagnostic — pkg + peer +
        // the integration it unlocks + the fix — never the bare
        // NoModule text. The pinned survey text, verbatim.
        let mut s = Session::new();
        s.load_manifest(JSON).unwrap();
        let err = s.resolve("pouch").unwrap_err();
        assert_eq!(
            err.to_string(),
            "cannot resolve `pouch` — `json`'s pouch integration is not mounted because the optional peer `pouch` is absent from this program's closure; add `\"pouch\": { \"path\": \"..\" }` to your `rut.jsonc` `deps`"
        );
        // a name NO pkg declares as a peer stays the bare miss
        let err = s.resolve("stranger").unwrap_err();
        assert_eq!(err, ResolveError::NoModule { spec: "stranger".into() });
        // a REQUIRED peer's absence is D1's business (loud at mount);
        // reached gate-less it stays the bare miss, not a false D2
        let mut s = Session::new();
        s.load_manifest(
            r#"{"name": "j", "entry": {"lib": "./j.rut"}, "peer-deps": {"nmapset": {"path": "../nmapset"}}}"#,
        )
        .unwrap();
        let err = s.resolve("nmapset").unwrap_err();
        assert_eq!(err, ResolveError::NoModule { spec: "nmapset".into() });
    }

    #[test]
    fn malformed_specifier() {
        let s = Session::new();
        assert!(matches!(s.resolve("just-a-name"), Err(ResolveError::BadSpec { .. })));
        assert!(matches!(s.resolve("ink:host"), Err(ResolveError::BadSpec { .. })));
    }

    #[test]
    fn programmatic_mount_sets_spec() {
        let mut s = Session::new();
        s.register_module("my_map", Pkg::source("my_map", "...")).unwrap();
        let m = s.resolve("my_map").unwrap();
        assert!(matches!(&m.body, PkgBody::Source { text, .. } if text == "..."));
    }

    #[test]
    fn session_records_peer_declarations() {
        let mut s = Session::new();
        s.load_manifest(JSON).unwrap();
        let decl = s.resolve("json").unwrap().peers.get("pouch").unwrap();
        assert_eq!(
            *decl,
            PeerDecl {
                optional: true,
                lib: Some("./serde_pouch.rut".into()),
                path: "../pouch".into()
            }
        );
        // a recorded peer group rides the pkg, never the body
        let mut s = Session::new();
        s.register_module("m", Pkg::source("m", "fn a() {}")).unwrap();
        s.resolve_mut("m").unwrap().peer_groups.push("fn b() {}".to_string());
        let m = s.resolve("m").unwrap();
        assert!(
            matches!(&m.body, PkgBody::Source { text, .. } if text == "fn a() {}"),
            "{:?}",
            m.body
        );
        assert_eq!(m.peer_groups, vec!["fn b() {}".to_string()]);
        // a host body (no rut source) carries no group text either —
        // groups compile into SOURCE units only
        s.register_module("d", Pkg::host("d", vec![])).unwrap();
        assert!(s.resolve("d").unwrap().peer_groups.is_empty());
    }
}
