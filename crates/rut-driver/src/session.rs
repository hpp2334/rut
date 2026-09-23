//! Module mounts & resolution — the compile-time half of RFC 0035 §1,
//! shaped by RFC 0029 §5 and RFC 0003 (§2).
//!
//! **One directory is one module.** Its `rut.toml` names the exact
//! package it answers to and how to reach its surface and body:
//!
//! ```toml
//! # rut/pouch/rut.toml
//! name = "pouch"
//! entry.type = "./pouch.d.rut"        # the surface (RFC 0029)
//! entry.lib  = "./pouch.rut"          # the body (omitted while surface-only)
//! ```
//!
//! A consumer mounts modules by exact name → directory:
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
//! The reader below parses only the TOML subset the format uses
//! (comments, `key = "string"`, dotted keys, `[section]`, inline tables)
//! so the driver stays dependency-free and wasm-compatible.

use std::collections::BTreeMap;

/// A module's entry points: where its surface and body live, relative to
/// the module directory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    /// `.d.rut` surface path — the declaration/type half (RFC 0029)
    pub type_path: Option<String>,
    /// body path — `.rut` source, or a compiled `.rutc`/`.d.ir`
    pub lib: Option<String>,
    /// compiled IR path (`.d.ir`)
    pub ir: Option<String>,
}

/// One mounted module: the bare package name it answers to, its entry
/// files, and (for embedded hosts) in-memory source/surface text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Module {
    /// the exact package name — bare `[a-zA-Z0-9_]+`
    pub spec: String,
    /// The namespace head for qualified member access (`Math.sqrt`) —
    /// `None` when the module has no namespace form (RFC 0028).
    pub namespace: Option<String>,
    pub entry: Entry,
    /// in-memory `.rut` body (wasm hosts, tests, plugins)
    pub source: Option<String>,
    /// in-memory `.d.rut` surface
    pub decl: Option<String>,
    /// the body itself is a `.d.rut` surface (RFC 0029): parse in
    /// declaration mode — nothing to compile or run, but `rut dump`
    /// shows the AST
    pub is_decl: bool,
    /// in-memory HOST surface (RFC 0022/0026): bodyless functions, bound by
    /// the embedder at run time — `(name, params, ret)`. A module with these
    /// and no `source` is a native module.
    pub host_funcs: Vec<(String, Vec<rut_core::types::TypeId>, rut_core::types::TypeId)>,
    /// The host-fn registration scope — the `FuncCode` host-id prefix — when
    /// it must differ from the package name. `rt` stays the logger host
    /// module's use path while its internal registration naming remains
    /// `rt:log` (`rt:log::create_logger`), untouched since RFC 0022.
    pub host_scope: Option<String>,
    /// Builtin-impl methods (`core` only, RFC 0032 §1.1 R2): the integer
    /// primitives' numeric methods — `(receiver prim, name, lowering id)`.
    /// Bodyless and hostless — `rut-lir` expands the method call
    /// (`x.wrapping_add(y)`) inline, ambient on the primitive.
    pub native_impls: Vec<(rut_core::types::TypeId, String, rut_core::ops::Intrinsic)>,
    /// Exported constants: `(name, type, raw bits)` — `calc::PI`.
    pub consts: Vec<(String, rut_core::types::TypeId, u64)>,
    /// Builtin containers published by name (`core` only): the type is
    /// the compiler's own; the NAME is use-gated (RFC 0028)
    pub native_types: Vec<(String, rut_core::binary::NativeTy)>,
    /// Builtin traits published by name (`core` only)
    pub native_traits: Vec<(String, rut_core::binary::NativeTrait)>,
    /// Compiler-lowered builtin function names (`core` only) — no
    /// bodies; rut-lir lowers them, reached only through the use
    pub native_fns: Vec<String>,
    /// Force source-inlining into every consumer (`ink`): a module whose
    /// class methods must resolve at the call site cannot be linked.
    pub inline: bool,
}

