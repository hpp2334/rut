//! The package value: its body kinds, the generic-source rider, and
//! the peer declarations a manifest recorded.
//!
//! A `Pkg` is pure data — a named package. The walk (rut-native)
//! yields them; [`crate::run::RutRun`] offers them; the compile-time
//! mount table (the crate-internal `Session`) holds them. Walker-found
//! metadata (the deps table, the peer declarations and their
//! integration texts) is pub data on it; the walk's own bookkeeping
//! (where the pkg's files live) never rides — it died with the walk's
//! move to rut-native.

use std::collections::BTreeMap;

use crate::bundle::Entry;

/// What a mounted pkg's body IS. The graph dispatches on this:
/// a source body compiles (and may splice), a compiled body pushes as
/// decoded, a host body synthesizes its placeholder program.
#[derive(Clone, Debug)]
pub enum PkgBody {
    /// a `.rut` body — compile it. `is_decl` marks a declaration-mode
    /// pkg (a `.d.rut` surface parsed as its own unit):
    /// nothing to compile or run, but `rut dump` shows the AST.
    Source { text: String, is_decl: bool },
    /// a decoded `.rutc` program (a v7 compiled bundle's payload): the
    /// body already exists — the graph assigns it a fresh scope,
    /// rebases its packed ids, and pushes it. The loader's decode gate
    /// (version + surface verification) has passed.
    Compiled(rut_core::binary::Program),
    /// a native/host module: no rut body — bodyless
    /// functions the embedder binds at run time, exported constants,
    /// and the builtin rows (`core`'s prelude, `calc`'s `Math`). The
    /// graph synthesizes a placeholder program from these rows.
    Host {
        /// `(name, params, ret, is_async)` — `is_async` marks a
        /// `host async fn` (the host future lane)
        host_funcs: Vec<HostRow>,
        /// exported constants: `(name, type, raw bits)` — `calc::PI`
        consts: Vec<(String, rut_core::types::TypeId, u64)>,
        /// Builtin containers published by name (`core` only), each
        /// with its ambient bit — `true` (`prelude builtin`) binds in
        /// every unit with no `use`, `false` (`pub builtin`) resolves
        /// only through `use`
        native_types: Vec<(String, rut_core::binary::NativeTy, bool)>,
        /// Builtin traits published by name (`core` only), same
        /// ambient-bit law as [`PkgBody::Host`]'s `native_types`
        /// Compiler-lowered builtin function names (`core` only) — no
        /// bodies; rut-lir lowers them. Each row carries its ambient
        /// bit (same law)
        native_fns: Vec<(String, bool)>,
        /// Builtin-impl methods (`core` only): the
        /// integer primitives' numeric methods — `(receiver prim, name,
        /// lowering id)`. Bodyless and hostless — rut-lir expands the
        /// method call inline, ambient on the primitive.
        native_impls: Vec<(rut_core::types::TypeId, String, rut_core::ops::Intrinsic)>,
    },
}

/// One declared host row: `(name, params, ret, is_async)`. `is_async`
/// marks a `host async fn` (the row family expands at the mount).
pub type HostRow = (String, Vec<rut_core::types::TypeId>, rut_core::types::TypeId, bool);

impl Default for PkgBody {
    /// An empty source body — the manifest-only pkg (a pkg whose
    /// entries ride on other fields) parses to an empty unit.
    fn default() -> PkgBody {
        PkgBody::Source { text: String::new(), is_decl: false }
    }
}

/// The generic-bearing source a compiled bundle unit rides beside its
/// binary (generic-source riding): the pkg's own source text (the entry
/// lib + `entry.libs`, spliced — the exact text a directory mount
/// compiles) and the `[peer-deps]` group files keyed by peer spec, in
/// manifest (peer-name) order. A compiled pkg whose surface exports
/// generics rides this so consumer-spelled shapes stay servable; the
/// graph lowers it in the CONSUMER's session only when a request misses
/// the pack-time ledger — nothing persists, `.rutc` caches stay
/// pack-time. `None` for directory mounts, for non-generic compiled
/// pkgs, and for legacy bundles that predate the riding.
#[derive(Clone, Debug)]
pub struct GenSource {
    /// the pkg's own source: `entry.lib` + `entry.libs`, '\n'-joined —
    /// the same splice shape a source mount reads back
    pub text: String,
    /// `(peer spec, group file text)` — the `[peer-deps]` `lib` files;
    /// the recompile splices exactly the rows whose peer is in the
    /// program's closure (the presence law, peer-name order)
    pub peers: Vec<(String, String)>,
}

