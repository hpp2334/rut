//! Module mounts & resolution — the compile-time half of RFC 0035 §1,
//! shaped by RFC 0029 §5 (source resolution) and RFC 0003 (§2).
//!
//! **One directory is one module.** Its `rut.toml` names the exact
//! specifier it answers to and how to reach its surface and body:
//!
//! ```toml
//! # rut/std-collection/rut.toml
//! name = "std:collection"
//! entry.type = "./collection.d.rut"   # the surface (RFC 0029)
//! entry.lib  = "./collection.rut"     # the body (omitted while surface-only)
//! ```
//!
//! A consumer mounts modules by exact specifier → directory:
//!
//! ```toml
//! [deps]
//! "std:collection" = { path = "rut/std-collection" }
//! ```
//!
//! Resolution is exact and single-step: a specifier resolves only if a
//! module with that `name` is mounted — nothing is derived. Since the
//! specifier is `<scope>:<name>`, a miss is diagnosable by scope
//! ("package `std` is missing").
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

/// One mounted module: the specifier it answers to, its entry files, and
/// (for embedded hosts) in-memory source/surface text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Module {
    /// the exact specifier, `"<scope>:<name>"`
    pub spec: String,
    pub entry: Entry,
    /// in-memory `.rut` body (wasm hosts, tests, plugins)
    pub source: Option<String>,
    /// in-memory `.d.rut` surface
    pub decl: Option<String>,
    /// in-memory HOST surface (RFC 0022/0026): bodyless functions, bound by
    /// the embedder at run time — `(name, params, ret)`. A module with these
    /// and no `source` is a native module.
    pub host_funcs: Vec<(String, Vec<rut_core::types::TypeId>, rut_core::types::TypeId)>,
}

/// A parsed `rut.toml` — either a module manifest (`name` + `entry.*`) or
/// a consumer manifest (`[deps]`), or both.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    pub name: Option<String>,
    pub entry: Entry,
    /// `[deps]` — exact specifier → descriptor (`path = "..."`). Kept for
    /// the host to resolve; the Session does not read the filesystem.
    pub deps: BTreeMap<String, BTreeMap<String, String>>,
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

/// Why a specifier did not resolve. Each variant names the scope so the
/// diagnostic can say which package is missing (RFC 0030 §2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveError {
    /// not `<scope>:<name>`
    BadSpec { spec: String },
    /// no module with that exact specifier is mounted
    NoModule { spec: String, scope: String },
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::BadSpec { spec } => write!(
                f,
                "malformed module specifier `{spec}` — expected `<scope>:<name>`"
            ),
            ResolveError::NoModule { spec, scope } => write!(
                f,
                "cannot resolve `{spec}` — package `{scope}` is not mounted"
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
}

impl Session {
    pub fn new() -> Session {
        Session::default()
    }

