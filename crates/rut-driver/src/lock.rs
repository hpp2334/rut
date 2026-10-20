//! rut.lock — the pin ledger beside a walk root's `rut.jsonc`. One
//! lock per root; the url rows a walk collects are pinned HERE, not in
//! the manifest (the `deps` url rows lost their `sha256` — the key is
//! retired from the descriptor grammar).
//!
//! The file is MACHINE-GENERATED (`rut install`) and byte-
//! deterministic: rows sorted by url, fixed field order
//! (`pkg`/`url`/`sha256`), 2-space indent, the header comment, a
//! trailing newline — the same closure renders the same bytes, so the
//! file diffs cleanly and commits without churn. The url is the
//! identity (the fetch key); `pkg` is display-only (the first spec in
//! collect order that named the url): the same name under two urls is
//! two rows, the same url under two names is one.
//!
//! Parsing goes through the manifest's JSONC front stage
//! ([`jsonc::strip`], the position law lives there) — comments and
//! trailing commas are legal, so hand-edits still load — then
//! `serde_json`, then the value laws, PATH-TARGETED like the
//! manifest's: `deps[1].sha256: must be 64 hex digits…`, hex stored
//! lowercase (uppercase input normalizes), a duplicate url across rows
//! refused, `format` must be `"rutlock"`, `format_version` must be the
//! exact known version. Unknown keys ride — forward compatibility, the
//! manifest's rule.
//!
//! [`jsonc::strip`]: crate::bundle::manifest::jsonc::strip

use std::collections::BTreeSet;

use serde_json::Value;
use thiserror::Error;

use crate::bundle::manifest::jsonc::strip;
use crate::bundle::manifest::walk::syntax_error;

/// The only format spell the grammar knows.
pub const FORMAT: &str = "rutlock";

/// The only format_version the grammar knows — an exact match, the
/// same forward-compat story as every versioned key: a NEW version
/// refuses loudly until this grammar learns it.
pub const FORMAT_VERSION: u64 = 1;

/// A malformed rut.lock — [`ManifestError`]'s twin: a load error,
/// never a runtime trap; one line, path-targeted.
///
/// [`ManifestError`]: crate::bundle::manifest::ManifestError
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct LockError(pub String);

/// One lock row: the url is the identity, the sha256 is the law at
/// the mount door, and `pkg` is display-only (the first spec in
/// collect order that named the url — never the fetch key).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockRow {
    pub pkg: String,
    pub url: String,
    pub sha256: String,
}

/// A parsed rut.lock — the rows exactly as the file lists them
/// ([`render_lock`] is what sorts by url).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LockFile {
    deps: Vec<LockRow>,
}

impl LockFile {
    /// Assemble a lock from collected rows — the `rut install` side
    /// (the walk's closure, deduped by url). Rendering sorts by url;
    /// the rows' order here is not load-bearing.
    pub fn from_rows(rows: Vec<LockRow>) -> Self {
        Self { deps: rows }
    }

    /// The rows, in file order.
    pub fn deps(&self) -> &[LockRow] {
        &self.deps
    }
}

/// The "found X" text of the value refusals: the value as JSON spells
/// it, or `nothing` when the key is absent.
fn found(value: Option<&Value>) -> String {
    match value {
        Some(v) => v.to_string(),
        None => "nothing".into(),
    }
}

