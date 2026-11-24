//! File modules in the editor index — the adapters that carry the
//! world's stamped files into rut-lir's pure mod laws (the editor face
//! of the phase-3 semantic core).
//!
//! A package's files are keyed by the (module, mod_path) pair:
//! `module` is the bare pkg name a `use` path spells, `mod_path` the
//! full file-module path (`""` = the pkg root's `mod.rut`,
//! `"layout/grid"` = that child). The pair is a STAMP the mount lanes
//! lay down — a lookup keys on it exactly ([`matches_mod`]); the
//! `matches_pkg` URI-segment derivation answers the FLAT sweeps'
//! question only (which unnamed files speak for a package name) and
//! never lands a mod-tree lookup, so an unstamped file cannot
//! impersonate its package's root. Each file records the
//! `mod NAME;` edges its decl map carries; the mounted child lives in
//! its own file, found through the world at the child's mod path.
//! Declarations are the graph, exactly as the loader mounts them —
//! nothing here reads a directory listing.
//!
//! The WALKS are rut-lir's, one home each — this module builds the
//! tree ([`mod_tree`]) and calls the law:
//! - the own-package head → `rut_lir::check::mod_head`
//!   ([`position_head`]);
//! - the intra-package interior walk (any declared edge; the LEAF
//!   members tier-gate by visibility — `rut_lir::check::vis_crosses`)
//!   → `rut_lir::check::mod_walk` ([`position_walk`]);
//! - the cross-package walk (every edge `pub` — a bare `mod`/`pub(pkg)`
//!   child never crosses) → `rut_lir::check::mod_walk_pub`
//!   ([`use_walk`]).

use rut_ast::Vis;
use rut_lir::check::{child_path, parent_of, ModInputs, ModScope};

use crate::world::{File, FileId, World};

/// the file's mod path — `None` answers the root (today's
/// single-file shapes: the prelude, unnamed workspace files,
/// flat deps)
pub(crate) fn mod_path_of(f: &File) -> &str {
    f.stamp.mod_path.as_deref().unwrap_or("")
}

/// does this file speak for `pkg`'s file module `path`? A mod-tree
/// lookup keys on the STAMP a mount laid down — the explicit manifest
/// name beside the mod path — never on a URI-segment derivation: an
/// unstamped file (`Stamp::scratch`/`Stamp::workspace` with no pkg)
/// must not land a package's root because its path happens to name
/// the package. A flat named file
/// (`mod_path` `None` — a single-file dep, a package's surface beside
/// its root) speaks for its package's ROOT only. `pkg` `None` matches
/// UNNAMED files only, and only where a face STAMPED the mod path.
pub fn matches_mod(f: &File, pkg: Option<&str>, path: &str) -> bool {
    match pkg {
        Some(p) => {
            f.stamp.module.as_deref() == Some(p)
                && match f.stamp.mod_path.as_deref() {
                    Some(mp) => mp == path,
                    // the flat shape sits at its package's root
                    None => path.is_empty(),
                }
        }
        None => f.stamp.module.is_none() && f.stamp.mod_path.as_deref() == Some(path),
    }
}

/// the world's file that IS `pkg`'s module `path`: the STAMPED tree
/// entry first (the mounted root's `(pkg, Some(""))` outranks a flat
/// one of the same package — a lib's surface beside its body), the
/// flat shape only when no stamped entry stands.
pub(crate) fn find_mod(w: &World, pkg: Option<&str>, path: &str) -> Option<FileId> {
    w.files()
        .find(|id| matches_mod(w.file(*id), pkg, path) && w.file(*id).stamp.mod_path.is_some())
        .or_else(|| w.files().find(|id| matches_mod(w.file(*id), pkg, path)))
}

/// the file that speaks for a FOREIGN package's root — its stamped
/// root ([`find_mod`]); an UNNAMED flat file the host never named
/// answers by the path derivation ([`crate::resolve::pkg::matches_pkg`]) only when no
/// stamped root stands. The queried file never lands here (its own
/// package rides the own lanes; a path coincidence is no root) — and
/// a use leaf that names a MODULE — `use calc::{ math }`: the walked
/// module's `pub mod math;` child, its file beside its mod path. The
/// cross-package door only (a module is no import inside a package —
/// the compiler names the walk instead); `None` = the leaf is no
/// module of the walked path.
pub(crate) fn use_module_leaf(
    w: &World,
    pkg: &str,
    walked: &str,
    name: &str,
) -> Option<(FileId, String)> {
    let parent = find_mod(w, Some(pkg), walked)?;
    let edge = w
        .file(parent)
        .mods()
        .into_iter()
        .find(|m| m.name() == name && m.vis() == Vis::Pub)?;
    let path = child_path(walked, edge.name());
    find_mod(w, Some(pkg), &path).map(|f| (f, path))
}

