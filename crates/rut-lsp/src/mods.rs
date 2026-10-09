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
pub(crate) fn vis_allows(from: &str, def_mod: &str, vis: Vis) -> bool {
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
pub(crate) fn matches_mod(i: &DefIndex, pkg: Option<&str>, path: &str) -> bool {
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
pub(crate) fn walk_use_mods(idxs: &[&DefIndex], pkg: &str, segs: &[String]) -> Option<String> {
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
pub(crate) fn resolve_position_head(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hover::types::{ModDef, TyDef, TyForm};
    use rut_ast::ast::Vis;
    use rut_lexer::span::Span;

    /// a named index standing at `mod_path` with the given mod edges
    fn mod_index(pkg: &str, mod_path: &str, edges: &[(&str, Vis)]) -> DefIndex {
        let mut i = DefIndex::default();
        i.module = Some(pkg.to_string());
        if !mod_path.is_empty() || !edges.is_empty() {
            i.mod_path = Some(mod_path.to_string());
        }
        for (name, vis) in edges {
            i.mods.push(ModDef {
                name: name.to_string(),
                name_span: None,
                vis: *vis,
                doc: Vec::new(),
                span: Span::new(0, 0),
                line: 1,
            });
        }
        i
    }

    /// one pub type row
    fn pub_ty(name: &str, vis: Vis) -> TyDef {
        TyDef {
            name: name.to_string(),
            name_span: None,
            form: TyForm::Struct,
            vis,
            is_pub: vis == Vis::Pub,
            generics: Vec::new(),
            fields: Vec::new(),
            methods: Vec::new(),
            alias_target: None,
            doc: Vec::new(),
            span: Span::new(0, 0),
            line: 1,
        }
    }

    #[test]
    fn the_cross_package_walk_gates_pub_edges() {
        let root = mod_index("gadgets", "", &[("layout", Vis::Pub), ("secret", Vis::Self_)]);
        let layout = mod_index("gadgets", "layout", &[("grid", Vis::Pub)]);
        let grid = mod_index("gadgets", "layout/grid", &[]);
        let idxs = [&root, &layout, &grid];

        assert_eq!(walk_use_mods(&idxs, "gadgets", &["layout".into()]).as_deref(), Some("layout"));
        assert_eq!(
            walk_use_mods(&idxs, "gadgets", &["layout".into(), "grid".into()]).as_deref(),
            Some("layout/grid")
        );
        // a bare `mod` child never crosses
        assert_eq!(walk_use_mods(&idxs, "gadgets", &["secret".into()]), None);
        // an unknown segment is a miss
        assert_eq!(walk_use_mods(&idxs, "gadgets", &["ghost".into()]), None);
        // an unknown package is a miss
        assert_eq!(walk_use_mods(&idxs, "pouch", &["layout".into()]), None);
    }

    #[test]
    fn the_position_head_walks_ancestors_and_the_pkg_name() {
        let root = mod_index("app", "", &[("layout", Vis::Pub)]);
        let layout = mod_index("app", "layout", &[("grid", Vis::Pub)]);
        let grid = mod_index("app", "layout/grid", &[("util", Vis::Self_)]);
        let util = mod_index("app", "layout/grid/util", &[]);
        let idxs = [&root, &layout, &grid, &util];
        let mk = |p: &str| DocMods { mod_path: Some(p.into()), pkg: Some("app".into()) };

        // from layout/grid: a sibling resolves through the parent scope
        let doc = mk("layout/grid");
        assert_eq!(
            resolve_position_head(&doc, &grid, &idxs, "util").as_deref(),
            Some("layout/grid/util")
        );
        // the own-pkg head names the root
        assert_eq!(resolve_position_head(&doc, &grid, &idxs, "app").as_deref(), Some(""));
        // from layout: its own child (read off the doc's own rows when
        // the doc index IS that module)
        let doc2 = mk("layout");
        assert_eq!(
            resolve_position_head(&doc2, &layout, &idxs, "grid").as_deref(),
            Some("layout/grid")
        );
        // unknown pkg name: a miss
        assert_eq!(resolve_position_head(&doc2, &layout, &idxs, "pouch"), None);
        // an unstamped doc (mod_path None) resolves nothing
        let flat = DocMods::default();
        assert_eq!(resolve_position_head(&flat, &root, &idxs, "grid"), None);
    }

    #[test]
    fn the_vis_predicate_mirrors_the_compiler_tiers() {
        // pub + pub(pkg) pass within the package
        assert!(vis_allows("", "layout", Vis::Pub));
        assert!(vis_allows("layout/grid", "layout", Vis::Pkg));
        // private: the declaring module and descendants only
        assert!(vis_allows("layout", "layout", Vis::Self_));
        assert!(vis_allows("layout/grid", "layout", Vis::Self_));
        assert!(!vis_allows("", "layout", Vis::Self_));
        assert!(!vis_allows("layout/util", "layout/grid", Vis::Self_));
        // pub(super): the parent module's subtree — a pub(super) decl
        // in a ROOT-level module reaches the root's subtree (the whole
        // package, the empty-ancestor law `under` speaks)
        assert!(vis_allows("", "layout", Vis::Super));
        assert!(vis_allows("elsewhere", "layout", Vis::Super));
        // nested: the parent's subtree only
        assert!(vis_allows("layout/other", "layout/grid", Vis::Super));
        assert!(!vis_allows("elsewhere", "layout/grid", Vis::Super));
        // the segment law: `ab` is not under `a`
        assert!(!vis_allows("ab", "a", Vis::Self_));
        assert!(vis_allows("a/b", "a", Vis::Self_));
    }

    #[test]
    fn matches_mod_keys_on_the_pair() {
        let i = mod_index("gadgets", "layout", &[]);
        assert!(matches_mod(&i, Some("gadgets"), "layout"));
        assert!(!matches_mod(&i, Some("gadgets"), ""));
        assert!(!matches_mod(&i, Some("pouch"), "layout"));
        // an unnamed index participates only where a face STAMPED it —
        // unstamped files never impersonate a module, not even the root
        // (the doc's own scope reads off its own rows, not a lookup)
        let mut stray = DefIndex::default();
        assert!(!matches_mod(&stray, None, ""));
        assert!(!matches_mod(&stray, None, "layout"));
        stray.mod_path = Some("layout".into());
        assert!(matches_mod(&stray, None, "layout"));
        assert!(!matches_mod(&stray, Some("gadgets"), "layout"), "unnamed never answers a named pkg");
        // mod_path None answers the root for a named index
        let flat = mod_index("pouch", "", &[]);
        assert!(matches_mod(&flat, Some("pouch"), ""));
    }

    #[test]
    fn display_spells_the_mounted_file() {
        assert_eq!(display(""), "mod.rut");
        assert_eq!(display("layout/grid"), "layout/grid/mod.rut");
    }

    // keep the unused-import lint honest — pub_ty shapes a row the
    // vis tests assert through completion.rs
    #[test]
    fn pub_ty_rows_carry_their_vis() {
        let t = pub_ty("Column", Vis::Pub);
        assert!(t.is_pub);
        let t = pub_ty("Hidden", Vis::Pkg);
        assert!(!t.is_pub && t.vis == Vis::Pkg);
    }
}
