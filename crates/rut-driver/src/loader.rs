//! Filesystem loader — read a directory's `rut.toml` graph into a [`Session`].
//!
//! One module is one directory. Its entry file may `import { .. } from
//! "./sibling.rut"`: those are *intra*-module includes (one scope), so the
//! loader inlines them into a single compilation unit. `import ... from
//! "scope:name"` stays an inter-module import, resolved by the `Session`.
//!
//! The `Session` itself does no I/O (wasm hosts mount in memory); this
//! native helper is the counterpart that reads files.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::session::{parse_manifest, Entry, Module, Session};

/// Read `path` and inline its relative imports (`./x.rut`) recursively, so a
/// multi-file module becomes one compilation unit (one interner, one scope).
pub fn expand_module_source(path: &Path) -> std::io::Result<String> {
    let mut seen = HashSet::new();
    seen.insert(path.to_path_buf());
    expand(path, &mut seen)
}

fn expand(path: &Path, seen: &mut HashSet<PathBuf>) -> std::io::Result<String> {
    let src = std::fs::read_to_string(path)?;
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut out = String::new();
    for line in src.lines() {
        match relative_import(line) {
            Some(rel) => {
                let child = dir.join(rel);
                if seen.insert(child.clone()) {
                    out.push_str(&expand(&child, seen)?);
                }
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }
    Ok(out)
}

/// The relative specifier of an `import ... from "./x.rut"` line, if any.
fn relative_import(line: &str) -> Option<String> {
    let t = line.trim_start();
    if !t.starts_with("import") {
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

fn load_entry(dir: &Path, entry: &Entry) -> Result<String, String> {
    let rel = entry
        .lib
        .as_ref()
        .or(entry.type_path.as_ref())
        .or(entry.ir.as_ref())
        .ok_or_else(|| format!("module in {} has no entry", dir.display()))?;
    expand_module_source(&dir.join(rel)).map_err(|e| e.to_string())
}

/// Read a directory's `rut.toml` graph and compile it to one linked program.
pub fn compile_dir(dir: &Path) -> Result<crate::graph::GraphOutput, String> {
    let (session, root) = load_dir_session(dir)?;
    Ok(crate::compile_graph(&session, &root))
}