/// the `mod NAME;` edges a scope file offers, the child's full mod
/// path attached — the rows [`mod_tree`] builds each scope's children
/// from
fn mod_edges(f: &File, path: &str) -> std::collections::BTreeMap<String, (String, Vis)> {
    f.mods()
        .into_iter()
        .map(|m| {
            let name = m.name().to_string();
            let child = child_path(path, &name);
            (name, (child, m.vis()))
        })
        .collect()
}

/// The mod tree the world's stamped files spell for `pkg` — the
/// [`ModInputs`] rut-lir's pure laws walk. One scope row per file
/// module: a stamped file keys its `stamp.mod_path`, a flat named
/// file (`mod_path` `None`) speaks for the package's root
/// ([`matches_mod`]'s law). The store order answers everywhere —
/// the first file in store order supplies a shared scope
/// ([`find_mod`]'s law) — and the `prefer` file's own rows read first
/// for its own path (the document's own scope). Every stamped child
/// path implies its ancestor rows (bare where no file stands): the
/// tree is structurally complete exactly as the compiler's
/// [`ModInputs`] is. The ROOT's `""` row exists only where edges
/// imply it — the file that speaks for the root fills it, and a
/// childless root stays absent — so `is_flat` answers the compiler's
/// question: a package with no `mod` child anywhere has NO tree, and
/// `mod_head`'s own-spec arm refuses its own name (the compiler's
/// flat law).
pub(crate) fn mod_tree(w: &World, pkg: Option<&str>, prefer: FileId) -> ModInputs {
    let mut tree = ModInputs::default();
    let mine = |f: &File| f.stamp.module.as_deref() == pkg;
    // the stamped rows: every file stamped at a child path speaks for
    // that module; the first in store order supplies the scope (the
    // `prefer` overwrite below re-answers for the document's own)
    let mut stamped: Vec<&str> = Vec::new();
    for id in w.files() {
        let f = w.file(id);
        if !mine(f) {
            continue;
        }
        let Some(p) = f.stamp.mod_path.as_deref() else {
            continue; // the flat shape rides the root pass below
        };
        if p.is_empty() {
            continue; // the root pass below owns the `""` row
        }
        stamped.push(p);
        tree.scopes
            .entry(p.to_string())
            .or_insert_with(|| ModScope {
                parent: parent_of(p).to_string(),
                children: mod_edges(f, p),
            });
    }
    // the ancestor rows every stamped child implies — bare where no
    // file stands (the walk passes through), never clobbering a
    // standing row
    for p in &stamped {
        let mut anc = parent_of(p).to_string();
        loop {
            if tree.scopes.contains_key(&anc) {
                break; // the row stands; its own chain came with it
            }
            let parent = parent_of(&anc).to_string();
            tree.scopes.insert(
                anc.clone(),
                ModScope {
                    parent: parent.clone(),
                    children: std::collections::BTreeMap::new(),
                },
            );
            if anc.is_empty() {
                break;
            }
            anc = parent;
        }
    }
    // the ROOT row: the file that speaks for the root — the first
    // stamped `""` file in store order, a flat named file only where
    // no stamped root stands ([`matches_mod`]'s law) — fills it with
    // its edges; a childless root contributes no row (the flat law)
    let root = w
        .files()
        .map(|id| w.file(id))
        .find(|f| mine(f) && f.stamp.mod_path.as_deref() == Some(""))
        .or_else(|| {
            pkg.and_then(|_| {
                w.files()
                    .map(|id| w.file(id))
                    .find(|f| mine(f) && f.stamp.mod_path.is_none())
            })
        });
    if let Some(f) = root {
        let children = mod_edges(f, "");
        if !children.is_empty() {
            let row = tree.scopes.entry(String::new()).or_default();
            row.parent = String::new();
            row.children = children;
        }
    }
    // the document's own rows read first: the `prefer` file overwrites
    // the scope at its own path (its buffer is the truth). A childless
    // root stays absent — an empty overwrite would forge the row the
    // flat law refuses
    let doc = w.file(prefer);
    if mine(doc) {
        if let Some(p) = doc.stamp.mod_path.as_deref() {
            let children = mod_edges(doc, p);
            if !children.is_empty() || !p.is_empty() {
                let row = tree.scopes.entry(p.to_string()).or_default();
                row.parent = parent_of(p).to_string();
                row.children = children;
            }
        }
    }
    tree
}