/// A parsed `rut.toml` — either a module manifest (`name` + `entry.*`) or
/// a consumer manifest (`[deps]`), or both.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    pub name: Option<String>,
    pub entry: Entry,
    /// `format = "rutbundle"` — bundle-shaped manifests (RFC 0038 §2);
    /// directory loading ignores it
    pub format: Option<String>,
    /// `format_version` — the bundle LAYOUT version, refused on load when
    /// unknown (RFC 0038 §4)
    pub format_version: Option<u64>,
    /// `[deps]` — exact specifier → descriptor (`path = "..."`). Kept for
    /// the host to resolve; the Session does not read the filesystem.
    pub deps: BTreeMap<String, BTreeMap<String, String>>,
    /// `[peer-deps]` (RFC 0045 §2) — REQUIRED by default; the CONSUMER
    /// supplies the peer, it is never pulled transitively. `optional =
    /// true` marks the presence-mounted kind whose integration group is
    /// the descriptor's `lib` file. Descriptors: `path` (string),
    /// `optional` (bool), `lib` (string) — nothing else.
    pub peer_deps: BTreeMap<String, BTreeMap<String, String>>,
    /// `[dev-deps]` (RFC 0045 §2) — mounted ONLY when building/testing
    /// the pkg itself (the program root), never in a consumer's world.
    /// Same descriptor shape as `[deps]`.
    pub dev_deps: BTreeMap<String, BTreeMap<String, String>>,
    /// `host_scope` — the host-fn registration prefix when it must differ
    /// from the package name (`rt` keeps its historical `rt:log` scope,
    /// RFC 0022)
    pub host_scope: Option<String>,
    /// `inline = true` — force source-inlining into every consumer
    /// (`ink`: a module whose class methods must resolve at the call
    /// site cannot be linked)
    pub inline: bool,
}

