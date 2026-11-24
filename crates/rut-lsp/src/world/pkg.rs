//! Package identity and file identity — the world facts the
//! use-graph gates read. [`matches_pkg`] decides which mounted files
//! speak for a package name (the FLAT sweeps' question — never a
//! mod-tree lookup's, which keys on the stamp); [`file_key`] is one
//! physical file's identity however the lanes spelled its origin;
//! [`import_map`] is the document's imported (name, pkg) set (the
//! auto-import tier's dedup and typeDefinition's computed-head gate —
//! the file-scope lanes answer through `Chain::file_scope` instead).

use std::collections::HashMap;

use crate::world::collection::{Chain, MountedFile};

/// does this file speak for `pkg`? — the explicit manifest module
/// name FIRST (`Mount::module`, the dep/prelude file's `Some`); on
/// `None` the old derivation verbatim: exact origin, a path segment
/// named `pkg` (…/pouch/pouch.rut), or the file stem
/// (…/gadgets/lib.rut for pkg `gadgets`). This is the FLAT sweeps'
/// question ("which unnamed files answer for this package name") —
/// never a mod-tree lookup's: a package-scoped module lookup keys on
/// the stamp ([`crate::world::mods::matches_mod`]), so the open
/// document's unstamped entry cannot land a package's root because
/// its path happens to name the package.
pub fn matches_pkg(f: &MountedFile, pkg: &str) -> bool {
    if let Some(m) = &f.mount.module {
        return m == pkg;
    }
    let hay = f.mount.src_path.as_deref().unwrap_or(&f.mount.origin);
    if hay == pkg {
        return true;
    }
    hay.split(['/', '\\'])
        .any(|seg| seg.strip_suffix(".rut").unwrap_or(seg) == pkg)
}

/// a rooted path, lexically normalized — `.` dropped, `..` collapsed
/// segment by segment (a path dep's origins carry the joined `..` —
/// the walk never munges them; the KEY reads through). No `std::path`
/// (the crate's law): the string walk the hosts' env resolves with.
pub(crate) fn lexical(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    format!("/{}", out.join("/"))
}

/// one physical file's identity — the path its origin names, however
/// the mounting lane spelled it: the workspace indexer's uri
/// (`file:///ws/app/src/mod.rut`) and the dep walk's rooted string
/// (`/ws/app/src/mod.rut`) are ONE file, lexically normalized. An
/// origin naming no rooted path (the prelude's `core`, a store key, a
/// virtual-fs uri) is its own key.
pub(crate) fn file_key(origin: &str) -> String {
    let path = origin.strip_prefix("file://").unwrap_or(origin);
    if path.starts_with('/') {
        lexical(path)
    } else {
        origin.to_string()
    }
}

/// the document's import map: imported name -> its `use` pkg — the
/// (name, pkg) set shape the auto-import tier dedups against (and
/// typeDefinition's computed-head gate restricts by; the bare/type
/// tiers offer through the file scope's own resolve,
/// `Chain::use_bound_scope`). The explicit leaves answer exactly as
/// before (leaf →
/// head); a glob leaf (`use pkg::*`) binds every pub name the walked
/// path exports — enumerated through the chain (the same surface walk
/// the use tier offers), keyed to the pkg each name declares in (the
/// head itself on the direct shape), inserted ONLY when the key is
/// absent (explicit wins; stable first-occurrence merge).
pub(crate) fn import_map<'a>(
    chain: &'a Chain<'a>,
    doc: &'a MountedFile,
) -> HashMap<&'a str, &'a str> {
    let mut out: HashMap<&'a str, &'a str> = doc
        .uses()
        .into_iter()
        .map(|u| (u.name(), u.pkg()))
        .collect();
    for g in doc.glob_uses() {
        for (name, pkg) in chain.surface_names(g.pkg, &g.interior) {
            out.entry(name).or_insert(pkg);
        }
    }
    out
}
