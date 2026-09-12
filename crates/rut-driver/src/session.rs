//! Module mounts & resolution — the compile-time half of RFC 0035 §1,
//! shaped by RFC 0029 §5 (source resolution) and RFC 0003 (§2 packages
//! and visibility).
//!
//! A **package** is a `rut.toml` naming a scope and the modules it exposes:
//!
//! ```toml
//! # std/rut.toml
//! name = "std"
//! [entries]
//! "std:core" = "core.d.rut"
//! ```
//!
//! Every entry key must be `"<name>:<local>"` — a package may only answer
//! for its own scope. A **consumer** mounts packages with `[deps]`:
//!
//! ```toml
//! [deps]
//! std = { path = "<sysroot>/std" }
//! ```
//!
//! Resolution is exact and two-step, never derived: `"std:vec"` → the
//! `std` package → its `"std:vec"` entry. Because the dep key *is* the
//! scope, a missing package is diagnosable by name ("lacking pkg `std`").
//!
//! The manifest reader below intentionally parses only the TOML subset the
//! format uses (comments, `key = "string"`, `[section]`, inline tables) so
//! the driver stays dependency-free and wasm-compatible. The full
//! `rut.toml` contract lives in RFC 0038 §2.

use std::collections::BTreeMap;

/// One module as the resolver sees it. A file-backed module carries a
/// `path` (and optionally a `decl_path`) relative to its package root; an
/// embedded module carries its `source`/`decl` text directly (wasm hosts,
/// tests, plugins).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Module {
    /// `.rut` implementation file, relative to the package root
    pub path: Option<String>,
    /// `.d.rut` surface file, relative to the package root
    pub decl_path: Option<String>,
    /// in-memory `.rut` text (no filesystem)
    pub source: Option<String>,
    /// in-memory `.d.rut` text (no filesystem)
    pub decl: Option<String>,
}

/// A parsed `rut.toml`: a scope name and the modules it exposes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub entries: BTreeMap<String, Module>,
    /// `[deps]` — package scope → its descriptor (`path = "..."`). Kept so
    /// a consumer manifest round-trips; loading dep packages is the host's
    /// job (RFC 0035 §1).
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
    /// no package mounted for `<scope>`
    NoPackage { spec: String, scope: String },
    /// package exists but exposes no such entry
    NoEntry { spec: String, scope: String, available: Vec<String> },
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::BadSpec { spec } => write!(
                f,
                "malformed module specifier `{spec}` — expected `<scope>:<name>`"
            ),
            ResolveError::NoPackage { spec, scope } => write!(
                f,
                "cannot resolve `{spec}` — no package `{scope}` in deps"
            ),
            ResolveError::NoEntry { spec, scope, available } => {
                write!(f, "package `{scope}` has no entry `{spec}`")?;
                if available.is_empty() {
                    write!(f, " (it exposes nothing)")
                } else {
                    write!(f, " — available: {}", available.join(", "))
                }
            }
        }
    }
}
impl std::error::Error for ResolveError {}

/// The business-owned mount table. The host decides what exists; the
/// resolver only maps exact specifiers. Mirrors the runtime
/// `vm.register_module` (RFC 0035 §3).
#[derive(Clone, Debug, Default)]
pub struct Session {
    packages: BTreeMap<String, Package>,
}

impl Session {
    pub fn new() -> Session {
        Session::default()
    }

    /// Register a whole package. Fails if any entry breaks scope
    /// ownership (`"<name>:<local>"`).
    pub fn register_package(&mut self, pkg: Package) -> Result<(), ManifestError> {
        for spec in pkg.entries.keys() {
            let want = format!("{}:", pkg.name);
            if !spec.starts_with(&want) {
                return Err(ManifestError(format!(
                    "package `{}` may only declare entries in its own scope — `{spec}` must start with `{want}`",
                    pkg.name
                )));
            }
        }
        self.packages.insert(pkg.name.clone(), pkg);
        Ok(())
    }

    /// Programmatic single-module mount (wasm/tests/plugins) — no manifest
    /// file needed.
    pub fn register_module(&mut self, spec: &str, module: Module) -> Result<(), ManifestError> {
        let Some((scope, _)) = spec.split_once(':') else {
            return Err(ManifestError(format!(
                "malformed module specifier `{spec}` — expected `<scope>:<name>`"
            )));
        };
        let pkg = self.packages.entry(scope.to_string()).or_insert_with(|| Package {
            name: scope.to_string(),
            entries: BTreeMap::new(),
            deps: BTreeMap::new(),
        });
        pkg.entries.insert(spec.to_string(), module);
        Ok(())
    }

    /// Exact two-step resolution: specifier → package → entry.
    pub fn resolve(&self, spec: &str) -> Result<&Module, ResolveError> {
        let Some((scope, _)) = spec.split_once(':') else {
            return Err(ResolveError::BadSpec { spec: spec.to_string() });
        };
        let Some(pkg) = self.packages.get(scope) else {
            return Err(ResolveError::NoPackage { spec: spec.to_string(), scope: scope.to_string() });
        };
        match pkg.entries.get(spec) {
            Some(m) => Ok(m),
            None => Err(ResolveError::NoEntry {
                spec: spec.to_string(),
                scope: scope.to_string(),
                available: pkg.entries.keys().cloned().collect(),
            }),
        }
    }

