//! The package value: its body kinds.
//!
//! A `Mounted` is pure data — a named package. The walk (rut-native)
//! yields them; [`crate::run::RutRun`] offers them; the compile-time
//! mount table (the crate-internal `Session`) holds them. Walker-found
//! metadata (the pkg's own manifest — dep tables included) is pub data
//! on it; the walk's own bookkeeping (where the pkg's files live) never
//! rides — it died with the walk's move to rut-native.

use std::collections::BTreeMap;

use crate::bundle::{Entry, Manifest};
use crate::mods::ModSource;

/// What a mounted pkg's body IS. The graph dispatches on this:
/// a source body compiles (and may splice), a compiled body pushes as
/// decoded, a host body synthesizes its placeholder program.
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
// the payloads ARE the enum: decoded bundles move as values
// through the graph — boxing would churn every construction/match
// for a size win the graph never sees
pub enum MountedBody {
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
        /// the declared rows — [`HostRow`] carries the mod path each
        /// row lives under (`""` = the pkg root's own surface),
        /// `is_async` marks a `host async fn` (the row family expands
        /// at the mount)
        host_funcs: Vec<HostRow>,
        /// exported constants: `(name, type, raw bits)` — `calc::PI`
        consts: Vec<(String, rut_core::types::TypeId, u64)>,
        /// Builtin containers published by name (`core` only), each
        /// with its ambient bit — `true` (`prelude builtin`) binds in
        /// every unit with no `use`, `false` (`pub builtin`) resolves
        /// only through `use`
        native_types: Vec<(String, rut_core::binary::NativeTy, bool)>,
        /// Builtin traits published by name (`core` only), same
        /// ambient-bit law as [`MountedBody::Host`]'s `native_types`
        /// Compiler-lowered builtin function names (`core` only) — no
        /// bodies; rut-lir lowers them. Each row carries its ambient
        /// bit (same law)
        native_fns: Vec<(String, bool)>,
        /// Builtin-impl methods (`core` only): the
        /// integer primitives' numeric methods — `(receiver prim, name,
        /// lowering id)`. Bodyless and hostless — rut-lir expands the
        /// method call inline, ambient on the primitive.
        native_impls: Vec<(rut_core::types::TypeId, String, rut_core::ops::Intrinsic)>,
        /// the `.d.rut` root surface the rows lowered from — `Some`
        /// for a loaded `type = "host"` pkg (the rows' only carrier;
        /// the dep-side decl tables of phase 2 read the tree off it),
        /// `None` for hand-built row sets (`core`, `Mounted::host`)
        root_surface: Option<String>,
    },
}

/// One declared host row: the mod path it lives under (`""` = the
/// pkg root's own surface, `"net"`, `"net/tls"`), its own name, the
/// signature, and the async flag (`is_async` marks a `host async fn`
/// — the row family expands at the mount).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostRow {
    /// the module path relative to the pkg root — `""` for a row on
    /// the root surface (flat pkgs), `"net"` / `"net/tls"` under a
    /// `mod` tree
    pub path: String,
    /// the row's own name (`connect`) — never path-qualified
    pub name: String,
    pub params: Vec<rut_core::types::TypeId>,
    pub ret: rut_core::types::TypeId,
    pub is_async: bool,
}

impl HostRow {
    /// A root-surface row (`path = ""`) — the flat spelling every
    /// pre-`mod` surface carries.
    pub fn root(
        name: &str,
        params: Vec<rut_core::types::TypeId>,
        ret: rut_core::types::TypeId,
        is_async: bool,
    ) -> HostRow {
        HostRow {
            path: String::new(),
            name: name.to_string(),
            params,
            ret,
            is_async,
        }
    }

    /// The scope-relative registration spelling: `name` at the root,
    /// `path::name` under a module (each mod path segment a `::`
    /// component — `net/tls` + `handshake` spells `net::tls::handshake`)
    /// — what [`rut_vm::HostPkg`]'s builder accepts and the mount
    /// table's per-scope keys carry (the pkg scope prefixes at the
    /// install).
    pub fn rel_name(&self) -> String {
        let mut out = String::new();
        for seg in self.path.split('/') {
            if !out.is_empty() {
                out.push_str("::");
            }
            out.push_str(seg);
        }
        if !out.is_empty() {
            out.push_str("::");
        }
        out.push_str(&self.name);
        out
    }

    /// The FULL registration name — `pkg::path::name` (`myhost::net::connect`).
    /// The one source for the registration string: the synthesized
    /// surface's host ids and the expected-fns table both spell it
    /// through here, and the flat rows are the `path = ""` case
    /// (`myhost::name`) — byte-identical to the old shape.
    pub fn full_name(&self, pkg: &str) -> String {
        format!("{pkg}::{}", self.rel_name())
    }
}

impl Default for MountedBody {
    /// An empty source body — the manifest-only pkg (a pkg whose
    /// entries ride on other fields) parses to an empty unit.
    fn default() -> MountedBody {
        MountedBody::Source {
            text: String::new(),
            is_decl: false,
        }
    }
}