/// Parse the lock: **JSONC** syntax (comments + trailing commas,
/// stripped before the parser sees them — the positions never move),
/// then standard JSON (`serde_json`), then the value laws. Syntax
/// errors keep the `line N:` prefix (the ORIGINAL file's line); the
/// value laws are path-targeted.
pub fn parse_lock(text: &str) -> Result<LockFile, LockError> {
    let plain = strip(text);
    let root: Value = serde_json::from_str(&plain)
        .map_err(|e| LockError(syntax_error(&e).0))?;
    let Some(root) = root.as_object() else {
        return Err(LockError("rut.lock: expected a JSON object".into()));
    };
    // the format gates: an exact spell and an exact version — the
    // lock is machine-generated, so anything else is a hand-edit or a
    // future file this grammar must refuse loudly, not misread
    match root.get("format") {
        Some(Value::String(s)) if s == FORMAT => {}
        other => {
            return Err(LockError(format!(
                "format: expected '{FORMAT}', found {}",
                found(other)
            )));
        }
    }
    match root.get("format_version").and_then(Value::as_u64) {
        Some(FORMAT_VERSION) => {}
        Some(other) => {
            return Err(LockError(format!(
                "format_version: expected {FORMAT_VERSION}, found {other}"
            )));
        }
        None => {
            return Err(LockError(format!(
                "format_version: expected an integer, found {}",
                found(root.get("format_version"))
            )));
        }
    }
    let Some(rows) = root.get("deps").and_then(Value::as_array) else {
        return Err(LockError(format!(
            "deps: expected an array, found {}",
            found(root.get("deps"))
        )));
    };
    let mut out = Vec::with_capacity(rows.len());
    let mut seen = BTreeSet::new();
    for (i, row) in rows.iter().enumerate() {
        let path = format!("deps[{i}]");
        let Some(obj) = row.as_object() else {
            return Err(LockError(format!(
                "{path}: expected an object, found {}",
                found(Some(row))
            )));
        };
        // the three known fields; every other key rides (forward
        // compatibility, the manifest's rule)
        let pkg = field_string(&path, obj, "pkg")?;
        let url = field_string(&path, obj, "url")?;
        // the url is the identity: one url, one row — a duplicate is
        // a corrupt or hand-mangled lock, refused at parse
        if !seen.insert(url.clone()) {
            return Err(LockError(format!(
                "{path}: url '{url}' appears twice — the url is the identity: one url, one row"
            )));
        }
        let sha256 = pin(&path, obj)?;
        out.push(LockRow { pkg, url, sha256 });
    }
    Ok(LockFile { deps: out })
}

/// A row's string field — the diagnostic names the row's path and the
/// key (`deps[1]: expected a string for 'pkg'`), the manifest's
/// `expect_field_string` shape.
fn field_string(path: &str, obj: &serde_json::Map<String, Value>, key: &str) -> Result<String, LockError> {
    let Some(v) = obj.get(key) else {
        return Err(LockError(format!("{path}: missing '{key}'")));
    };
    match v.as_str() {
        Some(s) => Ok(s.to_string()),
        None => Err(LockError(format!(
            "{path}: expected a string for '{key}'"
        ))),
    }
}

/// The row's pin: exactly 64 hex digits, stored LOWERCASE — the same
/// law the manifest's compat lane keeps, and the law the mount door
/// compares the fetched bytes' hash against (byte-exact, so the parse
/// normalizes first).
fn pin(path: &str, obj: &serde_json::Map<String, Value>) -> Result<String, LockError> {
    let v = field_string(path, obj, "sha256")?;
    if v.len() != 64 || !v.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(LockError(format!(
            "{path}.sha256: must be 64 hex digits, found '{v}'"
        )));
    }
    Ok(v.to_ascii_lowercase())
}

