//! File modules in the editor index — the namespace walk over the
//! world's files (the editor face of the phase-3 semantic core).
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
//! the walk below never reads a directory listing.
//!
//! Two walks, the compiler's two gates mirrored:
//! - **cross-package** (a `use` path after the pkg head): every edge
//!   must be `pub` — a bare `mod`/`pub(pkg)` child never crosses;
//! - **intra-package** (a qualified position after `modname::`): any
//!   declared edge walks; the LEAF members tier-gate by visibility
//!   (`vis_allows` — the `vis_crosses` predicate verbatim).

use rut_ast::Vis;

use crate::world::collection::ModView;
use crate::world::{File, FileId, World};

/// the file's mod path — `None` answers the root (today's
/// single-file shapes: the prelude, unnamed workspace files,
/// flat deps)
pub(crate) fn mod_path_of(f: &File) -> &str {
    f.stamp.mod_path.as_deref().unwrap_or("")
}

/// a child's full mod path (`""` + `layout` → `"layout"`,
/// `"layout"` + `grid` → `"layout/grid"`)
pub(crate) fn child_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

/// A mod path's parent (`"a/b"` → `"a"`, `""`/`"a"` → `""`) — the
/// phase-3 helper mirrored (paths are pkg-relative, `/`-joined).
pub(crate) fn parent_of(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some((p, _)) => p,
        None => "",
    }
}

/// Segment-aware subtree test: is `from` `anc` itself or one of its
/// descendants? Plain `starts_with` would read `ab` under `a`.
fn under(from: &str, anc: &str) -> bool {
    if anc.is_empty() {
        return true;
    }
    from == anc || from.starts_with(&format!("{anc}/"))
}

/// the visibility predicate — may `from` (the reading file's mod
/// path) see a decl declared in `def_mod` with visibility `vis`?
/// The phase-3 `vis_crosses` verbatim: `pub`/`pub(pkg)` pass within
/// the package (only `pub` crosses packages — the use-walk gates the
/// crossing itself); `pub(super)` reaches the parent module's
/// subtree; a bare decl reaches the declaring module and descendants.
pub fn vis_allows(from: &str, def_mod: &str, vis: Vis) -> bool {
    match vis {
        Vis::Pub | Vis::Pkg => true,
        Vis::Super => under(from, parent_of(def_mod)),
        Vis::Self_ => under(from, def_mod),
    }
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

/// the `mod NAME;` edge `parent` (a world file) declares for `name`,
/// the child's full mod path attached
fn child_edge<'a>(parent: &'a File, name: &str) -> Option<(ModView<'a>, String)> {
    let from = mod_path_of(parent);
    parent
        .mods()
        .into_iter()
        .find(|m| m.name() == name)
        .map(|m| {
            let path = child_path(from, m.name());
            (m, path)
        })
}

/// The CROSS-PACKAGE walk: each segment after the pkg head must be a
/// `pub` edge of the module the walk stands at (`pub(pkg)`/bare
/// `mod` children never offered cross-package). Answers the resolved
/// mod path; `None` = a segment does not resolve or is not `pub` —
/// the caller offers nothing rather than wrong names.
pub fn walk_use_mods(w: &World, pkg: &str, segs: &[String]) -> Option<String> {
    let mut cur = String::new();
    for seg in segs {
        let parent = find_mod(w, Some(pkg), &cur)?;
        let (edge, path) = child_edge(w.file(parent), seg)?;
        if edge.vis() != Vis::Pub {
            return None;
        }
        cur = path;
    }
    Some(cur)
}

/// the file that speaks for a FOREIGN package's root — its stamped
/// root ([`find_mod`]); an UNNAMED flat file the host never named
/// answers by the path derivation ([`crate::world::pkg::matches_pkg`]) only when no
/// stamped root stands. The queried file never lands here (its own
/// package rides the own lanes; a path coincidence is no root).
pub(crate) fn pkg_root_file(w: &World, from: FileId, pkg: &str) -> Option<FileId> {
    find_mod(w, Some(pkg), "").or_else(|| {
        w.chain(from).find(|id| {
            *id != from
                && w.file(*id).stamp.module.is_none()
                && crate::world::pkg::matches_pkg(w.file(*id), pkg)
        })
    })
}

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

/// The INTRA-PACKAGE head resolution — the phase-3 `resolve_mod_head`
/// mirrored onto the world: `head` must be a `mod` child of the
/// file's own module or of one of its ancestors (nearest wins), and
/// the package's own name names the root module. Any declared edge
/// walks (visibility gates the LEAF members, not the head). The
/// file's own rows read first for its own scope; ancestor scopes
/// ride the world's stamped files. `file` is the READING file — its
/// own stamp keys the walk (the open document adopted its loader's
/// stamp, so this is exactly the pair the face knows).
pub fn resolve_position_head(w: &World, file: FileId, head: &str) -> Option<String> {
    let doc = w.file(file);
    let from = doc.stamp.mod_path.clone()?;
    let mut cur = from.clone();
    let mut own = true;
    loop {
        let scope: Option<FileId> = if own && cur == from {
            Some(file)
        } else {
            find_mod(w, doc.stamp.module.as_deref(), &cur)
        };
        own = false;
        if let Some(edge) =
            scope.and_then(|s| w.file(s).mods().into_iter().find(|m| m.name() == head))
        {
            return Some(child_path(&cur, edge.name()));
        }
        if cur.is_empty() {
            break;
        }
        cur = parent_of(&cur).to_string();
    }
    // the own-pkg head: the root module (the only spelling that
    // reaches the root's decls from a child)
    if doc.stamp.module.as_deref() == Some(head) {
        return Some(String::new());
    }
    None
}

/// The INTRA-PACKAGE interior walk: each further segment must be a
/// declared child of the module the walk stands at (any edge vis —
/// the phase-3 `walk_mod_path` shape; the leaf members tier-gate).
pub(crate) fn walk_position_mods(
    w: &World,
    pkg: Option<&str>,
    start: &str,
    segs: &[String],
) -> Option<String> {
    let mut cur = start.to_string();
    for seg in segs {
        let parent = find_mod(w, pkg, &cur)?;
        let (_edge, path) = child_edge(w.file(parent), seg)?;
        cur = path;
    }
    Some(cur)
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