/// One package: the bare name it answers to, its entry files, its body
/// ([`MountedBody`]), and the walker-found metadata. Pure data — offer it
/// to a run with [`crate::run::RutRun::pkg`].
#[derive(Clone, Debug, Default)]
pub struct Mounted {
    /// the exact package name — bare `[a-zA-Z0-9_]+`
    pub spec: String,
    /// The namespace head for qualified member access (`Math.sqrt`) —
    /// `None` when the pkg has no namespace form.
    pub namespace: Option<String>,
    pub entry: Entry,
    /// the body: `.rut` source, a decoded `.rutc`, or the native rows
    pub body: MountedBody,
    /// The mounted bundle's own scope ledger — the pack-time module set
    /// (this root plus its groups), already namespaced into this
    /// session's numbering. The compiled walk's dep list: v21's
    /// split-pack root carries no foreign references (its bodies ride as
    /// source), so the binary's foreign scan sees none of these. Empty
    /// for source, host, and decl-root mounts.
    pub bundle_scopes: Vec<(rut_core::id::ScopeId, String)>,
    /// The pkg's mounted file modules (the `mod` tree's children),
    /// keyed by mod path (`"layout"`, `"layout/grid"`) — the loader
    /// mounts them by DECLARATION (`mod NAME;` resolves against
    /// `NAME/mod.rut` beside the declaring file), never a directory
    /// listing. The root file itself is the body's text, never a row
    /// here. Empty for flat packages: every manifest-spelled source
    /// set stays byte-stable, and a flat pkg's compile is untouched.
    pub mods: BTreeMap<String, ModSource>,
    /// The pkg's own parsed manifest — the STRUCTURAL form every
    /// producer stamps: the walk's `rut.jsonc` read, a bundle group's
    /// in-archive manifest, a host's in-memory build
    /// ([`Mounted::manifest`]). Json text is only a loader input; this is
    /// the one mechanism — the dep tables (`deps`, `dev-deps`) are read
    /// off it, never off pkg fields (the old flat `deps` field folded
    /// in here). For a walked pkg the WHOLE
    /// manifest rides, dev rows included, so a program root's
    /// self-build allowlist reads its own `dev-deps` back later.
    pub manifest: Manifest,
    /// The first-mount edge: WHO pulled this pkg into the world — the
    /// parent spec whose dep row declared it (a dir walk), the archive
    /// root whose group it rides (a bundle), or `None` (a hand offer,
    /// or the walk's own root). Pure diagnostic data: the uses
    /// diagnostic's later "pulled by `a`" clause reads it.
    pub mounted_by: Option<String>,
}

impl Mounted {
    /// The same pkg under another name — `lower_decl_module`'s surface
    /// with the registration scope spelled at the offer.
    pub fn named(mut self, spec: &str) -> Mounted {
        self.spec = spec.to_string();
        self
    }

    /// Attach the parsed structural manifest — the builder-style door
    /// for hosts building worlds in memory
    /// (`Mounted::source("app", src).manifest(Manifest::lib("app", &["ink"]))`).
    /// The walk's producers stamp the same field; a hand offer without
    /// one carries `Manifest::default()`. Parsing json text is never
    /// this method's business — the host calls
    /// [`crate::bundle::parse_manifest`] itself.
    pub fn manifest(mut self, m: Manifest) -> Mounted {
        self.manifest = m;
        self
    }

    /// A source pkg — the ordinary `.rut` body. `Mounted::source("app", src)`.
    pub fn source(name: &str, text: impl Into<String>) -> Mounted {
        Mounted {
            spec: name.to_string(),
            body: MountedBody::Source {
                text: text.into(),
                is_decl: false,
            },
            ..Default::default()
        }
    }

    /// A declaration-mode pkg — a `.d.rut` surface parsed as its own
    /// unit (the surface-only dev state).
    pub fn decl(name: &str, text: impl Into<String>) -> Mounted {
        Mounted {
            spec: name.to_string(),
            body: MountedBody::Source {
                text: text.into(),
                is_decl: true,
            },
            ..Default::default()
        }
    }

    /// A host-ABI pkg: declared rows, no rut body — the embedding Rust
    /// binds them at run time. Async rows carry `is_async = true` and
    /// expand into their five-row family at the mount.
    pub fn host(name: &str, rows: Vec<HostRow>) -> Mounted {
        Mounted {
            spec: name.to_string(),
            body: MountedBody::Host {
                host_funcs: rows,
                consts: vec![],
                native_types: vec![],
                native_fns: vec![],
                native_impls: vec![],
                root_surface: None,
            },
            ..Default::default()
        }
    }

    /// A decoded `.rutc` program as a pkg's body (a compiled bundle's
    /// payload, already gated).
    pub fn compiled(name: &str, program: rut_core::binary::Program) -> Mounted {
        Mounted {
            spec: name.to_string(),
            body: MountedBody::Compiled(program),
            ..Default::default()
        }
    }

    /// The pure container parse: a packed `.rutbundle`'s bytes → the
    /// walked yield ([`crate::run::Walk`]) — gates, groups, and the
    /// scope ledger. No filesystem, no network: the same contract a
    /// url dep rides. The legacy one-shot spelling — decode ONCE and
    /// reuse the handle instead ([`crate::Bundle::parse`], then offer
    /// it to any number of runs).
    pub fn from_bundle(bytes: &[u8]) -> Result<crate::run::Walk, crate::run::RunError> {
        crate::run::Bundle::parse(bytes)?.walk()
    }
}
