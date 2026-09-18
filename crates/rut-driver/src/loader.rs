//! Filesystem loader — read a module directory (`rut.toml`) or a packed
//! `.rutbundle` (RFC 0038) into a [`Session`].
//!
//! One module is one directory with ONE entry file: its `use` statements
//! are all inter-module paths (`use <pkg>::{A, B};`), resolved by the
//! `Session` — there is no intra-module include form. A `.rutbundle` is
//! the same contract zipped: `rut.toml` plus the rut entry source.
//!
//! The `Session` itself does no I/O (wasm hosts mount in memory); this
//! native helper is the counterpart that reads files.

use std::path::Path;

use crate::bundle::{parse_bundle, write_bundle};
use crate::session::{parse_manifest, Entry, Module, Session};

/// Bundle entry normalization: `./x.rut` → `x.rut`; anything reaching
/// outside the archive root is refused (v1 bundles are flat).
fn bundle_key(rel: &str) -> Result<String, String> {
    let key = rel.strip_prefix("./").unwrap_or(rel);
    if key.starts_with('/') || key.split('/').any(|p| p == "..") {
        return Err(format!("bundle entry `{rel}` escapes the bundle root"));
    }
    Ok(key.to_string())
}

fn read_entry(entries: &[(String, Vec<u8>)], key: &str) -> Result<String, String> {
    match entries.iter().find(|(n, _)| n == key) {
        Some((_, bytes)) => String::from_utf8(bytes.clone())
            .map_err(|_| format!("bundle entry `{key}` is not UTF-8")),
        None => Err(format!("bundle has no entry `{key}`")),
    }
}

/// Read one module source file. One file is one module unit — there is
/// no include form to expand (RFC 0035 §1: use paths are inter-module).
pub fn load_module_source(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Mount a directory's consumer manifest: its own module (if named) and every
/// `[deps]` module, reading each dep's `rut.toml` and entry source. Returns the
/// session and the root spec (the directory's own `name`).
pub fn load_dir_session(dir: &Path) -> Result<(Session, String), String> {
    let text = std::fs::read_to_string(dir.join("rut.toml")).map_err(|e| e.to_string())?;
    let manifest = parse_manifest(&text).map_err(|e| e.to_string())?;
    let mut session = Session::new();

    let root = manifest
        .name
        .clone()
        .ok_or_else(|| format!("{} has no `name`", dir.join("rut.toml").display()))?;
    let root_module = load_entry_module(dir, &manifest)?;
    session
        .register_module(&root, root_module)
        .map_err(|e| e.to_string())?;
    resolve_deps(&mut session, dir, &manifest, &mut vec![dir.to_path_buf()])?;
    Ok((session, root))
}

/// Mount a `.rutbundle` file (RFC 0038): one module, its source read from
/// the archive. The manifest's `format`/`format_version` are checked
/// before anything else is read — an unknown layout is refused, never
/// guessed at.
pub fn load_bundle_session(path: &Path) -> Result<(Session, String), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    load_bundle_bytes(&bytes, path)
}

/// Build a package's entry [`Module`] from bundle entries under
/// `prefix` (empty for the root, `<pkg>/` for a dep group) — the
/// in-archive counterpart of `load_entry_module`: `entry.type`-only is
/// a host pkg (decl surface), else the lib source.
fn bundle_entry_module(
    entries: &[(String, Vec<u8>)],
    prefix: &str,
    manifest: &crate::session::Manifest,
) -> Result<Module, String> {
    let read = |rel: &str| -> Result<String, String> {
        let rel = rel.strip_prefix("./").unwrap_or(rel);
        let key = bundle_key(&format!("{prefix}{rel}"))?;
        read_entry(entries, &key)
    };
    if manifest.entry.lib.is_none() && manifest.entry.type_path.is_some() {
        let rel = manifest.entry.type_path.as_ref().unwrap();
        let src = read(rel)?;
        let mut m = crate::decl::lower_decl_module(&src, &format!("{prefix}{rel}"))?;
        m.host_scope = manifest.host_scope.clone();
        m.inline = manifest.inline;
        m.entry = manifest.entry.clone();
        return Ok(m);
    }
    let rel = entry_rel(manifest)
        .ok_or_else(|| format!("module at bundle prefix `{prefix}` has no entry"))?;
    let src = read(rel)?;
    Ok(Module {
        source: Some(src),
        entry: manifest.entry.clone(),
        inline: manifest.inline,
        host_scope: manifest.host_scope.clone(),
        ..Default::default()
    })
}

