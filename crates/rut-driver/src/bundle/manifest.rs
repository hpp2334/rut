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
//! The TOML text itself parses with `toml_edit` — standard TOML, every
//! legal spelling (escapes, `'literal'` strings, multi-line strings,
//! underscored integers) processed by the parser; the laws below are
//! the manifest's value rules over the parsed document.

use std::collections::BTreeMap;

use thiserror::Error;
use toml_edit::{Document, Item, Table, TableLike, TomlError, Value};

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
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct ManifestError(pub String);

/// A bare package name: `[a-zA-Z0-9_]+`, non-empty. The same charset
/// governs manifest `name` values and `[deps]` keys.
pub fn valid_spec(spec: &str) -> bool {
    !spec.is_empty() && spec.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Parse `rut.toml`: standard TOML (`toml_edit`), then this format's
/// value laws — top-level `name`, `type` (the declared kind),
/// `entry.type` / `entry.lib` / `entry.libs` (bare or under
/// `[entry]`), and the dep tables `[deps]` / `[peer-deps]` /
/// `[dev-deps]` whose values are inline tables.
pub fn parse_manifest(text: &str) -> Result<Manifest, ManifestError> {
    let doc = Document::parse(text).map_err(|e| syntax_error(text, &e))?;
    let mut m = Manifest::default();
    // was `type` spelled? (the no-inference law keys on it: an absent
    // kind with an `entry.type`-only pkg is the ambiguity)
    let mut declared_type = false;
    let root = doc.as_table();

    for (key, item) in root.iter() {
        match item {
            // an explicit `[section]` header: the five known sections
            // walk their laws; any other header — including sub-tables
            // like `[deps.pouch]` — is the unknown-section refusal,
            // named as the document spells it, at the header's line
            Item::Table(t) if !t.is_dotted() => match key {
                "entry" => walk_entry(text, t, &mut m)?,
                "deps" => walk_deps(text, t, &mut m)?,
                "peer-deps" => walk_peers(text, t, "peer-deps", &mut m)?,
                "dev-deps" => walk_peers(text, t, "dev-deps", &mut m)?,
                "style" => walk_style(text, t, &mut m)?,
                other => return Err(unknown_section(text, root, other, other, t)),
            },
            // a dotted key's implicit table: `entry.type = "..."` spells
            // the entry keys in place; other dotted keys ride (the
            // forward-compat law covers them as it covers unknown
            // scalar keys)
            Item::Table(t) if key == "entry" => walk_entry(text, t, &mut m)?,
            Item::Table(_) => {}
            Item::Value(v) => walk_top(text, root, key, v, &mut declared_type, &mut m)?,
            // `[[array-of-tables]]` — no rut.toml section takes the shape
            Item::ArrayOfTables(_) => {
                return Err(ManifestError(format!(
                    "line {}: unknown section `[[{key}]]`",
                    key_line(text, root, key)
                )));
            }
            Item::None => {}
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
    // any kind and a body are refused; the no-inference law makes an
    // `entry.type`-only lib spelling an error — spell the kind, either
    // way. The bundle-root keys (`format`/`format_version`) are LEGAL on
    // a host manifest: a host pkg packs as a v6 decl root (its root is
    // the declaration surface itself). Directory loading ignores the
    // keys either way.
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

// ---- the syntax boundary: toml_edit's diagnostics become the crate's
// single-line `line N:` shape ----

/// A TOML syntax error as one `line N: <message>` diagnostic — the
/// error's span pins the line, the parser's own wording carries the
/// rest.
fn syntax_error(text: &str, err: &TomlError) -> ManifestError {
    let line = err.span().map_or(1, |s| line_of(text, s.start));
    let message = err
        .message()
        .split('\n')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    ManifestError(format!("line {line}: {message}"))
}

/// 1-based line of a byte offset — the `line N:` prefix convention.
fn line_of(text: &str, at: usize) -> usize {
    1 + text.as_bytes()[..at.min(text.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
}

/// The line a key sits on (its span's first line; 1 when absent).
fn key_line(text: &str, container: &impl TableLike, key: &str) -> usize {
    container
        .key(key)
        .and_then(|k| k.span())
        .map_or(1, |s| line_of(text, s.start))
}

/// The value as spelled — the "found `X`" text of the type refusals.
fn raw(value: &Value) -> String {
    value.to_string().trim().to_string()
}

/// The unknown-section refusal: the header named as the document
/// spells it (a `[a.b]` chain descends to its deepest header), at that
/// header's line (`span_key` is the header's own first segment, for
/// the span lookup in `parent`).
fn unknown_section(
    text: &str,
    parent: &impl TableLike,
    span_key: &str,
    name: &str,
    table: &Table,
) -> ManifestError {
    match table
        .iter()
        .find(|(_, i)| i.as_table().is_some_and(|t| !t.is_dotted()))
    {
        Some((k, item)) => {
            let t = item.as_table().expect("found a table above");
            unknown_section(text, table, k, &format!("{name}.{k}"), t)
        }
        _ => ManifestError(format!(
            "line {}: unknown section `[{name}]`",
            key_line(text, parent, span_key)
        )),
    }
}

/// A section's rows, dotted keys flattened to their leaf paths (`a.b`
/// — the spelling the laws below match on), document order kept, each
/// row carrying the line its first key sits on. An explicit sub-table
/// header (`[deps.pouch]`) is nobody's row: the unknown-section
/// refusal names it (`section` gives the header its full path).
fn flatten<'a>(
    text: &str,
    container: &'a impl TableLike,
    prefix: &str,
    section: &str,
    out: &mut Vec<(String, usize, &'a Item)>,
) -> Result<(), ManifestError> {
    for (key, item) in container.iter() {
        let path = if prefix.is_empty() {
            key.to_string()
        } else {
            format!("{prefix}.{key}")
        };
        match item {
            Item::Table(t) if !t.is_dotted() => {
                let header = if section.is_empty() {
                    path.clone()
                } else {
                    format!("{section}.{path}")
                };
                return Err(unknown_section(text, container, key, &header, t));
            }
            Item::Table(t) => flatten(text, t, &path, section, out)?,
            Item::ArrayOfTables(_) => {
                let header = if section.is_empty() {
                    path
                } else {
                    format!("{section}.{path}")
                };
                return Err(ManifestError(format!(
                    "line {}: unknown section `[[{header}]]`",
                    key_line(text, container, key)
                )));
            }
            Item::None => {}
            leaf => out.push((path, key_line(text, container, key), leaf)),
        }
    }
    Ok(())
}

// ---- the sections ----

/// Top-level scalar keys. Unknown keys ride (forward compatibility),
/// `host_scope` refuses loudly (the retirement law).
fn walk_top(
    text: &str,
    root: &Table,
    key: &str,
    value: &Value,
    declared_type: &mut bool,
    m: &mut Manifest,
) -> Result<(), ManifestError> {
    let line = key_line(text, root, key);
    match key {
        "name" => {
            let name = expect_string(line, value)?;
            if !valid_spec(&name) {
                return Err(ManifestError(format!(
                    "line {line}: module name `{name}` is not a bare package name — expected `[a-zA-Z0-9_]+`"
                )));
            }
            m.name = Some(name);
        }
        // the declared kind: `lib` (the ordinary source package)
        // or `host` (a pure declaration surface). Absent ⇒ lib —
        // a host pkg SPELLS itself
        "type" => {
            *declared_type = true;
            match expect_string(line, value)?.as_str() {
                "lib" => m.pkg_type = PkgType::Lib,
                "host" => m.pkg_type = PkgType::Host,
                other => {
                    return Err(ManifestError(format!(
                        "line {line}: `type` is `\"lib\"` or `\"host\"`, found `{other}`"
                    )));
                }
            }
        }
        // bundle-shaped manifests
        "format" => m.format = Some(expect_string(line, value)?),
        "format_version" => m.format_version = Some(expect_u64(line, value)?),
        // `host_scope` (the retired registration-prefix override)
        // refuses LOUDLY: the key was load-bearing for
        // registration names, so a silent ignore would surface as
        // a confusing boot panic later — the error names the fix
        // (the registration scope is the package name)
        "host_scope" => {
            return Err(ManifestError(format!(
                "line {line}: `host_scope` is retired — the registration scope is the package name; delete the key (rename the pkg if its scope must change)"
            )));
        }
        _ => {} // forward-compatible: ignore unknown top-level keys
    }
    Ok(())
}

/// `[entry]` (or the dotted `entry.*` spelling): the entry keys plus
/// the retired `entry.ir`, which rides; unknown keys are refused (the
/// `[entry]` strictness).
fn walk_entry(text: &str, table: &Table, m: &mut Manifest) -> Result<(), ManifestError> {
    let mut rows = Vec::new();
    flatten(text, table, "", "entry", &mut rows)?;
    for (path, line, item) in rows {
        let Item::Value(v) = item else { continue };
        match path.as_str() {
            "type" => m.entry.type_path = Some(expect_string(line, v)?),
            "lib" => m.entry.lib = Some(expect_string(line, v)?),
            "libs" => m.entry.libs = expect_string_array(text, line, v)?,
            "ir" => {}
            other => {
                return Err(ManifestError(format!(
                    "line {line}: unknown `[entry]` key `{other}`"
                )));
            }
        }
    }
    Ok(())
}

/// `[deps]`: exact specifier → descriptor; the descriptor laws are
/// [`parse_deps_descriptor`]'s.
fn walk_deps(text: &str, table: &Table, m: &mut Manifest) -> Result<(), ManifestError> {
    let mut rows = Vec::new();
    flatten(text, table, "", "deps", &mut rows)?;
    for (spec, line, item) in rows {
        check_dep_key(&spec, line)?;
        let Item::Value(v) = item else { continue };
        m.deps.insert(spec, parse_deps_descriptor(text, line, v)?);
    }
    Ok(())
}

/// `[peer-deps]` / `[dev-deps]`: same descriptor walk with the
/// `optional` flag and the `lib` group key.
fn walk_peers(
    text: &str,
    table: &Table,
    kind: &str,
    m: &mut Manifest,
) -> Result<(), ManifestError> {
    let mut rows = Vec::new();
    flatten(text, table, "", kind, &mut rows)?;
    for (spec, line, item) in rows {
        check_dep_key(&spec, line)?;
        let Item::Value(v) = item else { continue };
        let desc = parse_peer_descriptor(text, line, kind, v)?;
        match kind {
            "peer-deps" => {
                m.peer_deps.insert(spec, desc);
            }
            _ => {
                m.dev_deps.insert(spec, desc);
            }
        }
    }
    Ok(())
}

/// `[style]`: `key = value` rows, string-typed, schema-free — the fmt
/// crate validates keys and parses values (the manifest stays
/// schema-free, the deps-table law); unknown keys ride, refused only
/// by the tool that knows its schema.
fn walk_style(text: &str, table: &Table, m: &mut Manifest) -> Result<(), ManifestError> {
    let mut rows = Vec::new();
    flatten(text, table, "", "style", &mut rows)?;
    for (key, line, item) in rows {
        let Item::Value(v) = item else { continue };
        m.style.insert(key, expect_string(line, v)?);
    }
    Ok(())
}

// ---- the typed values: the laws the manifest itself owns. Type
// mismatches keep today's refusals ("expected a quoted string, found
// `X`", "expected an integer", ...); the TOML spellings are the
// parser's business, not the laws'. ----

/// A `key = "string"` value.
fn expect_string(lineno: usize, value: &Value) -> Result<String, ManifestError> {
    match value.as_str() {
        Some(s) => Ok(s.to_string()),
        None => Err(ManifestError(format!(
            "line {lineno}: expected a quoted string, found `{}`",
            raw(value)
        ))),
    }
}

/// A non-negative integer (`format_version`).
fn expect_u64(lineno: usize, value: &Value) -> Result<u64, ManifestError> {
    match value.as_integer().and_then(|i| u64::try_from(i).ok()) {
        Some(v) => Ok(v),
        None => Err(ManifestError(format!(
            "line {lineno}: expected an integer, found `{}`",
            raw(value)
        ))),
    }
}

/// The `entry.libs` string array. Strict: every element a string (the
/// bare-word and non-string refusals fall out of the same check). An
/// empty array is nobody's multi-lib entry — drop the key.
fn expect_string_array(
    text: &str,
    lineno: usize,
    value: &Value,
) -> Result<Vec<String>, ManifestError> {
    let Some(arr) = value.as_array() else {
        return Err(ManifestError(format!(
            "line {lineno}: expected a `[\"..\", ..]` string array, found `{}`",
            raw(value)
        )));
    };
    if arr.is_empty() {
        return Err(ManifestError(format!(
            "line {lineno}: `libs` cannot be empty — drop the key for a single-file module"
        )));
    }
    let mut out = Vec::new();
    for el in arr.iter() {
        let eline = el.span().map_or(lineno, |s| line_of(text, s.start));
        out.push(expect_string(eline, el)?);
    }
    Ok(out)
}

/// A dep-table key is a bare package name — the same charset law as the
/// manifest `name`.
fn check_dep_key(key: &str, lineno: usize) -> Result<(), ManifestError> {
    if !valid_spec(key) {
        return Err(ManifestError(format!(
            "line {lineno}: dep `{key}` is not a bare package name — expected `[a-zA-Z0-9_]+`"
        )));
    }
    Ok(())
}

/// A `[deps]` descriptor: exactly ONE source — `path` (a directory) or
/// `url` (a remote `.rutbundle`), never both, never neither — plus,
/// only beside `url`, the `sha256` pin. `optional` is rejected (it is
/// a `[peer-deps]` attribute); any other key is the `[entry]`
/// strictness — a line-targeted error.
fn parse_deps_descriptor(
    text: &str,
    lineno: usize,
    value: &Value,
) -> Result<BTreeMap<String, String>, ManifestError> {
    let Some(inline) = value.as_inline_table() else {
        return Err(ManifestError(format!(
            "line {lineno}: expected an inline table `{{ key = \"value\" }}`, found `{}`",
            raw(value)
        )));
    };
    let mut rows = Vec::new();
    flatten(text, inline, "", "", &mut rows)?;
    let mut out = BTreeMap::new();
    for (k, vline, item) in rows {
        let Item::Value(v) = item else { continue };
        match k.as_str() {
            "path" | "url" => {
                out.insert(k, expect_string(vline, v)?);
            }
            "sha256" => {
                out.insert(k, parse_pin(vline, v)?);
            }
            "optional" => {
                return Err(ManifestError(format!(
                    "line {vline}: `optional` is a `[peer-deps]` attribute — `[deps]` has no options"
                )));
            }
            other => {
                return Err(ManifestError(format!(
                    "line {vline}: unknown `[deps]` key `{other}`"
                )));
            }
        }
    }
    match (out.contains_key("path"), out.contains_key("url")) {
        (true, true) => {
            return Err(ManifestError(format!(
                "line {lineno}: `path` and `url` are both set — one descriptor, one source: a directory or a `.rutbundle` url, never both"
            )));
        }
        (false, false) => {
            return Err(ManifestError(format!(
                "line {lineno}: the descriptor has no source — `path = \"..\"` for a directory, or `url = \"https://…\"` for a packed bundle"
            )));
        }
        _ => {}
    }
    if let Some(url) = out.get("url") {
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err(ManifestError(format!(
                "line {lineno}: `url` must be an http(s) url — found `{url}`"
            )));
        }
    } else if out.contains_key("sha256") {
        return Err(ManifestError(format!(
            "line {lineno}: `sha256` pins a url — beside `path` it has no meaning"
        )));
    }
    Ok(out)
}

/// The sha256 pin: exactly 64 hex digits, stored LOWERCASE — the pin
/// law compares the fetched bytes' hash against it, so the manifest
/// normalizes first and the comparison is byte-exact.
fn parse_pin(lineno: usize, value: &Value) -> Result<String, ManifestError> {
    let v = expect_string(lineno, value)?;
    if v.len() != 64 || !v.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ManifestError(format!(
            "line {lineno}: `sha256` must be 64 hex digits, found `{v}`"
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
    text: &str,
    lineno: usize,
    table: &str,
    value: &Value,
) -> Result<BTreeMap<String, String>, ManifestError> {
    let Some(inline) = value.as_inline_table() else {
        return Err(ManifestError(format!(
            "line {lineno}: expected an inline table `{{ key = \"value\" }}`, found `{}`",
            raw(value)
        )));
    };
    let mut rows = Vec::new();
    flatten(text, inline, "", "", &mut rows)?;
    let mut out = BTreeMap::new();
    for (k, vline, item) in rows {
        let Item::Value(v) = item else { continue };
        match k.as_str() {
            "path" => {
                out.insert(k, expect_string(vline, v)?);
            }
            "lib" => {
                let v = expect_string(vline, v)?;
                if v.ends_with(".d.rut") {
                    return Err(ManifestError(format!(
                        "line {vline}: `lib` must be a `.rut` source — a `.d.rut` decl surface does not gate"
                    )));
                }
                out.insert(k, v);
            }
            "optional" => {
                out.insert(k, expect_optional(vline, v)?);
            }
            other => {
                return Err(ManifestError(format!(
                    "line {vline}: unknown `[{table}]` key `{other}`"
                )));
            }
        }
    }
    Ok(out)
}

/// `optional` is the one bool the descriptor grammar learns;
/// peers are REQUIRED by default, so the flag must say `true` or
/// `false` exactly. Stored as its canonical spelling.
fn expect_optional(lineno: usize, value: &Value) -> Result<String, ManifestError> {
    match value.as_bool() {
        Some(b) => Ok(b.to_string()),
        None => Err(ManifestError(format!(
            "line {lineno}: `optional` expects `true` or `false`"
        ))),
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
    fn host_pkg_takes_the_bundle_root_keys() {
        // the v6 grammar: `format`/`format_version` are LEGAL on a
        // `type = "host"` manifest — a host pkg packs as a decl root
        let m = parse_manifest(
            "format = \"rutbundle\"\nformat_version = 6\nname = \"h\"\ntype = \"host\"\nentry.type = \"./h.d.rut\"\n",
        )
        .unwrap();
        assert_eq!(m.pkg_type, PkgType::Host);
        assert_eq!(m.format.as_deref(), Some("rutbundle"));
        assert_eq!(m.format_version, Some(6));
    }

    #[test]
    fn host_bundle_manifest_still_refuses_its_other_laws() {
        // the bundle keys change nothing else: the deps tables, the body
        // refusal, the `entry.type` requirement all still hold WITH the
        // keys present
        for table in ["deps", "peer-deps", "dev-deps"] {
            let err = parse_manifest(&format!(
                "format = \"rutbundle\"\nformat_version = 6\nname = \"h\"\ntype = \"host\"\nentry.type = \"./h.d.rut\"\n[{table}]\nink = {{ path = \"../ink\" }}\n"
            ))
            .unwrap_err();
            assert!(err.to_string().contains("a host pkg is pure surface"), "[{table}]: {err}");
        }
        let err = parse_manifest(
            "format = \"rutbundle\"\nformat_version = 6\nname = \"h\"\ntype = \"host\"\nentry.type = \"./h.d.rut\"\nentry.lib = \"./h.rut\"\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("no body"), "{err}");
        let err = parse_manifest("format = \"rutbundle\"\nformat_version = 6\nname = \"h\"\ntype = \"host\"\n")
            .unwrap_err();
        assert!(err.to_string().contains("needs `entry.type`"), "{err}");
    }

    #[test]
    fn format_version_must_be_an_integer() {
        // a TOML-legal but non-integer value trips the value law
        let err = parse_manifest(
            "format = \"rutbundle\"\nformat_version = \"1\"\nname = \"h\"\ntype = \"host\"\nentry.type = \"./h.d.rut\"\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("line 2"), "{err}");
        assert!(err.to_string().contains("expected an integer"), "{err}");
    }

    #[test]
    fn syntax_errors_normalize_to_line_n() {
        // `six` is not a TOML value — the syntax error carries the
        // `line N:` prefix with the parser's own wording
        let err = parse_manifest("name = \"x\"\nformat_version = six\n").unwrap_err();
        let msg = err.to_string();
        assert!(msg.starts_with("line 2: "), "{msg}");
        assert!(msg.contains("expected literal string"), "{msg}");
    }

    #[test]
    fn full_toml_spellings_parse() {
        // standard TOML now: 'literal' strings, \uXXXX escapes,
        // underscored integers, multi-line strings — the parser
        // processes them, the value laws are unchanged
        let m = parse_manifest("name = 'pouch'\nentry.lib = \"./p\\u006Fuch.rut\"\n").unwrap();
        assert_eq!(m.name.as_deref(), Some("pouch"));
        assert_eq!(m.entry.lib.as_deref(), Some("./pouch.rut"));
        let m = parse_manifest(
            "name = \"x\"\nformat = \"rutbundle\"\nformat_version = 1_0\n",
        )
        .unwrap();
        assert_eq!(m.format_version, Some(10));
        let m = parse_manifest("name = \"x\"\nentry.lib = \"\"\"\n./a.rut\"\"\"\n").unwrap();
        assert_eq!(m.entry.lib.as_deref(), Some("./a.rut"));
    }

    #[test]
    fn name_type_mismatch_is_line_targeted() {
        // an integer is not a string — the value law's wording, today's
        let err = parse_manifest("name = 3\n").unwrap_err();
        assert_eq!(err.to_string(), "line 1: expected a quoted string, found `3`");
    }

    #[test]
    fn sub_tables_are_unknown_sections() {
        // `[deps.pouch]` is nobody's row — the unknown-section refusal
        // names the header, at the header's line
        let err = parse_manifest("name = \"x\"\n[deps.pouch]\npath = \"p\"\n").unwrap_err();
        assert_eq!(err.to_string(), "line 2: unknown section `[deps.pouch]`");
    }

    #[test]
    fn repeated_headers_are_toml_errors() {
        // today's reader merged a repeated header silently (last wins);
        // TOML refuses the duplicate
        let err = parse_manifest("[deps]\n[deps]\n").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("line 2"), "{msg}");
        assert!(msg.contains("duplicate key"), "{msg}");
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
        // a duplicate source key is a TOML duplicate-key parse error now
        let err = parse_manifest("[deps]\npouch = { url = \"https://x/a\", url = \"https://x/b\" }\n")
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("line 2"), "{msg}");
        assert!(msg.contains("duplicate key"), "{msg}");
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
