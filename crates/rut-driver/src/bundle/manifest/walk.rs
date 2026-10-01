//! The section walkers: top-level keys, `[entry]`, the dep tables,
//! `[style]` — plus the syntax boundary (toml_edit's diagnostics
//! become the module's single-line `line N:` shape) and the row
//! flattener the walkers share.

use toml_edit::{Item, Table, TableLike, TomlError, Value};

use super::descriptor::{parse_deps_descriptor, parse_peer_descriptor};
use super::expect::{expect_string, expect_string_array, expect_u64};
use super::descriptor::check_dep_key;
use super::{valid_spec, Manifest, ManifestError, PkgType};
// ---- the syntax boundary: toml_edit's diagnostics become the crate's
// single-line `line N:` shape ----

/// A TOML syntax error as one `line N: <message>` diagnostic — the
/// error's span pins the line, the parser's own wording carries the
/// rest.
pub(super) fn syntax_error(text: &str, err: &TomlError) -> ManifestError {
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
pub(super) fn line_of(text: &str, at: usize) -> usize {
    1 + text.as_bytes()[..at.min(text.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
}

/// The line a key sits on (its span's first line; 1 when absent).
pub(super) fn key_line(text: &str, container: &impl TableLike, key: &str) -> usize {
    container
        .key(key)
        .and_then(|k| k.span())
        .map_or(1, |s| line_of(text, s.start))
}

/// The value as spelled — the "found `X`" text of the type refusals.
pub(super) fn raw(value: &Value) -> String {
    value.to_string().trim().to_string()
}

/// The unknown-section refusal: the header named as the document
/// spells it (a `[a.b]` chain descends to its deepest header), at that
/// header's line (`span_key` is the header's own first segment, for
/// the span lookup in `parent`).
pub(super) fn unknown_section(
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
pub(super) fn flatten<'a>(
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
pub(super) fn walk_top(
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
pub(super) fn walk_entry(text: &str, table: &Table, m: &mut Manifest) -> Result<(), ManifestError> {
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
pub(super) fn walk_deps(text: &str, table: &Table, m: &mut Manifest) -> Result<(), ManifestError> {
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
pub(super) fn walk_peers(
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
pub(super) fn walk_style(text: &str, table: &Table, m: &mut Manifest) -> Result<(), ManifestError> {
    let mut rows = Vec::new();
    flatten(text, table, "", "style", &mut rows)?;
    for (key, line, item) in rows {
        let Item::Value(v) = item else { continue };
        m.style.insert(key, expect_string(line, v)?);
    }
    Ok(())
}