    /// Mount a module. Its `spec` must be `<scope>:<name>`.
    pub fn mount(&mut self, module: Module) -> Result<(), ManifestError> {
        if !valid_spec(&module.spec) {
            return Err(ManifestError(format!(
                "`{}` is not a module specifier — expected `<scope>:<name>`",
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

    /// Exact resolution: the specifier must be mounted as-is.
    pub fn resolve(&self, spec: &str) -> Result<&Module, ResolveError> {
        if !valid_spec(spec) {
            return Err(ResolveError::BadSpec { spec: spec.to_string() });
        }
        match self.modules.get(spec) {
            Some(m) => Ok(m),
            None => Err(ResolveError::NoModule {
                spec: spec.to_string(),
                scope: scope_of(spec).to_string(),
            }),
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
        }
        for (spec, dep) in &manifest.deps {
            if !valid_spec(spec) {
                return Err(ManifestError(format!(
                    "dep `{spec}` is not a module specifier — expected `<scope>:<name>`"
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

/// `<scope>:<name>` with a non-empty scope and name.
fn valid_spec(spec: &str) -> bool {
    match spec.split_once(':') {
        Some((scope, name)) => !scope.is_empty() && !name.is_empty(),
        None => false,
    }
}

fn scope_of(spec: &str) -> &str {
    spec.split_once(':').map(|(s, _)| s).unwrap_or(spec)
}

/// Parse the `rut.toml` subset: top-level `name`, `entry.type` /
/// `entry.lib` / `entry.ir` (bare or under `[entry]`), and `[deps]`
/// entries whose values are inline tables.
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
                "name" => m.name = Some(parse_string(value, lineno)?),
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
                m.deps.insert(key, parse_inline_table(value, lineno)?);
            }
        }
    }
    Ok(m)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Top,
    Entry,
    Deps,
}

fn parse_string(value: &str, lineno: usize) -> Result<String, ManifestError> {
    let v = value.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        Ok(v[1..v.len() - 1].to_string())
    } else {
        Err(ManifestError(format!("line {}: expected a quoted string, found `{v}`", lineno + 1)))
    }
}

/// Parse `{ k = "v", k2 = "v2" }` (single-line, string values only).
fn parse_inline_table(value: &str, lineno: usize) -> Result<BTreeMap<String, String>, ManifestError> {
    let v = value.trim();
    let Some(inner) = v.strip_prefix('{').and_then(|s| s.strip_suffix('}')) else {
        return Err(ManifestError(format!(
            "line {}: expected an inline table `{{ key = \"value\" }}`, found `{v}`",
            lineno + 1
        )));
    };
    let mut out = BTreeMap::new();
    for part in split_commas(inner) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((k, val)) = split_eq(part) else {
            return Err(ManifestError(format!("line {}: bad table item `{part}`", lineno + 1)));
        };
        out.insert(unquote(k.trim()).to_string(), parse_string(val.trim(), lineno)?);
    }
    Ok(out)
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

    const COLLECTION: &str = r#"
# rut/std-collection/rut.toml
name = "std:collection"
entry.type = "./collection.d.rut"
"#;

    #[test]
    fn parses_name_and_entry() {
        let m = parse_manifest(COLLECTION).unwrap();
        assert_eq!(m.name.as_deref(), Some("std:collection"));
        assert_eq!(m.entry.type_path.as_deref(), Some("./collection.d.rut"));
    }

    #[test]
    fn mount_and_resolve() {
        let mut s = Session::new();
        s.load_manifest(COLLECTION).unwrap();
        let m = s.resolve("std:collection").unwrap();
        assert_eq!(m.entry.type_path.as_deref(), Some("./collection.d.rut"));
    }

    #[test]
    fn entry_section_form() {
        let text = "name = \"std:vec\"\n[entry]\ntype = \"./vec.d.rut\"\nlib = \"./vec.rut\"\n";
        let m = parse_manifest(text).unwrap();
        assert_eq!(m.entry.lib.as_deref(), Some("./vec.rut"));
    }

    #[test]
    fn missing_module_names_the_scope() {
        let s = Session::new();
        let err = s.resolve("std:vec").unwrap_err();
        assert_eq!(
            err,
            ResolveError::NoModule { spec: "std:vec".into(), scope: "std".into() }
        );
        assert!(err.to_string().contains("package `std` is not mounted"));
    }

    #[test]
    fn malformed_specifier() {
        let s = Session::new();
        assert!(matches!(s.resolve("just-a-name"), Err(ResolveError::BadSpec { .. })));
    }

    #[test]
    fn programmatic_mount_sets_spec() {
        let mut s = Session::new();
        s.register_module(
            "plugin:my_map",
            Module { source: Some("...".into()), ..Default::default() },
        )
        .unwrap();
        assert_eq!(s.resolve("plugin:my_map").unwrap().source.as_deref(), Some("..."));
    }

    #[test]
    fn deps_parse() {
        let text = "name = \"app\"\n[deps]\n\"std:core\" = { path = \"rut/std-core\" }\n";
        let m = parse_manifest(text).unwrap();
        assert_eq!(
            m.deps.get("std:core").unwrap().get("path").unwrap(),
            "rut/std-core"
        );
    }
}
