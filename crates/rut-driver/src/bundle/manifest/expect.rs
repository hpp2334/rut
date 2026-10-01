//! The typed-value laws: what a key's value must BE. Type mismatches
//! are PATH-TARGETED — the diagnostic names the key's path (`name`,
//! `entry` + the key, `deps.pouch`) instead of a line; only the JSON
//! syntax layer keeps a `line N:` prefix.

use serde_json::Value;

use super::ManifestError;

// ---- the typed values: the laws the manifest itself owns. A type
// mismatch names the key's path (the plan's `entry: expected a string
// for 'lib'` shape); the JSON spellings are the parser's business,
// not the laws'. ----

/// A top-level `key = "string"` value — the key IS the path.
pub(super) fn expect_string(path: &str, value: &Value) -> Result<String, ManifestError> {
    match value.as_str() {
        Some(s) => Ok(s.to_string()),
        None => Err(ManifestError(format!(
            "{path}: expected a string, found {value}"
        ))),
    }
}

/// A non-negative integer (`format_version`).
pub(super) fn expect_u64(path: &str, value: &Value) -> Result<u64, ManifestError> {
    match value.as_u64() {
        Some(v) => Ok(v),
        None => Err(ManifestError(format!(
            "{path}: expected an integer, found {value}"
        ))),
    }
}

/// A string field inside a table — the diagnostic names the TABLE's
/// path and the key (`entry: expected a string for 'lib'`).
pub(super) fn expect_field_string(
    table: &str,
    key: &str,
    value: &Value,
) -> Result<String, ManifestError> {
    match value.as_str() {
        Some(s) => Ok(s.to_string()),
        None => Err(ManifestError(format!(
            "{table}: expected a string for '{key}'"
        ))),
    }
}

/// The `entry.libs` string array. Strict: every element a string (the
/// bare-word and non-string refusals fall out of the same check). An
/// empty array is nobody's multi-lib entry — drop the key.
pub(super) fn expect_field_string_array(
    table: &str,
    key: &str,
    value: &Value,
) -> Result<Vec<String>, ManifestError> {
    let Some(arr) = value.as_array() else {
        return Err(ManifestError(format!(
            "{table}: expected a string array for '{key}'"
        )));
    };
    if arr.is_empty() {
        return Err(ManifestError(format!(
            "{table}.{key}: cannot be empty — drop the key for a single-file module"
        )));
    }
    let mut out = Vec::new();
    for el in arr {
        match el.as_str() {
            Some(s) => out.push(s.to_string()),
            None => {
                return Err(ManifestError(format!(
                    "{table}.{key}: expected a string, found {el}"
                )));
            }
        }
    }
    Ok(out)
}