/// [`load_bundle_session`] over in-memory bytes (tests, embedders).
pub fn load_bundle_bytes(bytes: &[u8], origin: &Path) -> Result<(Session, String), String> {
    let entries = parse_bundle(bytes).map_err(|e| format!("{}: {e}", origin.display()))?;
    let toml = read_entry(&entries, "rut.toml")
        .map_err(|_| format!("{}: no `rut.toml` entry — not a rut bundle", origin.display()))?;
    let manifest = parse_manifest(&toml).map_err(|e| format!("{}: {e}", origin.display()))?;
    // §4: the layout version gates everything — refuse before reading
    // any other entry. v1: one module, rut sources only. v2: the whole
    // dep graph rides `<pkg>/` groups (RFC 0038 OQ-3 answered).
    if manifest.format.as_deref() != Some("rutbundle") {
        return Err(format!(
            "{}: rut.toml has no `format = \"rutbundle\"` — not a rut bundle",
            origin.display()
        ));
    }
    match manifest.format_version {
        Some(1) | Some(2) => {}
        other => {
            return Err(format!(
                "{}: unknown bundle format_version {other:?} — this loader knows versions 1 and 2",
                origin.display()
            ));
        }
    }
    let root = manifest
        .name
        .clone()
        .ok_or_else(|| format!("{}: rut.toml has no `name`", origin.display()))?;
    let mut session = Session::new();
    if manifest.format_version == Some(1) {
        // §1: one module per bundle in v1
        if !manifest.deps.is_empty() {
            return Err(format!(
                "{}: rut.toml declares `[deps]` — a v1 bundle is one module (RFC 0038 OQ-3)",
                origin.display()
            ));
        }
        if manifest.entry.type_path.is_some() || manifest.entry.ir.is_some() {
            return Err(format!(
                "{}: rut.toml has entry.type/entry.ir — v1 bundles carry rut sources only",
                origin.display()
            ));
        }
        let rel = manifest
            .entry
            .lib
            .as_ref()
            .ok_or_else(|| format!("{}: rut.toml has no entry.lib", origin.display()))?;
        let key = bundle_key(rel)?;
        let src = read_entry(&entries, &key)?;
        session
            .register_module(
                &root,
                Module { source: Some(src), entry: manifest.entry.clone(), ..Default::default() },
            )
            .map_err(|e| e.to_string())?;
        return Ok((session, root));
    }

    // ---- v2: the root plus every `<pkg>/` dep group, resolved by NAME
    // (the deps' `path` keys are directory-time only) ----
    let root_module = bundle_entry_module(&entries, "", &manifest)
        .map_err(|e| format!("{}: {e}", origin.display()))?;
    session
        .register_module(&root, root_module)
        .map_err(|e| e.to_string())?;
    let mut groups: std::collections::BTreeSet<String> = entries
        .iter()
        .filter_map(|(n, _)| n.split_once('/').map(|(p, _)| p.to_string()))
        .collect();
    groups.remove("rut.toml");
    for pkg in groups {
        let dep_toml = read_entry(&entries, &format!("{pkg}/rut.toml")).map_err(|_| {
            format!("{}: bundle group `{pkg}/` has no `rut.toml`", origin.display())
        })?;
        let dm = parse_manifest(&dep_toml)
            .map_err(|e| format!("{}: {pkg}/rut.toml: {e}", origin.display()))?;
        let name =
            dm.name.clone().ok_or_else(|| format!("{}: {pkg}/rut.toml has no `name`", origin.display()))?;
        if session.resolve(&name).is_ok() {
            continue; // first mount wins (the root, an earlier group)
        }
        let m = bundle_entry_module(&entries, &format!("{pkg}/"), &dm)
            .map_err(|e| format!("{}: {e}", origin.display()))?;
        session
            .register_module(&name, m)
            .map_err(|e| e.to_string())?;
    }
    // every declared dep must be satisfied by a group
    for spec in manifest.deps.keys() {
        if session.resolve(spec).is_err() {
            return Err(format!(
                "{}: the bundle is missing its `{spec}` dependency group",
                origin.display()
            ));
        }
    }
    Ok((session, root))
}

/// Load a module directory (`rut.toml`) or a `.rutbundle` file — the two
/// packed forms of the same contract. A loose `.rut` file is NOT this: it
/// is a single-file module with no manifest.
pub fn load_path_session(path: &Path) -> Result<(Session, String), String> {
    if path.is_dir() {
        return load_dir_session(path);
    }
    if path.extension().map_or(false, |e| e == "rutbundle") {
        return load_bundle_session(path);
    }
    Err(format!(
        "{} is neither a module directory (no `rut.toml`) nor a `.rutbundle`",
        path.display()
    ))
}

