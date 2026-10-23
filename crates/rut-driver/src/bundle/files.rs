//! Entry-key and manifest-reference helpers over in-memory values:
//! normalizing bundle entry names, reading one entry's text, and the
//! manifest's file name. Same input ⇒ same bytes; the collectors only
//! ever read manifest-named paths, never list directories. The
//! file-reading half (a directory's `rut.jsonc`, a package's source
//! file set, the output-path suggestion) lives in rut-native — the
//! native host's crate.

/// The manifest's file name — ONE name, no fallback lane: a directory
/// is one module and its manifest is `rut.jsonc` (JSONC: comments and
/// trailing commas legal). A directory still holding the retired
/// `rut.json` gets the pointed refusal (rut-native's reader).
pub const MANIFEST_NAME: &str = "rut.jsonc";

/// The lib pkg's root module file — under the package's `src/` shell.
/// Children mount beside the root file, so the whole module tree lives
/// under `src/`.
pub const ROOT_MODULE: &str = "src/mod.rut";

/// The host pkg's default root surface file — the lib root d-flavored
/// ([`crate::decl::DECL_CHILD_FILE`] remains the child file name and
/// the archive's canonical root name).
pub const DECL_ROOT_MODULE: &str = "src/mod.d.rut";

/// A spelled surface's manifest-relative path under the package's
/// `src/` shell: `entry.type` resolves inside the module directory
/// (`./calc.d.rut` → `src/calc.d.rut`). The archive keeps the ONE
/// canonical wire names — this shell lives in the directory grammar
/// only.
pub fn src_rel(rel: &str) -> String {
    format!("src/{}", rel.strip_prefix("./").unwrap_or(rel))
}

/// The cache filename's extension for one url — the artifact kind the
/// url's own spelling declares (`.zip` = source transport,
/// `.rutbundle` = compiled). The bytes behind a url are pinned by
/// sha256 either way; the suffix only keeps the two transports honest
/// at a glance.
pub fn cache_ext(url: &str) -> &'static str {
    if url
        .rsplit('.')
        .next()
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
    {
        ".zip"
    } else {
        ".rutbundle"
    }
}

/// Bundle entry normalization: `./x.rut` → `x.rut`; anything reaching
/// outside the archive root is refused (entries are archive-relative).
pub fn bundle_key(rel: &str) -> Result<String, String> {
    let key = rel.strip_prefix("./").unwrap_or(rel);
    if key.starts_with('/') || key.split('/').any(|p| p == "..") {
        return Err(format!("bundle entry `{rel}` escapes the bundle root"));
    }
    Ok(key.to_string())
}

/// One archive entry's text — UTF-8-checked; a miss names the key.
pub fn read_entry(entries: &[(String, Vec<u8>)], key: &str) -> Result<String, String> {
    match entries.iter().find(|(n, _)| n == key) {
        Some((_, bytes)) => String::from_utf8(bytes.clone())
            .map_err(|_| format!("bundle entry `{key}` is not UTF-8")),
        None => Err(format!("bundle has no entry `{key}`")),
    }
}

/// The manifest's surface file (the `.d.rut` entry key — the only key
/// left; the body is `mod.rut` by convention, never a key).
pub fn entry_rel(manifest: &Manifest) -> Option<&String> {
    manifest.entry.type_path.as_ref()
}

use super::manifest::Manifest;
