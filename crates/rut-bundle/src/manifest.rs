//! The `rut.toml` grammar — a module manifest (`name`, `type`, +
//! `entry.*`) or a consumer manifest (`[deps]`, `[peer-deps]`,
//! `[dev-deps]`), or both, parsed into [`Manifest`].
//!
//! **One directory is one module.** Its `rut.toml` names the exact
//! package it answers to and how to reach its surface and body:
//!
//! ```toml
//! # rut/pouch/rut.toml
//! name = "pouch"
//! entry.type = "./pouch.d.rut"        # the surface
//! entry.lib  = "./pouch.rut"          # the body (omitted while surface-only)
//! ```
//!
//! The declared kind: `type = "lib"` (the default) is the ordinary
//! source package; `type = "host"` is a pure declaration surface the
//! embedding Rust binds at run time — a host pkg SPELLS itself, the
//! kind is never inferred.
//!
//! A consumer mounts modules by exact name → directory:
//!
//! ```toml
//! [deps]
//! "pouch" = { path = "rut/pouch" }
//! ```
//!
//! or by url — a packed `.rutbundle` fetched by the host and pinned by
//! its sha256:
//!
//! ```toml
//! [deps]
//! "pouch" = { url = "https://example.com/pouch.rutbundle", sha256 = "<64-hex>" }
//! ```
//!
//! Resolution is exact and single-step: a use path resolves only if a
//! module with that `name` is mounted — nothing is derived. Package
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
//! The reader below parses only the TOML subset the format uses
//! (comments, `key = "string"`, dotted keys, `[section]`, inline tables)
//! so the crate stays dependency-free and wasm-compatible.

use std::collections::BTreeMap;

/// A module's entry points: where its surface and body live, relative to
/// the module directory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    /// `.d.rut` surface path — the declaration/type half
    pub type_path: Option<String>,
    /// body path — a `.rut` source
    pub lib: Option<String>,
    /// additional `.rut` body files, spliced after `lib` in listed
    /// order — ONE module, one namespace (the multi-lib
    /// entry; contrast the presence-gated impl-only peer groups, which
    /// ride `[peer-deps]` instead)
    pub libs: Vec<String>,
}

/// The pkg's kind. `lib` is the ordinary rut package (a source body,
/// deps allowed, no host rows); `host` is a pure declaration surface
/// the embedding Rust binds at run time. Absent `type` ⇒ `lib` —
/// a host pkg SPELLS itself (refuse, never guess).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PkgType {
    #[default]
    Lib,
    Host,
}

/// A parsed `rut.toml` — either a module manifest (`name` + `entry.*`) or
/// a consumer manifest (`[deps]`), or both.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    pub name: Option<String>,
    /// `type = "lib" | "host"` — the declared kind; absent ⇒ [`PkgType::Lib`]
    pub pkg_type: PkgType,
    pub entry: Entry,
    /// `format = "rutbundle"` — bundle-shaped manifests;
    /// directory loading ignores it
    pub format: Option<String>,
    /// `format_version` — the bundle LAYOUT version, refused on load when
    /// unknown
    pub format_version: Option<u64>,
    /// `[deps]` — exact specifier → descriptor: exactly one source key,
    /// `path` (a directory) or `url` (a remote `.rutbundle`), plus
    /// `sha256` (the pin) only beside `url`. Kept for the host to
    /// resolve; the Session does not read the filesystem or the network.
    pub deps: BTreeMap<String, BTreeMap<String, String>>,
    /// `[peer-deps]` — REQUIRED by default; the CONSUMER
    /// supplies the peer, it is never pulled transitively. `optional =
    /// true` marks the presence-mounted kind whose integration group is
    /// the descriptor's `lib` file. Descriptors: `path` (string),
    /// `optional` (bool), `lib` (string) — nothing else.
    pub peer_deps: BTreeMap<String, BTreeMap<String, String>>,
    /// `[dev-deps]` — mounted ONLY when building/testing
    /// the pkg itself (the program root), never in a consumer's world.
    /// Same descriptor shape as `[deps]`.
    pub dev_deps: BTreeMap<String, BTreeMap<String, String>>,
    /// `[style]` — the pkg's formatter knobs (the rut-fmt batch): flat
    /// `key = value` rows read by the fmt crate's consumer, which
    /// validates the keys and parses the values (the manifest layer
    /// stays schema-free like the deps tables). Unknown keys are
    /// ignored (the forward-compat rule); unknown VALUES trip the
    /// formatter tool, never the loader.
    pub style: BTreeMap<String, String>,
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

