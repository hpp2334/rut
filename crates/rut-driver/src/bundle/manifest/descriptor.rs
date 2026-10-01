//! The dep tables' descriptor grammars: `[deps]` (one source —
//! `path` or `url` — plus, only beside `url`, the `sha256` pin) and
//! the `[peer-deps]`/`[dev-deps]` shape (`path`, `lib`, `optional`).

use std::collections::BTreeMap;

use toml_edit::{Item, Value};

use super::expect::expect_string;
use super::walk::{flatten, raw};
use super::{valid_spec, ManifestError};

/// A dep-table key is a bare package name — the same charset law as the
/// manifest `name`.
pub(super) fn check_dep_key(key: &str, lineno: usize) -> Result<(), ManifestError> {
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
pub(super) fn parse_deps_descriptor(
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
pub(super) fn parse_pin(lineno: usize, value: &Value) -> Result<String, ManifestError> {
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
pub(super) fn parse_peer_descriptor(
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
pub(super) fn expect_optional(lineno: usize, value: &Value) -> Result<String, ManifestError> {
    match value.as_bool() {
        Some(b) => Ok(b.to_string()),
        None => Err(ManifestError(format!(
            "line {lineno}: `optional` expects `true` or `false`"
        ))),
    }
}