/// The own-package head resolution — `head` must be a `mod` child of
/// the reading file's module or of one of its ancestors (nearest
/// wins; any declared edge walks — visibility gates the LEAF members,
/// not the head), else the package's own name names the root module.
/// rut-lir's [`ModInputs`]-walking `mod_head` is the one home; this
/// reads the doc's stamp into it ([`mod_tree`] keys the tree off the
/// same stamp, so this is exactly the pair the face knows). A doc
/// with no mod path is in no package — the own lane answers nothing
/// for it.
pub(crate) fn position_head(w: &World, file: FileId, head: &str) -> Option<String> {
    let doc = w.file(file);
    let from = doc.stamp.mod_path.clone()?;
    let pkg = doc.stamp.module.as_deref();
    rut_lir::check::mod_head(&mod_tree(w, pkg, file), pkg.unwrap_or(""), &from, head)
}

/// The intra-package interior walk — each further segment must be a
/// declared child of the module the walk stands at (any edge vis; the
/// LEAF members tier-gate). rut-lir's `mod_walk` over the world's
/// tree; `None` = a segment does not resolve — the caller offers
/// nothing rather than wrong names.
pub(crate) fn position_walk(
    w: &World,
    pkg: Option<&str>,
    file: FileId,
    start: &str,
    segs: &[String],
) -> Option<String> {
    let segs: Vec<&str> = segs.iter().map(String::as_str).collect();
    rut_lir::check::mod_walk(&mod_tree(w, pkg, file), start, &segs).ok()
}

/// The CROSS-PACKAGE walk — as [`position_walk`], and every edge must
/// be `pub` (rut-lir's `mod_walk_pub`; a bare `mod`/`pub(pkg)` child
/// never crosses). `None` = a segment does not resolve or is not
/// `pub` — the caller offers nothing rather than wrong names.
pub(crate) fn use_walk(w: &World, pkg: &str, file: FileId, segs: &[String]) -> Option<String> {
    let segs: Vec<&str> = segs.iter().map(String::as_str).collect();
    rut_lir::check::mod_walk_pub(&mod_tree(w, Some(pkg), file), "", &segs).ok()
}

/// the world file's display path — `""` is `mod.rut`, a child
/// `<parent>/mod.rut` (the loader's spelling; hover and completion
/// details show it)
pub fn display(path: &str) -> String {
    if path.is_empty() {
        "mod.rut".to_string()
    } else {
        format!("{path}/mod.rut")
    }
}

/// [`display`] with the REAL file name when the child is mounted as a
/// single-file module: its file's origin basename (`imp.rut`), never
/// the directory-form guess. The loader's dir-form spelling stays the
/// fallback — the child not indexed, or mounted directory-form (whose
/// origin basename is the flavor's `mod.rut`/`mod.d.rut`).
pub fn display_of(w: &World, pkg: Option<&str>, child: &str) -> String {
    let mounted = w.files().find(|id| {
        mod_path_of(w.file(*id)) == child
            && pkg.is_none_or(|p| w.file(*id).stamp.module.as_deref() == Some(p))
    });
    match mounted
        .map(|id| w.file(id))
        .and_then(|f| origin_file_name(&f.stamp.origin))
    {
        Some(name) if name != "mod.rut" && name != rut_driver::decl::DECL_CHILD_FILE => name,
        _ => display(child),
    }
}

/// [`display_of`] over ONE resolved file — the render's shape (the
/// resolved module's own real name; the dir-form fallback)
pub(crate) fn display_of_single(f: &File, child: &str) -> String {
    match origin_file_name(&f.stamp.origin) {
        Some(name) if name != "mod.rut" && name != rut_driver::decl::DECL_CHILD_FILE => name,
        _ => display(child),
    }
}

/// the origin's `.rut` file name (its last path segment when it spells
/// one) — `file:///…/imp.rut`, `rut/pouch/src/mod.rut`, a bundle key;
/// `core` (and any non-file origin) answers `None`.
fn origin_file_name(origin: &str) -> Option<String> {
    let name = origin.rsplit('/').next()?;
    (name.ends_with(".rut")).then(|| name.to_string())
}