/// A bare package name: `[a-zA-Z0-9_]+`, non-empty. The same charset
/// governs manifest `name` values and `[deps]` keys.
pub fn valid_spec(spec: &str) -> bool {
    !spec.is_empty() && spec.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Parse the `rut.toml` subset: top-level `name`, `type` (the declared
/// kind), `entry.type` / `entry.lib` / `entry.libs` (bare or under
/// `[entry]`), and the dep tables `[deps]` / `[peer-deps]` /
/// `[dev-deps]` whose values are inline tables.
pub fn parse_manifest(text: &str) -> Result<Manifest, ManifestError> {
    let mut m = Manifest::default();
    let mut section = Section::Top;
    // was `type` spelled? (the no-inference law keys on it: an absent
    // kind with an `entry.type`-only pkg is the ambiguity)
    let mut declared_type = false;

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
                "style" => Section::Style,
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
                // the declared kind: `lib` (the ordinary source package)
                // or `host` (a pure declaration surface). Absent ⇒ lib —
                // a host pkg SPELLS itself
                "type" => {
                    declared_type = true;
                    match parse_string(value, lineno)?.as_str() {
                        "lib" => m.pkg_type = PkgType::Lib,
                        "host" => m.pkg_type = PkgType::Host,
                        other => {
                            return Err(ManifestError(format!(
                                "line {}: `type` is `\"lib\"` or `\"host\"`, found `{other}`",
                                lineno + 1
                            )))
                        }
                    }
                }
                // bundle-shaped manifests
                "format" => m.format = Some(parse_string(value, lineno)?),
                "format_version" => m.format_version = Some(parse_u64(value, lineno)?),
                // `host_scope` (the retired registration-prefix override)
                // refuses LOUDLY: the key was load-bearing for
                // registration names, so a silent ignore would surface as
                // a confusing boot panic later — the error names the fix
                // (the registration scope is the package name)
                "host_scope" => {
                    return Err(ManifestError(format!(
                        "line {}: `host_scope` is retired — the registration scope is the package name; delete the key (rename the pkg if its scope must change)",
                        lineno + 1
                    )))
                }
                // `inline` (the retired source-inlining flag) rides as an
                // unknown key: old manifests keep parsing
                // dotted entry keys: `entry.type = "..."` etc.
                "entry.type" => m.entry.type_path = Some(parse_string(value, lineno)?),
                "entry.lib" => m.entry.lib = Some(parse_string(value, lineno)?),
                "entry.libs" => m.entry.libs = parse_string_array(value, lineno)?,
                // `entry.ir` (the deleted DeclIr cache key) rides as an
                // unknown key: old manifests keep parsing
                "entry.ir" => {}
                _ => {} // forward-compatible: ignore unknown top-level keys
            },
            Section::Entry => match key.as_str() {
                "type" => m.entry.type_path = Some(parse_string(value, lineno)?),
                "lib" => m.entry.lib = Some(parse_string(value, lineno)?),
                "libs" => m.entry.libs = parse_string_array(value, lineno)?,
                // `ir` (the deleted DeclIr cache key) rides as an unknown
                // key: old manifests keep parsing
                "ir" => {}
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
            Section::Style => {
                // `key = value` rows, string-typed, schema-free: the fmt
                // crate validates keys and parses values (the manifest
                // stays schema-free, the deps-table law); unknown keys
                // ride, refused only by the tool that knows its schema
                m.style.insert(key, parse_string(value, lineno)?);
            }
        }
    }
    // D4: `[peer-deps]` + `[dev-deps]` is the sanctioned
    // both-kinds pairing (the ruling); anything riding `[deps]` beside
    // either is a manifest error naming both rows — a pkg is either
    // pulled transitively or required of the consumer / held for
    // development, never both.
    for name in m.deps.keys() {
        if m.peer_deps.contains_key(name) {
            return Err(ManifestError(format!(
                "`{name}` appears in both `[deps]` and `[peer-deps]` — a package is either pulled transitively or required of the consumer, never both"
            )));
        }
        if m.dev_deps.contains_key(name) {
            return Err(ManifestError(format!(
                "`{name}` appears in both `[deps]` and `[dev-deps]` — a package is either pulled transitively or held for development, never both"
            )));
        }
    }
    // The declared kind's rules. Host-pkg-ness is DECLARED (`type =
    // "host"`), never inferred: a host pkg is pure surface, so deps of
    // any kind, a body, and the bundle-root keys are all refused; the
    // no-inference law makes an `entry.type`-only lib spelling an error —
    // spell the kind, either way.
    match m.pkg_type {
        PkgType::Host => {
            // deps of any kind — name the table AND the row (D4 style):
            // a host pkg is pure surface; the wrapper lib declares the
            // dep, never the host pkg
            for (table, rows) in [
                ("deps", &m.deps),
                ("peer-deps", &m.peer_deps),
                ("dev-deps", &m.dev_deps),
            ] {
                if let Some(spec) = rows.keys().next() {
                    return Err(ManifestError(format!(
                        "`{spec}` appears in `[{table}]` of a `type = \"host\"` pkg — \
                         a host pkg is pure surface; the wrapper lib declares the dep, \
                         never the host pkg"
                    )));
                }
            }
            if m.entry.lib.is_some() || !m.entry.libs.is_empty() {
                return Err(ManifestError(
                    "a `type = \"host\"` pkg has no body — drop `entry.lib`/`entry.libs` \
                     (the wrapper lib owns the source), or declare the pkg `type = \"lib\"`"
                        .into(),
                ));
            }
            if m.entry.type_path.is_none() {
                return Err(ManifestError(
                    "a `type = \"host\"` pkg needs `entry.type` — the declaration surface \
                     is the whole pkg"
                        .into(),
                ));
            }
            if m.format.is_some() || m.format_version.is_some() {
                return Err(ManifestError(
                    "a `type = \"host\"` pkg cannot be a bundle root — `format`/`format_version` \
                     belong to a compiled lib pkg"
                        .into(),
                ));
            }
        }
        PkgType::Lib => {
            // the no-inference law: `entry.type` with no lib and NO
            // DECLARED kind is ambiguous — spell it (this is what makes
            // old host-pkg manifests fail LOUDLY instead of silently
            // becoming empty libs). An explicit `type = "lib"` with a
            // surface and no body is the sanctioned dev state (the
            // loader mounts it as a decl unit).
            if !declared_type && m.entry.type_path.is_some() && m.entry.lib.is_none() {
                return Err(ManifestError(
                    "an `entry.type`-only pkg spells its kind — `type = \"host\"` for a \
                     host pkg, or add `entry.lib` (a `type = \"lib\"` surface-only pkg \
                     is the dev state)"
                        .into(),
                ));
            }
        }
    }
    // The multi-lib entry law: `libs` is an ordered tail
    // on the base `lib`, so the base must exist; every element is a
    // `.rut` source (a `.d.rut` is a decl surface, not a body — the
    // same refusal `[peer-deps]` `lib` gets); and no file rides twice
    // — the splice would duplicate every name in it.
    if !m.entry.libs.is_empty() {
        if m.entry.lib.is_none() {
            return Err(ManifestError(format!(
                "`entry.libs` needs `entry.lib` — the multi-lib entry is a base file plus an ordered tail"
            )));
        }
        let base = m.entry.lib.as_deref().expect("checked just above");
        let mut seen = vec![base.to_string()];
        for lib in &m.entry.libs {
            if lib == base || seen.contains(lib) {
                return Err(ManifestError(format!(
                    "`entry.libs` names `{lib}` twice — the splice would duplicate every name in it"
                )));
            }
            if lib.ends_with(".d.rut") || !lib.ends_with(".rut") {
                return Err(ManifestError(format!(
                    "`entry.libs` names `{lib}` — every lib is a `.rut` source (a `.d.rut` decl surface is not a body)"
                )));
            }
            seen.push(lib.clone());
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
    /// `[style]` — the pkg's formatter knobs (the rut-fmt batch): flat
    /// `key = value` rows, string-typed and schema-free (the consumer
    /// of the manifest — the fmt crate — validates keys and parses
    /// values; unknown keys are ignored, the forward-compat rule).
    Style,
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

/// Parse a single-line TOML string array — `["./a.rut", "./b.rut"]` —
/// the value shape `entry.libs` takes. Strict: quoted
/// strings, comma-separated, no trailing comma, nothing else.
fn parse_string_array(value: &str, lineno: usize) -> Result<Vec<String>, ManifestError> {
    let v = value.trim();
    let Some(inner) = v.strip_prefix('[').and_then(|v| v.strip_suffix(']')) else {
        return Err(ManifestError(format!(
            "line {}: expected a `[\"..\", ..]` string array, found `{v}`",
            lineno + 1
        )));
    };
    let inner = inner.trim();
    if inner.is_empty() {
        return Err(ManifestError(format!(
            "line {}: `libs` cannot be empty — drop the key for a single-file module",
            lineno + 1
        )));
    }
    let mut out = Vec::new();
    for part in inner.split(',') {
        let part = part.trim();
        // every element must be a quoted string — the trailing-comma
        // and bare-word refusals fall out of the same check
        out.push(parse_string(part, lineno)?);
    }
    Ok(out)
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
/// learns).
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

/// A `[deps]` descriptor: exactly ONE source — `path` (a directory) or
/// `url` (a remote `.rutbundle`), never both, never neither — plus,
/// only beside `url`, the `sha256` pin. `optional` is rejected (it is
/// a `[peer-deps]` attribute); any other key is the `[entry]`
/// strictness — a line-targeted error.
fn parse_deps_descriptor(
    value: &str,
    lineno: usize,
) -> Result<BTreeMap<String, String>, ManifestError> {
    let mut out = BTreeMap::new();
    for (k, val) in inline_table_parts(value, lineno)? {
        match k.as_str() {
            "path" | "url" => {
                if out.insert(k.clone(), parse_string(&val, lineno)?).is_some() {
                    return Err(ManifestError(format!(
                        "line {}: `{k}` appears twice — one descriptor names one source",
                        lineno + 1
                    )));
                }
            }
            "sha256" => {
                out.insert(k, parse_pin(&val, lineno)?);
            }
            "optional" => {
                return Err(ManifestError(format!(
                    "line {}: `optional` is a `[peer-deps]` attribute — `[deps]` has no options",
                    lineno + 1
                )));
            }
            other => {
                return Err(ManifestError(format!(
                    "line {}: unknown `[deps]` key `{other}`",
                    lineno + 1
                )));
            }
        }
    }
    match (out.contains_key("path"), out.contains_key("url")) {
        (true, true) => {
            return Err(ManifestError(format!(
                "line {}: `path` and `url` are both set — one descriptor, one source: a directory or a `.rutbundle` url, never both",
                lineno + 1
            )));
        }
        (false, false) => {
            return Err(ManifestError(format!(
                "line {}: the descriptor has no source — `path = \"..\"` for a directory, or `url = \"https://…\"` for a packed bundle",
                lineno + 1
            )));
        }
        _ => {}
    }
    if let Some(url) = out.get("url") {
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err(ManifestError(format!(
                "line {}: `url` must be an http(s) url — found `{url}`",
                lineno + 1
            )));
        }
    } else if out.contains_key("sha256") {
        return Err(ManifestError(format!(
            "line {}: `sha256` pins a url — beside `path` it has no meaning",
            lineno + 1
        )));
    }
    Ok(out)
}

/// The sha256 pin: exactly 64 hex digits, stored LOWERCASE — the pin
/// law compares the fetched bytes' hash against it, so the manifest
/// normalizes first and the comparison is byte-exact.
fn parse_pin(value: &str, lineno: usize) -> Result<String, ManifestError> {
    let v = parse_string(value, lineno)?;
    if v.len() != 64 || !v.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ManifestError(format!(
            "line {}: `sha256` must be 64 hex digits, found `{v}`",
            lineno + 1
        )));
    }
    Ok(v.to_ascii_lowercase())
}

/// A `[peer-deps]`/`[dev-deps]` descriptor: `path` and
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
                        "line {}: `lib` must be a `.rut` source — a `.d.rut` decl surface does not gate",
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

/// `optional` is the one bool the inline-table grammar learns;
/// peers are REQUIRED by default, so the flag must say `true` or
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
entry.lib = "./pouch.rut"
"#;

    #[test]
    fn parses_name_and_entry() {
        let m = parse_manifest(POUCH).unwrap();
        assert_eq!(m.name.as_deref(), Some("pouch"));
        assert_eq!(m.entry.type_path.as_deref(), Some("./pouch.d.rut"));
        assert_eq!(m.entry.lib.as_deref(), Some("./pouch.rut"));
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
    fn deps_parse() {
        let text = "name = \"app\"\n[deps]\n\"core\" = { path = \"rut/core\" }\n";
        let m = parse_manifest(text).unwrap();
        assert_eq!(m.deps.get("core").unwrap().get("path").unwrap(), "rut/core");
    }

    #[test]
    fn pkg_type_defaults_to_lib_and_both_spellings_parse() {
        // absent `type` ⇒ lib — the ordinary source package
        let m = parse_manifest("name = \"pouch\"\nentry.lib = \"./pouch.rut\"\n").unwrap();
        assert_eq!(m.pkg_type, PkgType::Lib);
        let m = parse_manifest(
            "name = \"pouch\"\ntype = \"lib\"\nentry.lib = \"./pouch.rut\"\n",
        )
        .unwrap();
        assert_eq!(m.pkg_type, PkgType::Lib);
        // a host pkg SPELLS itself
        let m = parse_manifest(
            "name = \"ink_host\"\ntype = \"host\"\nentry.type = \"./ink_host.d.rut\"\n",
        )
        .unwrap();
        assert_eq!(m.pkg_type, PkgType::Host);
    }

    #[test]
    fn pkg_type_bad_value_is_line_targeted() {
        let err = parse_manifest("name = \"x\"\ntype = \"Host\"\n").unwrap_err();
        assert_eq!(err.to_string(), "line 2: `type` is `\"lib\"` or `\"host\"`, found `Host`");
        let err = parse_manifest("name = \"x\"\ntype = \"native\"\n").unwrap_err();
        assert!(err.to_string().contains("line 2"), "{err}");
    }

    #[test]
    fn host_pkg_refuses_every_deps_table() {
        for table in ["deps", "peer-deps", "dev-deps"] {
            let err = parse_manifest(&format!(
                "name = \"h\"\ntype = \"host\"\nentry.type = \"./h.d.rut\"\n[{table}]\nink = {{ path = \"../ink\" }}\n"
            ))
            .unwrap_err();
            let want = format!(
                "`ink` appears in `[{table}]` of a `type = \"host\"` pkg — a host pkg is pure surface; the wrapper lib declares the dep, never the host pkg",
                table = table
            );
            assert_eq!(err.to_string(), want, "[{table}]");
        }
    }

    #[test]
    fn host_pkg_needs_entry_type() {
        let err = parse_manifest("name = \"h\"\ntype = \"host\"\n").unwrap_err();
        assert!(err.to_string().contains("needs `entry.type`"), "{err}");
    }

    #[test]
    fn host_pkg_refuses_a_body() {
        let err = parse_manifest(
            "name = \"h\"\ntype = \"host\"\nentry.type = \"./h.d.rut\"\nentry.lib = \"./h.rut\"\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("no body"), "{err}");
        // the libs tail is a body too
        let err = parse_manifest(
            "name = \"h\"\ntype = \"host\"\nentry.type = \"./h.d.rut\"\nentry.libs = [\"./more.rut\"]\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("no body"), "{err}");
    }

    #[test]
    fn host_pkg_refuses_the_bundle_root_keys() {
        let err = parse_manifest(
            "format = \"rutbundle\"\nformat_version = 5\nname = \"h\"\ntype = \"host\"\nentry.type = \"./h.d.rut\"\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("bundle root"), "{err}");
    }

    #[test]
    fn entry_type_only_without_a_declared_kind_is_the_ambiguity() {
        // the no-inference law: an entry.type-only pkg spells its kind —
        // old host-pkg manifests fail LOUDLY here, not as empty libs
        let err = parse_manifest("name = \"rt\"\nentry.type = \"./rt.d.rut\"\n").unwrap_err();
        assert!(
            err.to_string()
                .contains("an `entry.type`-only pkg spells its kind"),
            "{err}"
        );
        assert!(err.to_string().contains("`type = \"host\"`"), "{err}");
        // either fix passes: declare host…
        let m = parse_manifest("name = \"rt\"\ntype = \"host\"\nentry.type = \"./rt.d.rut\"\n")
            .unwrap();
        assert_eq!(m.pkg_type, PkgType::Host);
        // …or add the body (an ordinary lib pkg)
        let m = parse_manifest(
            "name = \"dev\"\ntype = \"lib\"\nentry.type = \"./dev.d.rut\"\nentry.lib = \"./dev.rut\"\n",
        )
        .unwrap();
        assert_eq!(m.pkg_type, PkgType::Lib);
        // an EXPLICIT `type = "lib"` with a surface and no body is the
        // sanctioned surface-only dev state — spelled, so no ambiguity
        let m = parse_manifest(
            "name = \"dev\"\ntype = \"lib\"\nentry.type = \"./dev.d.rut\"\n",
        )
        .unwrap();
        assert_eq!(m.pkg_type, PkgType::Lib);
    }

    #[test]
    fn host_scope_retires_loudly_and_inline_rides() {
        // `host_scope` (the retired registration-prefix override)
        // refuses at parse, naming the fix — the registration scope is
        // the package name. `inline` (the retired source-inlining flag)
        // still parses as an unknown key: old manifests keep loading,
        // the flag does nothing.
        let err = parse_manifest(
            "name = \"ink_host\"\nentry.type = \"./ink_host.d.rut\"\nhost_scope = \"ink_host\"\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("line 3"), "{err}");
        assert!(err.to_string().contains("`host_scope` is retired"), "{err}");
        assert!(err.to_string().contains("the registration scope is the package name"), "{err}");
        let m = parse_manifest("name = \"ink\"\nentry.lib = \"./ink.rut\"\ninline = true\n")
            .unwrap();
        assert_eq!(m.entry.lib.as_deref(), Some("./ink.rut"));
    }

    // ---- the dep kinds: the three tables, `optional`, the
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
            "line 2: `lib` must be a `.rut` source — a `.d.rut` decl surface does not gate"
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
            "`pouch` appears in both `[deps]` and `[peer-deps]` — a package is either pulled transitively or required of the consumer, never both"
        );
        let err = parse_manifest(
            "name = \"j\"\n[deps]\npouch = { path = \"../pouch\" }\n[dev-deps]\npouch = { path = \"../pouch\" }\n",
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "`pouch` appears in both `[deps]` and `[dev-deps]` — a package is either pulled transitively or held for development, never both"
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

    // ---- the url source kind: one descriptor, one source; the pin ----

    #[test]
    fn url_dep_parses_and_pin_normalizes_to_lowercase() {
        let m = parse_manifest(
            "name = \"app\"\n[deps]\npouch = { url = \"https://example.com/pouch.rutbundle\", sha256 = \"ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789\" }\n",
        )
        .unwrap();
        let d = m.deps.get("pouch").unwrap();
        assert_eq!(d.get("url").unwrap(), "https://example.com/pouch.rutbundle");
        // the pin law compares byte-exactly — the manifest normalizes first
        assert_eq!(
            d.get("sha256").unwrap(),
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
        );
        // an unpinned url is legal (the pin is optional; reproducible
        // packs want one)
        let m = parse_manifest(
            "name = \"app\"\n[deps]\npouch = { url = \"http://localhost:8080/pouch.rutbundle\" }\n",
        )
        .unwrap();
        assert!(m.deps.get("pouch").unwrap().get("sha256").is_none());
    }

    #[test]
    fn url_descriptor_refusal_matrix() {
        // url + path: one descriptor, one source
        let err = parse_manifest(
            "[deps]\npouch = { url = \"https://x/p.rutbundle\", path = \"../pouch\" }\n",
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "line 2: `path` and `url` are both set — one descriptor, one source: a directory or a `.rutbundle` url, never both"
        );
        // neither
        let err = parse_manifest("[deps]\npouch = { }\n").unwrap_err();
        assert_eq!(
            err.to_string(),
            "line 2: the descriptor has no source — `path = \"..\"` for a directory, or `url = \"https://…\"` for a packed bundle"
        );
        // sha256 beside path (no url) is meaningless
        let err = parse_manifest(
            "[deps]\npouch = { path = \"../pouch\", sha256 = \"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\" }\n",
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "line 2: `sha256` pins a url — beside `path` it has no meaning"
        );
        // bad hex
        let err = parse_manifest(
            "[deps]\npouch = { url = \"https://x/p.rutbundle\", sha256 = \"nothex\" }\n",
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "line 2: `sha256` must be 64 hex digits, found `nothex`"
        );
        // non-http(s)
        let err = parse_manifest("[deps]\npouch = { url = \"ftp://x/p.rutbundle\" }\n").unwrap_err();
        assert_eq!(
            err.to_string(),
            "line 2: `url` must be an http(s) url — found `ftp://x/p.rutbundle`"
        );
        // a duplicate source key is the same one-source refusal
        let err = parse_manifest("[deps]\npouch = { url = \"https://x/a\", url = \"https://x/b\" }\n")
            .unwrap_err();
        assert!(err.to_string().contains("`url` appears twice"), "{err}");
        // unknown keys keep the [entry] strictness
        let err = parse_manifest("[deps]\npouch = { url = \"https://x/p\", feats = \"x\" }\n")
            .unwrap_err();
        assert_eq!(err.to_string(), "line 2: unknown `[deps]` key `feats`");
    }

    #[test]
    fn peer_tables_refuse_the_url_kind() {
        // `[peer-deps]` stays path-only — its path is directory-time
        // metadata for the declarer's own build, nothing to fetch
        let err = parse_manifest("[peer-deps]\npouch = { url = \"https://x/p.rutbundle\" }\n")
            .unwrap_err();
        assert_eq!(err.to_string(), "line 2: unknown `[peer-deps]` key `url`");
        let err = parse_manifest("[dev-deps]\npouch = { url = \"https://x/p.rutbundle\" }\n")
            .unwrap_err();
        assert_eq!(err.to_string(), "line 2: unknown `[dev-deps]` key `url`");
    }
}
