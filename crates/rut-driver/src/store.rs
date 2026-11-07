//! The global dep store's derivation core — the ONE place the store's
//! addresses, file names, and meta shape are spelled. rut-cli (the
//! writer: `rut install`; the readers: `run` / `pack`)
//! and the editor hosts both sit on these helpers, so no second table
//! can drift — the address is DERIVED, never tabulated (the same law
//! rut.lock's pin table lives by).
//!
//! Layout (under `$RUT_HOME`, default `~/.rut`):
//!
//! ```text
//! $RUT_HOME/deps/                       # RUT_HOME overrides; ~/.rut fallback
//!   <name>@<sha256[:16]>/               # the pin IS the address
//!     artifact.<ext>                    # the pinned bytes as downloaded
//!     meta.json                         # the verification surface (below)
//!     unpack/                           # the extracted tree
//! $RUT_HOME/run/<key>.rutbundle         # the run-key cache
//! $RUT_HOME/locks/<project-hash>.lock   # `rut install`'s per-project mutex
//! ```
//!
//! The `<name>@<sha256[:16]>` address is derived from the manifest
//! row's name + the lock pin ([`store_dir`]) — string munging only,
//! so the same helper serves writers and readers. `meta.json` is the
//! readers' verification surface: the FULL sha256, the source url,
//! and the artifact kind — a reader never re-hashes the artifact
//! (install-time verification is the only hash pass).
//!
//! PROVABLY PURE like the rest of the crate: string munging and env
//! reads only — no fs, no net, no `std::path`. Every helper answers a
//! `String`; callers wrap them in their own `PathBuf`s.

use serde_json::Value;

use crate::bundle::cache_ext;
use crate::loader::sha256_hex;

/// The store root under the rut home: every pinned dep dir.
pub const DEPS_DIR: &str = "deps";

/// The run-key cache root under the rut home.
pub const RUN_DIR: &str = "run";

/// The per-project install locks' root under the rut home.
pub const LOCKS_DIR: &str = "locks";

/// Inside a dep dir: the pinned bytes as downloaded — the extension
/// follows the url's own artifact kind ([`store_artifact_name`]).
pub const ARTIFACT_STEM: &str = "artifact";

/// Inside a dep dir: the verification surface ([`StoreMeta`]).
pub const META_NAME: &str = "meta.json";

/// Inside a dep dir: the extracted tree (zip entries laid verbatim,
/// bundle entries likewise — `rut.jsonc`, `src/…`, embedded dep
/// groups).
pub const UNPACK_DIR: &str = "unpack";

/// The only meta grammar this store spells — an exact match at parse,
/// the same forward-compat story as rut.lock's gates: a NEW version
/// refuses loudly until this grammar learns it.
pub const STORE_META_FORMAT_VERSION: u64 = 1;

/// `$RUT_HOME` — the global rut home directory: the env override,
/// else `~/.rut` (`$HOME`; no `HOME` at all, the degenerate `.rut`
/// beside the working directory). A `String`, not a `Path` — the
/// purity law; callers wrap it.
pub fn rut_home() -> String {
    if let Some(home) = std::env::var_os("RUT_HOME") {
        return home.to_string_lossy().into_owned();
    }
    match std::env::var_os("HOME") {
        Some(home) => format!("{}/.rut", home.to_string_lossy()),
        None => ".rut".into(),
    }
}

/// `<rut_home>/deps` — the store root the dep dirs land in.
pub fn deps_dir() -> String {
    format!("{}/{DEPS_DIR}", rut_home())
}

/// `<name>@<sha256[:16]>` — the pin IS the address: derived from the
/// manifest row's name + the lock pin, lowercase (the lock stores
/// pins lowercase; an uppercase caller normalizes), truncated to 16
/// hex digits. Same inputs ⇒ same address, everywhere, forever.
pub fn store_dir(name: &str, pin: &str) -> String {
    let pin = pin.to_ascii_lowercase();
    let end = pin.len().min(16);
    format!("{name}@{}", &pin[..end])
}