/// One package: the bare name it answers to, its entry files, its body
/// ([`PkgBody`]), and the walker-found metadata. Pure data — offer it
/// to a run with [`crate::run::RutRun::pkg`].
#[derive(Clone, Debug, Default)]
pub struct Pkg {
    /// the exact package name — bare `[a-zA-Z0-9_]+`
    pub spec: String,
    /// The namespace head for qualified member access (`Math.sqrt`) —
    /// `None` when the pkg has no namespace form.
    pub namespace: Option<String>,
    pub entry: Entry,
    /// the body: `.rut` source, a decoded `.rutc`, or the native rows
    pub body: PkgBody,
    /// the generic-bearing source a compiled bundle unit rides beside
    /// its binary ([`GenSource`]) — the on-demand recompile's input
    pub gen_source: Option<GenSource>,
    /// The mounted bundle's own scope ledger — the pack-time module set
    /// (this root plus its groups), already namespaced into this
    /// session's numbering. The compiled walk's dep list: v21's
    /// split-pack root carries no foreign references (its bodies ride as
    /// source), so the binary's foreign scan sees none of these. Empty
    /// for source, host, and decl-root mounts.
    pub bundle_scopes: Vec<(rut_core::id::ScopeId, String)>,
    /// The pkg's own `[deps]` table as the walk found it — the parsed
    /// manifest's dep rows (spec → descriptor). Pub data: an embedder
    /// can inspect what the closure declares.
    pub deps: BTreeMap<String, BTreeMap<String, String>>,
    /// The pkg's own `[peer-deps]` declarations — declaring name →
    /// declaration. Pub data: the peer gate reads it post-closure; the
    /// reference-site missing-peer diagnostic resolves against it.
    pub peers: BTreeMap<String, PeerDecl>,
    /// The pkg's declared peer-INTEGRATION texts, read by the walk at
    /// mount: peer name → the descriptor's `lib` file. `Some` when the
    /// read succeeded, `None` when the file could not be read (the
    /// gate's loud D3 packaging-bug error — but ONLY when the peer is
    /// present; an absent optional peer stays inert). A name absent
    /// from the map altogether is a hand-offered pkg whose world
    /// carries no files — the gate's "not mounted" error. Pure data:
    /// the gate moves a text onto [`Pkg::peer_groups`] on presence.
    pub peer_libs: BTreeMap<String, Option<String>>,
    /// Presence-gated peer-integration groups recorded for this pkg: the
    /// gate moved the descriptor's `lib` text here because the peer is
    /// in the program's closure. The graph compiles them INTO this
    /// pkg's unit (after its own source) — a mounted pkg's body is
    /// never mutated. A host whose world has no filesystem (wasm)
    /// appends the same texts by hand — the pkg is pure data, the
    /// graph only reads.
    pub peer_groups: Vec<String>,
    /// Have this pkg's peer groups already been appended by an earlier
    /// gate pass (or by the offering host)? A second pass never
    /// double-appends.
    pub groups_mounted: bool,
}

impl Pkg {
    /// The same pkg under another name — `lower_decl_module`'s surface
    /// with the registration scope spelled at the offer.
    pub fn named(mut self, spec: &str) -> Pkg {
        self.spec = spec.to_string();
        self
    }

    /// A source pkg — the ordinary `.rut` body. `Pkg::source("app", src)`.
    pub fn source(name: &str, text: impl Into<String>) -> Pkg {
        Pkg {
            spec: name.to_string(),
            body: PkgBody::Source { text: text.into(), is_decl: false },
            ..Default::default()
        }
    }

    /// A declaration-mode pkg — a `.d.rut` surface parsed as its own
    /// unit (the surface-only dev state).
    pub fn decl(name: &str, text: impl Into<String>) -> Pkg {
        Pkg {
            spec: name.to_string(),
            body: PkgBody::Source { text: text.into(), is_decl: true },
            ..Default::default()
        }
    }

    /// A host-ABI pkg: declared rows, no rut body — the embedding Rust
    /// binds them at run time. Async rows carry `is_async = true` and
    /// expand into their five-row family at the mount.
    pub fn host(name: &str, rows: Vec<HostRow>) -> Pkg {
        Pkg {
            spec: name.to_string(),
            body: PkgBody::Host {
                host_funcs: rows,
                consts: vec![],
                native_types: vec![],
                native_fns: vec![],
                native_impls: vec![],
            },
            ..Default::default()
        }
    }

    /// A decoded `.rutc` program as a pkg's body (a compiled bundle's
    /// payload, already gated).
    pub fn compiled(name: &str, program: rut_core::binary::Program) -> Pkg {
        Pkg {
            spec: name.to_string(),
            body: PkgBody::Compiled(program),
            ..Default::default()
        }
    }

    /// The pure container parse: a packed `.rutbundle`'s bytes → the
    /// walked yield ([`crate::run::Loaded`]) — gates, groups, the scope
    /// ledger, and the ONE peer gate over the archive's own pkgs. No
    /// filesystem, no network: the same contract a url dep rides.
    /// Pure data: the gate moves a text onto [`Pkg::peer_groups`] on presence.
    pub fn from_bundle(bytes: &[u8]) -> Result<crate::run::Loaded, crate::run::RunError> {
        crate::loader::bundle_walk_bytes(bytes)
    }
}

/// One recorded `[peer-deps]` declaration: the declaring
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
    /// The declaration a descriptor table describes — the walk and
    /// the manifest lanes share the reading.
    pub fn of(desc: &BTreeMap<String, String>) -> PeerDecl {
        PeerDecl {
            optional: desc.get("optional").map(|v| v == "true").unwrap_or(false),
            lib: desc.get("lib").cloned(),
            path: desc.get("path").cloned().unwrap_or_default(),
        }
    }
}
