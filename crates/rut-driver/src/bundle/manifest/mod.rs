//! The manifest grammar — a module manifest (`name`, `type`, +
//! `entry`) or a consumer manifest (`deps`, `peer-deps`,
//! `dev-deps`), or both, parsed into [`Manifest`].
//!
//! **One directory is one module.** Its manifest names the exact
//! package it answers to and how to reach its surface and body:
//!
//! ```json
//! {
//!   "name": "pouch",
//!   "entry": { "type": "./pouch.d.rut", "lib": "./pouch.rut" }
//! }
//! ```
//!
//! (`entry.type` is the surface, `entry.lib` the body — omitted while
//! surface-only.)
//!
//! The declared kind: `"type": "lib"` (the default) is the ordinary
//! source package; `"type": "host"` is a pure declaration surface the
//! embedding Rust binds at run time — a host pkg SPELLS itself, the
//! kind is never inferred.
//!
//! A consumer mounts modules by exact name → directory:
//!
//! ```json
//! { "deps": { "pouch": { "path": "rut/pouch" } } }
//! ```
//!
//! or by url — a packed `.rutbundle` fetched by the host and pinned by
//! its sha256:
//!
//! ```json
//! { "deps": { "pouch": { "url": "https://example.com/pouch.rutbundle",
//!                        "sha256": "<64-hex>" } } }
//! ```
//!
//! Resolution is exact and single-step: a use path resolves only if a
//! module with that `name` is mounted — nothing is derived. Package
//! names are bare `[a-zA-Z0-9_]+` identifiers; a miss points at the
//! consumer manifest (the `deps` object).
//!
//! The dep kinds: `deps` is today's transitively-mounted table;
//! `peer-deps` is REQUIRED by default (the consumer supplies the peer)
//! with `"optional": true` marking the presence-mounted kind whose
//! integration group is the descriptor's `lib` file; `dev-deps` mount
//! only while building the pkg itself (the loader's law — the Session
//! never sees a dev table).
//!
//! The manifest text is standard JSON, parsed with `serde_json`. The
//! syntax lane keeps a location: a malformed file's error carries the
//! parser's own wording under a `line N:` prefix. Everything above the
//! syntax is PATH-TARGETED — a value-law error names the key's path
//! (`deps.pouch: unknown key 'feats'`, `entry: expected a string for
//! 'lib'`), never a line. JSON has no comments, so keys starting with
//! `_` (e.g. `"_comment"`) ride IGNORED in every table — the prose
//! stays in the file; and duplicate keys are last-wins (standard JSON
//! semantics — the grammar adds no machinery).

mod descriptor;
mod expect;
mod walk;

use std::collections::BTreeMap;

use serde_json::Value;
use walk::{ignored, syntax_error, walk_top};

use thiserror::Error;

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
    /// ride `peer-deps` instead)
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

