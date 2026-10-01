//! The mounted module: its body kinds, the generic-source rider,
//! and the peer declarations a manifest recorded.

use std::collections::BTreeMap;

use crate::bundle::Entry;

/// What a mounted module's body IS. The graph dispatches on this:
/// a source body compiles (and may splice), a compiled body pushes as
/// decoded, a host body synthesizes its placeholder program.
#[derive(Clone, Debug)]
pub enum ModuleBody {
    /// a `.rut` body — compile it. `is_decl` marks a declaration-mode
    /// module (a `.d.rut` surface parsed as its own unit):
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
        /// Builtin-impl methods (`core` only): the
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

/// One mounted module: the bare package name it answers to, its entry
/// files, and its body ([`ModuleBody`]).
#[derive(Clone, Debug, Default)]
pub struct Module {
    /// the exact package name — bare `[a-zA-Z0-9_]+`
    pub spec: String,
    /// The namespace head for qualified member access (`Math.sqrt`) —
    /// `None` when the module has no namespace form.
    pub namespace: Option<String>,
    pub entry: Entry,
    /// the body: `.rut` source, a decoded `.rutc`, or the native rows
    pub body: ModuleBody,
    /// the generic-bearing source a compiled bundle unit rides beside
    /// its binary ([`GenSource`]) — the on-demand recompile's input
    pub gen_source: Option<GenSource>,
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