    /// Parse a `rut.toml` and mount it.
    pub fn load_manifest(&mut self, text: &str) -> Result<(), ManifestError> {
        self.register_package(parse_manifest(text)?)
    }

    pub fn packages(&self) -> impl Iterator<Item = (&String, &Package)> {
        self.packages.iter()
    }
}

/// Parse the `rut.toml` subset: top-level `name`, an `[entries]` table
/// whose values are strings or `{ path = "...", decl = "..." }` inline
/// tables, and an optional `[deps]` table (parsed, not resolved).
pub fn parse_manifest(text: &str) -> Result<Package, ManifestError> {
    let mut name: Option<String> = None;
    let mut entries: BTreeMap<String, Module> = BTreeMap::new();
    let mut deps: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut section = Section::Top;

    for (lineno, raw) in text.lines().enumerate() {
        let line = strip_comment(raw).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if let Some(inner) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = match inner.trim() {
                "entries" => Section::Entries,
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
            Section::Top => {
                if key == "name" {
                    name = Some(parse_string(value, lineno)?);
                }
                // other top-level keys (format, version) are tolerated
            }
            Section::Entries => {
                entries.insert(key, parse_module(value, lineno)?);
            }
            Section::Deps => {
                let table = parse_inline_table(value, lineno)?;
                deps.insert(key, table);
            }
        }
    }

    let Some(name) = name else {
        return Err(ManifestError("manifest has no `name`".to_string()));
    };
    Ok(Package { name, entries, deps })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Top,
    Entries,
    Deps,
}

fn parse_module(value: &str, lineno: usize) -> Result<Module, ManifestError> {
    if value.starts_with('{') {
        let table = parse_inline_table(value, lineno)?;
        let mut m = Module::default();
        for (k, v) in table {
            match k.as_str() {
                "path" => m.path = Some(v),
                "decl" => m.decl_path = Some(v),
                other => {
                    return Err(ManifestError(format!(
                        "line {}: unknown entry key `{other}` (expected `path` or `decl`)",
                        lineno + 1
                    )))
                }
            }
        }
        if m.path.is_none() && m.decl_path.is_none() {
            return Err(ManifestError(format!("line {}: entry table needs `path`", lineno + 1)));
        }
        Ok(m)
    } else {
        Ok(Module { path: Some(parse_string(value, lineno)?), ..Default::default() })
    }
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
        let k = unquote(k.trim());
        let val = parse_string(val.trim(), lineno)?;
        out.insert(k.to_string(), val);
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

    const STD: &str = r#"
# std/rut.toml
name = "std"
[entries]
"std:core" = "core.d.rut"
"std:math" = "math.d.rut"
"std:vec"  = { path = "vec.rut", decl = "vec.d.rut" }
"#;

    #[test]
    fn parses_entries_and_resolves() {
        let mut s = Session::new();
        s.load_manifest(STD).expect("manifest");
        let m = s.resolve("std:vec").expect("std:vec");
        assert_eq!(m.path.as_deref(), Some("vec.rut"));
        assert_eq!(m.decl_path.as_deref(), Some("vec.d.rut"));
        assert_eq!(s.resolve("std:core").unwrap().path.as_deref(), Some("core.d.rut"));
    }

    #[test]
    fn missing_package_names_the_scope() {
        let mut s = Session::new();
        s.load_manifest(STD).unwrap();
        let err = s.resolve("imaging:gfx").unwrap_err();
        assert_eq!(
            err,
            ResolveError::NoPackage { spec: "imaging:gfx".into(), scope: "imaging".into() }
        );
        assert!(err.to_string().contains("no package `imaging`"));
    }

    #[test]
    fn missing_entry_lists_available() {
        let mut s = Session::new();
        s.load_manifest(STD).unwrap();
        let err = s.resolve("std:log").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("package `std` has no entry `std:log`"), "{msg}");
        assert!(msg.contains("std:core"), "{msg}");
    }

    #[test]
    fn malformed_specifier() {
        let s = Session::new();
        assert!(matches!(s.resolve("just-a-name"), Err(ResolveError::BadSpec { .. })));
    }

    #[test]
    fn scope_ownership_is_enforced() {
        let bad = "name = \"std\"\n[entries]\n\"other:thing\" = \"x.rut\"\n";
        let mut s = Session::new();
        let err = s.load_manifest(bad).unwrap_err();
        assert!(err.to_string().contains("its own scope"), "{err}");
    }

    #[test]
    fn programmatic_mount() {
        let mut s = Session::new();
        s.register_module(
            "plugin:my_map",
            Module { source: Some("...".into()), ..Default::default() },
        )
        .unwrap();
        assert_eq!(s.resolve("plugin:my_map").unwrap().source.as_deref(), Some("..."));
    }

    #[test]
    fn deps_parse_but_do_not_resolve() {
        let text = "name = \"app\"\n[deps]\nstd = { path = \"../std\" }\n";
        let pkg = parse_manifest(text).unwrap();
        assert_eq!(pkg.deps.get("std").unwrap().get("path").unwrap(), "../std");
        assert!(pkg.entries.is_empty());
    }
}