/// A parsed manifest — either a module manifest (`name` + `entry`) or
/// a consumer manifest (`deps`), or both.
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
    /// `deps` — exact specifier → descriptor: exactly one source key,
    /// `path` (a directory) or `url` (a remote `.rutbundle`), plus
    /// `sha256` (the pin) only beside `url`. Kept for the host to
    /// resolve; the Session does not read the filesystem or the network.
    pub deps: BTreeMap<String, BTreeMap<String, String>>,
    /// `peer-deps` — REQUIRED by default; the CONSUMER
    /// supplies the peer, it is never pulled transitively. `optional =
    /// true` marks the presence-mounted kind whose integration group is
    /// the descriptor's `lib` file. Descriptors: `path` (string),
    /// `optional` (bool), `lib` (string) — nothing else.
    pub peer_deps: BTreeMap<String, BTreeMap<String, String>>,
    /// `dev-deps` — mounted ONLY when building/testing
    /// the pkg itself (the program root), never in a consumer's world.
    /// Same descriptor shape as `deps`.
    pub dev_deps: BTreeMap<String, BTreeMap<String, String>>,
    /// `style` — the pkg's formatter knobs (the rut-fmt batch): flat
    /// `"key": "value"` rows read by the fmt crate's consumer, which
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
/// governs manifest `name` values and dep-table keys.
pub fn valid_spec(spec: &str) -> bool {
    !spec.is_empty() && spec.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Parse the manifest: standard JSON (`serde_json`), then this
/// format's value laws — top-level `name`, `type` (the declared kind),
/// the `entry` object, and the dep objects `deps` / `peer-deps` /
/// `dev-deps` whose values are descriptor objects. Syntax errors keep
/// the `line N:` prefix; value laws are path-targeted.
pub fn parse_manifest(text: &str) -> Result<Manifest, ManifestError> {
    let root: Value = serde_json::from_str(text).map_err(|e| syntax_error(&e))?;
    let Some(root) = root.as_object() else {
        return Err(ManifestError("manifest: expected a JSON object".into()));
    };
    let mut m = Manifest::default();
    // was `type` spelled? (the no-inference law keys on it: an absent
    // kind with an `entry.type`-only pkg is the ambiguity)
    let mut declared_type = false;
    for (key, value) in root {
        if ignored(key) {
            continue;
        }
        walk_top(key, value, &mut declared_type, &mut m)?;
    }
    // D4: `peer-deps` + `dev-deps` is the sanctioned
    // both-kinds pairing (the ruling); anything riding `deps` beside
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
    // a host manifest: a host pkg packs as a decl root (its root is
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
    // same refusal `peer-deps` `lib` gets); and no file rides twice
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

#[cfg(test)]
mod tests {
    use super::*;

    const POUCH: &str = r#"
{
  "_comment": "pouch — the growable sequence package: Vec<T> in rut",
  "name": "pouch",
  "entry": { "type": "./pouch.d.rut", "lib": "./pouch.rut" }
}
"#;

    #[test]
    fn parses_name_and_entry() {
        let m = parse_manifest(POUCH).unwrap();
        assert_eq!(m.name.as_deref(), Some("pouch"));
        assert_eq!(m.entry.type_path.as_deref(), Some("./pouch.d.rut"));
        assert_eq!(m.entry.lib.as_deref(), Some("./pouch.rut"));
    }

    #[test]
    fn bundle_manifest_keys() {
        let text = r#"{"format": "rutbundle", "format_version": 1, "name": "x", "entry": {"lib": "./x.rut"}}"#;
        let m = parse_manifest(text).unwrap();
        assert_eq!(m.format.as_deref(), Some("rutbundle"));
        assert_eq!(m.format_version, Some(1));
    }

    #[test]
    fn manifest_names_must_be_bare_package_names() {
        // scoped manifest names are retired — the diagnostic is
        // path-targeted and names the charset
        let err = parse_manifest(r#"{"name": "std:core"}"#).unwrap_err();
        assert_eq!(
            err.to_string(),
            "name: 'std:core' is not a bare package name — expected [a-zA-Z0-9_]+"
        );
        let err = parse_manifest(r#"{"deps": {"std:math": {"path": "rut/calc"}}}"#).unwrap_err();
        assert_eq!(
            err.to_string(),
            "deps: 'std:math' is not a bare package name — expected [a-zA-Z0-9_]+"
        );
    }

    #[test]
    fn deps_parse() {
        let text = r#"{"name": "app", "deps": {"core": {"path": "rut/core"}}}"#;
        let m = parse_manifest(text).unwrap();
        assert_eq!(m.deps.get("core").unwrap().get("path").unwrap(), "rut/core");
    }

    #[test]
    fn pkg_type_defaults_to_lib_and_both_spellings_parse() {
        // absent `type` ⇒ lib — the ordinary source package
        let m = parse_manifest(r#"{"name": "pouch", "entry": {"lib": "./pouch.rut"}}"#).unwrap();
        assert_eq!(m.pkg_type, PkgType::Lib);
        let m = parse_manifest(
            r#"{"name": "pouch", "type": "lib", "entry": {"lib": "./pouch.rut"}}"#,
        )
        .unwrap();
        assert_eq!(m.pkg_type, PkgType::Lib);
        // a host pkg SPELLS itself
        let m = parse_manifest(
            r#"{"name": "ink_host", "type": "host", "entry": {"type": "./ink_host.d.rut"}}"#,
        )
        .unwrap();
        assert_eq!(m.pkg_type, PkgType::Host);
    }

    #[test]
    fn pkg_type_bad_value_is_path_targeted() {
        let err = parse_manifest(r#"{"name": "x", "type": "Host"}"#).unwrap_err();
        assert_eq!(err.to_string(), "type: expected 'lib' or 'host', found 'Host'");
        let err = parse_manifest(r#"{"name": "x", "type": "native"}"#).unwrap_err();
        assert_eq!(err.to_string(), "type: expected 'lib' or 'host', found 'native'");
    }

    #[test]
    fn host_pkg_refuses_every_deps_table() {
        for table in ["deps", "peer-deps", "dev-deps"] {
            let err = parse_manifest(&format!(
                r#"{{"name": "h", "type": "host", "entry": {{"type": "./h.d.rut"}}, "{table}": {{"ink": {{"path": "../ink"}}}}}}"#
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
        let err = parse_manifest(r#"{"name": "h", "type": "host"}"#).unwrap_err();
        assert!(err.to_string().contains("needs `entry.type`"), "{err}");
    }

    #[test]
    fn host_pkg_refuses_a_body() {
        let err = parse_manifest(
            r#"{"name": "h", "type": "host", "entry": {"type": "./h.d.rut", "lib": "./h.rut"}}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("no body"), "{err}");
        // the libs tail is a body too
        let err = parse_manifest(
            r#"{"name": "h", "type": "host", "entry": {"type": "./h.d.rut", "libs": ["./more.rut"]}}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("no body"), "{err}");
    }

    #[test]
    fn host_pkg_takes_the_bundle_root_keys() {
        // the decl-root grammar: `format`/`format_version` are LEGAL on a
        // `type = "host"` manifest — a host pkg packs as a decl root
        let m = parse_manifest(
            r#"{"format": "rutbundle", "format_version": 8, "name": "h", "type": "host", "entry": {"type": "./h.d.rut"}}"#,
        )
        .unwrap();
        assert_eq!(m.pkg_type, PkgType::Host);
        assert_eq!(m.format.as_deref(), Some("rutbundle"));
        assert_eq!(m.format_version, Some(8));
    }

    #[test]
    fn host_bundle_manifest_still_refuses_its_other_laws() {
        // the bundle keys change nothing else: the deps tables, the body
        // refusal, the `entry.type` requirement all still hold WITH the
        // keys present
        for table in ["deps", "peer-deps", "dev-deps"] {
            let err = parse_manifest(&format!(
                r#"{{"format": "rutbundle", "format_version": 8, "name": "h", "type": "host", "entry": {{"type": "./h.d.rut"}}, "{table}": {{"ink": {{"path": "../ink"}}}}}}"#
            ))
            .unwrap_err();
            assert!(err.to_string().contains("a host pkg is pure surface"), "[{table}]: {err}");
        }
        let err = parse_manifest(
            r#"{"format": "rutbundle", "format_version": 8, "name": "h", "type": "host", "entry": {"type": "./h.d.rut", "lib": "./h.rut"}}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("no body"), "{err}");
        let err = parse_manifest(
            r#"{"format": "rutbundle", "format_version": 8, "name": "h", "type": "host"}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("needs `entry.type`"), "{err}");
    }

    #[test]
    fn format_version_must_be_an_integer() {
        // a JSON-legal but non-integer value trips the value law
        let err = parse_manifest(
            r#"{"format": "rutbundle", "format_version": "1", "name": "h", "type": "host", "entry": {"type": "./h.d.rut"}}"#,
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"format_version: expected an integer, found "1""#
        );
        // a float is not an integer either — the JSON spellings are
        // the parser's business, the law only sees u64
        let err = parse_manifest(r#"{"format_version": 1.0}"#).unwrap_err();
        assert_eq!(err.to_string(), "format_version: expected an integer, found 1.0");
    }

    #[test]
    fn syntax_errors_normalize_to_line_n() {
        // the syntax lane keeps the location: serde_json's message
        // under a `line N:` prefix
        let err = parse_manifest("{\n  \"name\": \"x\",\n}\n").unwrap_err();
        let msg = err.to_string();
        assert!(msg.starts_with("line 3: "), "{msg}");
        assert!(msg.contains("trailing comma"), "{msg}");
        let err = parse_manifest("{\"name\": six}").unwrap_err();
        let msg = err.to_string();
        assert!(msg.starts_with("line 1: "), "{msg}");
        assert!(msg.contains("expected value"), "{msg}");
    }

    #[test]
    fn full_json_spellings_parse() {
        // standard JSON: \uXXXX escapes process to their characters,
        // integers ride u64 — the parser processes the spellings, the
        // value laws are unchanged
        let m = parse_manifest(r#"{"name": "pouch", "entry": {"lib": "./p\u006Fuch.rut"}}"#).unwrap();
        assert_eq!(m.name.as_deref(), Some("pouch"));
        assert_eq!(m.entry.lib.as_deref(), Some("./pouch.rut"));
        let m = parse_manifest(
            r#"{"name": "x", "format": "rutbundle", "format_version": 18446744073709551615}"#,
        )
        .unwrap();
        assert_eq!(m.format_version, Some(u64::MAX));
    }

    #[test]
    fn duplicate_keys_are_last_wins() {
        // standard JSON semantics — serde_json keeps the last spelling,
        // the grammar adds no machinery (the TOML duplicate-key refusal
        // retired with the syntax)
        let m = parse_manifest(r#"{"name": "a", "name": "b", "entry": {"lib": "./x.rut"}}"#)
            .unwrap();
        assert_eq!(m.name.as_deref(), Some("b"));
        let m = parse_manifest(
            r#"{"deps": {"p": {"url": "https://x/a.rutbundle", "url": "https://x/b.rutbundle"}}}"#,
        )
        .unwrap();
        assert_eq!(
            m.deps.get("p").unwrap().get("url").unwrap(),
            "https://x/b.rutbundle"
        );
    }

    #[test]
    fn name_type_mismatch_is_path_targeted() {
        // an integer is not a string — the value law's wording, the
        // path names the key
        let err = parse_manifest(r#"{"name": 3}"#).unwrap_err();
        assert_eq!(err.to_string(), "name: expected a string, found 3");
    }

    #[test]
    fn unknown_top_level_keys_and_objects_ride() {
        // forward compatibility, any shape: an unknown scalar, object,
        // or array at the top level is ignored (JSON has no section
        // headers to refuse — the five known objects walk, the rest
        // rides)
        let m = parse_manifest(
            r#"{"name": "x", "entry": {"lib": "./x.rut"}, "whatever": {"a": [1, 2]}, "future": true}"#,
        )
        .unwrap();
        assert_eq!(m.entry.lib.as_deref(), Some("./x.rut"));
    }

    #[test]
    fn the_known_objects_must_be_objects() {
        let err = parse_manifest(r#"{"entry": "./x.rut"}"#).unwrap_err();
        assert_eq!(err.to_string(), "entry: expected an object, found \"./x.rut\"");
        let err = parse_manifest(r#"{"deps": []}"#).unwrap_err();
        assert_eq!(err.to_string(), "deps: expected an object, found []");
        let err = parse_manifest(r#"{"style": "indent=2"}"#).unwrap_err();
        assert_eq!(err.to_string(), "style: expected an object, found \"indent=2\"");
        // and so must the whole document
        let err = parse_manifest("[1, 2]").unwrap_err();
        assert_eq!(err.to_string(), "manifest: expected a JSON object");
    }

    #[test]
    fn entry_type_only_without_a_declared_kind_is_the_ambiguity() {
        // the no-inference law: an entry.type-only pkg spells its kind —
        // old host-pkg manifests fail LOUDLY here, not as empty libs
        let err = parse_manifest(r#"{"name": "rt", "entry": {"type": "./rt.d.rut"}}"#).unwrap_err();
        assert!(
            err.to_string()
                .contains("an `entry.type`-only pkg spells its kind"),
            "{err}"
        );
        assert!(err.to_string().contains("`type = \"host\"`"), "{err}");
        // either fix passes: declare host…
        let m = parse_manifest(
            r#"{"name": "rt", "type": "host", "entry": {"type": "./rt.d.rut"}}"#,
        )
        .unwrap();
        assert_eq!(m.pkg_type, PkgType::Host);
        // …or add the body (an ordinary lib pkg)
        let m = parse_manifest(
            r#"{"name": "dev", "type": "lib", "entry": {"type": "./dev.d.rut", "lib": "./dev.rut"}}"#,
        )
        .unwrap();
        assert_eq!(m.pkg_type, PkgType::Lib);
        // an EXPLICIT `type = "lib"` with a surface and no body is the
        // sanctioned surface-only dev state — spelled, so no ambiguity
        let m = parse_manifest(
            r#"{"name": "dev", "type": "lib", "entry": {"type": "./dev.d.rut"}}"#,
        )
        .unwrap();
        assert_eq!(m.pkg_type, PkgType::Lib);
    }

    #[test]
    fn entry_keys_are_path_targeted() {
        // the plan's canonical shape: the table's path, the key named
        // in the message
        let err = parse_manifest(r#"{"entry": {"lib": 3}}"#).unwrap_err();
        assert_eq!(err.to_string(), "entry: expected a string for 'lib'");
        let err = parse_manifest(r#"{"entry": {"libs": "./a.rut"}}"#).unwrap_err();
        assert_eq!(err.to_string(), "entry: expected a string array for 'libs'");
        let err = parse_manifest(r#"{"entry": {"libs": []}}"#).unwrap_err();
        assert_eq!(
            err.to_string(),
            "entry.libs: cannot be empty — drop the key for a single-file module"
        );
        let err = parse_manifest(r#"{"entry": {"libs": ["./a.rut", 1]}}"#).unwrap_err();
        assert_eq!(err.to_string(), "entry.libs: expected a string, found 1");
        // unknown entry keys refuse (the entry strictness)
        let err = parse_manifest(r#"{"entry": {"lib": "./x.rut", "feats": 1}}"#).unwrap_err();
        assert_eq!(err.to_string(), "entry: unknown key 'feats'");
        // the retired `ir` rides
        let m = parse_manifest(r#"{"name": "x", "entry": {"lib": "./x.rut", "ir": false}}"#)
            .unwrap();
        assert_eq!(m.entry.lib.as_deref(), Some("./x.rut"));
    }

    #[test]
    fn style_rows_are_string_typed() {
        let m = parse_manifest(r#"{"name": "x", "entry": {"lib": "./x.rut"}, "style": {"indent": "2"}}"#)
            .unwrap();
        assert_eq!(m.style.get("indent").map(String::as_str), Some("2"));
        let err = parse_manifest(r#"{"style": {"indent": 2}}"#).unwrap_err();
        assert_eq!(err.to_string(), "style: expected a string for 'indent'");
    }

    #[test]
    fn underscore_prefixed_keys_ride_everywhere() {
        // JSON has no comments — the `_` prefix is the prose lane, in
        // EVERY table (the forward-compat law extended one notch)
        let m = parse_manifest(
            r#"{
                "_comment": "the header prose",
                "name": "x",
                "entry": {"_note": "why", "lib": "./x.rut"},
                "deps": {"_private": "why", "core": {"_hint": "why", "path": "rut/core"}},
                "peer-deps": {"p": {"_hint": "why", "path": "..", "optional": true}}
            }"#,
        )
        .unwrap();
        assert_eq!(m.name.as_deref(), Some("x"));
        assert_eq!(m.entry.lib.as_deref(), Some("./x.rut"));
        assert_eq!(m.deps.get("core").unwrap().get("path").unwrap(), "rut/core");
        assert_eq!(m.peer_deps.get("p").unwrap().get("optional").unwrap(), "true");
    }

    #[test]
    fn host_scope_retires_loudly_and_inline_rides() {
        // `host_scope` (the retired registration-prefix override)
        // refuses at parse, naming the fix — the registration scope is
        // the package name. `inline` (the retired source-inlining flag)
        // still parses as an unknown key: old manifests keep loading,
        // the flag does nothing.
        let err = parse_manifest(
            r#"{"name": "ink_host", "entry": {"type": "./ink_host.d.rut"}, "host_scope": "ink_host"}"#,
        )
        .unwrap_err();
        assert!(err.to_string().starts_with("host_scope: retired"), "{err}");
        assert!(err.to_string().contains("the registration scope is the package name"), "{err}");
        let m = parse_manifest(
            r#"{"name": "ink", "entry": {"lib": "./ink.rut"}, "inline": true}"#,
        )
        .unwrap();
        assert_eq!(m.entry.lib.as_deref(), Some("./ink.rut"));
    }

    // ---- the dep kinds: the three tables, `optional`, the
    // `lib` group key, D4 — the T11 manifest-error shapes ----

    /// The pinned grammar (survey §0), plus the §2.3 `lib` keys.
    const JSON: &str = r#"
{
  "name": "json",
  "entry": { "lib": "./json.rut" },

  "peer-deps": {
    "pouch":   { "path": "../pouch",   "optional": true, "lib": "./serde_pouch.rut" },
    "nmapset": { "path": "../nmapset", "optional": true, "lib": "./serde_nmapset.rut" }
  },

  "dev-deps": {
    "pouch":   { "path": "../pouch" },
    "nmapset": { "path": "../nmapset" }
  }
}
"#;

    #[test]
    fn peer_and_dev_tables_parse() {
        let m = parse_manifest(JSON).unwrap();
        let pouch = m.peer_deps.get("pouch").unwrap();
        assert_eq!(pouch.get("path").unwrap(), "../pouch");
        assert_eq!(pouch.get("optional").unwrap(), "true");
        assert_eq!(pouch.get("lib").unwrap(), "./serde_pouch.rut");
        // `optional` defaults to false — REQUIRED by default
        let req = parse_manifest(
            r#"{"name": "j", "peer-deps": {"nmapset": {"path": "../nmapset"}}}"#,
        )
        .unwrap();
        assert_eq!(req.peer_deps.get("nmapset").unwrap().get("optional"), None);
        // the sanctioned both-kinds pairing parses (pouch in peer + dev)
        assert!(m.dev_deps.contains_key("pouch"));
        assert_eq!(m.dev_deps.get("pouch").unwrap().get("path").unwrap(), "../pouch");
    }

    #[test]
    fn t11_optional_rejected_inside_deps() {
        let err =
            parse_manifest(r#"{"deps": {"pouch": {"path": "../pouch", "optional": true}}}"#)
                .unwrap_err();
        assert_eq!(
            err.to_string(),
            "deps.pouch: 'optional' is a peer-deps attribute — deps has no options"
        );
    }

    #[test]
    fn t11_optional_must_be_a_bool() {
        let err = parse_manifest(
            r#"{"peer-deps": {"pouch": {"path": "..", "optional": "yes"}}}"#,
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "peer-deps.pouch: 'optional' expects true or false"
        );
    }

    #[test]
    fn t11_unknown_descriptor_key_is_path_targeted() {
        let err = parse_manifest(
            r#"{"peer-deps": {"pouch": {"path": "..", "feats": "x"}}}"#,
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "peer-deps.pouch: unknown key 'feats'");
        let err = parse_manifest(
            r#"{"dev-deps": {"pouch": {"path": "..", "git": "x"}}}"#,
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "dev-deps.pouch: unknown key 'git'");
    }

    #[test]
    fn t11_descriptor_must_be_an_object() {
        let err = parse_manifest(r#"{"deps": {"pouch": "../pouch"}}"#).unwrap_err();
        assert_eq!(err.to_string(), "deps.pouch: expected an object, found \"../pouch\"");
    }

    #[test]
    fn t11_lib_must_be_a_rut_source() {
        // a `.d.rut` lib key is a load error: decl surfaces don't gate
        let err = parse_manifest(
            r#"{"peer-deps": {"pouch": {"path": "..", "optional": true, "lib": "./pouch.d.rut"}}}"#,
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "peer-deps.pouch: 'lib' must be a .rut source — a .d.rut decl surface does not gate"
        );
    }

    #[test]
    fn t11_d4_deps_beside_peer_or_dev_is_the_collision() {
        let err = parse_manifest(
            r#"{"name": "j", "deps": {"pouch": {"path": "../pouch"}}, "peer-deps": {"pouch": {"path": "../pouch", "optional": true}}}"#,
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "`pouch` appears in both `[deps]` and `[peer-deps]` — a package is either pulled transitively or required of the consumer, never both"
        );
        let err = parse_manifest(
            r#"{"name": "j", "deps": {"pouch": {"path": "../pouch"}}, "dev-deps": {"pouch": {"path": "../pouch"}}}"#,
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
        let err = parse_manifest(r#"{"peer-deps": {"std:pouch": {"path": ".."}}}"#).unwrap_err();
        assert_eq!(
            err.to_string(),
            "peer-deps: 'std:pouch' is not a bare package name — expected [a-zA-Z0-9_]+"
        );
    }

    // ---- the url source kind: one descriptor, one source; the pin ----

    #[test]
    fn url_dep_parses_and_pin_normalizes_to_lowercase() {
        let m = parse_manifest(
            r#"{"name": "app", "deps": {"pouch": {"url": "https://example.com/pouch.rutbundle", "sha256": "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789"}}}"#,
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
            r#"{"name": "app", "deps": {"pouch": {"url": "http://localhost:8080/pouch.rutbundle"}}}"#,
        )
        .unwrap();
        assert!(m.deps.get("pouch").unwrap().get("sha256").is_none());
    }

    #[test]
    fn url_descriptor_refusal_matrix() {
        // url + path: one descriptor, one source
        let err = parse_manifest(
            r#"{"deps": {"pouch": {"url": "https://x/p.rutbundle", "path": "../pouch"}}}"#,
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "deps.pouch: 'path' and 'url' are both set — one descriptor, one source: a directory or a .rutbundle url, never both"
        );
        // neither
        let err = parse_manifest(r#"{"deps": {"pouch": {}}}"#).unwrap_err();
        assert_eq!(
            err.to_string(),
            "deps.pouch: the descriptor has no source — 'path' for a directory, or 'url' for a packed bundle"
        );
        // sha256 beside path (no url) is meaningless
        let err = parse_manifest(
            r#"{"deps": {"pouch": {"path": "../pouch", "sha256": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"}}}"#,
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "deps.pouch: 'sha256' pins a url — beside 'path' it has no meaning"
        );
        // bad hex
        let err = parse_manifest(
            r#"{"deps": {"pouch": {"url": "https://x/p.rutbundle", "sha256": "nothex"}}}"#,
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "deps.pouch: 'sha256' must be 64 hex digits, found 'nothex'"
        );
        // non-http(s)
        let err =
            parse_manifest(r#"{"deps": {"pouch": {"url": "ftp://x/p.rutbundle"}}}"#).unwrap_err();
        assert_eq!(
            err.to_string(),
            "deps.pouch: 'url' must be an http(s) url, found 'ftp://x/p.rutbundle'"
        );
        // unknown keys keep the entry strictness
        let err = parse_manifest(
            r#"{"deps": {"pouch": {"url": "https://x/p", "feats": "x"}}}"#,
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "deps.pouch: unknown key 'feats'");
    }

    #[test]
    fn peer_tables_refuse_the_url_kind() {
        // `peer-deps` stays path-only — its path is directory-time
        // metadata for the declarer's own build, nothing to fetch
        let err =
            parse_manifest(r#"{"peer-deps": {"pouch": {"url": "https://x/p.rutbundle"}}}"#)
                .unwrap_err();
        assert_eq!(err.to_string(), "peer-deps.pouch: unknown key 'url'");
        let err =
            parse_manifest(r#"{"dev-deps": {"pouch": {"url": "https://x/p.rutbundle"}}}"#)
                .unwrap_err();
        assert_eq!(err.to_string(), "dev-deps.pouch: unknown key 'url'");
    }
}
