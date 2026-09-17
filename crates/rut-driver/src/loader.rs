//! Filesystem loader — read a module directory (`rut.toml`) or a packed
//! `.rutbundle` (RFC 0038) into a [`Session`].
//!
//! One module is one directory. Its entry file may `use { .. } from
//! "./sibling.rut"`: those are *intra*-module includes (one scope), so the
//! loader inlines them into a single compilation unit. `use ... from
//! "scope:name"` stays an inter-module use, resolved by the `Session`.
//! A `.rutbundle` is the same contract zipped: `rut.toml` plus the rut
//! sources, includes inlined from archive entries instead of the
//! filesystem.
//!
//! The `Session` itself does no I/O (wasm hosts mount in memory); this
//! native helper is the counterpart that reads files.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::bundle::{parse_bundle, write_bundle};
use crate::session::{parse_manifest, Entry, Module, Session};

/// One chunk of a source scan: a plain line, or a use statement
/// buffered whole (it may span lines — the multi-line form must be
/// treated exactly like the single-line one).
enum Chunk<'a> {
    Line(&'a str),
    Use(String),
}

fn chunks(src: &str) -> Vec<Chunk<'_>> {
    let mut out = Vec::new();
    let mut lines = src.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim_start().starts_with("use") {
            let mut stmt = vec![line];
            while !stmt.last().map(|l| l.trim_end().ends_with(';')).unwrap_or(true) {
                match lines.next() {
                    Some(l) => stmt.push(l),
                    None => break,
                }
            }
            out.push(Chunk::Use(stmt.join("\n")));
        } else {
            out.push(Chunk::Line(line));
        }
    }
    out
}

/// The relative specifier of a `use ... from "./x.rut"` statement, if
/// any.
fn relative_use(stmt: &str) -> Option<String> {
    let t = stmt.trim_start();
    if !t.starts_with("use") {
        return None;
    }
    let after = t.split_once("from")?.1.trim_start();
    let spec = after.strip_prefix('"')?.split('"').next()?;
    if spec.starts_with("./") || spec.starts_with("../") {
        Some(spec.to_string())
    } else {
        None
    }
}

/// Expand one source through `resolve`: a relative use is replaced by
/// its expanded child (`Ok(None)` = already included — dropped, matching
/// the include-once rule); anything else is kept verbatim.
fn expand_scan(
    src: &str,
    resolve: &mut impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<String, String> {
    let mut out = String::new();
    for chunk in chunks(src) {
        match chunk {
            Chunk::Line(line) => {
                out.push_str(line);
                out.push('\n');
            }
            Chunk::Use(whole) => {
                let child = match relative_use(&whole) {
                    Some(rel) => resolve(&rel)?,
                    None => None,
                };
                match child {
                    Some(expanded) => out.push_str(&expanded),
                    None => out.push_str(&whole),
                }
                out.push('\n');
            }
        }
    }
    Ok(out)
}

/// Filesystem include expansion: read `path`, inline its relative uses
/// recursively (each file's own directory is the include root, so
/// `../` reaches sibling directories).
fn expand_fs(path: &Path, seen: &mut HashSet<PathBuf>) -> Result<String, String> {
    let src = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    expand_scan(&src, &mut |rel| {
        let child = dir.join(rel);
        if !seen.insert(child.clone()) {
            return Ok(None);
        }
        Ok(Some(expand_fs(&child, seen)?))
    })
}

/// Read `path` and inline its relative uses (`./x.rut`) recursively, so a
/// multi-file module becomes one compilation unit (one interner, one scope).
pub fn expand_module_source(path: &Path) -> Result<String, String> {
    let mut seen = HashSet::new();
    seen.insert(path.to_path_buf());
    expand_fs(path, &mut seen)
}

/// Bundle include normalization: `./x.rut` → `x.rut`; anything reaching
/// outside the archive root is refused (v1 bundles are flat).
fn bundle_key(rel: &str) -> Result<String, String> {
    let key = rel.strip_prefix("./").unwrap_or(rel);
    if key.starts_with('/') || key.split('/').any(|p| p == "..") {
        return Err(format!("bundle include `{rel}` escapes the bundle root"));
    }
    Ok(key.to_string())
}

/// Bundle include expansion: same include-once rule, but entries come from
/// the archive, not the filesystem.
fn expand_bundle(
    entries: &[(String, Vec<u8>)],
    key: &str,
    seen: &mut HashSet<String>,
) -> Result<String, String> {
    let src = read_entry(entries, key)?;
    expand_scan(&src, &mut |rel| {
        let child = bundle_key(rel)?;
        if !seen.insert(child.clone()) {
            return Ok(None);
        }
        Ok(Some(expand_bundle(entries, &child, seen)?))
    })
}

fn read_entry(entries: &[(String, Vec<u8>)], key: &str) -> Result<String, String> {
    match entries.iter().find(|(n, _)| n == key) {
        Some((_, bytes)) => String::from_utf8(bytes.clone())
            .map_err(|_| format!("bundle entry `{key}` is not UTF-8")),
        None => Err(format!("bundle has no entry `{key}`")),
    }
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
    let src = load_entry(dir, &manifest.entry)?;
    session
        .register_module(
            &root,
            Module { source: Some(src), entry: manifest.entry.clone(), ..Default::default() },
        )
        .map_err(|e| e.to_string())?;

    for (spec, desc) in &manifest.deps {
        let rel = desc
            .get("path")
            .ok_or_else(|| format!("dep `{spec}` has no `path`"))?;
        let dep_dir = dir.join(rel);
        let dt = std::fs::read_to_string(dep_dir.join("rut.toml")).map_err(|e| e.to_string())?;
        let dm = parse_manifest(&dt).map_err(|e| e.to_string())?;
        let src = load_entry(&dep_dir, &dm.entry)?;
        session
            .register_module(
                spec,
                Module { source: Some(src), entry: dm.entry, ..Default::default() },
            )
            .map_err(|e| e.to_string())?;
    }
    Ok((session, root))
}

/// Mount a `.rutbundle` file (RFC 0038): one module, its sources read from
/// the archive. The manifest's `format`/`format_version` are checked
/// before anything else is read — an unknown layout is refused, never
/// guessed at.
pub fn load_bundle_session(path: &Path) -> Result<(Session, String), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    load_bundle_bytes(&bytes, path)
}

