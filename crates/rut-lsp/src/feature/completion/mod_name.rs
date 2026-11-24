//! The mod-name completion slot's disk probe — the module files
//! beside the open document, exactly the loader's child spellings
//! (the mount law's one home is rut-driver's `mount_edges`). The env
//! capability's second pure read: [`list_child_mods`] asks the host's
//! `read_dir`; [`file_dir`] spells the open uri's directory.

use std::collections::HashSet;

use crate::env::Io;

/// one module candidate the mod-name slot offers — the name and the
/// file spelling the loader would mount (`harness/mod.rut`,
/// `util.rut`)
pub(crate) struct ModCandidate {
    pub name: String,
    pub file: String,
}

/// the doc uri's directory, `file://` uris only — the scheme KEPT
/// (`file:///ws/app/src/mod.rut` → `file:///ws/app/src`): the probe
/// hands the env a `file://`-shaped path (the doc uri's directory,
/// verbatim) and the HOST resolves it (`fileURLToPath` in the
/// extension; a host that cannot resolve answers `null` → an empty
/// list). Any other scheme (`untitled:`, the playground's) is no disk
/// to ask — `None`. Join with `/` only; `std::path` never enters.
pub(crate) fn file_dir(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let i = rest.rfind('/')?;
    Some(format!("file://{}", &rest[..i]))
}

/// the mod-name slot's disk probe — the module files beside the
/// current file, exactly the user's rule: the ONE directory probed is
/// the open document's own directory, and a candidate is what the
/// LOADER mounts there ([`rut_driver::mount_edges`]'s spellings, the one
/// home of that law): a folder containing the document flavor's
/// module file (`mod.d.rut` for a `.d.rut` doc, else `mod.rut`), or —
/// Impl flavor only — the single-file `NAME.rut` twin. Nothing else:
/// not the indexed chain (it can be stale, it cannot see an
/// undeclared sibling — the disk is the truth the loader reads), not
/// a nested folder, not an empty folder, not the document's own kind
/// (`mod.rut`, any `*.d.rut`), and a name under BOTH spellings is the
/// loader's loud ambiguity — never offered. Non-identifier or
/// reserved-word names drop. Sorted by name; no dir → nothing.
pub(crate) fn list_child_mods(io: &dyn Io, dir: &str, decl: bool) -> Vec<ModCandidate> {
    let Some(entries) = io.read_dir(dir) else {
        return Vec::new();
    };
    // the flavor's child file — the loader's own spelling
    let child_file = if decl {
        rut_driver::decl::DECL_CHILD_FILE
    } else {
        "mod.rut"
    };
    let mut out: Vec<ModCandidate> = Vec::new();
    for e in entries {
        if e.ends_with(".rut") {
            // the file form: the doc's own kind never rides, and the
            // Decl flavor never probes the single-file twin
            if decl || e == "mod.rut" || e.ends_with(".d.rut") {
                continue;
            }
            let name = e.strip_suffix(".rut").unwrap_or(&e).to_string();
            if is_mod_name(&name) {
                out.push(ModCandidate { name, file: e });
            }
        } else {
            // the dir form: the entry must CONTAIN the flavor's child
            // file — an empty folder, or a folder with some other
            // spelling, is not a candidate
            let has_child = io
                .read_dir(&format!("{dir}/{e}"))
                .is_some_and(|kids| kids.iter().any(|k| k == child_file));
            if !has_child || !is_mod_name(&e) {
                continue;
            }
            let file = format!("{e}/{child_file}");
            out.push(ModCandidate { name: e, file });
        }
    }
    // a name that arrived under BOTH spellings is dropped — the
    // loader refuses the ambiguity, so an offered name is always one
    // it mounts
    let mut seen: HashSet<&str> = HashSet::new();
    let mut dupes: HashSet<String> = HashSet::new();
    for c in &out {
        if !seen.insert(c.name.as_str()) {
            dupes.insert(c.name.clone());
        }
    }
    out.retain(|c| !dupes.contains(&c.name));
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// a legal mod name: non-empty, `[A-Za-z_][A-Za-z0-9_]*`, and not a
/// reserved word (the grammar's own table)
fn is_mod_name(name: &str) -> bool {
    let mut chars = name.chars();
    if !matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric()) && !rut_ast::parser::is_reserved_kw(name)
}