/// Pack a module directory into a deterministic `.rutbundle` (RFC 0038 §3):
/// `rut.toml` first, then the entry source. Same input directory ⇒
/// byte-identical bundle.
/// The manifest's entry file (lib preferred, else the `.d.rut` surface).
fn entry_rel(manifest: &crate::session::Manifest) -> Option<&String> {
    manifest.entry.lib.as_ref().or(manifest.entry.type_path.as_ref())
}

/// Collect a package's files for a bundle: its `rut.toml` (byte-for-byte)
/// plus its entry file, under `prefix` (empty for the root, `<pkg>/`
/// for a dep).
fn collect_pkg_files(
    dir: &Path,
    manifest: &crate::session::Manifest,
    prefix: &str,
    out: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), String> {
    let text = std::fs::read_to_string(dir.join("rut.toml")).map_err(|e| e.to_string())?;
    out.push((format!("{prefix}rut.toml"), text.into_bytes()));
    let rel = entry_rel(manifest)
        .ok_or_else(|| format!("module in {} has no entry", dir.display()))?;
    // normalize the entry's `./` prefix before the group prefix joins it
    let rel = rel.strip_prefix("./").unwrap_or(rel);
    let key = bundle_key(&format!("{prefix}{rel}"))?;
    let src = std::fs::read_to_string(dir.join(rel))
        .map_err(|e| format!("cannot read {}: {e}", dir.join(rel).display()))?;
    out.push((key, src.into_bytes()));
    Ok(())
}

/// Pack a module directory into a deterministic `.rutbundle` (RFC 0038 §3):
/// `rut.toml` first, then the entry source, then — the v2 layout — the
/// whole `[deps]` graph, each package under its own `<pkg>/` group
/// (manifest + entry), recursively and deduplicated. Same input
/// directory ⇒ byte-identical bundle.
///
/// This answers RFC 0038 OQ-3: a bundle is self-contained; its deps'
/// `path` keys are directory-time only — the loader resolves groups by
/// NAME. A v1 bundle (no deps, source-only) is the one-module special
/// case the loader still accepts.
pub fn pack_dir(dir: &Path) -> Result<Vec<u8>, String> {
    let manifest = read_manifest(dir)?;
    if manifest.name.is_none() {
        return Err(format!("{} has no `name`", dir.join("rut.toml").display()));
    }
    // the packed rut.toml is the directory's rut.toml byte-for-byte, so
    // the bundle keys must already be there — directory loading ignores
    // them, but a bundle loader refuses without them (RFC 0038 §2)
    if manifest.format.as_deref() != Some("rutbundle") || manifest.format_version != Some(2) {
        return Err(format!(
            "{} is not bundle-shaped — add `format = \"rutbundle\"` and `format_version = 2`",
            dir.join("rut.toml").display()
        ));
    }
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    collect_pkg_files(dir, &manifest, "", &mut entries)?;
    // the dep graph, recursively, deduplicated by package name
    fn collect_deps(
        dir: &Path,
        manifest: &crate::session::Manifest,
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
            let dm = read_manifest(&dep_dir)?;
            if dm.name.as_deref() != Some(spec.as_str()) {
                return Err(format!(
                    "dep `{spec}` points at `{}` — the manifest there names it `{}`",
                    dep_dir.display(),
                    dm.name.as_deref().unwrap_or("<unnamed>")
                ));
            }
            collect_pkg_files(&dep_dir, &dm, &format!("{spec}/"), out)?;
            collect_deps(&dep_dir, &dm, out, seen)?;
        }
        Ok(())
    }
    collect_deps(dir, &manifest, &mut entries, &mut std::collections::BTreeSet::new())?;
    write_bundle(&entries).map_err(|e| e.to_string())
}