/// Render the lock: the header comment, the format gates, the rows
/// sorted by url — each row one line, fields in the fixed order
/// `pkg`/`url`/`sha256`, the pkg and url columns aligned, 2-space
/// indent, trailing newline. Deterministic: the same closure renders
/// byte-identical text.
pub fn render_lock(lock: &LockFile) -> String {
    let mut rows: Vec<&LockRow> = lock.deps.iter().collect();
    rows.sort_by(|a, b| a.url.cmp(&b.url));
    // the aligned columns: each token is quoted and comma-carrying, so
    // a column's pad is the widest content plus the quotes and comma
    let pkg_w = rows.iter().map(|r| r.pkg.len() + 3).max().unwrap_or(0);
    let url_w = rows.iter().map(|r| r.url.len() + 3).max().unwrap_or(0);
    let mut out = String::from(
        "// rut.lock — generated by `rut install`; commit it. Rows sorted by url;\n\
         // the sha256 is law at the mount door on every load.\n\
         {\n\
         \x20 \"format\": \"rutlock\",\n\
         \x20 \"format_version\": 1,\n",
    );
    if rows.is_empty() {
        out.push_str("  \"deps\": []\n");
    } else {
        out.push_str("  \"deps\": [\n");
        let last = rows.len() - 1;
        for (i, row) in rows.iter().enumerate() {
            let comma = if i == last { "" } else { "," };
            out.push_str(&format!(
                "    {{ \"pkg\": {:<pw$} \"url\": {:<uw$} \"sha256\": \"{}\" }}{comma}\n",
                format!("\"{}\",", row.pkg),
                format!("\"{}\",", row.url),
                row.sha256,
                comma = comma,
                pw = pkg_w,
                uw = url_w,
            ));
        }
        out.push_str("  ]\n");
    }
    out.push('}');
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const INK_SHA: &str = "4197b14904bd9a5d8b0995d86b7d1b1e50d80e9cb1a1c1e6e2a5e8e9c0d1b2a3";
    const POUCH_SHA: &str = "2f6e6a20c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e";

    fn ink_row() -> LockRow {
        LockRow {
            pkg: "ink".into(),
            url: "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v12/dist/std/ink.rutbundle".into(),
            sha256: INK_SHA.into(),
        }
    }

    fn pouch_row() -> LockRow {
        LockRow {
            pkg: "pouch".into(),
            url: "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v12/dist/std/pouch.rutbundle".into(),
            sha256: POUCH_SHA.into(),
        }
    }

    /// The plan's rendered shape, verbatim (the header, the format
    /// gates, the aligned rows).
    const RENDERED: &str = "\
// rut.lock — generated by `rut install`; commit it. Rows sorted by url;
// the sha256 is law at the mount door on every load.
{
  \"format\": \"rutlock\",
  \"format_version\": 1,
  \"deps\": [
    { \"pkg\": \"ink\",   \"url\": \"https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v12/dist/std/ink.rutbundle\",   \"sha256\": \"4197b14904bd9a5d8b0995d86b7d1b1e50d80e9cb1a1c1e6e2a5e8e9c0d1b2a3\" },
    { \"pkg\": \"pouch\", \"url\": \"https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v12/dist/std/pouch.rutbundle\", \"sha256\": \"2f6e6a20c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e\" }
  ]
}
";

    // ---- the value laws ----

    #[test]
    fn the_plain_lock_parses() {
        let lock = parse_lock(RENDERED).expect("the plan's shape parses");
        assert_eq!(lock.deps().len(), 2);
        assert_eq!(lock.deps()[0].pkg, "ink");
        assert_eq!(lock.deps()[0].sha256, INK_SHA);
        assert_eq!(lock.deps()[1].pkg, "pouch");
    }

    #[test]
    fn hex_must_be_64_digits_and_the_error_is_path_targeted() {
        let text = format!(
            r#"{{"format": "rutlock", "format_version": 1, "deps": [{{ "pkg": "pouch", "url": "https://x/p.rutbundle", "sha256": "nothex" }}]}}"#
        );
        let err = parse_lock(&text).unwrap_err();
        assert_eq!(
            err.to_string(),
            "deps[0].sha256: must be 64 hex digits, found 'nothex'"
        );
        // 63 digits is not 64
        let text = format!(
            r#"{{"format": "rutlock", "format_version": 1, "deps": [{{ "pkg": "pouch", "url": "https://x/p.rutbundle", "sha256": "{}" }}]}}"#,
            &INK_SHA[1..]
        );
        let err = parse_lock(&text).unwrap_err();
        assert!(err.to_string().starts_with("deps[0].sha256: must be 64 hex digits"), "{err}");
        // a non-string is not a pin either
        let err = parse_lock(
            r#"{"format": "rutlock", "format_version": 1, "deps": [{"pkg": "p", "url": "https://x/p", "sha256": 3}]}"#,
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "deps[0]: expected a string for 'sha256'");
        // and the pin is not optional — every row pins its url
        let err = parse_lock(
            r#"{"format": "rutlock", "format_version": 1, "deps": [{"pkg": "p", "url": "https://x/p"}]}"#,
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "deps[0]: missing 'sha256'");
    }

    #[test]
    fn uppercase_hex_normalizes_to_lowercase() {
        // the pin law compares byte-exactly — the parse normalizes first
        let text = format!(
            r#"{{"format": "rutlock", "format_version": 1, "deps": [{{ "pkg": "pouch", "url": "https://x/p.rutbundle", "sha256": "{}" }}]}}"#,
            INK_SHA.to_ascii_uppercase()
        );
        let lock = parse_lock(&text).unwrap();
        assert_eq!(lock.deps()[0].sha256, INK_SHA);
    }

    #[test]
    fn a_duplicate_url_refuses() {
        // the url is the identity: one url, one row — the same url
        // under two names is ONE row (the collect law), so two rows
        // naming it is a corrupt lock
        let text = format!(
            r#"{{"format": "rutlock", "format_version": 1, "deps": [
              {{ "pkg": "ink", "url": "https://x/p.rutbundle", "sha256": "{INK_SHA}" }},
              {{ "pkg": "pigment", "url": "https://x/p.rutbundle", "sha256": "{POUCH_SHA}" }}
            ]}}"#
        );
        let err = parse_lock(&text).unwrap_err();
        assert_eq!(
            err.to_string(),
            "deps[1]: url 'https://x/p.rutbundle' appears twice — the url is the identity: one url, one row"
        );
    }

    #[test]
    fn the_format_gates_are_exact() {
        // the missing-key spellings are real documents with the key absent
        let err = parse_lock(r#"{"format": "rutlock", "deps": []}"#).unwrap_err();
        assert_eq!(
            err.to_string(),
            "format_version: expected an integer, found nothing"
        );
        let err = parse_lock(r#"{"format_version": 1, "deps": []}"#).unwrap_err();
        assert_eq!(err.to_string(), "format: expected 'rutlock', found nothing");
        let base = |format: &str, version: &str| {
            format!(
                r#"{{"format": {format}, "format_version": {version}, "deps": []}}"#
            )
        };
        let err = parse_lock(&base("\"rutlock2\"", "1")).unwrap_err();
        assert_eq!(err.to_string(), "format: expected 'rutlock', found \"rutlock2\"");
        let err = parse_lock(&base("null", "1")).unwrap_err();
        assert_eq!(err.to_string(), "format: expected 'rutlock', found null");
        let err = parse_lock(&base("\"rutlock\"", "\"1\"")).unwrap_err();
        assert_eq!(
            err.to_string(),
            "format_version: expected an integer, found \"1\""
        );
        let err = parse_lock(&base("\"rutlock\"", "2")).unwrap_err();
        assert_eq!(err.to_string(), "format_version: expected 1, found 2");
        let err = parse_lock(&base("\"rutlock\"", "1.0")).unwrap_err();
        assert_eq!(
            err.to_string(),
            "format_version: expected an integer, found 1.0"
        );
    }

    #[test]
    fn unknown_keys_ride() {
        // forward compatibility, the manifest's rule — at every level
        let text = format!(
            r#"{{"format": "rutlock", "format_version": 1, "future": {{"a": [1, 2]}}, "deps": [
              {{ "pkg": "ink", "url": "https://x/i.rutbundle", "sha256": "{INK_SHA}", "fetched": "2026-10-12" }}
            ]}}"#
        );
        let lock = parse_lock(&text).expect("unknown keys ride");
        assert_eq!(lock.deps()[0].pkg, "ink");
    }

    #[test]
    fn the_known_shapes_are_typed() {
        // the root, the rows array, each row
        let err = parse_lock("[1, 2]").unwrap_err();
        assert_eq!(err.to_string(), "rut.lock: expected a JSON object");
        let err = parse_lock(r#"{"format": "rutlock", "format_version": 1, "deps": {}}"#)
            .unwrap_err();
        assert_eq!(err.to_string(), "deps: expected an array, found {}");
        let err = parse_lock(
            r#"{"format": "rutlock", "format_version": 1, "deps": ["pouch"]}"#,
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "deps[0]: expected an object, found \"pouch\"");
        let err = parse_lock(
            r#"{"format": "rutlock", "format_version": 1, "deps": [{"url": "https://x/p", "sha256": "abcdefgh"}]}"#,
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "deps[0]: missing 'pkg'");
        let err = parse_lock(
            r#"{"format": "rutlock", "format_version": 1, "deps": [{"pkg": 3, "url": "https://x/p"}]}"#,
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "deps[0]: expected a string for 'pkg'");
        let err = parse_lock(
            r#"{"format": "rutlock", "format_version": 1, "deps": [{"pkg": "p"}]}"#,
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "deps[0]: missing 'url'");
    }

    // ---- the JSONC front stage ----

    #[test]
    fn jsonc_comments_and_trailing_commas_parse() {
        // the lock parses through the SAME front stage as the manifest:
        // hand-edits keep their prose
        let text = format!(
            "// the std set, pinned\n{{\n  // the gates\n  \"format\": \"rutlock\", /* mid */\n  \"format_version\": 1,\n  \"deps\": [\n    {{ \"pkg\": \"ink\", \"url\": \"https://x/i.rutbundle\", \"sha256\": \"{INK_SHA}\", }}, // trailing prose\n  ],\n}}"
        );
        let lock = parse_lock(&text).expect("JSONC leniency");
        assert_eq!(lock.deps()[0].pkg, "ink");
    }

    #[test]
    fn syntax_errors_name_the_original_line() {
        // the position law: a comment above the mistake does not move
        // the line serde_json reports
        let text = "{\n  // the header\n  \"format\": six,\n}";
        let err = parse_lock(text).unwrap_err();
        let msg = err.to_string();
        assert!(msg.starts_with("line 3: "), "{msg}");
        assert!(msg.contains("expected value"), "{msg}");
    }

    // ---- the renderer ----

    #[test]
    fn render_matches_the_plans_shape_byte_for_byte() {
        let lock = LockFile::from_rows(vec![pouch_row(), ink_row()]);
        assert_eq!(render_lock(&lock), RENDERED);
    }

    #[test]
    fn render_is_deterministic() {
        // rows sorted by url — insertion order is not load-bearing;
        // rendering twice is byte-identical
        let a = LockFile::from_rows(vec![pouch_row(), ink_row()]);
        let b = LockFile::from_rows(vec![ink_row(), pouch_row()]);
        assert_eq!(render_lock(&a), render_lock(&b));
        assert_eq!(render_lock(&a), render_lock(&a));
    }

    #[test]
    fn render_an_empty_lock() {
        let out = render_lock(&LockFile::default());
        assert_eq!(
            out,
            "// rut.lock — generated by `rut install`; commit it. Rows sorted by url;\n// the sha256 is law at the mount door on every load.\n{\n  \"format\": \"rutlock\",\n  \"format_version\": 1,\n  \"deps\": []\n}\n"
        );
    }

    #[test]
    fn parse_render_round_trip() {
        // a canonical lock is a fixed point of its own grammar:
        // parse(render(f)) == f, and the round trip re-renders to the
        // same bytes
        let lock = LockFile::from_rows(vec![ink_row(), pouch_row()]);
        let text = render_lock(&lock);
        let reparsed = parse_lock(&text).expect("the render parses");
        assert_eq!(reparsed, lock);
        assert_eq!(render_lock(&reparsed), text);
        // a NON-canonical lock parses to its file order; rendering is
        // what sorts, so the pair parse∘render normalizes it
        let shuffled = LockFile::from_rows(vec![pouch_row(), ink_row()]);
        assert_eq!(render_lock(&parse_lock(&render_lock(&shuffled)).unwrap()), render_lock(&lock));
    }

    #[test]
    fn parse_accepts_hand_edits_and_renders_the_canonical_shape() {
        // a hand-edited lock (different order, unaligned, commented)
        // parses, and re-rendering snaps it to the canonical bytes
        let hand = format!(
            "// my edits\n{{\"deps\": [{{ \"url\": \"https://x/p.rutbundle\", \"pkg\": \"pouch\", \"sha256\": \"{POUCH_SHA}\" }}, {{ \"pkg\": \"ink\", \"url\": \"https://x/i.rutbundle\", \"sha256\": \"{INK_SHA}\" }}], \"format\": \"rutlock\", \"format_version\": 1}}"
        );
        let lock = parse_lock(&hand).unwrap();
        let out = render_lock(&lock);
        // rows sorted by url: https://x/i… sorts before https://x/p…
        let sorted = parse_lock(&out).expect("the render parses");
        assert_eq!(sorted.deps()[0].pkg, "ink");
        assert_eq!(sorted.deps()[1].pkg, "pouch");
        // and the canonical bytes are a fixed point
        assert_eq!(render_lock(&sorted), out);
    }
}
