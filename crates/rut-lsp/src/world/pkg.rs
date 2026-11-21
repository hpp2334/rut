//! Package identity and the doc's import table — the world facts the
//! use-graph gates read. [`matches_pkg`] decides which mounted files
//! speak for a package name; [`import_map`] is the document's
//! imported (name, pkg) set the tier and chain gates match against.

use std::collections::HashMap;

use crate::world::collection::MountedFile;

/// does this file speak for `pkg`? — the explicit manifest module
/// name FIRST (`Mount::module`, the dep/prelude file's `Some`); on
/// `None` the old derivation verbatim: exact origin, a path segment
/// named `pkg` (…/pouch/pouch.rut), or the file
/// stem (…/gadgets/lib.rut for pkg `gadgets`)
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

/// the document's import map: imported name -> its `use` pkg — the
/// (name, pkg) set shape the completion tiers match foreign names
/// against (the bare tier's imported gate and the auto-import tier's
/// dedup)
pub(crate) fn import_map(doc: &MountedFile) -> HashMap<&str, &str> {
    doc.uses()
        .into_iter()
        .map(|u| (u.name(), u.pkg()))
        .collect()
}
