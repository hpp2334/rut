//! The dep tables' descriptor grammars: `deps` (one source — `path` or
//! `url`) and `dev-deps` (`path` only). All diagnostics are
//! PATH-TARGETED: the descriptor's path (`deps.pouch`) prefixes every
//! refusal.

use std::collections::BTreeMap;

use serde_json::Value;

use super::expect::expect_field_string;
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
/// `url` (a remote `.rutbundle`), never both, never neither. The
/// `sha256` pin is RETIRED from the grammar — pins live in `rut.lock`
/// — so the key refuses loudly (the `host_scope` treatment: a silent
/// ignore would surface as an unpinned fetch later). `optional` is
/// rejected (not a deps key — the strict-keys law); any other key is
/// the entry strictness — a path-targeted error.
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
            // the retired prose lane — loud, never a silent ignore
            return Err(super::walk::underscore_refused(&path, k));
        }
        match k.as_str() {
            "path" | "url" => {
                out.insert(k.clone(), expect_field_string(&path, k, v)?);
            }
            "sha256" => {
                return Err(ManifestError(format!(
                    "{path}: 'sha256' retired — pins live in rut.lock (run `rut install`)"
                )));
            }
            "optional" => {
                return Err(ManifestError(format!(
                    "{path}: 'optional' is not a deps-descriptor key"
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
    }
    Ok(out)
}

/// A `dev-deps` descriptor: `path` is the one key — dev presence is
/// name-based at the root (the dev pass mounts the table it sees), so
/// the descriptor carries no flags and no group key (`lib`/`optional`
/// were the peer machinery's bits and died with `peer-deps`). Any
/// other key is the entry strictness — a path-targeted error.
pub(super) fn parse_dev_descriptor(
    table: &str,
    spec: &str,
    value: &Value,
) -> Result<BTreeMap<String, String>, ManifestError> {
    let path = format!("{table}.{spec}");
    let obj = expect_object(&path, value)?;
    let mut out = BTreeMap::new();
    for (k, v) in obj {
        if k.starts_with('_') {
            // the retired prose lane — loud, never a silent ignore
            return Err(super::walk::underscore_refused(&path, k));
        }
        match k.as_str() {
            "path" => {
                out.insert(k.clone(), expect_field_string(&path, k, v)?);
            }
            other => {
                return Err(ManifestError(format!("{path}: unknown key '{other}'")));
            }
        }
    }
    Ok(out)
}
