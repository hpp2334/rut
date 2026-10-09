//! File modules — the `mod.rut` mount (phase 2 of the file-modules
//! batch). Pinned here:
//!
//! - the mount: `mod layout;` → `layout/mod.rut` → `mod grid;` →
//!   `layout/grid/mod.rut`, two levels, each file its own module root,
//!   `pub mod` vs `mod` carried on the edge;
//! - the loud mounting laws: a missing child names both spellings, a
//!   same-named FILE is not a module directory, a directory without
//!   its `mod.rut` says so, and a symlinked cycle is the loud
//!   `cyclic mod` error (the dep walk's law mirrored);
//! - the collection: every mounted file's declarations gathered with
//!   the mod path attached (the plumbing phase 3 gates on) — and a
//!   mounted module nothing declares is named at compile;
//! - the transitional dual-read: `entry.lib` manifests splice exactly
//!   as always (AND mount children); no `entry.lib` → the root module
//!   is `mod.rut` beside the manifest; the surface-only dev state
//!   stands when no `mod.rut` exists;
//! - bundles: a mod package packs its tree as the `rut.mods` rows
//!   section and loads mounting the same tree; the committed std
//!   artifacts (the OLD flat envelope) still load, and a flat package
//!   repacks byte-identically (the freshness gate over all 16);
//! - a mod file's `use` statements join the run chain's closure check.

use std::path::{Path, PathBuf};

use rut_ast::ast::Vis;
use rut_driver::mods::{collect_module_set, DeclKind, ModSource};
use rut_driver::{Loaded, Pkg, PkgBody};

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-mods-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, text).unwrap();
}

/// The two-level fixture: `pkg/mod.rut` declares `layout`; `layout`
/// declares `pub mod grid`. `bundle` spells the bundle keys (for the
/// pack lanes) or not (plain directory pkg).
fn world(tag: &str, bundle: bool) -> PathBuf {
    let root = scratch(tag);
    let pkg = root.join("pkg");
    let keys = if bundle {
        r#""format": "rutbundle", "format_version": 10, "#
    } else {
        ""
    };
    write(&pkg, "rut.jsonc", &format!("{{{keys}\"name\": \"pkg\"}}"));
    write(
        &pkg,
        "mod.rut",
        "// pkg root — the root module\nmod layout;\nentry fn main() -> i32 { return 7; }\n",
    );
    write(
        &pkg,
        "layout/mod.rut",
        "// layout — declares the grandchild\npub mod grid;\npub fn span() -> i32 { return 1; }\n",
    );
    write(
        &pkg,
        "layout/grid/mod.rut",
        "// grid\npub fn cell() -> i32 { return 2; }\n",
    );
    root
}

fn load(dir: &Path) -> Loaded {
    rut_native::load_dir(dir).expect("load the walk")
}

fn run_main(loaded: Loaded) -> i64 {
    let c = rut_driver::RutRun::new()
        .pkgs(&loaded)
        .entrypoint(&loaded.root)
        .compile()
        .expect("compile the walk");
    assert!(c.graph.diags.is_empty(), "{:?}", c.graph.diags);
    let flat = rut_core::link::flatten(c.graph.program.expect("linked program"));
    rut_vm::verify::verify(&flat).expect("verify");
    let mut vm = rut_vm::interp::Vm::builder()
        .program(std::rc::Rc::new(flat))
        .limits(rut_vm::interp::Limits {
            fuel: Some(2_000_000),
            heap_limit_bytes: Some(16 * 1024 * 1024),
            interrupt_every: 1024,
        })
        .hooks(rut_vm::interp::HostHooks::default())
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("vm");
    vm.call::<_, i32>("main", ()).expect("run main").into()
}

// ---------------------------------------------------------------------
// the mount
// ---------------------------------------------------------------------

#[test]
fn mod_tree_mounts_two_levels_and_runs() {
    let root = world("two-levels", false);
    let loaded = load(&root.join("pkg"));
    assert_eq!(loaded.root, "pkg");
    let pkg = loaded.pkg("pkg").expect("pkg mounted");
    // the root file IS the body — never spliced, never a child row
    let PkgBody::Source { text, is_decl } = &pkg.body else {
        panic!("pkg has a source body");
    };
    assert!(!*is_decl);
    assert!(text.starts_with("// pkg root"), "{text:?}");
    // the children: declarations are the graph — both levels mounted
    let keys: Vec<&str> = pkg.mods.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["layout", "layout/grid"]);
    // `pub mod` vs `mod` carried faithfully on the edge
    assert_eq!(pkg.mods["layout"].vis, Vis::Self_);
    assert_eq!(pkg.mods["layout/grid"].vis, Vis::Pub);
    assert!(pkg.mods["layout/grid"].text.contains("pub fn cell()"));
    // the root compiles (the decl is inert — resolution is phase 3)
    // and runs
    assert_eq!(run_main(loaded), 7);
}