/// `artifact.<ext>` — the pinned bytes as downloaded; the extension
/// is the url's own artifact kind (`.zip` source transport,
/// `.rutbundle` compiled), the same law the old flat cache named by.
pub fn store_artifact_name(url: &str) -> String {
    format!("{ARTIFACT_STEM}{}", cache_ext(url))
}

/// The artifact kind the url declares — meta's `format` value: `zip`
/// (the source transport) or `rutbundle` (the compiled wire).
pub fn store_format(url: &str) -> &str {
    cache_ext(url).trim_start_matches('.')
}

/// `<rut_home>/run/<key>.rutbundle` — the run-key cache: the
/// `--repack` output shared across projects (the key is the closure's
/// content hash; same key ⇒ byte-identical bundle).
pub fn run_cache_path(key: &str) -> String {
    format!("{}/{RUN_DIR}/{key}.rutbundle", rut_home())
}

/// `<rut_home>/locks/<sha256(canonical project path)[:16]>.lock` — the
/// per-project INSTALL lock: `rut install`'s kernel-enforced mutual
/// exclusion (readers never lock; the global store stays lock-free).
/// The input must be the project dir's CANONICAL spelling (the caller
/// canonicalizes) so symlinked paths share one lockfile. The file is a
/// STABLE flock target — created if missing, never renamed or deleted
/// (locking rut.lock itself is broken by its own tmp+rename write),
/// and accumulation is accepted (a future `--gc` prunes).
pub fn install_lock_path(canonical_project: &str) -> String {
    let hash = sha256_hex(canonical_project.as_bytes());
    format!("{}/{LOCKS_DIR}/{}.lock", rut_home(), &hash[..16])
}

/// One dep dir's `meta.json` — the verification surface for readers:
/// the full sha256 (the dir name carries only its 16-hex prefix), the
/// url the bytes were fetched from, and the artifact kind. A reader
/// never re-hashes the artifact: install-time verification is the
/// only hash pass, and this file is what it vouched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreMeta {
    /// the manifest row's name — the address's left half
    pub name: String,
    /// the url the bytes were fetched from
    pub url: String,
    /// the FULL sha256, lowercase hex — the pin the bytes verify to
    pub sha256: String,
    /// the artifact kind: `zip` or `rutbundle` ([`store_format`])
    pub format: String,
    /// the meta grammar's own version — [`STORE_META_FORMAT_VERSION`]
    pub format_version: u64,
}

/// A malformed `meta.json` — [`LockError`]'s twin: a load error, one
/// line, path-targeted.
///
/// [`LockError`]: crate::lock::LockError
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct StoreMetaError(pub String);

/// Render the meta: fixed field order (`name`/`url`/`sha256`/
/// `format`/`format_version`), 2-space indent, trailing newline —
/// byte-deterministic, the rut.lock renderer's law (the same closure
/// renders the same bytes).
pub fn store_meta_json(meta: &StoreMeta) -> String {
    // serde_json spells each field as a JSON string literal (escaping
    // included) — the values are plain, but the law is the law
    let field = |v: &str| serde_json::to_string(v).unwrap_or_default();
    format!(
        "{{\n  \"name\": {},\n  \"url\": {},\n  \"sha256\": {},\n  \"format\": {},\n  \
         \"format_version\": {}\n}}\n",
        field(&meta.name),
        field(&meta.url),
        field(&meta.sha256),
        field(&meta.format),
        meta.format_version,
    )
}

