//! The manifest grammar — a module manifest (`name`, `type`, +
//! `entry`) or a consumer manifest (`deps`, `peer-deps`,
//! `dev-deps`), or both, parsed into [`Manifest`].
//!
//! **One directory is one module.** Its manifest names the exact
//! package it answers to and how to reach its surface:
//!
//! ```json
//! {
//!   "name": "calc",
//!   "type": "host",
//!   "entry": { "type": "./calc.d.rut" }
//! }
//! ```
//!
//! (`entry.type` is the surface — the ONLY entry key left. The body is
//! `mod.rut` beside the manifest, the package's root module; its `mod`
//! declarations mount the child tree. The repealed `entry.lib` /
//! `entry.libs` keys refuse loudly, naming the fix — except inside a
//! PUBLISHED bundle, whose manifest is forever old: the reader's
//! compat lane parses them onto [`Manifest::legacy_entry`] so the
//! pinned CDN tags keep loading. That is a loading law, not a
//! transition.)
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
//! The manifest text is **JSONC** — `//` line comments, `/* */` block
//! comments, and trailing commas are all legal — parsed by
//! `serde_json` behind a syntax-stripping front stage
//! ([`jsonc::strip`], the position law lives there): the comment and
//! comma bytes become spaces, `serde_json` sees plain JSON, and a
//! malformed file's error keeps the parser's own wording under a
//! `line N:` prefix that names the ORIGINAL file's line. Everything
//! above the syntax is PATH-TARGETED — a value-law error names the
//! key's path (`deps.pouch: unknown key 'feats'`, `entry: expected a
//! string for 'lib'`), never a line. The old `_`-prefixed prose lane
//! (`"_comment"`) RETIRED with the JSONC cutover — an `_`-key refuses
//! loudly naming the fix; and duplicate keys are last-wins (standard
//! JSON semantics — the grammar adds no machinery).

mod descriptor;
mod expect;
mod jsonc;
mod walk;

use std::collections::BTreeMap;

use serde_json::Value;
use walk::{syntax_error, underscore_refused, walk_top_lane};

use thiserror::Error;

/// A module's entry: the `.d.rut` surface path, relative to the module
/// directory — the ONLY entry key the grammar keeps. The body is
/// `mod.rut` beside the manifest (the root module), never a manifest
/// key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    /// `.d.rut` surface path — the declaration/type half
    pub type_path: Option<String>,
}

