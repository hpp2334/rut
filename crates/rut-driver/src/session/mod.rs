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
//! Two doors share that resolution: [`Session::resolve`] (the raw
//! mount lookup) and [`Session::resolve_use`] — the USE door, which
//! adds the nameability law on top: a mounted head passes only when
//! the consumer DECLARES it (its manifest's `deps`, plus the
//! program root's `dev-deps`; `core` rides unconditionally). The
//! mounted-but-undeclared miss is D5, the dedicated diagnostic. The
//! sibling [`Session::own_use_head`] answers the own-package lane: a
//! use head that is no declared dep may name the consumer's OWN
//! package — its spec (the root module) or a `mod` edge visible-walked
//! from the importing file.
//!
//! The dep kinds: `[deps]` is today's transitively-mounted
//! table; `[dev-deps]`
//! mounts only while building the pkg itself (the loader's law — the
//! Session never sees a dev table). The retired `[peer-deps]` table
//! left the grammar (strict keys); its mount machinery rides inert
//! until phase 2 deletes it.
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

    /// The single use-door: resolve `head` FOR `consumer` under the
    /// nameability law. The order: a malformed name is
    /// [`ResolveError::BadSpec`]; an UNMOUNTED head keeps the existing
    /// shapes (D2 when the miss is a declared optional peer of a
    /// mounted pkg, else NoModule — unchanged); a MOUNTED head passes
    /// only when `consumer` DECLARES it — [`Session::declares`]'s
    /// allowlist — otherwise D5, the dedicated
    /// mounted-but-undeclared diagnostic. `core` passes
    /// unconditionally: the graph already rides the prelude into every
    /// unit. `root` is the program root's spec — the one consumer whose
    /// `dev-deps` are nameable (dev tables mount only for a self-build).
    pub(crate) fn resolve_use(
        &self,
        consumer: &str,
        head: &str,
        root: &str,
    ) -> Result<&Pkg, ResolveError> {
        let pkg = self.resolve(head)?;
        if head == "core" || self.declares(consumer, head, root) {
            Ok(pkg)
        } else {
            Err(ResolveError::not_declared(head, consumer, pkg.mounted_by.as_deref()))
        }
    }

    /// The own-package answer for a use head — the sibling the use
    /// validation consults beside [`Session::resolve_use`]. `Some` when
    /// the CONSUMER's own package claims the head: a `mod` edge
    /// visible-walked from `from` (the importing file's mod path; the
    /// current module first, then the ancestors, nearest wins — the
    /// qualified positions' walk exactly), or the consumer's OWN spec
    /// naming the root module. The walk ignores edge visibility: mod
    /// edges are intra-package namespaces, the LEAF gate decides what
    /// crosses. A flat package (no mod children) claims nothing — its
    /// own name was never a legal use head there.
    pub(crate) fn own_use_head(&self, consumer: &str, head: &str, from: &str) -> Option<String> {
        let pkg = self.resolve(consumer).ok()?;
        let mut cur = from.to_string();
        loop {
            let child = if cur.is_empty() {
                head.to_string()
            } else {
                format!("{cur}/{head}")
            };
            if pkg.mods.contains_key(&child) {
                return Some(child);
            }
            let parent = match cur.rsplit_once('/') {
                Some((p, _)) => p.to_string(),
                None => String::new(),
            };
            if parent == cur {
                break;
            }
            cur = parent;
        }
        if !pkg.mods.is_empty() && head == pkg.spec {
            return Some(String::new());
        }
        None
    }

    /// The nameability allowlist: the heads `consumer` may `use` — its
    /// own manifest's `deps` table, plus `dev-deps` when
    /// the consumer IS the program root. Read off `pkg.manifest` (the
    /// STRUCTURAL form every producer stamps — the walk's jsonc read, a
    /// bundle's in-archive manifest, a host's `Pkg::manifest(..)` /
    /// `parse_manifest(..)`), never jsonc text: a stamped pkg is the
    /// one mechanism, and hosts build worlds where no file exists.
    fn declares(&self, consumer: &str, head: &str, root: &str) -> bool {
        let Some(c) = self.modules.get(consumer) else {
            return false;
        };
        c.manifest.deps.contains_key(head)
            || (c.spec == root && c.manifest.dev_deps.contains_key(head))
    }

    /// The mounted host pkgs' declared rows, partitioned by
    /// registration scope — the driver's distillation of its mount
    /// knowledge into the vm-side value `HostRegistry::install_host_pkg`
    /// checks against. One walk replaces the whole helper family:
    /// scope = spec (the registration naming has no override), each row
    /// carries its `(params, ret)` keyed by its scope-relative name
    /// (`name` at the root, `path::name` under a module — the full
    /// registration string is `scope::<rel>`), and async rows expand
    /// into their family (`__start` params→opaque, `__yield`
    /// opaque,opaque→i32, `__take` opaque→ret, `__cancel` opaque→nil)
    /// onto that same qualified name, exactly as
    /// `register_async!`'s emitter spells them. Snapshot semantics:
    /// build once per boot lane; rebuild if mounts change after.
    pub(crate) fn host_pkg_context(&self) -> rut_vm::interp::HostPkgContext {
        use rut_core::types::{TY_I32, TY_NIL, TY_OPAQUE};
        let mut ctx = rut_vm::interp::HostPkgContext::default();
        for (spec, m) in &self.modules {
            let PkgBody::Host { host_funcs, .. } = &m.body else {
                continue; // only host bodies declare host rows
            };
            for row in host_funcs {
                let rel = row.rel_name();
                ctx.declare(spec, &rel, row.params.clone(), row.ret);
                if !row.is_async {
                    continue;
                }
                // the row family (the weave's wire, phase 4): start
                // answers the state cell (the Completer box, `opaque`),
                // yield is the resumption probe, take marshals the
                // answer through the decl's own return type, cancel is
                // the best-effort abort arm
                ctx.declare(spec, &format!("{rel}__start"), row.params.clone(), TY_OPAQUE);
                ctx.declare(spec, &format!("{rel}__yield"), vec![TY_OPAQUE, TY_OPAQUE], TY_I32);
                ctx.declare(spec, &format!("{rel}__take"), vec![TY_OPAQUE], row.ret);
                ctx.declare(spec, &format!("{rel}__cancel"), vec![TY_OPAQUE], TY_NIL);
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

    /// The whole table — the pure peer gate's pass runs over it at the
    /// close of the world.
    pub(crate) fn table_mut(&mut self) -> &mut BTreeMap<String, Pkg> {
        &mut self.modules
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
                // phase-2 deletes the peers lane: `[peer-deps]` left the
                // grammar, so no parse ever fills `Pkg::peers` now
                manifest: manifest.clone(),
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
  "entry": { "type": "./pouch.d.rut" }
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
    fn peer_groups_ride_the_pkg_never_the_body() {
        // a recorded peer group rides the pkg, never the body (the
        // mount lane that used to feed it — the manifest's
        // `[peer-deps]` — left the grammar; the mechanism itself is
        // phase-2's deletion)
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
