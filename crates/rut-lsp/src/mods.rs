//! File modules in the editor index — the namespace walk over the
//! def-index chain (the editor face of the phase-3 semantic core).
//!
//! A package's def indexes are keyed by the (module, mod_path) pair:
//! `module` is the bare pkg name a `use` path spells (the
//! `matches_pkg` fallback laws ride unchanged), `mod_path` is the
//! full file-module path (`""` = the pkg root's `mod.rut`,
//! `"layout/grid"` = that child). Each index records the `mod NAME;`
//! edges its file declares; the mounted child lives in its own index,
//! found through the chain at the child's mod path. Declarations are
//! the graph, exactly as the loader mounts them — the walk below
//! never reads a directory listing.
//!
//! Two walks, the compiler's two gates mirrored:
//! - **cross-package** (a `use` path after the pkg head): every edge
//!   must be `pub` — a bare `mod`/`pub(pkg)` child never crosses;
//! - **intra-package** (a qualified position after `modname::`): any
//!   declared edge walks; the LEAF members tier-gate by visibility
//!   (`vis_allows` — the `vis_crosses` predicate verbatim).

use rut_ast::ast::Vis;

use crate::hover::types::DefIndex;

/// what the face knows about the OPEN document's place in its
/// package's mod tree — the input that turns position-path completion
/// on. Faces derive it from what they already know (the stdio server
/// resolves the doc's manifest dir on the fs; the wasm face reads the
/// (module, mod_path) stamps its dep walk / workspace indexer laid
/// down); a default (all `None`) keeps position-path completion off
/// — the use-path tiers never needed it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocMods {
    /// the doc's own mod path within its package (`""` = the root's
    /// mod.rut); `None` = unknown / not a package file
    pub mod_path: Option<String>,
    /// the package's manifest name (the own-pkg head spelling —
    /// `app::helper` from a child names the root module)
    pub pkg: Option<String>,
}

/// the index's mod path — `None` answers the root (today's
/// single-file shapes: the std surface, unnamed workspace files,
/// flat deps)
pub(crate) fn mod_path_of(i: &DefIndex) -> &str {
    i.mod_path.as_deref().unwrap_or("")
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

/// does this index speak for `pkg`'s file module `path`? `pkg`
/// `None` matches UNNAMED indexes only (a workspace package's files
/// — and only where a face STAMPED the mod path, so unrelated
/// unstamped files can never impersonate a module).
pub fn matches_mod(i: &DefIndex, pkg: Option<&str>, path: &str) -> bool {
    match pkg {
        Some(p) => crate::definition::matches_pkg(i, p) && mod_path_of(i) == path,
        None => i.module.is_none() && i.mod_path.as_deref() == Some(path),
    }
}

/// the chain's index that IS `pkg`'s module `path`
pub(crate) fn find_mod<'a>(
    idxs: &[&'a DefIndex],
    pkg: Option<&str>,
    path: &str,
) -> Option<&'a DefIndex> {
    idxs.iter().copied().find(|i| matches_mod(i, pkg, path))
}

/// the `mod NAME;` edge `parent` (an index) declares for `name`,
/// the child's full mod path attached
fn child_edge<'a>(parent: &'a DefIndex, name: &str) -> Option<(&'a crate::hover::ModDef, String)> {
    let from = mod_path_of(parent);
    parent
        .mods
        .iter()
        .find(|m| m.name == name)
        .map(|m| (m, child_path(from, &m.name)))
}

/// The CROSS-PACKAGE walk: each segment after the pkg head must be a
/// `pub` edge of the module the walk stands at (`pub(pkg)`/bare
/// `mod` children never offered cross-package). Answers the resolved
/// mod path; `None` = a segment does not resolve or is not `pub` —
/// the caller offers nothing rather than wrong names.
pub fn walk_use_mods(idxs: &[&DefIndex], pkg: &str, segs: &[String]) -> Option<String> {
    let mut cur = String::new();
    for seg in segs {
        let parent = find_mod(idxs, Some(pkg), &cur)?;
        let (_edge, path) = child_edge(parent, seg)?;
        if _edge.vis != Vis::Pub {
            return None;
        }
        cur = path;
    }
    Some(cur)
}

/// The INTRA-PACKAGE head resolution — the phase-3 `resolve_mod_head`
/// mirrored onto the chain: `head` must be a `mod` child of the
/// doc's own module or of one of its ancestors (nearest wins), and
/// the package's own name names the root module. Any declared edge
/// walks (visibility gates the LEAF members, not the head). The doc's
/// own index reads first for its own scope (the freshest rows — the
/// chain's stamped twin may lag an open edit); ancestor scopes ride
/// the chain's stamped indexes.
pub fn resolve_position_head(
    doc: &DocMods,
    doc_index: &DefIndex,
    idxs: &[&DefIndex],
    head: &str,
) -> Option<String> {
    let from = doc.mod_path.clone()?;
    let mut cur = from.clone();
    let mut own = true;
    loop {
        let scope: Option<&DefIndex> = if own && cur == from {
            Some(doc_index)
        } else {
            find_mod(idxs, doc.pkg.as_deref(), &cur)
        };
        own = false;
        if let Some(edge) = scope.and_then(|s| s.mods.iter().find(|m| m.name == head)) {
            return Some(child_path(&cur, &edge.name));
        }
        if cur.is_empty() {
            break;
        }
        cur = parent_of(&cur).to_string();
    }
    // the own-pkg head: the root module (the only spelling that
    // reaches the root's decls from a child)
    if doc.pkg.as_deref() == Some(head) {
        return Some(String::new());
    }
    None
}

/// The INTRA-PACKAGE interior walk: each further segment must be a
/// declared child of the module the walk stands at (any edge vis —
/// the phase-3 `walk_mod_path` shape; the leaf members tier-gate).
pub(crate) fn walk_position_mods(
    idxs: &[&DefIndex],
    pkg: Option<&str>,
    start: &str,
    segs: &[String],
) -> Option<String> {
    let mut cur = start.to_string();
    for seg in segs {
        let parent = find_mod(idxs, pkg, &cur)?;
        let (_edge, path) = child_edge(parent, seg)?;
        cur = path;
    }
    Some(cur)
}

/// the mounted file's display path — `""` is `mod.rut`, a child
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
/// single-file module: its index's origin basename (`imp.rut`), never
/// the directory-form guess. The loader's dir-form spelling stays the
/// fallback — the child not indexed, or mounted directory-form (whose
/// origin basename is the flavor's `mod.rut`/`mod.d.rut`).
pub fn display_of(idxs: &[&DefIndex], pkg: Option<&str>, child: &str) -> String {
    let mounted = idxs.iter().copied().find(|i| {
        mod_path_of(i) == child && pkg.is_none_or(|p| crate::definition::matches_pkg(i, p))
    });
    match mounted.and_then(|i| origin_file_name(&i.origin)) {
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
