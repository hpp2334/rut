//! Package identity and file identity — the world facts the
//! use-graph gates read. [`matches_pkg`] decides which world files
//! speak for a package name (the FLAT sweeps' question — never a
//! mod-tree lookup's, which keys on the stamp); the file-identity key
//! the store maps origins through lives with the store itself
//! (`world::file_key`); [`import_map`] is the document's imported
//! (name, pkg) set (the
//! auto-import tier's dedup and typeDefinition's computed-head gate —
//! the file-scope lanes answer through the file scope instead).

use std::collections::HashMap;

use crate::world::File;

/// does this file speak for `pkg`? — the explicit manifest module
/// name FIRST (`Stamp::module`, the dep/prelude file's `Some`); on
/// `None` the old derivation verbatim: exact origin, a path segment
/// named `pkg` (…/pouch/pouch.rut), or the file stem
/// (…/gadgets/lib.rut for pkg `gadgets`). This is the FLAT sweeps'
/// question ("which unnamed files answer for this package name") —
/// never a mod-tree lookup's: a package-scoped module lookup keys on
/// the stamp ([`crate::resolve::mods::matches_mod`]), so an unstamped
/// file cannot land a package's root because its path happens to name
/// the package.
pub fn matches_pkg(f: &File, pkg: &str) -> bool {
    if let Some(m) = &f.stamp.module {
        return m == pkg;
    }
    let hay = f.stamp.src_path.as_deref().unwrap_or(&f.stamp.origin);
    if hay == pkg {
        return true;
    }
    hay.split(['/', '\\'])
        .any(|seg| seg.strip_suffix(".rut").unwrap_or(seg) == pkg)
}

/// a module let crosses NO package surface — the compiler's own law
/// (`program.rs`: "a module let does not cross package surfaces; it
/// binds only inside its own package") — the offer tiers' gate: only
/// a file of the ASKING file's own package answers a let. The
/// `Option<String>` stamp comparison: an unowned asking file keeps
/// only unowned rows. THE ONE EXEMPTION the compiler itself spells: a
/// HOST pkg's surface `pub let` folds into const rows that bind BARE
/// at the pkg's root and DO cross — `use calc_host::{ PI }` compiles
/// and runs (the std math family, the book's documented spelling,
/// host-fns.md) — and the mount's word for a host file is the Decl
/// mode (a mount stamps the mode it parsed with), so a Decl-mode
/// file's lets answer every asker. Probed live under the store guard:
/// the lib pkg's module let import errors naming the law above, the
/// host surface's `PI` import compiles clean.
pub fn same_pkg(asking: Option<&str>, f: &File) -> bool {
    f.stamp.mode == crate::doc::Mode::Decl || f.stamp.module.as_deref() == asking
}

/// the document's import map: imported name -> its `use` pkg — the
/// (name, pkg) set shape the auto-import tier dedups against (and
/// typeDefinition's computed-head gate restricts by; the bare/type
/// tiers offer through the file scope's own resolve,
/// `use_bound_scope`). A `use` names its leaves: every entry is an
/// explicit leaf beside the head it rode, and an already-imported
/// name is imported.
pub(crate) fn import_map(doc: &File) -> HashMap<&str, &str> {
    doc.uses()
        .into_iter()
        .map(|u| (u.name(), u.pkg()))
        .collect()
}