#[test]
fn missing_child_names_both_spellings() {
    let root = world("missing", false);
    let pkg = root.join("pkg");
    std::fs::remove_dir_all(pkg.join("layout/grid")).unwrap();
    let err = rut_native::load_dir(&pkg).unwrap_err().to_string();
    assert!(
        err.contains("`mod grid;` in layout/mod.rut — no `grid/mod.rut` beside it"),
        "{err}"
    );
}

#[test]
fn a_same_named_file_is_not_a_module_directory() {
    let root = world("notadir", false);
    let pkg = root.join("pkg");
    std::fs::remove_dir_all(pkg.join("layout/grid")).unwrap();
    write(&pkg, "layout/grid.rut", "pub fn cell() -> i32 { return 2; }\n");
    let err = rut_native::load_dir(&pkg).unwrap_err().to_string();
    assert!(
        err.contains("`mod grid;` in layout/mod.rut — `grid.rut` is a file, not a module directory"),
        "{err}"
    );
}

#[test]
fn a_directory_without_mod_rut_is_loud() {
    let root = world("nomodrut", false);
    let pkg = root.join("pkg");
    std::fs::remove_file(pkg.join("layout/grid/mod.rut")).unwrap();
    write(&pkg, "layout/grid/helpers.rut", "// no mod.rut here\n");
    let err = rut_native::load_dir(&pkg).unwrap_err().to_string();
    assert!(
        err.contains("`mod grid;` in layout/mod.rut — `grid/` has no `mod.rut`"),
        "{err}"
    );
}

#[test]
fn a_symlinked_cycle_is_the_loud_cyclic_mod_error() {
    let root = world("cycle", false);
    let pkg = root.join("pkg");
    // the root declares `mod a;`; a/mod.rut declares `mod b;`; `b`
    // aliases `a` itself — the canonical identity of a/mod.rut repeats
    // inside the mount chain
    write(&pkg, "mod.rut", "mod a;\nentry fn main() -> i32 { return 7; }\n");
    write(&pkg, "a/mod.rut", "mod b;\npub fn one() -> i32 { return 1; }\n");
    std::os::unix::fs::symlink(pkg.join("a").canonicalize().unwrap(), pkg.join("a/b")).unwrap();
    let err = rut_native::load_dir(&pkg).unwrap_err().to_string();
    assert!(err.contains("cyclic mod:"), "{err}");
    assert!(err.contains("`b`"), "{err}");
}

// ---------------------------------------------------------------------
// the collection (the compile lane's plumbing)
// ---------------------------------------------------------------------

#[test]
fn collection_gathers_every_file_with_its_mod_path() {
    let root = world("collect", false);
    let loaded = load(&root.join("pkg"));
    let pkg = loaded.pkg("pkg").expect("pkg mounted");
    let (set, problems) = collect_module_set("pkg", pkg);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(set.paths(), vec!["", "layout", "layout/grid"]);
    let root_unit = &set.units[0];
    assert_eq!(root_unit.path, "");
    let layout_rows = &set.units[1];
    assert_eq!(layout_rows.path, "layout");
    let grid = &layout_rows
        .decls
        .iter()
        .find(|d| d.kind == DeclKind::Mod)
        .expect("layout declares grid");
    assert_eq!(grid.name, "grid");
    assert_eq!(grid.vis, Vis::Pub);
    let span_fn = &layout_rows
        .decls
        .iter()
        .find(|d| d.kind == DeclKind::Fn)
        .expect("layout declares span");
    assert_eq!(span_fn.name, "span");
    let cell_fn = &set.units[2]
        .decls
        .iter()
        .find(|d| d.kind == DeclKind::Fn)
        .expect("grid declares cell");
    assert_eq!(cell_fn.name, "cell");
}

#[test]
fn a_mounted_module_nothing_declares_is_named_at_compile() {
    let root = world("undeclared", false);
    let loaded = load(&root.join("pkg"));
    let mut pkg = loaded.pkg("pkg").expect("pkg mounted").clone();
    // a hand-offered disagreement: the loader never mounts one of
    // these, so the compile lane names it instead of guessing around
    pkg.mods.insert(
        "ghost".to_string(),
        ModSource {
            path: "ghost".to_string(),
            vis: Vis::Self_,
            text: "pub fn boo() -> i32 { return 0; }\n".to_string(),
        },
    );
    let c = rut_driver::RutRun::new()
        .pkg(pkg)
        .entrypoint("pkg")
        .compile()
        .expect("compile carries the diags");
    assert!(
        c.graph.diags.iter().any(|d| d
            .msg
            .contains("`ghost/mod.rut` is mounted but no `mod ghost;` declares it")),
        "{:?}",
        c.graph.diags
    );
}

