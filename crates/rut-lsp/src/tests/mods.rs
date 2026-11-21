use crate::world::collection::Chain;
use crate::world::mods::*;
use crate::world::Mount;
use crate::world::MountedFile;
use rut_ast::Vis;

/// a named mounted file standing at `mod_path`, its `mod NAME;` edges
/// parsed from fixture text the way the dep walk mounts them
fn mod_index(pkg: &str, mod_path: &str, edges: &[(&str, Vis)]) -> MountedFile {
    let mut src = String::new();
    for (name, vis) in edges {
        if *vis == Vis::Pub {
            src.push_str(&format!("pub mod {name};\n"));
        } else {
            src.push_str(&format!("mod {name};\n"));
        }
    }
    MountedFile::from_src(
        Mount {
            module: Some(pkg.to_string()),
            mod_path: (!mod_path.is_empty() || !edges.is_empty()).then(|| mod_path.to_string()),
            ..Mount::default()
        },
        src,
    )
}

/// the chain over the fixture files, in order
fn chain(files: &[MountedFile]) -> Chain<'_> {
    Chain::new(files.iter().collect())
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
    let files = [root, layout, grid];
    let c = chain(&files);

    assert_eq!(
        walk_use_mods(&c, "gadgets", &["layout".into()]).as_deref(),
        Some("layout")
    );
    assert_eq!(
        walk_use_mods(&c, "gadgets", &["layout".into(), "grid".into()]).as_deref(),
        Some("layout/grid")
    );
    // a bare `mod` child never crosses
    assert_eq!(walk_use_mods(&c, "gadgets", &["secret".into()]), None);
    // an unknown segment is a miss
    assert_eq!(walk_use_mods(&c, "gadgets", &["ghost".into()]), None);
    // an unknown package is a miss
    assert_eq!(walk_use_mods(&c, "pouch", &["layout".into()]), None);
}

#[test]
fn the_position_head_walks_ancestors_and_the_pkg_name() {
    let root = mod_index("app", "", &[("layout", Vis::Pub)]);
    let layout = mod_index("app", "layout", &[("grid", Vis::Pub)]);
    let grid = mod_index("app", "layout/grid", &[("util", Vis::Self_)]);
    let util = mod_index("app", "layout/grid/util", &[]);
    let files = [root, layout, grid, util];
    let c = chain(&files);
    let mk = |p: &str| DocMods {
        mod_path: Some(p.into()),
        pkg: Some("app".into()),
    };

    // from layout/grid: a sibling resolves through the parent scope
    let doc = mk("layout/grid");
    assert_eq!(
        resolve_position_head(&doc, &files[2], &c, "util").as_deref(),
        Some("layout/grid/util")
    );
    // the own-pkg head names the root
    assert_eq!(
        resolve_position_head(&doc, &files[2], &c, "app").as_deref(),
        Some("")
    );
    // from layout: its own child (read off the doc's own rows when
    // the doc IS that module)
    let doc2 = mk("layout");
    assert_eq!(
        resolve_position_head(&doc2, &files[1], &c, "grid").as_deref(),
        Some("layout/grid")
    );
    // unknown pkg name: a miss
    assert_eq!(resolve_position_head(&doc2, &files[1], &c, "pouch"), None);
    // an unstamped doc (mod_path None) resolves nothing
    let flat = DocMods::default();
    assert_eq!(resolve_position_head(&flat, &files[0], &c, "grid"), None);
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
    let f = mod_index("gadgets", "layout", &[]);
    assert!(matches_mod(&f, Some("gadgets"), "layout"));
    assert!(!matches_mod(&f, Some("gadgets"), ""));
    assert!(!matches_mod(&f, Some("pouch"), "layout"));
    // an unnamed file participates only where a face STAMPED it —
    // unstamped files never impersonate a module, not even the root
    // (the doc's own scope reads off its own rows, not a lookup)
    let stray = MountedFile::from_src(Mount::default(), "entry fn main() -> nil { }\n".into());
    assert!(!matches_mod(&stray, None, ""));
    assert!(!matches_mod(&stray, None, "layout"));
    let stamped = MountedFile::from_src(
        Mount {
            mod_path: Some("layout".into()),
            ..Mount::default()
        },
        "entry fn main() -> nil { }\n".into(),
    );
    assert!(matches_mod(&stamped, None, "layout"));
    assert!(
        !matches_mod(&stamped, Some("gadgets"), "layout"),
        "unnamed never answers a named pkg"
    );
    // mod_path None answers the root for a named file
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
    // a single-file module's REAL name rides its file's origin
    // basename; the dir-form guess stays the fallback
    let mut file_child = mod_index("gadgets", "imp", &[]);
    file_child.mount.origin = "file:///ws/imp.rut".to_string();
    let mut dir_child = mod_index("gadgets", "layout", &[]);
    dir_child.mount.origin = "file:///ws/layout/mod.rut".to_string();
    let files = [file_child, dir_child];
    let c = chain(&files);
    assert_eq!(display_of(&c, Some("gadgets"), "imp"), "imp.rut");
    // the dir-form child keeps the loader's spelling (its origin
    // basename is the flavor's `mod.rut`)
    assert_eq!(display_of(&c, Some("gadgets"), "layout"), "layout/mod.rut");
    // an unindexed child (or a non-file origin — `core`) falls back
    assert_eq!(display_of(&c, Some("gadgets"), "ghost"), "ghost/mod.rut");
    let mut core = mod_index("core", "prim", &[]);
    core.mount.origin = "core".to_string();
    let single = [core];
    let c1 = chain(&single);
    assert_eq!(display_of(&c1, None, "prim"), "prim/mod.rut");
}

// the vis rows the completion tests mount — a pub struct parses with
// its vis verdict on the node (the fixture lane's shape)
#[test]
fn pub_ty_rows_carry_their_vis() {
    let f = MountedFile::from_src(
        Mount::default(),
        "pub struct Column { w: i32; }\npub(pkg) struct Hidden { h: i32; }\n".to_string(),
    );
    let t = f.ty("Column").expect("Column parses");
    assert!(t.is_pub());
    let t = f.ty("Hidden").expect("Hidden parses");
    assert!(!t.is_pub() && t.vis() == Vis::Pkg);
}