/// Build a directory's entry [`Module`] from its manifest (RFC 0029 §5):
///
/// - `entry.lib` (with or without `entry.type`) — a SOURCE module: the
///   body compiles; the surface derives from its exports.
/// - `entry.type` ALONE — a **host pkg**: a pure declaration surface.
///   The `.d.rut` parses in declaration mode and lowers into the
///   module's host fns (RFC 0025); `host_scope`/`inline` ride the
///   manifest. No body exists — the embedding Rust binds it at run
///   time.
fn load_entry_module(dir: &Path, manifest: &crate::session::Manifest) -> Result<Module, String> {
    if manifest.entry.lib.is_none() && manifest.entry.type_path.is_some() {
        let rel = manifest.entry.type_path.as_ref().unwrap();
        let origin = format!("{}/{}", dir.display(), rel);
        let src = load_module_source(&dir.join(rel))?;
        let mut m = crate::decl::lower_decl_module(&src, &origin)?;
        m.host_scope = manifest.host_scope.clone();
        m.inline = manifest.inline;
        m.entry = manifest.entry.clone();
        return Ok(m);
    }
    let src = load_entry(dir, &manifest.entry)?;
    Ok(Module {
        source: Some(src),
        entry: manifest.entry.clone(),
        // the manifest's mount properties ride the module regardless of
        // entry shape (`inline` is ink's; `host_scope` matters only for
        // host pkgs but is harmless elsewhere)
        inline: manifest.inline,
        host_scope: manifest.host_scope.clone(),
        ..Default::default()
    })
}

fn load_entry(dir: &Path, entry: &Entry) -> Result<String, String> {
    let rel = entry
        .lib
        .as_ref()
        .or(entry.type_path.as_ref())
        .or(entry.ir.as_ref())
        .ok_or_else(|| format!("module in {} has no entry", dir.display()))?;
    load_module_source(&dir.join(rel))
}

fn read_manifest(dir: &Path) -> Result<crate::session::Manifest, String> {
    let text = std::fs::read_to_string(dir.join("rut.toml")).map_err(|e| e.to_string())?;
    parse_manifest(&text).map_err(|e| e.to_string())
}

/// Resolve a manifest's `[deps]` recursively (RFC 0041 §3): each entry
/// is a relative `path` to a package directory, loaded and mounted
/// under its key. **First mount wins** — a name already in the session
/// (the embedder's, the root's, or an earlier dep's) is never
/// overwritten; a dep whose manifest `name` disagrees with its key is
/// an error naming both. `visiting` guards cycles.
fn resolve_deps(
    session: &mut Session,
    dir: &Path,
    manifest: &crate::session::Manifest,
    visiting: &mut Vec<std::path::PathBuf>,
) -> Result<(), String> {
    for (spec, desc) in &manifest.deps {
        if session.resolve(spec).is_ok() {
            continue; // already mounted — the embedder's (or an earlier) mount wins
        }
        let rel = desc
            .get("path")
            .ok_or_else(|| format!("dep `{spec}` has no `path`"))?;
        let dep_dir = dir.join(rel);
        if visiting.contains(&dep_dir) {
            return Err(format!(
                "cyclic use: `{spec}` ({}) is already being loaded",
                dep_dir.display()
            ));
        }
        let dm = read_manifest(&dep_dir)?;
        if dm.name.as_deref() != Some(spec.as_str()) {
            return Err(format!(
                "dep `{spec}` points at `{}` — the manifest there names it `{}`",
                dep_dir.display(),
                dm.name.as_deref().unwrap_or("<unnamed>")
            ));
        }
        let dep_module = load_entry_module(&dep_dir, &dm)?;
        session
            .register_module(spec, dep_module)
            .map_err(|e| e.to_string())?;
        visiting.push(dep_dir.clone());
        resolve_deps(session, &dep_dir, &dm, visiting)?;
        visiting.pop();
    }
    Ok(())
}

/// Mount one package directory — and, recursively, its `[deps]` — into
/// an EXISTING session: the programmatic counterpart of a manifest's
/// dep walk (native hosts, tests, plugin loaders). Returns the
/// package's own name. A name already mounted wins (RFC 0029 §4).
pub fn mount_dir(session: &mut Session, dir: &Path) -> Result<String, String> {
    let manifest = read_manifest(dir)?;
    let name = manifest
        .name
        .clone()
        .ok_or_else(|| format!("{} has no `name`", dir.join("rut.toml").display()))?;
    if session.resolve(&name).is_ok() {
        return Ok(name); // the embedder's mount outranks the directory
    }
    let module = load_entry_module(dir, &manifest)?;
    session
        .register_module(&name, module)
        .map_err(|e| e.to_string())?;
    let mut visiting = vec![dir.to_path_buf()];
    resolve_deps(session, dir, &manifest, &mut visiting)?;
    Ok(name)
}

/// Read a directory's `rut.toml` graph and compile it to one linked program.
pub fn compile_dir(dir: &Path) -> Result<crate::graph::GraphOutput, String> {
    let (session, root) = load_dir_session(dir)?;
    Ok(crate::compile_graph(&session, &root))
}