/// [`load_bundle_session`] over in-memory bytes (tests, embedders).
pub fn load_bundle_bytes(bytes: &[u8], origin: &Path) -> Result<(Session, String), String> {
    let entries = parse_bundle(bytes).map_err(|e| format!("{}: {e}", origin.display()))?;
    let toml = read_entry(&entries, "rut.toml")
        .map_err(|_| format!("{}: no `rut.toml` entry — not a rut bundle", origin.display()))?;
    let manifest = parse_manifest(&toml).map_err(|e| format!("{}: {e}", origin.display()))?;
    // §4: the layout version gates everything — refuse before reading
    // any other entry
    if manifest.format.as_deref() != Some("rutbundle") {
        return Err(format!(
            "{}: rut.toml has no `format = \"rutbundle\"` — not a rut bundle",
            origin.display()
        ));
    }
    if manifest.format_version != Some(1) {
        return Err(format!(
            "{}: unknown bundle format_version {:?} — this loader knows version 1",
            origin.display(),
            manifest.format_version
        ));
    }
    let root = manifest
        .name
        .clone()
        .ok_or_else(|| format!("{}: rut.toml has no `name`", origin.display()))?;
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
    let mut seen = HashSet::new();
    seen.insert(key.clone());
    let src = expand_bundle(&entries, &key, &mut seen)?;

    let mut session = Session::new();
    session
        .register_module(
            &root,
            Module { source: Some(src), entry: manifest.entry.clone(), ..Default::default() },
        )
        .map_err(|e| e.to_string())?;
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
/// `rut.toml` first, then the entry source and every transitive relative
/// include, in include order. Same input directory ⇒ byte-identical bundle.
pub fn pack_dir(dir: &Path) -> Result<Vec<u8>, String> {
    let text = std::fs::read_to_string(dir.join("rut.toml")).map_err(|e| e.to_string())?;
    let manifest = parse_manifest(&text).map_err(|e| e.to_string())?;
    if manifest.name.is_none() {
        return Err(format!("{} has no `name`", dir.join("rut.toml").display()));
    }
    // the packed rut.toml is the directory's rut.toml byte-for-byte, so
    // the bundle keys must already be there — directory loading ignores
    // them, but a bundle loader refuses without them (RFC 0038 §2)
    if manifest.format.as_deref() != Some("rutbundle") || manifest.format_version != Some(1) {
        return Err(format!(
            "{} is not bundle-shaped — add `format = \"rutbundle\"` and `format_version = 1`",
            dir.join("rut.toml").display()
        ));
    }
    if !manifest.deps.is_empty() {
        return Err(format!(
            "{} declares `[deps]` — a v1 bundle is one module (RFC 0038 OQ-3)",
            dir.join("rut.toml").display()
        ));
    }
    if manifest.entry.type_path.is_some() || manifest.entry.ir.is_some() {
        return Err(format!(
            "{} has entry.type/entry.ir — v1 bundles carry rut sources only",
            dir.join("rut.toml").display()
        ));
    }
    let rel = manifest
        .entry
        .lib
        .as_ref()
        .ok_or_else(|| format!("module in {} has no entry", dir.display()))?;
    let mut entries: Vec<(String, Vec<u8>)> = vec![("rut.toml".to_string(), text.into_bytes())];
    collect_includes(dir, rel, &mut HashSet::new(), &mut entries)?;
    write_bundle(&entries).map_err(|e| e.to_string())
}

/// Collect `rel` and its transitive relative includes, include-once, in
/// include order.
fn collect_includes(
    dir: &Path,
    rel: &str,
    seen: &mut HashSet<String>,
    out: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), String> {
    if !seen.insert(rel.to_string()) {
        return Ok(());
    }
    let path = dir.join(rel);
    let src =
        std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    out.push((bundle_key(rel)?, src.clone().into_bytes()));
    for chunk in chunks(&src) {
        if let Chunk::Use(stmt) = chunk {
            if let Some(r) = relative_use(&stmt) {
                collect_includes(dir, &r, seen, out)?;
            }
        }
    }
    Ok(())
}

fn load_entry(dir: &Path, entry: &Entry) -> Result<String, String> {
    let rel = entry
        .lib
        .as_ref()
        .or(entry.type_path.as_ref())
        .or(entry.ir.as_ref())
        .ok_or_else(|| format!("module in {} has no entry", dir.display()))?;
    let mut seen = HashSet::new();
    seen.insert(dir.join(rel));
    expand_fs(&dir.join(rel), &mut seen)
}

/// Read a directory's `rut.toml` graph and compile it to one linked program.
pub fn compile_dir(dir: &Path) -> Result<crate::graph::GraphOutput, String> {
    let (session, root) = load_dir_session(dir)?;
    Ok(crate::compile_graph(&session, &root))
}
