//! The typed-value laws: what a `key = value` row must BE. Type
//! mismatches keep today's refusals ("expected a quoted string, found
//! `X`", "expected an integer", ...); the TOML spellings are the
//! parser's business, not the laws'.

use toml_edit::Value;

use super::ManifestError;
use super::walk::{line_of, raw};

// ---- the typed values: the laws the manifest itself owns. Type
// mismatches keep today's refusals ("expected a quoted string, found
// `X`", "expected an integer", ...); the TOML spellings are the
// parser's business, not the laws'. ----

/// A `key = "string"` value.
pub(super) fn expect_string(lineno: usize, value: &Value) -> Result<String, ManifestError> {
    match value.as_str() {
        Some(s) => Ok(s.to_string()),
        None => Err(ManifestError(format!(
            "line {lineno}: expected a quoted string, found `{}`",
            raw(value)
        ))),
    }
}

/// A non-negative integer (`format_version`).
pub(super) fn expect_u64(lineno: usize, value: &Value) -> Result<u64, ManifestError> {
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
pub(super) fn expect_string_array(
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