/// A malformed manifest — a load error, never a runtime trap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestError(pub String);

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for ManifestError {}

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
    pub fn expected_host_fns(
        &self,
    ) -> std::collections::BTreeMap<String, (Vec<rut_core::types::TypeId>, rut_core::types::TypeId)> {
        let mut out = std::collections::BTreeMap::new();
        for (spec, m) in &self.modules {
            let scope = m.host_scope.as_deref().unwrap_or(spec);
            for (name, params, ret) in &m.host_funcs {
                out.insert(format!("{scope}::{name}"), (params.clone(), *ret));
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

    /// Append peer-group source to a mounted module's body (RFC 0045
    /// §3, presence-based group assembly): the combined text stays ONE
    /// source string, so every existing consumer of `Module.source` —
    /// the graph splice, bundles, the wasm mounts — is untouched.
    pub fn append_source(&mut self, spec: &str, text: &str) -> Result<(), ManifestError> {
        let Some(m) = self.modules.get_mut(spec) else {
            return Err(ManifestError(format!(
                "cannot append a peer group to `{spec}` — no such module is mounted"
            )));
        };
        match &mut m.source {
            Some(src) => {
                src.push('\n');
                src.push_str(text);
                Ok(())
            }
            None => Err(ManifestError(format!(
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

/// A bare package name: `[a-zA-Z0-9_]+`, non-empty. The same charset
/// governs manifest `name` values and `[deps]` keys.
fn valid_spec(spec: &str) -> bool {
    !spec.is_empty() && spec.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Parse the `rut.toml` subset: top-level `name`, `entry.type` /
/// `entry.lib` / `entry.ir` (bare or under `[entry]`), and the dep
/// tables `[deps]` / `[peer-deps]` / `[dev-deps]` (RFC 0045) whose
/// values are inline tables.
pub fn parse_manifest(text: &str) -> Result<Manifest, ManifestError> {
    let mut m = Manifest::default();
    let mut section = Section::Top;

    for (lineno, raw) in text.lines().enumerate() {
        let line = strip_comment(raw).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if let Some(inner) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = match inner.trim() {
                "entry" => Section::Entry,
                "deps" => Section::Deps,
                "peer-deps" => Section::PeerDeps,
                "dev-deps" => Section::DevDeps,
                other => {
                    return Err(ManifestError(format!(
                        "line {}: unknown section `[{}]`",
                        lineno + 1,
                        other.trim()
                    )))
                }
            };
            continue;
        }
        let Some((key, value)) = split_eq(&line) else {
            return Err(ManifestError(format!("line {}: expected `key = value`", lineno + 1)));
        };
        let key = unquote(key.trim()).to_string();
        let value = value.trim();
        match section {
            Section::Top => match key.as_str() {
                "name" => {
                    let name = parse_string(value, lineno)?;
                    if !valid_spec(&name) {
                        return Err(ManifestError(format!(
                            "line {}: module name `{name}` is not a bare package name — expected `[a-zA-Z0-9_]+`",
                            lineno + 1
                        )));
                    }
                    m.name = Some(name);
                }
                // bundle-shaped manifests (RFC 0038 §2)
                "format" => m.format = Some(parse_string(value, lineno)?),
                "format_version" => m.format_version = Some(parse_u64(value, lineno)?),
                // the host-fn registration prefix override (`rt` → `rt:log`)
                "host_scope" => m.host_scope = Some(parse_string(value, lineno)?),
                // force source-inlining into every consumer (`ink`)
                "inline" => m.inline = parse_bool(value, lineno)?,
                // dotted entry keys: `entry.type = "..."` etc.
                "entry.type" => m.entry.type_path = Some(parse_string(value, lineno)?),
                "entry.lib" => m.entry.lib = Some(parse_string(value, lineno)?),
                "entry.ir" => m.entry.ir = Some(parse_string(value, lineno)?),
                _ => {} // forward-compatible: ignore unknown top-level keys
            },
            Section::Entry => match key.as_str() {
                "type" => m.entry.type_path = Some(parse_string(value, lineno)?),
                "lib" => m.entry.lib = Some(parse_string(value, lineno)?),
                "ir" => m.entry.ir = Some(parse_string(value, lineno)?),
                other => {
                    return Err(ManifestError(format!(
                        "line {}: unknown `[entry]` key `{other}`",
                        lineno + 1
                    )))
                }
            },
            Section::Deps => {
                check_dep_key(&key, lineno)?;
                let desc = parse_deps_descriptor(value, lineno)?;
                m.deps.insert(key, desc);
            }
            Section::PeerDeps => {
                check_dep_key(&key, lineno)?;
                let desc = parse_peer_descriptor(value, lineno, "peer-deps")?;
                m.peer_deps.insert(key, desc);
            }
            Section::DevDeps => {
                check_dep_key(&key, lineno)?;
                let desc = parse_peer_descriptor(value, lineno, "dev-deps")?;
                m.dev_deps.insert(key, desc);
            }
        }
    }
    // D4 (RFC 0045 §2): `[peer-deps]` + `[dev-deps]` is the sanctioned
    // both-kinds pairing (the ruling); anything riding `[deps]` beside
    // either is a manifest error naming both rows — a pkg is either
    // pulled transitively or required of the consumer / held for
    // development, never both.
    for name in m.deps.keys() {
        if m.peer_deps.contains_key(name) {
            return Err(ManifestError(format!(
                "`{name}` appears in both `[deps]` and `[peer-deps]` — a package is either pulled transitively or required of the consumer, never both (RFC 0045 §2)"
            )));
        }
        if m.dev_deps.contains_key(name) {
            return Err(ManifestError(format!(
                "`{name}` appears in both `[deps]` and `[dev-deps]` — a package is either pulled transitively or held for development, never both (RFC 0045 §2)"
            )));
        }
    }
    Ok(m)
}

/// A dep-table key is a bare package name — the same charset law as the
/// manifest `name`.
fn check_dep_key(key: &str, lineno: usize) -> Result<(), ManifestError> {
    if !valid_spec(key) {
        return Err(ManifestError(format!(
            "line {}: dep `{key}` is not a bare package name — expected `[a-zA-Z0-9_]+`",
            lineno + 1
        )));
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Top,
    Entry,
    Deps,
    PeerDeps,
    DevDeps,
}

fn parse_string(value: &str, lineno: usize) -> Result<String, ManifestError> {
    let v = value.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        Ok(v[1..v.len() - 1].to_string())
    } else {
        Err(ManifestError(format!("line {}: expected a quoted string, found `{v}`", lineno + 1)))
    }
}

fn parse_u64(value: &str, lineno: usize) -> Result<u64, ManifestError> {
    let v = value.trim();
    v.parse::<u64>()
        .map_err(|_| ManifestError(format!("line {}: expected an integer, found `{v}`", lineno + 1)))
}

/// Parse `true` / `false`.
fn parse_bool(value: &str, lineno: usize) -> Result<bool, ManifestError> {
    let v = value.trim();
    match v {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ManifestError(format!(
            "line {}: expected `true` or `false`, found `{v}`",
            lineno + 1
        ))),
    }
}

/// Split `{ k = v, k2 = v2 }` (single-line) into raw key/value pairs —
/// values are NOT parsed here; each table's rules decide what a value
/// may be (strings everywhere; `optional` is the one bool the grammar
/// learns, RFC 0045 §2).
fn inline_table_parts(value: &str, lineno: usize) -> Result<Vec<(String, String)>, ManifestError> {
    let v = value.trim();
    let Some(inner) = v.strip_prefix('{').and_then(|s| s.strip_suffix('}')) else {
        return Err(ManifestError(format!(
            "line {}: expected an inline table `{{ key = \"value\" }}`, found `{v}`",
            lineno + 1
        )));
    };
    let mut out = Vec::new();
    for part in split_commas(inner) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((k, val)) = split_eq(part) else {
            return Err(ManifestError(format!("line {}: bad table item `{part}`", lineno + 1)));
        };
        out.push((unquote(k.trim()).to_string(), val.trim().to_string()));
    }
    Ok(out)
}

/// A `[deps]` descriptor: string-only values, and `optional` is
/// rejected — it is a `[peer-deps]` attribute (RFC 0045 §2).
fn parse_deps_descriptor(
    value: &str,
    lineno: usize,
) -> Result<BTreeMap<String, String>, ManifestError> {
    let mut out = BTreeMap::new();
    for (k, val) in inline_table_parts(value, lineno)? {
        if k == "optional" {
            return Err(ManifestError(format!(
                "line {}: `optional` is a `[peer-deps]` attribute — `[deps]` has no options",
                lineno + 1
            )));
        }
        out.insert(k, parse_string(&val, lineno)?);
    }
    Ok(out)
}

/// A `[peer-deps]`/`[dev-deps]` descriptor (RFC 0045 §2): `path` and
/// `lib` are strings (`lib` is the peer-gated integration file — an
/// impl-only `.rut` source; a `.d.rut` decl surface does not gate),
/// `optional` is the one bool, and any other key is the `[entry]`
/// strictness — a line-targeted error.
fn parse_peer_descriptor(
    value: &str,
    lineno: usize,
    table: &str,
) -> Result<BTreeMap<String, String>, ManifestError> {
    let mut out = BTreeMap::new();
    for (k, val) in inline_table_parts(value, lineno)? {
        match k.as_str() {
            "path" => {
                out.insert(k, parse_string(&val, lineno)?);
            }
            "lib" => {
                let v = parse_string(&val, lineno)?;
                if v.ends_with(".d.rut") {
                    return Err(ManifestError(format!(
                        "line {}: `lib` must be a `.rut` source — a `.d.rut` decl surface does not gate (RFC 0045 §3)",
                        lineno + 1
                    )));
                }
                out.insert(k, v);
            }
            "optional" => {
                out.insert(k, parse_optional_flag(&val, lineno)?);
            }
            other => {
                return Err(ManifestError(format!(
                    "line {}: unknown `[{table}]` key `{other}`",
                    lineno + 1
                )));
            }
        }
    }
    Ok(out)
}

/// `optional` is the one bool the inline-table grammar learns (RFC 0045
/// §2); peers are REQUIRED by default, so the flag must say `true` or
/// `false` exactly. Stored as its source spelling.
fn parse_optional_flag(value: &str, lineno: usize) -> Result<String, ManifestError> {
    match value.trim() {
        "true" => Ok("true".to_string()),
        "false" => Ok("false".to_string()),
        _ => Err(ManifestError(format!(
            "line {}: `optional` expects `true` or `false`",
            lineno + 1
        ))),
    }
}

/// Split at the first `=` outside a quoted string.
fn split_eq(s: &str) -> Option<(&str, &str)> {
    let mut in_str = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '=' if !in_str => return Some((&s[..i], &s[i + 1..])),
            _ => {}
        }
    }
    None
}

/// Split at `,` outside quotes.
fn split_commas(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut in_str = false;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_str = !in_str,
            ',' if !in_str => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// Strip a `# ...` comment, respecting quoted strings.
fn strip_comment(line: &str) -> &str {
    let mut in_str = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '#' if !in_str => return &line[..i],
            _ => {}
        }
    }
    line
}

fn unquote(s: &str) -> &str {
    let b = s.as_bytes();
    if b.len() >= 2 && b[0] == b'"' && b[b.len() - 1] == b'"' {
        &s[1..s.len() - 1]
    } else {
        s
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
    fn parses_name_and_entry() {
        let m = parse_manifest(POUCH).unwrap();
        assert_eq!(m.name.as_deref(), Some("pouch"));
        assert_eq!(m.entry.type_path.as_deref(), Some("./pouch.d.rut"));
    }

    #[test]
    fn mount_and_resolve() {
        let mut s = Session::new();
        s.load_manifest(POUCH).unwrap();
        let m = s.resolve("pouch").unwrap();
        assert_eq!(m.entry.type_path.as_deref(), Some("./pouch.d.rut"));
    }

    #[test]
    fn entry_section_form() {
        let text = "name = \"vec\"\n[entry]\ntype = \"./vec.d.rut\"\nlib = \"./vec.rut\"\n";
        let m = parse_manifest(text).unwrap();
        assert_eq!(m.entry.lib.as_deref(), Some("./vec.rut"));
    }

    #[test]
    fn bundle_manifest_keys() {
        let text = "format = \"rutbundle\"\nformat_version = 1\nname = \"x\"\nentry.lib = \"./x.rut\"\n";
        let m = parse_manifest(text).unwrap();
        assert_eq!(m.format.as_deref(), Some("rutbundle"));
        assert_eq!(m.format_version, Some(1));
    }

    #[test]
    fn missing_module_names_the_manifest() {
        let s = Session::new();
        let err = s.resolve("missing").unwrap_err();
        assert_eq!(err, ResolveError::NoModule { spec: "missing".into() });
        assert!(err.to_string().contains("`rut.toml` `[deps]`"), "{}", err);
    }

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
    fn manifest_names_must_be_bare_package_names() {
        // scoped manifest names are retired — the diagnostic is
        // line-targeted and names the charset
        let err = parse_manifest("name = \"std:core\"\n").unwrap_err();
        assert!(err.to_string().contains("line 1"), "{err}");
        assert!(err.to_string().contains("bare package name"), "{err}");
        let err = parse_manifest("[deps]\n\"std:math\" = { path = \"rut/calc\" }\n").unwrap_err();
        assert!(err.to_string().contains("line 2"), "{err}");
    }

    #[test]
    fn programmatic_mount_sets_spec() {
        let mut s = Session::new();
        s.register_module(
            "my_map",
            Module { source: Some("...".into()), ..Default::default() },
        )
        .unwrap();
        assert_eq!(s.resolve("my_map").unwrap().source.as_deref(), Some("..."));
    }

    #[test]
    fn deps_parse() {
        let text = "name = \"app\"\n[deps]\n\"core\" = { path = \"rut/core\" }\n";
        let m = parse_manifest(text).unwrap();
        assert_eq!(m.deps.get("core").unwrap().get("path").unwrap(), "rut/core");
    }

    #[test]
    fn host_scope_and_inline_parse() {
        // the host pkg's manifest keys (RFC 0022 / the host-pkgs plan):
        // `host_scope` overrides the registration prefix, `inline`
        // forces source-inlining into every consumer
        let m = parse_manifest(
            "name = \"rt\"\nentry.type = \"./rt.d.rut\"\nhost_scope = \"rt:log\"\n",
        )
        .unwrap();
        assert_eq!(m.host_scope.as_deref(), Some("rt:log"));
        assert!(!m.inline);
        let m = parse_manifest("name = \"ink\"\nentry.lib = \"./ink.rut\"\ninline = true\n")
            .unwrap();
        assert!(m.inline);
        assert_eq!(m.host_scope, None);
        // a non-bool `inline` is a load error
        assert!(parse_manifest("inline = yes\n").is_err());
    }

    // ---- the dep kinds (RFC 0045): the three tables, `optional`, the
    // `lib` group key, D4 — the T11 manifest-error shapes ----

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
    fn peer_and_dev_tables_parse() {
        let m = parse_manifest(JSON).unwrap();
        let pouch = m.peer_deps.get("pouch").unwrap();
        assert_eq!(pouch.get("path").unwrap(), "../pouch");
        assert_eq!(pouch.get("optional").unwrap(), "true");
        assert_eq!(pouch.get("lib").unwrap(), "./serde_pouch.rut");
        // `optional` defaults to false — REQUIRED by default
        let req = parse_manifest("name = \"j\"\n[peer-deps]\nnmapset = { path = \"../nmapset\" }\n")
            .unwrap();
        assert_eq!(req.peer_deps.get("nmapset").unwrap().get("optional"), None);
        // the sanctioned both-kinds pairing parses (pouch in peer + dev)
        assert!(m.dev_deps.contains_key("pouch"));
        assert_eq!(m.dev_deps.get("pouch").unwrap().get("path").unwrap(), "../pouch");
    }

    #[test]
    fn t11_optional_rejected_inside_deps() {
        let err = parse_manifest("[deps]\npouch = { path = \"../pouch\", optional = true }\n")
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "line 2: `optional` is a `[peer-deps]` attribute — `[deps]` has no options"
        );
    }

    #[test]
    fn t11_optional_must_be_a_bool() {
        let err =
            parse_manifest("[peer-deps]\npouch = { path = \"..\", optional = \"yes\" }\n")
                .unwrap_err();
        assert_eq!(err.to_string(), "line 2: `optional` expects `true` or `false`");
    }

    #[test]
    fn t11_unknown_descriptor_key_is_line_targeted() {
        let err =
            parse_manifest("[peer-deps]\npouch = { path = \"..\", feats = \"x\" }\n").unwrap_err();
        assert_eq!(err.to_string(), "line 2: unknown `[peer-deps]` key `feats`");
        let err = parse_manifest("[dev-deps]\npouch = { path = \"..\", git = \"x\" }\n").unwrap_err();
        assert_eq!(err.to_string(), "line 2: unknown `[dev-deps]` key `git`");
    }

    #[test]
    fn t11_lib_must_be_a_rut_source() {
        // a `.d.rut` lib key is a load error: decl surfaces don't gate
        let err = parse_manifest(
            "[peer-deps]\npouch = { path = \"..\", optional = true, lib = \"./pouch.d.rut\" }\n",
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "line 2: `lib` must be a `.rut` source — a `.d.rut` decl surface does not gate (RFC 0045 §3)"
        );
    }

    #[test]
    fn t11_d4_deps_beside_peer_or_dev_is_the_collision() {
        let err = parse_manifest(
            "name = \"j\"\n[deps]\npouch = { path = \"../pouch\" }\n[peer-deps]\npouch = { path = \"../pouch\", optional = true }\n",
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "`pouch` appears in both `[deps]` and `[peer-deps]` — a package is either pulled transitively or required of the consumer, never both (RFC 0045 §2)"
        );
        let err = parse_manifest(
            "name = \"j\"\n[deps]\npouch = { path = \"../pouch\" }\n[dev-deps]\npouch = { path = \"../pouch\" }\n",
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "`pouch` appears in both `[deps]` and `[dev-deps]` — a package is either pulled transitively or held for development, never both (RFC 0045 §2)"
        );
        // peer + dev together stays legal — the sanctioned pairing
        assert!(parse_manifest(JSON).is_ok());
    }

    #[test]
    fn t11_peer_dep_keys_are_bare_names() {
        let err = parse_manifest("[peer-deps]\n\"std:pouch\" = { path = \"..\" }\n").unwrap_err();
        assert!(err.to_string().contains("line 2"), "{err}");
        assert!(err.to_string().contains("bare package name"), "{err}");
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
        s.register_module("m", Module { source: Some("fn a() {}".into()), ..Default::default() })
            .unwrap();
        s.append_source("m", "fn b() {}").unwrap();
        assert_eq!(s.resolve("m").unwrap().source.as_deref(), Some("fn a() {}\nfn b() {}"));
        s.register_module("d", Module { ..Default::default() }).unwrap();
        assert!(s.append_source("d", "fn c() {}").is_err());
    }
}
