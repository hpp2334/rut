//! Manifest- and file-set helpers over a [`Source`]: reading a
//! directory's `rut.toml`, normalizing bundle entry names, and
//! collecting a package's **source file set** — the `rut.toml` +
//! entry + `libs` + peer-group files shape a source group rides inside
//! a v5 compiled bundle. The packer itself (which compiles the closure
//! and emits the `.rutc` groups) lives in `rut-driver` — it needs the
//! compiler, and this crate stays compiler-free. Same input ⇒ same
//! bytes; the packer only ever reads manifest-named paths, never lists
//! directories.

use std::path::{Path, PathBuf};

use crate::manifest::{parse_manifest, Manifest};
use crate::Source;

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

/// The manifest's entry file (lib preferred, else the `.d.rut` surface).
pub fn entry_rel(manifest: &Manifest) -> Option<&String> {
    manifest.entry.lib.as_ref().or(manifest.entry.type_path.as_ref())
}

/// `read_to_string` over a [`Source`] — bytes then UTF-8, answering the
/// same "stream did not contain valid UTF-8" text std's reader gives.
pub(crate) fn read_text(path: &Path, src: &dyn Source) -> Result<String, String> {
    let bytes = src.read(path)?;
    String::from_utf8(bytes).map_err(|_| "stream did not contain valid UTF-8".to_string())
}

/// Read a directory's `rut.toml` through `src` and parse it.
pub fn read_manifest(dir: &Path, src: &dyn Source) -> Result<Manifest, String> {
    let text = read_text(&dir.join("rut.toml"), src)?;
    parse_manifest(&text).map_err(|e| e.to_string())
}

/// Collect a package's SOURCE file set — the v4 group shape — under
/// `prefix` (empty for a root, `<pkg>/` for a dep group): its `rut.toml`
/// byte-for-byte, its entry file, each `entry.libs` file beside the
/// entry, and each `[peer-deps]` descriptor's `lib` group file.
/// Descriptor order is the manifest's (BTreeMap), so the archive stays
/// deterministic. A v5 compiled group does not take this shape (its
/// `.rutc` is the linking truth); splice-needed deps and host pkgs ride
/// the bundle exactly like this.
pub fn collect_source_group(
    dir: &Path,
    manifest: &Manifest,
    prefix: &str,
    src: &dyn Source,
    out: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), String> {
    let text = read_text(&dir.join("rut.toml"), src)?;
    out.push((format!("{prefix}rut.toml"), text.into_bytes()));
    let rel = entry_rel(manifest)
        .ok_or_else(|| format!("module in {} has no entry", dir.display()))?;
    // normalize the entry's `./` prefix before the group prefix joins it
    let rel = rel.strip_prefix("./").unwrap_or(rel);
    let key = bundle_key(&format!("{prefix}{rel}"))?;
    let body = read_text(&dir.join(rel), src)
        .map_err(|e| format!("cannot read {}: {e}", dir.join(rel).display()))?;
    out.push((key, body.into_bytes()));
    // each pkg's `entry.libs` files ride beside the entry, in manifest
    // order — the array IS the order the loader splices back, so the
    // archive stays deterministic
    for lib in &manifest.entry.libs {
        let rel = lib.strip_prefix("./").unwrap_or(lib);
        let key = bundle_key(&format!("{prefix}{rel}"))?;
        let body = read_text(&dir.join(rel), src)
            .map_err(|e| format!("cannot read {}: {e}", dir.join(rel).display()))?;
        out.push((key, body.into_bytes()));
    }
    for desc in manifest.peer_deps.values() {
        let Some(lib) = desc.get("lib") else {
            continue; // presence declared, no integration file to pack
        };
        let rel = lib.strip_prefix("./").unwrap_or(lib);
        let key = bundle_key(&format!("{prefix}{rel}"))?;
        let body = read_text(&dir.join(rel), src)
            .map_err(|e| format!("cannot read {}: {e}", dir.join(rel).display()))?;
        out.push((key, body.into_bytes()));
    }
    Ok(())
}

/// The conventional pack output: `<dirname>.rutbundle`, a sibling of
/// the directory (`demo/mod` → `demo/mod.rutbundle`).
pub fn default_out_path(dir: &Path) -> PathBuf {
    let stem = dir.file_name().unwrap_or(dir.as_os_str()).to_string_lossy();
    dir.with_file_name(format!("{stem}.rutbundle"))
}
