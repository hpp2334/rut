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

    for (spec, desc) in &manifest.deps {
        let rel = desc
            .get("path")
            .ok_or_else(|| format!("dep `{spec}` has no `path`"))?;
        let dep_dir = dir.join(rel);
        let dt = std::fs::read_to_string(dep_dir.join("rut.toml")).map_err(|e| e.to_string())?;
        let dm = parse_manifest(&dt).map_err(|e| e.to_string())?;
        let dep_module = load_entry_module(&dep_dir, &dm)?;
        session
            .register_module(spec, dep_module)
            .map_err(|e| e.to_string())?;
    }
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
    let src = read_entry(&entries, &key)?;

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
/// `rut.toml` first, then the entry source. Same input directory ⇒
/// byte-identical bundle.
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
    let key = bundle_key(rel)?;
    let src = std::fs::read_to_string(dir.join(rel))
        .map_err(|e| format!("cannot read {}: {e}", dir.join(rel).display()))?;
    let entries: Vec<(String, Vec<u8>)> =
        vec![("rut.toml".to_string(), text.into_bytes()), (key, src.into_bytes())];
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
    Ok(Module { source: Some(src), entry: manifest.entry.clone(), ..Default::default() })
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

/// Read a directory's `rut.toml` graph and compile it to one linked program.
pub fn compile_dir(dir: &Path) -> Result<crate::graph::GraphOutput, String> {
    let (session, root) = load_dir_session(dir)?;
    Ok(crate::compile_graph(&session, &root))
}
