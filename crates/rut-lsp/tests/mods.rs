use rut_ast::ast::Vis;
use rut_lexer::span::Span;
use rut_lsp::hover::DefIndex;
use rut_lsp::hover::{ModDef, TyDef, TyForm};
use rut_lsp::mods::*;

/// a named index standing at `mod_path` with the given mod edges
fn mod_index(pkg: &str, mod_path: &str, edges: &[(&str, Vis)]) -> DefIndex {
    let mut i = DefIndex {
        module: Some(pkg.to_string()),
        ..DefIndex::default()
    };
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
        ambient: false,
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
    let root = mod_index(
        "gadgets",
        "",
        &[("layout", Vis::Pub), ("secret", Vis::Self_)],
    );
    let layout = mod_index("gadgets", "layout", &[("grid", Vis::Pub)]);
    let grid = mod_index("gadgets", "layout/grid", &[]);
    let idxs = [&root, &layout, &grid];

    assert_eq!(
        walk_use_mods(&idxs, "gadgets", &["layout".into()]).as_deref(),
        Some("layout")
    );
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
    let mk = |p: &str| DocMods {
        mod_path: Some(p.into()),
        pkg: Some("app".into()),
    };

    // from layout/grid: a sibling resolves through the parent scope
    let doc = mk("layout/grid");
    assert_eq!(
        resolve_position_head(&doc, &grid, &idxs, "util").as_deref(),
        Some("layout/grid/util")
    );
    // the own-pkg head names the root
    assert_eq!(
        resolve_position_head(&doc, &grid, &idxs, "app").as_deref(),
        Some("")
    );
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
    assert!(
        !matches_mod(&stray, Some("gadgets"), "layout"),
        "unnamed never answers a named pkg"
    );
    // mod_path None answers the root for a named index
    let flat = mod_index("pouch", "", &[]);
    assert!(matches_mod(&flat, Some("pouch"), ""));
}

#[test]
fn display_spells_the_mounted_file() {
    assert_eq!(display(""), "mod.rut");
    assert_eq!(display("layout/grid"), "layout/grid/mod.rut");
}

#[test]
fn display_of_spells_a_single_file_child() {
    // a single-file module's REAL name rides its index's origin
    // basename; the dir-form guess stays the fallback
    let mut file_child = mod_index("gadgets", "imp", &[]);
    file_child.origin = "file:///ws/imp.rut".to_string();
    let mut dir_child = mod_index("gadgets", "layout", &[]);
    dir_child.origin = "file:///ws/layout/mod.rut".to_string();
    let idxs = [&file_child, &dir_child];
    assert_eq!(display_of(&idxs, Some("gadgets"), "imp"), "imp.rut");
    // the dir-form child keeps the loader's spelling (its origin
    // basename is the flavor's `mod.rut`)
    assert_eq!(
        display_of(&idxs, Some("gadgets"), "layout"),
        "layout/mod.rut"
    );
    // an unindexed child (or a non-file origin — `core`) falls back
    assert_eq!(display_of(&idxs, Some("gadgets"), "ghost"), "ghost/mod.rut");
    let mut core = mod_index("core", "prim", &[]);
    core.origin = "core".to_string();
    let single = [&core];
    assert_eq!(display_of(&single, None, "prim"), "prim/mod.rut");
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