#[test]
fn a_mod_files_uses_join_the_closure_check() {
    let root = world("child-uses", false);
    let pkg = root.join("pkg");
    write(
        &pkg,
        "layout/mod.rut",
        "use nosuch::{Thing};\npub fn span() -> i32 { return 1; }\n",
    );
    let err = rut_driver::RutRun::new()
        .pkgs(&load(&pkg))
        .entrypoint("pkg")
        .compile()
        .unwrap_err()
        .to_string();
    assert!(err.contains("nosuch"), "{err}");
    assert!(err.contains("layout"), "{err}");
}

// ---------------------------------------------------------------------
// the transitional dual-read (entry.lib stands; mod.rut is the default)
// ----------------------------------------------------------------------

#[test]
fn entry_lib_manifests_splice_and_mount_as_before() {
    let root = scratch("entry-lib");
    let pkg = root.join("pkg");
    write(
        &pkg,
        "rut.jsonc",
        r#"{"name": "pkg", "entry": {"lib": "./lib.rut", "libs": ["./tail.rut"]}}"#,
    );
    write(
        &pkg,
        "lib.rut",
        "mod layout;\nentry fn main() -> i32 { return 7; }\n",
    );
    write(&pkg, "tail.rut", "// the splice tail\n");
    write(&pkg, "layout/mod.rut", "pub fn span() -> i32 { return 1; }\n");
    let loaded = load(&pkg);
    let pkg = loaded.pkg("pkg").expect("pkg mounted");
    // the splice stands: base + libs, '\n'-joined — AND the declared
    // child mounts from the spliced text
    let PkgBody::Source { text, .. } = &pkg.body else { panic!("source body") };
    assert_eq!(
        text.as_str(),
        "mod layout;\nentry fn main() -> i32 { return 7; }\n\n// the splice tail\n"
    );
    assert_eq!(pkg.mods.keys().collect::<Vec<_>>(), vec!["layout"]);
    assert_eq!(run_main(loaded), 7);
}

#[test]
fn mod_rut_beside_an_entry_type_surface_is_the_root() {
    // no `entry.lib`, an `entry.type` surface, AND a `mod.rut` — the
    // root module is the `mod.rut` (the surface rides `entry.type`)
    let root = scratch("type-and-mod");
    let pkg = root.join("pkg");
    write(
        &pkg,
        "rut.jsonc",
        r#"{"name": "pkg", "type": "lib", "entry": {"type": "./pkg.d.rut"}}"#,
    );
    write(&pkg, "pkg.d.rut", "/// documented surface, no body yet.\n");
    write(&pkg, "mod.rut", "entry fn main() -> i32 { return 7; }\n");
    let loaded = load(&pkg);
    let pkg = loaded.pkg("pkg").expect("pkg mounted");
    let PkgBody::Source { text, is_decl } = &pkg.body else { panic!("source body") };
    assert!(!*is_decl, "the mod.rut is a body, not the dev state");
    assert_eq!(text, "entry fn main() -> i32 { return 7; }\n");
    assert_eq!(run_main(loaded), 7);
}

#[test]
fn entry_type_only_without_mod_rut_is_still_the_dev_state() {
    let root = scratch("dev-state");
    let pkg = root.join("pkg");
    write(
        &pkg,
        "rut.jsonc",
        r#"{"name": "pkg", "type": "lib", "entry": {"type": "./pkg.d.rut"}}"#,
    );
    write(&pkg, "pkg.d.rut", "/// documented surface, no body yet.\n");
    let loaded = load(&pkg);
    let pkg = loaded.pkg("pkg").expect("pkg mounted");
    let PkgBody::Source { is_decl, .. } = &pkg.body else { panic!("source body") };
    assert!(*is_decl, "no `mod.rut` — today's surface-only dev state stands");
    assert!(pkg.mods.is_empty());
}

