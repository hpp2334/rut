//! The dep tables' descriptor grammars: `deps` (one source — `path` or
//! `url` — plus, only beside `url`, the `sha256` pin) and the
//! `peer-deps`/`dev-deps` shape (`path`, `lib`, `optional`). All
//! diagnostics are PATH-TARGETED: the descriptor's path (`deps.pouch`)
//! prefixes every refusal.

use std::collections::BTreeMap;

use serde_json::Value;

use super::expect::{expect_field_string, expect_string};
use super::walk::expect_object;
use super::{valid_spec, ManifestError};

/// A dep-table key is a bare package name — the same charset law as the
/// manifest `name`.
pub(super) fn check_dep_key(table: &str, key: &str) -> Result<(), ManifestError> {
    if !valid_spec(key) {
        return Err(ManifestError(format!(
            "{table}: '{key}' is not a bare package name — expected [a-zA-Z0-9_]+"
        )));
    }
    Ok(())
}

/// A `deps` descriptor: exactly ONE source — `path` (a directory) or
/// `url` (a remote `.rutbundle`), never both, never neither — plus,
/// only beside `url`, the `sha256` pin. `optional` is rejected (it is
/// a `peer-deps` attribute); any other key is the entry strictness —
/// a path-targeted error.
pub(super) fn parse_deps_descriptor(
    table: &str,
    spec: &str,
    value: &Value,
) -> Result<BTreeMap<String, String>, ManifestError> {
    let path = format!("{table}.{spec}");
    let obj = expect_object(&path, value)?;
    let mut out = BTreeMap::new();
    for (k, v) in obj {
        if k.starts_with('_') {
            continue;
        }
        match k.as_str() {
            "path" | "url" => {
                out.insert(k.clone(), expect_field_string(&path, k, v)?);
            }
            "sha256" => {
                out.insert(k.clone(), parse_pin(&path, k, v)?);
            }
            "optional" => {
                return Err(ManifestError(format!(
                    "{path}: 'optional' is a peer-deps attribute — deps has no options"
                )));
            }
            other => {
                return Err(ManifestError(format!("{path}: unknown key '{other}'")));
            }
        }
    }
    match (out.contains_key("path"), out.contains_key("url")) {
        (true, true) => {
            return Err(ManifestError(format!(
                "{path}: 'path' and 'url' are both set — one descriptor, one source: a directory or a .rutbundle url, never both"
            )));
        }
        (false, false) => {
            return Err(ManifestError(format!(
                "{path}: the descriptor has no source — 'path' for a directory, or 'url' for a packed bundle"
            )));
        }
        _ => {}
    }
    if let Some(url) = out.get("url") {
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err(ManifestError(format!(
                "{path}: 'url' must be an http(s) url, found '{url}'"
            )));
        }
    } else if out.contains_key("sha256") {
        return Err(ManifestError(format!(
            "{path}: 'sha256' pins a url — beside 'path' it has no meaning"
        )));
    }
    Ok(out)
}

/// The sha256 pin: exactly 64 hex digits, stored LOWERCASE — the pin
/// law compares the fetched bytes' hash against it, so the manifest
/// normalizes first and the comparison is byte-exact.
pub(super) fn parse_pin(
    path: &str,
    key: &str,
    value: &Value,
) -> Result<String, ManifestError> {
    let v = expect_field_string(path, key, value)?;
    if v.len() != 64 || !v.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ManifestError(format!(
            "{path}: 'sha256' must be 64 hex digits, found '{v}'"
        )));
    }
    Ok(v.to_ascii_lowercase())
}

/// A `peer-deps`/`dev-deps` descriptor: `path` and
/// `lib` are strings (`lib` is the peer-gated integration file — an
/// impl-only `.rut` source; a `.d.rut` decl surface does not gate),
/// `optional` is the one bool, and any other key is the entry
/// strictness — a path-targeted error.
pub(super) fn parse_peer_descriptor(
    table: &str,
    spec: &str,
    value: &Value,
) -> Result<BTreeMap<String, String>, ManifestError> {
    let path = format!("{table}.{spec}");
    let obj = expect_object(&path, value)?;
    let mut out = BTreeMap::new();
    for (k, v) in obj {
        if k.starts_with('_') {
            continue;
        }
        match k.as_str() {
            "path" => {
                out.insert(k.clone(), expect_field_string(&path, k, v)?);
            }
            "lib" => {
                let v = expect_field_string(&path, k, v)?;
                if v.ends_with(".d.rut") {
                    return Err(ManifestError(format!(
                        "{path}: 'lib' must be a .rut source — a .d.rut decl surface does not gate"
                    )));
                }
                out.insert(k.clone(), v);
            }
            "optional" => {
                out.insert(k.clone(), expect_optional(&path, v)?);
            }
            other => {
                return Err(ManifestError(format!("{path}: unknown key '{other}'")));
            }
        }
    }
    Ok(out)
}

/// `optional` is the one bool the descriptor grammar learns;
/// peers are REQUIRED by default, so the flag must say `true` or
/// `false` exactly. Stored as its canonical spelling.
pub(super) fn expect_optional(path: &str, value: &Value) -> Result<String, ManifestError> {
    match value.as_bool() {
        Some(b) => Ok(b.to_string()),
        None => Err(ManifestError(format!(
            "{path}: 'optional' expects true or false"
        ))),
    }
}
