//! The deterministic packer: a module directory (+ its whole `[deps]`
//! graph) → `.rutbundle` bytes, read through a caller-supplied
//! [`Source`]. Same input directory ⇒ byte-identical bundle; the bytes
//! come back — writing the output file belongs to the caller (the
//! CLI). The packer only ever reads manifest-named paths, never lists
//! directories.

use std::path::{Path, PathBuf};

use crate::container::write_bundle;
use crate::manifest::{parse_manifest, Manifest};
use crate::Source;

/// Bundle entry normalization: `./x.rut` → `x.rut`; anything reaching
/// outside the archive root is refused (v1 bundles are flat).
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
fn read_text(path: &Path, src: &dyn Source) -> Result<String, String> {
    let bytes = src.read(path)?;
    String::from_utf8(bytes).map_err(|_| "stream did not contain valid UTF-8".to_string())
}

/// Read a directory's `rut.toml` through `src` and parse it.
pub fn read_manifest(dir: &Path, src: &dyn Source) -> Result<Manifest, String> {
    let text = read_text(&dir.join("rut.toml"), src)?;
    parse_manifest(&text).map_err(|e| e.to_string())
}

/// Collect a package's files for a bundle: its `rut.toml` (byte-for-byte),
/// its entry file, and — the v3 layout (RFC 0045 §3) — each
/// `[peer-deps]` descriptor's `lib` group file, and — the v4 layout
/// (RFC 0041 §5) — each pkg's `entry.libs` files beside the entry, all
/// under `prefix` (empty for the root, `<pkg>/` for a dep). Descriptor
/// order is the manifest's (BTreeMap), so the archive stays
/// deterministic.
fn collect_pkg_files(
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
    // the v4 layout (RFC 0041 §5): each pkg's `entry.libs` files ride
    // beside the entry, in manifest order — the array IS the order the
    // loader splices back, so the archive stays deterministic
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

/// The `[deps]` graph, recursively — each package under its own
/// `<pkg>/` group (manifest + entry), deduplicated by package name.
fn collect_deps(
    dir: &Path,
    manifest: &Manifest,
    src: &dyn Source,
    out: &mut Vec<(String, Vec<u8>)>,
    seen: &mut std::collections::BTreeSet<String>,
) -> Result<(), String> {
    for (spec, desc) in &manifest.deps {
        if !seen.insert(spec.clone()) {
            continue;
        }
        let rel = desc
            .get("path")
            .ok_or_else(|| format!("dep `{spec}` has no `path`"))?;
        let dep_dir = dir.join(rel);
        let dm = read_manifest(&dep_dir, src)?;
        if dm.name.as_deref() != Some(spec.as_str()) {
            return Err(format!(
                "dep `{spec}` points at `{}` — the manifest there names it `{}`",
                dep_dir.display(),
                dm.name.as_deref().unwrap_or("<unnamed>")
            ));
        }
        collect_pkg_files(&dep_dir, &dm, &format!("{spec}/"), src, out)?;
        collect_deps(&dep_dir, &dm, src, out, seen)?;
    }
    Ok(())
}

/// Pack a module directory into a deterministic `.rutbundle` (RFC 0038 §3):
/// `rut.toml` first, then the entry source, then — the v2 layout — the
/// whole `[deps]` graph, each package under its own `<pkg>/` group
/// (manifest + entry), recursively and deduplicated; — the v3 layout
/// (RFC 0045 §3) — each package's peer-gated integration files
/// (`[peer-deps]` `lib` keys) beside its entry in its group; and — the
/// v4 layout (RFC 0041 §5) — each package's `entry.libs` files beside
/// its entry too. Same input directory ⇒ byte-identical bundle.
///
/// This answers RFC 0038 OQ-3: a bundle is self-contained; its deps'
/// `path` keys are directory-time only — the loader resolves groups by
/// NAME. A v1 bundle (no deps, source-only) is the one-module special
/// case the loader still accepts; a v2 bundle still loads but no
/// longer packs — as does a v3 one — the packer emits v4, the layout
/// that carries multi-lib entries.
pub fn pack(root: &Path, src: &dyn Source) -> Result<Vec<u8>, String> {
    let manifest = read_manifest(root, src)?;
    if manifest.name.is_none() {
        return Err(format!("{} has no `name`", root.join("rut.toml").display()));
    }
    // the packed rut.toml is the directory's rut.toml byte-for-byte, so
    // the bundle keys must already be there — directory loading ignores
    // them, but a bundle loader refuses without them (RFC 0038 §2).
    // v4 since the multi-lib batch: `entry.libs` files ride the bundle
    // (an older loader refuses the version — refuse, never guess).
    if manifest.format.as_deref() != Some("rutbundle") || manifest.format_version != Some(4) {
        return Err(format!(
            "{} is not bundle-shaped — add `format = \"rutbundle\"` and `format_version = 4`",
            root.join("rut.toml").display()
        ));
    }
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    collect_pkg_files(root, &manifest, "", src, &mut entries)?;
    collect_deps(root, &manifest, src, &mut entries, &mut std::collections::BTreeSet::new())?;
    write_bundle(&entries).map_err(|e| e.to_string())
}

/// The conventional pack output: `<dirname>.rutbundle`, a sibling of
/// the directory (`demo/mod` → `demo/mod.rutbundle`).
pub fn default_out_path(dir: &Path) -> PathBuf {
    let stem = dir.file_name().unwrap_or(dir.as_os_str()).to_string_lossy();
    dir.with_file_name(format!("{stem}.rutbundle"))
}