/// Parse the meta: JSON (machine-written, but hand-edits load), then
/// the value laws — the exact format_version gate, every field a
/// string, the sha256 exactly 64 hex digits stored LOWERCASE (the
/// lock's normalization law), the url/name/format taken as spelled.
pub fn parse_store_meta(text: &str) -> Result<StoreMeta, StoreMetaError> {
    let root: Value =
        serde_json::from_str(text).map_err(|e| StoreMetaError(format!("meta.json: {e}")))?;
    let obj = root
        .as_object()
        .ok_or_else(|| StoreMetaError("meta.json: expected a JSON object".into()))?;
    let string_field = |key: &str| -> Result<String, StoreMetaError> {
        match obj.get(key) {
            Some(Value::String(s)) => Ok(s.clone()),
            Some(other) => Err(StoreMetaError(format!(
                "meta.json: expected a string for '{key}', found {other}"
            ))),
            None => Err(StoreMetaError(format!("meta.json: missing '{key}'"))),
        }
    };
    match obj.get("format_version").and_then(Value::as_u64) {
        Some(STORE_META_FORMAT_VERSION) => {}
        Some(other) => {
            return Err(StoreMetaError(format!(
                "meta.json: format_version: expected {STORE_META_FORMAT_VERSION}, found {other}"
            )));
        }
        None => {
            return Err(StoreMetaError(
                "meta.json: format_version: expected an integer, found nothing".into(),
            ));
        }
    }
    let name = string_field("name")?;
    let url = string_field("url")?;
    let format = string_field("format")?;
    let sha256 = string_field("sha256")?;
    if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(StoreMetaError(format!(
            "meta.json: sha256: must be 64 hex digits, found '{sha256}'"
        )));
    }
    Ok(StoreMeta {
        name,
        url,
        sha256: sha256.to_ascii_lowercase(),
        format,
        format_version: STORE_META_FORMAT_VERSION,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIN: &str = "4197b14904bd9a5d8b0995d86b7d1b1e50d80e9cb1a1c1e6e2a5e8e9c0d1b2a3";

    // ---- the address derivation ----

    #[test]
    fn the_pin_names_the_dir() {
        // the pin IS the address: name + the first 16 hex digits
        assert_eq!(store_dir("ink", PIN), "ink@4197b14904bd9a5d");
        // a different pin's prefix moves the address
        assert_eq!(
            store_dir(
                "pouch",
                "2f6e6a20c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e"
            ),
            "pouch@2f6e6a20c3d4e5f6"
        );
    }

    #[test]
    fn uppercase_pins_normalize_to_lowercase() {
        // the lock stores pins lowercase, but the derivation refuses
        // to depend on the caller's discipline
        assert_eq!(
            store_dir("ink", &PIN.to_ascii_uppercase()),
            "ink@4197b14904bd9a5d"
        );
    }

    #[test]
    fn a_short_pin_does_not_panic() {
        // total function: truncation to what is there
        assert_eq!(store_dir("x", "abc"), "x@abc");
        assert_eq!(store_dir("x", ""), "x@");
    }

    // ---- the home + the roots ----

    #[test]
    // Env mutation is safe under nextest's one-process-per-test model
    // (the suite's runner): this test owns its process's environment.
    fn the_home_is_the_override_then_the_default() {
        std::env::set_var("RUT_HOME", "/store-home");
        assert_eq!(rut_home(), "/store-home");
        assert_eq!(deps_dir(), "/store-home/deps");
        assert_eq!(run_cache_path("abc123"), "/store-home/run/abc123.rutbundle");
        std::env::remove_var("RUT_HOME");
        std::env::set_var("HOME", "/users/rut");
        assert_eq!(rut_home(), "/users/rut/.rut");
        assert_eq!(deps_dir(), "/users/rut/.rut/deps");
        std::env::remove_var("HOME");
        // degenerate: no override, no home — beside the cwd
        assert_eq!(rut_home(), ".rut");
    }

    #[test]
    // Env mutation is safe under nextest's one-process-per-test model
    // (the suite's runner): this test owns its process's environment.
    fn the_install_lock_names_the_canonical_project() {
        // the derivation: sha256 over the canonical path, first 16 hex
        // digits, under locks/ — one stable file per project
        std::env::set_var("RUT_HOME", "/store-home");
        let want = format!("/store-home/locks/{}.lock", &sha256_hex(b"/proj/app")[..16]);
        assert_eq!(install_lock_path("/proj/app"), want);
        // a different project is a different lock — installs on other
        // projects never wait on this one
        assert_ne!(install_lock_path("/proj/other"), want);
        std::env::remove_var("RUT_HOME");
    }

    // ---- the artifact name + kind ----

    #[test]
    fn the_artifact_name_follows_the_url_kind() {
        assert_eq!(store_artifact_name("https://x/ink.zip"), "artifact.zip");
        assert_eq!(
            store_artifact_name("https://x/pouch.rutbundle"),
            "artifact.rutbundle"
        );
        assert_eq!(store_format("https://x/ink.zip"), "zip");
        assert_eq!(store_format("https://x/pouch.rutbundle"), "rutbundle");
    }

    // ---- the meta shape ----

    fn meta() -> StoreMeta {
        StoreMeta {
            name: "ink".into(),
            url: "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v15/dist/std/ink.zip".into(),
            sha256: PIN.into(),
            format: "zip".into(),
            format_version: STORE_META_FORMAT_VERSION,
        }
    }

    #[test]
    fn the_meta_renders_canonically() {
        // fixed field order, 2-space indent, trailing newline
        let want = format!(
            "{{\n  \"name\": \"ink\",\n  \"url\": \"https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v15/dist/std/ink.zip\",\n  \"sha256\": \"{PIN}\",\n  \"format\": \"zip\",\n  \"format_version\": 1\n}}\n"
        );
        assert_eq!(store_meta_json(&meta()), want);
    }

    #[test]
    fn meta_parse_render_is_a_fixed_point() {
        // the canonical bytes parse to the same shape and re-render
        // byte-identically (the lock's round-trip law)
        let text = store_meta_json(&meta());
        let parsed = parse_store_meta(&text).expect("the canonical meta parses");
        assert_eq!(parsed, meta());
        assert_eq!(store_meta_json(&parsed), text);
    }

    #[test]
    fn meta_laws_refuse() {
        // a non-object
        let err = parse_store_meta("[]").unwrap_err();
        assert_eq!(err.to_string(), "meta.json: expected a JSON object");
        // the version gate is exact (the lock's forward-compat law)
        for bad in [
            r#"{"name":"n","url":"u","sha256":"0","format":"zip"}"#,
            r#"{"name":"n","url":"u","sha256":"0","format":"zip","format_version":"1"}"#,
            r#"{"name":"n","url":"u","sha256":"0","format":"zip","format_version":2}"#,
        ] {
            let err = parse_store_meta(bad).unwrap_err();
            assert!(err.to_string().contains("format_version"), "{err}");
        }
        // every field is a string and present
        let err = parse_store_meta(r#"{"url":"u","sha256":"0","format":"zip","format_version":1}"#)
            .unwrap_err();
        assert_eq!(err.to_string(), "meta.json: missing 'name'");
        let err = parse_store_meta(
            r#"{"name":3,"url":"u","sha256":"0","format":"zip","format_version":1}"#,
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "meta.json: expected a string for 'name', found 3"
        );
        // the sha256 is 64 hex digits, stored lowercase
        let err = parse_store_meta(
            r#"{"name":"n","url":"u","sha256":"nothex","format":"zip","format_version":1}"#,
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "meta.json: sha256: must be 64 hex digits, found 'nothex'"
        );
        let short = format!(
            r#"{{"name":"n","url":"u","sha256":"{}","format":"zip","format_version":1}}"#,
            &PIN[1..]
        );
        let err = parse_store_meta(&short).unwrap_err();
        assert!(err.to_string().contains("must be 64 hex digits"), "{err}");
        // uppercase input normalizes
        let upper = format!(
            r#"{{"name":"n","url":"u","sha256":"{}","format":"zip","format_version":1}}"#,
            PIN.to_ascii_uppercase()
        );
        let parsed = parse_store_meta(&upper).expect("uppercase hex normalizes");
        assert_eq!(parsed.sha256, PIN);
    }
}