/// The REPEALED entry keys, as an old published bundle's manifest
/// spells them. Reader compat for the flat envelope — a loading law,
/// not a transition: the pinned CDN tags are forever old, so a
/// bundle's own manifest parses through the compat lane and these
/// fields carry the keys. The directory grammar refuses both keys
/// loudly, and nothing NEW may spell them (a directory mount never
/// fills this).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LegacyEntry {
    /// the old body path — a `.rut` source
    pub lib: Option<String>,
    /// additional `.rut` body files, spliced after `lib` in listed
    /// order — ONE module, one namespace (the old multi-lib entry)
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
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Manifest {
    pub name: Option<String>,
    /// `type = "lib" | "host"` — the declared kind; absent ⇒ [`PkgType::Lib`]
    pub pkg_type: PkgType,
    /// was `type` spelled? The no-inference law keys on it: an
    /// `entry.type`-only pkg with no declared kind and no `mod.rut`
    /// body is the ambiguity (the loader lanes refuse, naming both
    /// fixes — the manifest text alone cannot see the file)
    pub type_declared: bool,
    pub entry: Entry,
    /// the repealed entry keys, reader compat for OLD published
    /// bundles (see [`LegacyEntry`]) — never filled by a directory
    /// parse
    pub legacy_entry: LegacyEntry,
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
    /// `namespace` — the qualified-access head (`calc`'s `Math`): the
    /// manifest's spelling of what the surface grammar cannot say.
    /// The walk rides it onto the pkg; `use calc::{Math}` binds it.
    pub namespace: Option<String>,
    /// `consts` — the host body's compiler-materialized constants
    /// (name → f64; `calc`'s `Math.PI` family). The surface grammar
    /// has no `static` field form, so the manifest is their only
    /// spelling.
    pub consts: BTreeMap<String, f64>,
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

/// The top-level shape of the `_`-key retirement: the path IS the key.
fn underscore_refused_top(key: &str) -> ManifestError {
    underscore_refused("manifest", key)
}

/// Parse the manifest: **JSONC** syntax (comments + trailing commas,
/// stripped by [`jsonc::strip`] before the parser sees them — the
/// positions never move), then standard JSON (`serde_json`), then this
/// format's value laws — top-level `name`, `type` (the declared kind),
/// the `entry` object, and the dep objects `deps` / `peer-deps` /
/// `dev-deps` whose values are descriptor objects. Syntax errors keep
/// the `line N:` prefix (the ORIGINAL file's line); value laws are
/// path-targeted.
///
/// This is the STRICT lane — the directory grammar: `entry.lib` /
/// `entry.libs` refuse loudly. The bundle reader parses old published
/// manifests through [`parse_manifest_compat`].
pub fn parse_manifest(text: &str) -> Result<Manifest, ManifestError> {
    parse_manifest_lane(text, false)
}

/// The bundle reader's compat lane: the same grammar, but a manifest
/// riding INSIDE an archive may be OLD — a published bundle is forever
/// old (the pinned CDN tags), so the repealed `entry.lib`/`entry.libs`
/// keys parse onto [`Manifest::legacy_entry`] under the old value laws
/// instead of refusing. Nothing else relaxes: the same tables, the
/// same value laws, the same loud shapes.
pub fn parse_manifest_compat(text: &str) -> Result<Manifest, ManifestError> {
    parse_manifest_lane(text, true)
}

fn parse_manifest_lane(text: &str, compat: bool) -> Result<Manifest, ManifestError> {
    let plain = jsonc::strip(text);
    let root: Value = serde_json::from_str(&plain).map_err(|e| syntax_error(&e))?;
    let Some(root) = root.as_object() else {
        return Err(ManifestError("manifest: expected a JSON object".into()));
    };
    let mut m = Manifest::default();
    // was `type` spelled? (the no-inference law keys on it: an absent
    // kind with an `entry.type`-only pkg is the ambiguity)
    let mut declared_type = false;
    for (key, value) in root {
        if key.starts_with('_') {
            // the retired prose lane — loud, never a silent ignore
            return Err(underscore_refused_top(key));
        }
        walk_top_lane(key, value, &mut declared_type, &mut m, compat)?;
    }
    m.type_declared = declared_type;
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
    // `entry.type`--only lib spelling an error — spell the kind, either
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
            // the body refusal: post-repeal the body is `mod.rut`
            // (filesystem — the loader lanes probe it), so the
            // parse-level law keys on the COMPAT keys only (an old
            // host manifest spelling a body refuses here; a new one is
            // refused at the mount door when a `mod.rut` sits beside it)
            if m.legacy_entry.lib.is_some() || !m.legacy_entry.libs.is_empty() {
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
            // the no-inference law, COMPAT shape: an old `entry.type` +
            // no-lib spelling with no declared kind is the ambiguity —
            // spell it. (The post-repeal grammar cannot see the body
            // from manifest text — `mod.rut` is a file — so the strict
            // lane's ambiguity law lives at the mount doors, which
            // probe the file.)
            if compat
                && !declared_type
                && m.entry.type_path.is_some()
                && m.legacy_entry.lib.is_none()
            {
                return Err(ManifestError(
                    "an `entry.type`-only pkg spells its kind — `type = \"host\"` for a \
                     host pkg, or add a body (a `type = \"lib\"` surface-only pkg \
                     is the dev state)"
                        .into(),
                ));
            }
        }
    }
    // The old multi-lib array law, compat only: `libs` is an ordered
    // tail on the base `lib`, so the base must exist; every element is
    // a `.rut` source (a `.d.rut` is a decl surface, not a body — the
    // same refusal `peer-deps` `lib` gets); and no file rides twice —
    // the splice would duplicate every name in it.
    if compat && !m.legacy_entry.libs.is_empty() {
        if m.legacy_entry.lib.is_none() {
            return Err(ManifestError(format!(
                "`entry.libs` needs `entry.lib` — the multi-lib entry is a base file plus an ordered tail"
            )));
        }
        let base = m.legacy_entry.lib.as_deref().expect("checked just above");
        let mut seen = vec![base.to_string()];
        for lib in &m.legacy_entry.libs {
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
// pouch — the growable sequence package: Vec<T> in rut
{
  "name": "pouch"
}
"#;

    #[test]
    fn parses_name_and_entry() {
        // the post-repeal shape: the body is `mod.rut` beside the
        // manifest — the manifest names the pkg, spells its kind and
        // (for a host pkg) its surface. A lib pkg like pouch carries no
        // entry keys at all.
        let m = parse_manifest(POUCH).unwrap();
        assert_eq!(m.name.as_deref(), Some("pouch"));
        assert_eq!(m.entry.type_path, None);
        assert_eq!(m.pkg_type, PkgType::Lib);
        assert!(!m.type_declared);
    }

    #[test]
    fn bundle_manifest_keys() {
        let text = r#"{"format": "rutbundle", "format_version": 10, "name": "x"}"#;
        let m = parse_manifest(text).unwrap();
        assert_eq!(m.format.as_deref(), Some("rutbundle"));
        assert_eq!(m.format_version, Some(10));
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
        let m = parse_manifest(r#"{"name": "pouch"}"#).unwrap();
        assert_eq!(m.pkg_type, PkgType::Lib);
        let m = parse_manifest(r#"{"name": "pouch", "type": "lib"}"#).unwrap();
        assert_eq!(m.pkg_type, PkgType::Lib);
        assert!(m.type_declared);
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
        // the body refusal lives on the COMPAT lane: a published
        // bundle's old host manifest spelling a body refuses at parse
        let err = parse_manifest_compat(
            r#"{"name": "h", "type": "host", "entry": {"type": "./h.d.rut", "lib": "./h.rut"}}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("no body"), "{err}");
        // the libs tail is a body too
        let err = parse_manifest_compat(
            r#"{"name": "h", "type": "host", "entry": {"type": "./h.d.rut", "libs": ["./more.rut"]}}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("no body"), "{err}");
        // post-repeal, a NEW host manifest cannot spell a body at all —
        // the keys refuse before the host law fires
        let err = parse_manifest(
            r#"{"name": "h", "type": "host", "entry": {"type": "./h.d.rut", "lib": "./h.rut"}}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("entry.lib: repealed"), "{err}");
    }

    #[test]
    fn host_pkg_takes_the_bundle_root_keys() {
        // the decl-root grammar: `format`/`format_version` are LEGAL on a
        // `type = "host"` manifest — a host pkg packs as a decl root
        let m = parse_manifest(
            r#"{"format": "rutbundle", "format_version": 10, "name": "h", "type": "host", "entry": {"type": "./h.d.rut"}}"#,
        )
        .unwrap();
        assert_eq!(m.pkg_type, PkgType::Host);
        assert_eq!(m.format.as_deref(), Some("rutbundle"));
        assert_eq!(m.format_version, Some(10));
    }

    #[test]
    fn host_bundle_manifest_still_refuses_its_other_laws() {
        // the bundle keys change nothing else: the deps tables, the body
        // refusal, the `entry.type` requirement all still hold WITH the
        // keys present
        for table in ["deps", "peer-deps", "dev-deps"] {
            let err = parse_manifest(&format!(
                r#"{{"format": "rutbundle", "format_version": 10, "name": "h", "type": "host", "entry": {{"type": "./h.d.rut"}}, "{table}": {{"ink": {{"path": "../ink"}}}}}}"#
            ))
            .unwrap_err();
            assert!(err.to_string().contains("a host pkg is pure surface"), "[{table}]: {err}");
        }
        let err = parse_manifest_compat(
            r#"{"format": "rutbundle", "format_version": 10, "name": "h", "type": "host", "entry": {"type": "./h.d.rut", "lib": "./h.rut"}}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("no body"), "{err}");
        let err = parse_manifest(
            r#"{"format": "rutbundle", "format_version": 10, "name": "h", "type": "host"}"#,
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
        // under a `line N:` prefix — naming the ORIGINAL file's line
        // (the JSONC front stage never moves one)
        let err = parse_manifest("{\"name\": six}").unwrap_err();
        let msg = err.to_string();
        assert!(msg.starts_with("line 1: "), "{msg}");
        assert!(msg.contains("expected value"), "{msg}");
        let err = parse_manifest("{\n  \"name\": \"x\",\n  /\n}").unwrap_err();
        let msg = err.to_string();
        assert!(msg.starts_with("line 3: "), "{msg}");
    }

    #[test]
    fn jsonc_comments_and_trailing_commas_parse() {
        // the full JSONC leniency: `//`, `/* */`, trailing commas —
        // at every table, the same value laws underneath
        let m = parse_manifest(
            r#"
// pouch — the growable sequence package
{
  // the header prose
  "name": "pouch", /* beside the value */
  "entry": {
    // the surface
    "type": "./pouch.d.rut", // trailing prose
  },
  "deps": {
    "core": { "path": "rut/core", }, // the descriptor's tail
  },
  "style": { "indent": "2", },
}"#,
        )
        .unwrap();
        assert_eq!(m.name.as_deref(), Some("pouch"));
        assert_eq!(m.entry.type_path.as_deref(), Some("./pouch.d.rut"));
        assert_eq!(m.deps.get("core").unwrap().get("path").unwrap(), "rut/core");
        assert_eq!(m.style.get("indent").map(String::as_str), Some("2"));
    }

    #[test]
    fn trailing_commas_are_legal_everywhere_json_was_not() {
        // the old syntax law refused these; the JSONC law parses them
        let m = parse_manifest(r#"{"name": "x", "deps": {"core": {"path": "p",},},}"#).unwrap();
        assert_eq!(m.deps.get("core").unwrap().get("path").unwrap(), "p");
    }

    #[test]
    fn a_syntax_error_after_jsonc_syntax_names_the_original_line() {
        // the front stage's position law, through parse_manifest: the
        // comment block and the elided comma above do not move the line
        let err = parse_manifest(
            "{\n  // the header\n  /* multi\n     line */\n  \"name\": \"x\",\n  \"entry\": {\"type\": six},\n}",
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.starts_with("line 6: "), "{msg}");
        assert!(msg.contains("expected value"), "{msg}");
    }

    #[test]
    fn full_json_spellings_parse() {
        // standard JSON: \uXXXX escapes process to their characters,
        // integers ride u64 — the parser processes the spellings, the
        // value laws are unchanged
        let m = parse_manifest(r#"{"name": "p\u006Fuch"}"#).unwrap();
        assert_eq!(m.name.as_deref(), Some("pouch"));
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
        let m = parse_manifest(r#"{"name": "a", "name": "b"}"#).unwrap();
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
            r#"{"name": "x", "whatever": {"a": [1, 2]}, "future": true}"#,
        )
        .unwrap();
        assert_eq!(m.name.as_deref(), Some("x"));
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
    fn the_ambiguity_law_rides_the_declared_kind_flag() {
        // the no-inference law: the manifest cannot see `mod.rut` (a
        // file), so the strict lane only records whether `type` was
        // spelled — the MOUNT DOORS refuse the ambiguity
        // (`entry.type` + no declared kind + no `mod.rut`), naming
        // both fixes. Here: the flag travels.
        let m = parse_manifest(r#"{"name": "rt", "entry": {"type": "./rt.d.rut"}}"#).unwrap();
        assert!(!m.type_declared, "the kind was not spelled");
        // either spelling records it
        let m = parse_manifest(
            r#"{"name": "rt", "type": "host", "entry": {"type": "./rt.d.rut"}}"#,
        )
        .unwrap();
        assert_eq!(m.pkg_type, PkgType::Host);
        assert!(m.type_declared);
        // an EXPLICIT `type = "lib"` with a surface and no body is the
        // sanctioned surface-only dev state — spelled, so no ambiguity
        let m = parse_manifest(
            r#"{"name": "dev", "type": "lib", "entry": {"type": "./dev.d.rut"}}"#,
        )
        .unwrap();
        assert_eq!(m.pkg_type, PkgType::Lib);
        assert!(m.type_declared);
        // the COMPAT lane keeps the OLD spelling's ambiguity: an old
        // `entry.type`-only manifest (no legacy lib) refuses, naming
        // both fixes — this is what made old host-pkg manifests fail
        // LOUDLY instead of silently becoming empty libs
        let err = parse_manifest_compat(r#"{"name": "rt", "entry": {"type": "./rt.d.rut"}}"#)
            .unwrap_err();
        assert!(
            err.to_string().contains("an `entry.type`-only pkg spells its kind"),
            "{err}"
        );
        assert!(err.to_string().contains("`type = \"host\"`"), "{err}");
    }

    #[test]
    fn the_repealed_entry_keys_refuse_loudly() {
        // THE REPEAL: the directory grammar refuses both keys, each
        // naming the fix — the root module is `mod.rut`; structure
        // lives in `mod` directories
        let err = parse_manifest(r#"{"name": "x", "entry": {"lib": "./x.rut"}}"#).unwrap_err();
        assert_eq!(
            err.to_string(),
            "entry.lib: repealed — the root module is `mod.rut` beside the manifest: rename the file, drop the key"
        );
        let err = parse_manifest(r#"{"name": "x", "entry": {"libs": ["./a.rut"]}}"#).unwrap_err();
        assert_eq!(
            err.to_string(),
            "entry.libs: repealed — structure lives in `mod` directories: each file becomes `NAME/mod.rut` and the root declares `mod NAME;`"
        );
        // and the refusal fires even value-shaped wrong (the key's
        // existence is the crime, not its value)
        let err = parse_manifest(r#"{"entry": {"libs": "./a.rut"}}"#).unwrap_err();
        assert!(err.to_string().starts_with("entry.libs: repealed"), "{err}");
    }

    #[test]
    fn the_compat_lane_maps_the_old_envelope() {
        // the READER's lane: a PUBLISHED bundle's manifest is forever
        // old (the pinned CDN tags), so the keys parse onto
        // `legacy_entry` under the old value laws — a loading law, not
        // a transition
        let m = parse_manifest_compat(
            r#"{"name": "pouch", "entry": {"lib": "./pouch.rut", "libs": ["./a.rut",]}}"#,
        )
        .unwrap();
        assert_eq!(m.legacy_entry.lib.as_deref(), Some("./pouch.rut"));
        assert_eq!(m.legacy_entry.libs, vec!["./a.rut".to_string()]);
        assert_eq!(m.entry.type_path, None, "the new Entry carries type only");
        // the old array laws hold there: no libs without a base
        let err = parse_manifest_compat(r#"{"entry": {"libs": ["./a.rut"]}}"#).unwrap_err();
        assert!(err.to_string().contains("`entry.libs` needs `entry.lib`"), "{err}");
        // no file rides twice
        let err = parse_manifest_compat(
            r#"{"entry": {"lib": "./a.rut", "libs": ["./a.rut"]}}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("names `./a.rut` twice"), "{err}");
        // every lib is a `.rut` source
        let err = parse_manifest_compat(
            r#"{"entry": {"lib": "./a.rut", "libs": ["./a.d.rut"]}}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("every lib is a `.rut` source"), "{err}");
        // the value laws are path-targeted there too
        let err = parse_manifest_compat(r#"{"entry": {"libs": []}}"#).unwrap_err();
        assert_eq!(
            err.to_string(),
            "entry.libs: cannot be empty — drop the key for a single-file module"
        );
        let err = parse_manifest_compat(r#"{"entry": {"lib": 3}}"#).unwrap_err();
        assert_eq!(err.to_string(), "entry: expected a string for 'lib'");
        // the retired `ir` rides either lane
        let m = parse_manifest_compat(r#"{"name": "x", "entry": {"lib": "./x.rut", "ir": false}}"#)
            .unwrap();
        assert_eq!(m.legacy_entry.lib.as_deref(), Some("./x.rut"));
    }

    #[test]
    fn entry_keys_are_path_targeted() {
        // the plan's canonical shape: the table's path, the key named
        // in the message (compat lane — the only lane where the old
        // keys' VALUE laws still speak)
        let err = parse_manifest_compat(r#"{"entry": {"lib": 3}}"#).unwrap_err();
        assert_eq!(err.to_string(), "entry: expected a string for 'lib'");
        let err = parse_manifest_compat(r#"{"entry": {"libs": "./a.rut"}}"#).unwrap_err();
        assert_eq!(err.to_string(), "entry: expected a string array for 'libs'");
        let err = parse_manifest_compat(r#"{"entry": {"libs": ["./a.rut", 1]}}"#).unwrap_err();
        assert_eq!(err.to_string(), "entry.libs: expected a string, found 1");
        // unknown entry keys refuse (the entry strictness)
        let err = parse_manifest(r#"{"entry": {"type": "./x.d.rut", "feats": 1}}"#).unwrap_err();
        assert_eq!(err.to_string(), "entry: unknown key 'feats'");
    }

    #[test]
    fn style_rows_are_string_typed() {
        let m = parse_manifest(r#"{"name": "x", "style": {"indent": "2"}}"#).unwrap();
        assert_eq!(m.style.get("indent").map(String::as_str), Some("2"));
        let err = parse_manifest(r#"{"style": {"indent": 2}}"#).unwrap_err();
        assert_eq!(err.to_string(), "style: expected a string for 'indent'");
    }

    #[test]
    fn underscore_keys_retire_loudly_everywhere() {
        // the retired prose lane: an `_`-key refuses at EVERY table,
        // naming the key and the JSONC fix — never a silent ignore
        // (the migrated manifests speak `//` comments, which parse)
        let err = parse_manifest(r#"{"_comment": "prose", "name": "x"}"#).unwrap_err();
        assert_eq!(
            err.to_string(),
            "manifest: `_comment` — the `_`-prefixed prose lane retired; JSONC comments are the prose now (write a `//` comment above the key)"
        );
        let err = parse_manifest(r#"{"entry": {"_note": "why", "lib": "./x.rut"}}"#).unwrap_err();
        assert!(err.to_string().starts_with("entry: `_note` — the `_`-prefixed prose lane retired"), "{err}");
        let err = parse_manifest(r#"{"deps": {"_private": "why", "core": {"path": "p"}}}"#)
            .unwrap_err();
        assert!(err.to_string().starts_with("deps: `_private`"), "{err}");
        let err = parse_manifest(r#"{"deps": {"core": {"_hint": "why", "path": "p"}}}"#)
            .unwrap_err();
        assert!(err.to_string().starts_with("deps.core: `_hint`"), "{err}");
        let err = parse_manifest(
            r#"{"peer-deps": {"p": {"_hint": "why", "path": "..", "optional": true}}}"#,
        )
        .unwrap_err();
        assert!(err.to_string().starts_with("peer-deps.p: `_hint`"), "{err}");
        let err = parse_manifest(r#"{"style": {"_why": "2"}}"#).unwrap_err();
        assert!(err.to_string().starts_with("style: `_why`"), "{err}");
        // the JSONC spelling of the same prose parses — the fix works
        let m = parse_manifest("// prose\n{\"name\": \"x\"}").unwrap();
        assert_eq!(m.name.as_deref(), Some("x"));
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
        let m = parse_manifest(r#"{"name": "ink", "inline": true}"#).unwrap();
        assert_eq!(m.name.as_deref(), Some("ink"));
    }

    // ---- the dep kinds: the three tables, `optional`, the
    // `lib` group key, D4 — the T11 manifest-error shapes ----

    /// The pinned grammar (survey §0), plus the §2.3 `lib` keys.
    const JSON: &str = r#"
{
  "name": "json",

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