#[test]
fn no_entry_at_all_names_the_mod_rut_recipe() {
    let root = scratch("no-entry");
    let pkg = root.join("pkg");
    write(&pkg, "rut.jsonc", r#"{"name": "pkg"}"#);
    let err = rut_native::load_dir(&pkg).unwrap_err().to_string();
    assert!(err.contains("has no entry"), "{err}");
    assert!(err.contains("`mod.rut`"), "{err}");
}

// ---------------------------------------------------------------------
// bundles: the rows section, the round trip, the old envelope
// ----------------------------------------------------------------------

#[test]
fn a_mod_package_packs_rows_and_loads_the_same_tree() {
    let root = world("pack", true);
    let bytes = rut_native::pack_dir(&root.join("pkg")).expect("pack");
    // deterministic: same dir ⇒ byte-identical bundle
    assert_eq!(bytes, rut_native::pack_dir(&root.join("pkg")).unwrap());
    // the additive section: the rows entry rides the compiled root
    let names: Vec<String> = rut_driver::bundle::parse_bundle(&bytes)
        .unwrap()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert!(names.contains(&"rut.mods".to_string()), "{names:?}");
    // loading the bundle mounts the tree exactly as the directory did
    let loaded = Pkg::from_bundle(&bytes).expect("load");
    let bundled = loaded.pkg("pkg").expect("pkg mounted");
    let dir = load(&root.join("pkg"));
    let on_disk = dir.pkg("pkg").expect("pkg mounted");
    assert_eq!(bundled.mods, on_disk.mods, "the bundle mounts the same tree");
    assert_eq!(run_main(loaded), 7);
}

#[test]
fn flat_packages_pack_without_a_rows_entry() {
    // a package with no mod children: no `rut.mods` — byte-stable
    // output (the freshness gate over the committed std bundles is the
    // exhaustive version of this)
    let root = scratch("flat-pack");
    let pkg = root.join("pkg");
    write(
        &pkg,
        "rut.jsonc",
        r#"{"name": "pkg", "format": "rutbundle", "format_version": 10, "entry": {"lib": "./lib.rut"}}"#,
    );
    write(&pkg, "lib.rut", "entry fn main() -> i32 { return 7; }\n");
    let bytes = rut_native::pack_dir(&pkg).expect("pack");
    let names: Vec<String> = rut_driver::bundle::parse_bundle(&bytes)
        .unwrap()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert!(!names.iter().any(|n| n.ends_with("rut.mods")), "{names:?}");
}

#[test]
fn the_committed_std_bundles_still_load_unchanged() {
    // the pin is law: every published std-v8-shape artifact (the OLD
    // flat envelope, no rows section) keeps loading through the same
    // reader — unchanged
    let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../dist/std"));
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("dist/std") {
        let path = entry.unwrap().path();
        if path.extension().map_or(false, |e| e == "rutbundle") {
            names.push(path.clone());
        }
    }
    names.sort();
    assert!(names.len() >= 16, "the committed std artifacts ride in dist/std: {names:?}");
    for path in &names {
        let bytes = std::fs::read(path).expect("read the bundle");
        let entries = rut_driver::bundle::parse_bundle(&bytes).unwrap();
        let entry_names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
        assert!(
            !entry_names.iter().any(|n| n.ends_with("rut.mods")),
            "{} is old-envelope — it carries no rows section",
            path.display()
        );
        let loaded = Pkg::from_bundle(&bytes)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(
            loaded.pkgs.iter().any(|p| p.spec == loaded.root),
            "{}: the root mounted",
            path.display()
        );
        // a flat envelope mounts no mod tree
        for pkg in &loaded.pkgs {
            assert!(
                pkg.mods.is_empty(),
                "{}: {} carries no mods",
                path.display(),
                pkg.spec
            );
        }
    }
}

#[test]
fn a_mod_dep_group_packs_its_rows_and_mounts_as_a_dep() {
    // the consumer shape: a mod-rooted pkg as a [deps] group — its
    // rows ride under `pkg/rut.mods` and mount at the consumer's load
    let root = world("group", true);
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        r#"{"name": "app", "format": "rutbundle", "format_version": 10, "entry": {"lib": "./lib.rut"}, "deps": {"pkg": {"path": "../pkg"}}}"#,
    );
    write(
        &app,
        "lib.rut",
        "use pkg::{who};\nentry fn main() -> str { return who(); }\n",
    );
    // pkg needs a pub fn the app imports — add it to the root module
    write(
        &root.join("pkg"),
        "mod.rut",
        "mod layout;\npub fn who() -> str { return \"pkg\"; }\nentry fn main() -> i32 { return 7; }\n",
    );
    let bytes = rut_native::pack_dir(&app).expect("pack");
    let names: Vec<String> = rut_driver::bundle::parse_bundle(&bytes)
        .unwrap()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert!(names.contains(&"pkg/rut.mods".to_string()), "{names:?}");
    let loaded = Pkg::from_bundle(&bytes).expect("load");
    let pkg = loaded.pkg("pkg").expect("pkg group mounted");
    let keys: Vec<&str> = pkg.mods.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["layout", "layout/grid"]);
}
