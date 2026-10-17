//! File modules — the `mod.rut` mount (phase 2 of the file-modules
//! batch). Pinned here:
//!
//! - the mount: `mod layout;` → `layout/mod.rut` → `mod grid;` →
//!   `layout/grid/mod.rut`, two levels, each file its own module root,
//!   `pub mod` vs `mod` carried on the edge;
//! - the single-file spelling: `mod imp;` mounts `imp.rut` beside the
//!   declaring file (mixed trees mount both forms), both spellings
//!   present is the loud ambiguity naming both, and a cycle through a
//!   file child is still the loud `cyclic mod` error;
//! - the loud mounting laws: a missing child names both spellings and
//!   a directory without its `mod.rut` says so (the symlinked cycle
//!   alias included);
//! - the collection: every mounted file's declarations gathered with
//!   the mod path attached (the plumbing phase 3 gates on) — and a
//!   mounted module nothing declares is named at compile;
//! - the entry repeal: the root module is `mod.rut` beside the
//!   manifest — the body keys refuse in a directory, the surface-only
//!   dev state stands when no `mod.rut` exists, and the ambiguity
//!   (`entry.type`, no kind, no body) names both fixes;
//! - bundles: a mod package packs its tree as the `rut.mods` rows
//!   section and loads mounting the same tree; the PUBLISHED std-v8
//!   artifacts (the OLD flat envelope, committed at `dist/std-v8/`)
//!   still load through the reader-compat lane, and a flat package
//!   repacks byte-identically (the freshness gate over all 16);
//! - a mod file's `use` statements join the run chain's closure check.

use std::path::{Path, PathBuf};

use rut_ast::ast::Vis;
use rut_driver::mods::{collect_module_set, DeclKind, ModSource};
use rut_driver::{Loaded, Pkg, PkgBody};

mod common;
use common::{dep_dir, pkg_named};

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
        r#""format": "rutbundle", "format_version": 11, "#
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
fn a_file_module_mounts_and_runs() {
    // the single-file spelling: `mod imp;` mounts `imp.rut` beside the
    // declaring file — the logical mod path is unchanged (`imp`), so
    // the qualified walk and the own-package use lanes ride it as-is
    let root = scratch("file-form");
    let pkg = root.join("pkg");
    write(&pkg, "rut.jsonc", r#"{"name": "pkg"}"#);
    write(
        &pkg,
        "mod.rut",
        "use imp::{ span };\nmod imp;\nentry fn main() -> i32 { return span() + imp::plus(2); }\n",
    );
    write(
        &pkg,
        "imp.rut",
        "pub fn span() -> i32 { return 5; }\npub fn plus(v: i32) -> i32 { return v + 40; }\n",
    );
    let loaded = load(&pkg);
    let pkg = loaded.pkg("pkg").expect("pkg mounted");
    let keys: Vec<&str> = pkg.mods.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["imp"], "the file child mounts at its logical mod path");
    assert!(pkg.mods["imp"].text.contains("pub fn span"));
    assert_eq!(run_main(loaded), 47);
}

#[test]
fn a_mixed_tree_mounts_both_forms_at_every_level() {
    // dir form and file form interleave: `layout/` is a directory
    // child, `helper.rut` a file child beside it, and the directory
    // child declares a file child of its own
    let root = scratch("mixed-tree");
    let pkg = root.join("pkg");
    write(&pkg, "rut.jsonc", r#"{"name": "pkg"}"#);
    write(
        &pkg,
        "mod.rut",
        "mod layout;\nmod helper;\nentry fn main() -> i32 { return layout::span() + helper::two() + layout::grid::cell(); }\n",
    );
    write(
        &pkg,
        "layout/mod.rut",
        "mod grid;\npub fn span() -> i32 { return 1; }\n",
    );
    write(&pkg, "layout/grid.rut", "pub fn cell() -> i32 { return 2; }\n");
    write(&pkg, "helper.rut", "pub fn two() -> i32 { return 20; }\n");
    let loaded = load(&pkg);
    let pkg = loaded.pkg("pkg").expect("pkg mounted");
    let keys: Vec<&str> = pkg.mods.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["helper", "layout", "layout/grid"]);
    assert_eq!(run_main(loaded), 23);
}

#[test]
fn both_forms_are_the_loud_ambiguity() {
    // `layout/grid/mod.rut` AND `layout/grid.rut` — which mounts is
    // undecidable; the error names both spellings
    let root = world("ambiguous", false);
    let pkg = root.join("pkg");
    write(&pkg, "layout/grid.rut", "pub fn cell() -> i32 { return 2; }\n");
    let err = rut_native::load_dir(&pkg).unwrap_err().to_string();
    assert!(
        err.contains("`mod grid;` in layout/mod.rut — both `grid/mod.rut` and `grid.rut` exist"),
        "{err}"
    );
    assert!(err.contains("keep one"), "{err}");
}

#[test]
fn a_cycle_through_a_file_child_is_the_loud_cyclic_mod_error() {
    // a.rut declares `mod b;`; `b.rut` aliases `a.rut` itself — the
    // canonical identity repeats inside the mount chain (the file
    // child's real name rides the message)
    let root = scratch("file-cycle");
    let pkg = root.join("pkg");
    write(&pkg, "rut.jsonc", r#"{"name": "pkg"}"#);
    write(&pkg, "mod.rut", "mod a;\nentry fn main() -> i32 { return 7; }\n");
    write(&pkg, "a.rut", "mod b;\npub fn one() -> i32 { return 1; }\n");
    std::os::unix::fs::symlink(pkg.join("a.rut").canonicalize().unwrap(), pkg.join("b.rut")).unwrap();
    let err = rut_native::load_dir(&pkg).unwrap_err().to_string();
    assert!(err.contains("cyclic mod:"), "{err}");
    assert!(err.contains("`b` (a/b/b.rut)"), "{err}");
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
            file: "mod.rut".to_string(),
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
// the entry repeal (the root module is `mod.rut`; the keys refuse)
// ----------------------------------------------------------------------

#[test]
fn entry_lib_manifests_refuse_loudly_naming_the_fix() {
    // THE REPEAL: a directory manifest spelling the body keys refuses
    // at parse — each message names the fix
    let root = scratch("entry-lib");
    let pkg = root.join("pkg");
    write(
        &pkg,
        "rut.jsonc",
        r#"{"name": "pkg", "entry": {"lib": "./lib.rut"}}"#,
    );
    write(&pkg, "lib.rut", "entry fn main() -> i32 { return 7; }\n");
    let err = rut_native::load_dir(&pkg).unwrap_err().to_string();
    assert!(err.contains("entry.lib: repealed"), "{err}");
    assert!(err.contains("the root module is `mod.rut` beside the manifest"), "{err}");
    assert!(err.contains("rename the file, drop the key"), "{err}");

    // `entry.libs` — structure lives in `mod` directories
    let pkg2 = root.join("pkg2");
    write(
        &pkg2,
        "rut.jsonc",
        r#"{"name": "pkg", "entry": {"libs": ["./tail.rut"]}}"#,
    );
    write(&pkg2, "tail.rut", "// the splice tail\n");
    let err = rut_native::load_dir(&pkg2).unwrap_err().to_string();
    assert!(err.contains("entry.libs: repealed"), "{err}");
    assert!(err.contains("structure lives in `mod` directories"), "{err}");
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
    // (the group mounts under its content identity)
    let loaded = Pkg::from_bundle(&bytes).expect("load");
    let bundled = pkg_named(&loaded, "pkg");
    let dir = load(&root.join("pkg"));
    let on_disk = dir.pkg("pkg").expect("pkg mounted");
    assert_eq!(bundled.mods, on_disk.mods, "the bundle mounts the same tree");
    assert_eq!(run_main(loaded), 7);
}

#[test]
fn flat_packages_pack_without_a_rows_entry() {
    // a package with no mod children: no `rut.mods` — the flat
    // envelope's pack shape (the freshness gate over the committed std
    // bundles is the exhaustive version of this)
    let root = scratch("flat-pack");
    let pkg = root.join("pkg");
    write(
        &pkg,
        "rut.jsonc",
        r#"{"name": "pkg", "format": "rutbundle", "format_version": 11}"#,
    );
    write(&pkg, "mod.rut", "entry fn main() -> i32 { return 7; }\n");
    let bytes = rut_native::pack_dir(&pkg).expect("pack");
    let names: Vec<String> = rut_driver::bundle::parse_bundle(&bytes)
        .unwrap()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert!(!names.iter().any(|n| n.ends_with("rut.mods")), "{names:?}");
}

#[test]
fn the_published_std_v8_bundles_refuse_at_the_version_gate() {
    // the wire law: every PUBLISHED std-v8 artifact (the OLD flat
    // envelope — manifests spelling the repealed keys, sources at the
    // legacy names) refuses at the ONE version gate with the re-pack
    // recipe — no compat lane crosses a wire number. The published
    // bytes are committed at `dist/std-v8/` so the gate never
    // networks; `dist/std` (the working envelope) is the freshness
    // gate's business and `dist/std-v11/` the pin envelope.
    //
    // The COMPAT LANE itself survives one level down: handed the
    // artifact's manifest TEXT directly, `parse_manifest_compat` still
    // reads the repealed entry keys (a repacked group manifest rides
    // byte-for-byte from any era) — the strict-keys law still refuses
    // `[peer-deps]` there (pinned in
    // `the_peer_deps_bundle_refuses_at_the_manifest_door` below).
    let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../dist/std-v8"));
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("dist/std") {
        let path = entry.unwrap().path();
        if path.extension().map_or(false, |e| e == "rutbundle") {
            names.push(path.clone());
        }
    }
    names.sort();
    let stems: Vec<String> = names
        .iter()
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        stems,
        vec!["http", "json", "nmapset", "pouch"],
        "the pinned artifacts ride in dist/std-v8: {names:?}"
    );
    for path in names.iter().filter(|p| {
        p.file_stem().unwrap().to_string_lossy() != "json"
    }) {
        let bytes = std::fs::read(path).expect("read the bundle");
        // the OLD manifest grammar still parses one level down: the
        // archive's own rut.jsonc spells the repealed keys, and the
        // compat parse answers them
        let manifest_text = rut_driver::bundle::Bundle::parse(&bytes)
            .unwrap()
            .read("rut.jsonc")
            .unwrap();
        let m = rut_driver::bundle::parse_manifest_compat(&manifest_text).unwrap();
        assert!(
            m.legacy_entry.lib.is_some(),
            "{}: the old envelope's manifest carries the legacy keys",
            path.display()
        );
        assert!(
            rut_driver::bundle::parse_manifest(&manifest_text).is_err(),
            "{}: the strict (directory) lane refuses the same text",
            path.display()
        );
        // the WIRE gate refuses the artifact itself — the re-pack
        // recipe, never a guessed load
        let err = Pkg::from_bundle(&bytes).unwrap_err().to_string();
        assert!(
            err.contains("reads bundle format_version 11 only"),
            "{}: {err}",
            path.display()
        );
        assert!(err.contains("re-pack the directory"), "{}: {err}", path.display());
    }
}

#[test]
fn the_peer_deps_bundle_refuses_at_the_manifest_door() {
    // the strict-keys law against a PUBLISHED artifact: the compat lane
    // relaxes the repealed entry keys (a loading law), but it grants NO
    // retired lane to `[peer-deps]` — json's std-v8 manifest carries
    // the key, and the grammar refuses it exactly like any unknown
    // key, at parse and at the bundle door. Old artifacts riding the
    // key must be re-packed.
    let bytes = std::fs::read(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../dist/std-v8/json.rutbundle"
    )))
    .expect("the pinned artifact");
    let manifest_text = rut_driver::bundle::Bundle::parse(&bytes)
        .unwrap()
        .read("rut.jsonc")
        .unwrap();
    let err = rut_driver::bundle::parse_manifest_compat(&manifest_text).unwrap_err();
    assert_eq!(err.to_string(), "manifest: unknown key 'peer-deps'", "{err}");
    // at the bundle door the manifest parse precedes the version gate —
    // the unknown key refuses first, exactly like any unknown key
    let err = Pkg::from_bundle(&bytes).unwrap_err().to_string();
    assert!(err.contains("manifest: unknown key 'peer-deps'"), "{err}");
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
        r#"{"name": "app", "format": "rutbundle", "format_version": 11, "deps": {"pkg": {"path": "../pkg"}}}"#,
    );
    write(
        &app,
        "mod.rut",
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
    assert!(
        names.contains(&format!("{}/rut.mods", dep_dir(&names, "pkg"))),
        "{names:?}"
    );
    let loaded = Pkg::from_bundle(&bytes).expect("load");
    let pkg = pkg_named(&loaded, "pkg");
    let keys: Vec<&str> = pkg.mods.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["layout", "layout/grid"]);
}

#[test]
fn an_old_envelopes_legacy_splice_still_reads_back() {
    // THE READER-COMPAT LAW, splice flavor: a PUBLISHED old-envelope
    // bundle carries the repealed keys and its source at the legacy
    // names — the reader mounts the body exactly as the old grammar
    // spliced it (base, then `libs` in listed order, '\n'-joined), and
    // the riding source a consumer's link needs carries the SAME
    // splice. Forged honestly: a real pack of the toolchain's pouch,
    // re-spelled into the flat envelope (the manifest swaps to the
    // legacy keys, the root source takes the legacy name, the rows
    // section is dropped).
    let bytes = rut_native::pack_dir(&PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../rut/pouch"
    )))
    .expect("pack the tree pouch");
    let pouch_src =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/pouch/mod.rut"))
            .unwrap();
    let legacy = r#"{"format": "rutbundle", "format_version": 11, "name": "pouch", "entry": {"lib": "./pouch.rut", "libs": ["./extra.rut"]}}"#;
    let entries: Vec<(String, Vec<u8>)> = rut_driver::bundle::parse_bundle(&bytes)
        .unwrap()
        .into_iter()
        .filter(|(n, _)| n != "rut.mods" && n != "mod.rut")
        .flat_map(|(n, b)| {
            if n == "rut.jsonc" {
                vec![
                    ("rut.jsonc".to_string(), legacy.as_bytes().to_vec()),
                    ("pouch.rut".to_string(), pouch_src.clone().into_bytes()),
                    ("extra.rut".to_string(), b"// the legacy splice tail\n".to_vec()),
                ]
            } else {
                vec![(n, b)]
            }
        })
        .collect();
    let legacy_bytes = rut_driver::bundle::write_bundle(&entries).unwrap();
    // v22: source riding is GONE — the legacy splice rides no more; the
    // bundle mounts compiled and the delivery law serves consumer-spelled
    // shapes from the binary's own templates
    let loaded = Pkg::from_bundle(&legacy_bytes).expect("the old envelope loads");
    let pkg = loaded.pkg("pouch").expect("pouch mounted");
    assert!(
        matches!(pkg.body, rut_driver::PkgBody::Compiled(_)),
        "the legacy envelope's pouch mounts compiled"
    );
}

// =====================================================================
// phase 3 — lir/resolution: visibility tiers, qualified positions,
// use-through-mods
//
// Spelling note: positions qualify with `.` (`a.Pair`, `layout::mk(3)`,
// `a.c.sup()`); `use` statements qualify with `::`. Positions never
// take a package head — uses are the only cross-package door.
// =====================================================================

/// A mod pkg offered by hand (the run lane's pure mount — the walker's
/// shape, no filesystem).
fn modpkg(spec: &str, root: &str, mods: Vec<(&str, Vis, &str)>) -> rut_driver::Pkg {
    rut_driver::Pkg {
        spec: spec.to_string(),
        body: rut_driver::PkgBody::Source { text: root.to_string(), is_decl: false },
        mods: mods
            .into_iter()
            .map(|(p, v, t)| {
                (
                    p.to_string(),
                    ModSource { path: p.to_string(), vis: v, text: t.to_string(), file: "mod.rut".to_string() },
                )
            })
            .collect(),
        ..Default::default()
    }
}

/// Compile the offered pkgs (entrypoint `spec`), handing back the
/// diagnostic messages.
fn compile_with_diags(pkgs: &[rut_driver::Pkg], entry: &str) -> Vec<String> {
    let mut run = rut_driver::RutRun::new();
    // the hand world declares loosely: every pkg names every other
    // (the law bounds uses by declarations, never declarations by
    // uses — these tests pin visibility/positions, not nameability)
    for p in pkgs {
        let heads: Vec<&str> =
            pkgs.iter().map(|q| q.spec.as_str()).filter(|s| *s != p.spec).collect();
        run = run.pkg(p.clone().manifest(rut_driver::Manifest::lib(&p.spec, &heads)));
    }
    let c = run.entrypoint(entry).compile().expect("compile carries the diags");
    c.graph.diags.iter().map(|d| d.msg.clone()).collect()
}

fn run_main_pkgs(pkgs: &[rut_driver::Pkg], entry: &str) -> i64 {
    let mut run = rut_driver::RutRun::new();
    // the same loose table as [`compile_with_diags`]
    for p in pkgs {
        let heads: Vec<&str> =
            pkgs.iter().map(|q| q.spec.as_str()).filter(|s| *s != p.spec).collect();
        run = run.pkg(p.clone().manifest(rut_driver::Manifest::lib(&p.spec, &heads)));
    }
    let c = run.entrypoint(entry).compile().expect("compile the walk");
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

#[test]
fn qualified_positions_resolve_type_and_value() {
    // the consumer's root: a's fn by qualified call, a's type by
    // qualified annotation — the field ride proves the type crossed
    let pkgs = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 {\n    let r: a::Pair = a::pair(3);\n    return r.x + r.y;\n}\n",
            vec![(
                "a",
                Vis::Self_,
                "pub struct Pair { x: i32, y: i32 }\npub fn pair(v: i32) -> Pair { return Pair { x: v, y: v + 1 }; }\n",
            )],
        ),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 7);
}

#[test]
fn qualified_calls_walk_ancestors_and_siblings() {
    // from a/c/c2: the root's fn via the pkg head (the ancestor walk's
    // root spelling), a's fn via the walked chain — and the root calls
    // the deep `a.c.c2.seven()` by qualified path
    let pkgs = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg(
            "pkg",
            "mod a;\npub fn root_side() -> i32 { return 30; }\nentry fn main() -> i32 { return a::c::c2::seven(); }\n",
            vec![
                ("a", Vis::Self_, "pub mod c;\npub fn five() -> i32 { return 5; }\n"),
                ("a/c", Vis::Pub, "pub mod c2;\n"),
                (
                    "a/c/c2",
                    Vis::Pub,
                    "pub fn seven() -> i32 {\n    return pkg::root_side() + pkg::a::five() + 7;\n}\n",
                ),
            ],
        ),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 30 + 5 + 7);
}

#[test]
fn leaf_not_found_names_the_module() {
    let pkgs = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { return a::ghost(); }\n",
            vec![("a", Vis::Self_, "pub fn five() -> i32 { return 5; }\n")],
        ),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    assert!(
        diags.iter().any(|d| d.contains("`ghost` is not declared in module `a` (a/mod.rut)")),
        "{diags:?}"
    );
}

#[test]
fn visibility_matrix_private_pub_super_pub_pkg_pub() {
    // private-in-mod: a's private fn is invisible from the root
    // (qualified or not) but visible inside a's own subtree
    let pkgs = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { return a::inner(); }\n",
            vec![(
                "a",
                Vis::Self_,
                "fn inner() -> i32 { return 1; }\npub fn uses_inner() -> i32 { return inner(); }\n",
            )],
        ),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    assert!(
        diags.iter().any(|d| d.contains("private to module `a`")),
        "{diags:?}"
    );
    // the same decl inside its module: fine
    let pkgs = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { return a::uses_inner(); }\n",
            vec![(
                "a",
                Vis::Self_,
                "fn inner() -> i32 { return 1; }\npub fn uses_inner() -> i32 { return inner(); }\n",
            )],
        ),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 1);

    // pub(super) one level: a/c's pub(super) fn is visible in a's
    // subtree (a, a/c) and NOWHERE else (not the root, not the sibling)
    // visible from a (the parent's subtree): a calls c.sup()
    let world = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { return a::go(); }\n",
            vec![
                ("a", Vis::Self_, "pub mod c;\npub fn go() -> i32 { return c::sup(); }\n"),
                ("a/c", Vis::Pub, "pub(super) fn sup() -> i32 { return 11; }\n"),
            ],
        ),
    ];
    assert_eq!(run_main_pkgs(&world, "pkg"), 11);
    // invisible from the root directly
    let world = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { return a::c::sup(); }\n",
            vec![
                ("a", Vis::Self_, "pub mod c;\n"),
                ("a/c", Vis::Pub, "pub(super) fn sup() -> i32 { return 11; }\n"),
            ],
        ),
    ];
    let diags = compile_with_diags(&world, "pkg");
    assert!(
        diags.iter().any(|d| d.contains("`pub(super)` — visible only in `a`'s subtree")),
        "{diags:?}"
    );
    // visible from the SIBLING subtree: pub(super) on a/c's decl is
    // the parent module's (a's) whole subtree — a/d reaches it through
    // the qualified path, exactly like Rust's pub(super)
    let d_world = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { return a::go(); }\n",
            vec![
                ("a", Vis::Self_, "pub mod c;\npub mod d;\npub fn go() -> i32 { return d::reach(); }\n"),
                ("a/c", Vis::Pub, "pub(super) fn sup() -> i32 { return 11; }\n"),
                ("a/d", Vis::Pub, "pub fn reach() -> i32 { return a::c::sup(); }\n"),
            ],
        ),
    ];
    assert_eq!(run_main_pkgs(&d_world, "pkg"), 11);
}

#[test]
fn pub_pkg_stays_inside_its_package() {
    // dep's alpha declares pub(pkg) fn inside(): visible THROUGHOUT
    // dep (the root file, via the pkg head), never across the use door
    let dep = modpkg(
        "dep",
        "pub mod alpha;\nentry fn main() -> i32 { return dep::alpha::inside(); }\n",
        vec![("alpha", Vis::Pub, "pub(pkg) fn inside() -> i32 { return 44; }\n")],
    );
    assert_eq!(run_main_pkgs(&[dep.clone()], "dep"), 44);
    // across the use door: refused — pub(pkg) does not cross packages
    let consumer = modpkg(
        "pkg",
        "use dep::alpha::{inside};\nentry fn main() -> i32 { return inside(); }\n",
        vec![],
    );
    let diags = compile_with_diags(&[dep, consumer], "pkg");
    assert!(
        diags.iter().any(|d| d.contains("is not `pub` — only `pub` names cross packages")),
        "{diags:?}"
    );
}

// =====================================================================
// the member-visibility law — METHODS (classes/enums take the `pub`
// dial: `pub fn` exports cross-module, a plain `fn` is module-private —
// the declaring module and its descendants). Fields and class literals
// gate in the section below.
// =====================================================================

/// The Counter-on-`a` fixture: a `pub class` whose inherent impl carries
/// a `bump` member spelled `vis` (`None` = plain `fn`), a `pub` maker,
/// and a same-module `inside` that exercises the member in its own
/// module.
fn counter_mod(vis: Option<&str>) -> String {
    let dial = vis.unwrap_or("");
    format!(
        "pub class Counter {{ n: i32 }}\n\
         impl Counter {{\n\
         \x20   {dial}fn bump(self) -> i32 {{ return self.n + 1; }}\n\
         }}\n\
         pub fn mk(n: i32) -> Counter {{ return Counter {{ n: n }}; }}\n\
         pub fn inside() -> i32 {{ let c = Counter {{ n: 41 }}; return c.bump(); }}\n"
    )
}

#[test]
fn member_visibility_methods_matrix() {
    let dep = modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]);
    let root_calls = "mod a;\nentry fn main() -> i32 { let c = a::mk(41); return c.bump(); }\n";

    // plain method from the ROOT: the new diagnostic — the owner
    // prefixed, the free-fn wording family after the dash
    let pkgs = vec![
        dep.clone(),
        modpkg("pkg", root_calls, vec![("a", Vis::Self_, counter_mod(None).as_str())]),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    assert!(
        diags.iter().any(|d| d.contains("`Counter.bump` — `bump` is private to module `a`")
            && d.contains("declare it `pub` (or `pub(pkg)`) to widen")),
        "{diags:?}"
    );

    // plain method from a SIBLING subtree (class in `a/c`, call in
    // `a/d`): refused — `a/d` is no descendant of `a/c`
    let pkgs = vec![
        dep.clone(),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { return a::d::go(); }\n",
            vec![
                ("a", Vis::Self_, "pub mod c;\npub mod d;\n"),
                ("a/c", Vis::Pub, counter_mod(None).as_str()),
                (
                    "a/d",
                    Vis::Pub,
                    "pub fn go() -> i32 { let c = a::c::mk(41); return c.bump(); }\n",
                ),
            ],
        ),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    assert!(
        diags.iter().any(|d| d.contains("`Counter.bump` — `bump` is private to module `a/c`")),
        "{diags:?}"
    );

    // plain method SAME-MODULE: the ordinary law, it runs
    let pkgs = vec![
        dep.clone(),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { return a::inside(); }\n",
            vec![("a", Vis::Self_, counter_mod(None).as_str())],
        ),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 42);

    // plain method from a DESCENDANT (call site in `a/sub`, class in
    // `a`): the declaring module's subtree sees it
    let pkgs = vec![
        dep.clone(),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { return a::sub::reach(); }\n",
            vec![
                ("a", Vis::Self_, format!("pub mod sub;\n{}", counter_mod(None)).as_str()),
                (
                    "a/sub",
                    Vis::Pub,
                    "pub fn reach() -> i32 { let c = a::mk(41); return c.bump(); }\n",
                ),
            ],
        ),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 42);

    // `pub`: crosses to the root
    let pkgs = vec![
        dep.clone(),
        modpkg("pkg", root_calls, vec![("a", Vis::Self_, counter_mod(Some("pub ")).as_str())]),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 42);

    // `pub(pkg)`: crosses within the package
    let pkgs = vec![
        dep.clone(),
        modpkg("pkg", root_calls, vec![("a", Vis::Self_, counter_mod(Some("pub(pkg) ")).as_str())]),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 42);

    // `pub(super)` (class in `a/c`): the parent module's subtree —
    // `a` reaches it, `a/d` (the sibling subtree) reaches it, the root
    // does not
    let pkgs = vec![
        dep.clone(),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { return a::go(); }\n",
            vec![
                (
                    "a",
                    Vis::Self_,
                    "pub mod c;\npub fn go() -> i32 { let c = a::c::mk(41); return c.bump(); }\n",
                ),
                ("a/c", Vis::Pub, counter_mod(Some("pub(super) ")).as_str()),
            ],
        ),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 42);

    let pkgs = vec![
        dep.clone(),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { return a::d::go(); }\n",
            vec![
                ("a", Vis::Self_, "pub mod c;\npub mod d;\n"),
                ("a/c", Vis::Pub, counter_mod(Some("pub(super) ")).as_str()),
                (
                    "a/d",
                    Vis::Pub,
                    "pub fn go() -> i32 { let c = a::c::mk(41); return c.bump(); }\n",
                ),
            ],
        ),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 42);

    let pkgs = vec![
        dep,
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { let c = a::c::mk(41); return c.bump(); }\n",
            vec![
                ("a", Vis::Self_, "pub mod c;\n"),
                ("a/c", Vis::Pub, counter_mod(Some("pub(super) ")).as_str()),
            ],
        ),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    assert!(
        diags.iter().any(|d| d.contains("`Counter.bump` — `bump` is `pub(super)` — visible only in `a`'s subtree")),
        "{diags:?}"
    );
}

#[test]
fn member_visibility_static_calls() {
    let dep = modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]);
    // a private static from the root: refused, owner prefixed
    let acc = String::from(
        "pub class Acc { n: i32 }\n\
         impl Acc {\n\
         \x20   fn make(n: i32) -> Acc { return Acc { n: n }; }\n\
         \x20   pub fn bump(self) -> i32 { return self.n + 1; }\n\
         }\n\
         pub fn inside() -> i32 { return Acc::make(41).bump(); }\n",
    );
    let pkgs = vec![
        dep.clone(),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { return a::Acc::make(41).bump(); }\n",
            vec![("a", Vis::Self_, acc.as_str())],
        ),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    assert!(
        diags.iter().any(|d| d.contains("`Acc.make` — `make` is private to module `a`")),
        "{diags:?}"
    );
    // ...and the same static SAME-MODULE runs
    let pkgs = vec![
        dep.clone(),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { return a::inside(); }\n",
            vec![("a", Vis::Self_, acc.as_str())],
        ),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 42);

    // the `[constructor]` sugar `Type(..)` IS the class-method call —
    // the seal rides the member's `pub`: a private ctor refuses from
    // the root, answers in its own module
    let point = String::from(
        "pub class Point { x: i32; y: i32 }\n\
         impl Point {\n\
         \x20   [constructor] fn from_xy(x: i32, y: i32) -> Self { return Self { x: x, y: y }; }\n\
         }\n\
         pub fn inside() -> i32 { return Point(40, 2).x + 2; }\n",
    );
    let pkgs = vec![
        dep.clone(),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { let p = a::Point(40, 2); return p.x; }\n",
            vec![("a", Vis::Self_, point.as_str())],
        ),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    assert!(
        diags.iter().any(|d| d.contains("`Point.from_xy` — `from_xy` is private to module `a`")),
        "{diags:?}"
    );
    let pkgs = vec![
        dep,
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { return a::inside(); }\n",
            vec![("a", Vis::Self_, point.as_str())],
        ),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 42);
}

#[test]
fn member_visibility_enum_methods() {
    let dep = modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]);
    // `flip`/`start` spell the two members' dials
    let mood = |flip: &str, start: &str| {
        format!(
            "pub enum Mood {{ Low, High }}\n\
             impl Mood {{\n\
             \x20   {flip}fn flipped(self) -> Mood {{ return when (self) {{ Mood::Low -> Mood::High, Mood::High -> Mood::Low }}; }}\n\
             \x20   {start}fn start() -> Self {{ return Mood::Low; }}\n\
             }}\n\
             pub fn low() -> Mood {{ return Mood::Low; }}\n\
             impl Mood {{ pub fn into_i(self) -> i32 {{ return 7; }} }}\n"
        )
    };
    // the RECEIVER lane, `pub`: crosses to the root
    let pkgs = vec![
        dep.clone(),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { let m = a::low(); return m.flipped().into_i(); }\n",
            vec![("a", Vis::Self_, mood("pub ", "pub ").as_str())],
        ),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 7);

    // ...private: refused with the member diagnostic
    let pkgs = vec![
        dep.clone(),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { let m = a::low(); return m.flipped().into_i(); }\n",
            vec![("a", Vis::Self_, mood("", "pub ").as_str())],
        ),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    assert!(
        diags.iter().any(|d| d.contains("`Mood.flipped` — `flipped` is private to module `a`")),
        "{diags:?}"
    );

    // the STATIC lane: a private enum static refuses, a pub one crosses
    let pkgs = vec![
        dep.clone(),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { return a::Mood::start().into_i(); }\n",
            vec![("a", Vis::Self_, mood("pub ", "").as_str())],
        ),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    assert!(
        diags.iter().any(|d| d.contains("`Mood.start` — `start` is private to module `a`")),
        "{diags:?}"
    );

    let pkgs = vec![
        dep,
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { return a::Mood::start().into_i(); }\n",
            vec![("a", Vis::Self_, mood("pub ", "pub ").as_str())],
        ),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 7);
}

#[test]
fn the_todolist_shape_private_impl_methods_are_errors() {
    // the repro's METHOD shape: `imp` declares a `pub class` whose
    // inherent impl members are plain `fn` — the root's `append_task`/
    // `toggle_task` calls refuse with the member diagnostic (the
    // field/literal halves land in the field phase)
    let imp = String::from(
        "pub class TodoList { _id: u64 }\n\
         impl TodoList {\n\
         \x20   fn toggle_task(mut self, id: u64) { self._id += id; }\n\
         \x20   fn append_task(mut self, t: str) { self._id += 1; }\n\
         }\n\
         pub fn mk() -> TodoList { return TodoList { _id: 0 }; }\n",
    );
    let main_src = "mod imp;\nentry fn main() -> i32 {\n\
                    \x20   let mut list = imp::mk();\n\
                    \x20   list.toggle_task(1);\n\
                    \x20   list.append_task(\"Shutdown\");\n\
                    \x20   return 0;\n\
                    }\n";
    let pkgs = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg("pkg", main_src, vec![("imp", Vis::Self_, imp.as_str())]),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    assert!(
        diags.iter().any(|d| d.contains("`TodoList.toggle_task` — `toggle_task` is private to module `imp`")),
        "{diags:?}"
    );
    assert!(
        diags.iter().any(|d| d.contains("`TodoList.append_task` — `append_task` is private to module `imp`")),
        "{diags:?}"
    );
    // the same class with `pub` members runs — the law is the dial,
    // never the class
    let pub_imp = imp
        .replace("fn toggle_task", "pub fn toggle_task")
        .replace("fn append_task", "pub fn append_task");
    let pkgs = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg("pkg", main_src, vec![("imp", Vis::Self_, pub_imp.as_str())]),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 0);
}

#[test]
fn for_of_gates_the_iterable_member() {
    // the designated `[iterable]` member takes the `pub` dial: the
    // weave's `xs.<member>(emit)` binds at the for-of site. The
    // accumulator rides a struct cell (the every-field-defaulted
    // literal) — the shape the for-of capture law drives cleanly.
    let count = |dial: &str| {
        format!(
            "pub struct Acc {{ hits: i32 = 0; }}\n\
             pub class Count {{ n: i32 }}\n\
             impl Count {{\n\
             \x20   fn new(n: i32) -> Self {{ return Self {{ n: n }}; }}\n\
             \x20   [iterable] {dial}fn iterate(self, emit: fn(i32) -> bool) {{\n\
             \x20       let mut i = 1;\n\
             \x20       while (i <= self.n) {{\n\
             \x20           if (!emit(i)) {{ return; }}\n\
             \x20           i += 1;\n\
             \x20       }}\n\
             \x20   }}\n\
             }}\n\
             pub fn mk(n: i32) -> Count {{ return Count::new(n); }}\n\
             pub fn new_acc() -> Acc {{ return Acc {{ }}; }}\n\
             pub fn sum_to(n: i32) -> i32 {{\n\
             \x20   let mut acc: ?Acc = Acc {{ }};\n\
             \x20   let c = Count::new(n);\n\
             \x20   for (let v of c) {{ acc.hits += 1; }}\n\
             \x20   return acc.hits;\n\
             }}\n"
        )
    };
    let root_loop = "mod a;\nentry fn main() -> i32 {\n\
                     \x20   let mut acc = a::new_acc();\n\
                     \x20   let c = a::mk(3);\n\
                     \x20   for (let v of c) { acc.hits += 1; }\n\
                     \x20   return acc.hits;\n\
                     }\n";
    let dep = modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]);
    // a private marked member refuses the root's for-of
    let pkgs = vec![
        dep.clone(),
        modpkg("pkg", root_loop, vec![("a", Vis::Self_, count("").as_str())]),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    assert!(
        diags.iter().any(|d| d.contains("`Count.iterate` — `iterate` is private to module `a`")),
        "{diags:?}"
    );
    // same-module for-of over the private member: runs (3 iterations)
    let pkgs = vec![
        dep.clone(),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { return a::sum_to(3); }\n",
            vec![("a", Vis::Self_, count("").as_str())],
        ),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 3);
    // and a `pub` marked member crosses: the root's for-of runs
    let pkgs = vec![
        dep,
        modpkg("pkg", root_loop, vec![("a", Vis::Self_, count("pub ").as_str())]),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 3);
}

// =====================================================================
// the member-visibility law — FIELDS + LITERALS (the field lanes): a
// class field's read, its write, and a literal NAMING it take the same
// `pub` dial the methods took above. Struct fields stay always-pub —
// the law's DataKind::Struct exemption is pinned separately below.
// =====================================================================

/// The Box-on-`a` fixture: a `pub class` with one field spelled `vis`
/// (`None` = plain), a `pub` getter, a `pub` maker, and the
/// same-module `inside_*` cells for every lane (read, write, literal,
/// and the `::`-spelled path read).
fn box_mod(vis: &str) -> String {
    format!(
        "pub class Box {{ {vis}v: i32 }}\n\
         impl Box {{ pub fn get(self) -> i32 {{ return self.v; }} }}\n\
         pub fn mk(n: i32) -> Box {{ return Box {{ v: n }}; }}\n\
         pub fn inside_read() -> i32 {{ let b = mk(41); return b.v; }}\n\
         pub fn inside_write() -> i32 {{ let mut b = mk(41); b.v = 42; return b.v; }}\n\
         pub fn inside_lit() -> i32 {{ let b = Box {{ v: 41 }}; return b.get(); }}\n\
         pub fn inside_colon() -> i32 {{ let b = mk(41); return b::v; }}\n"
    )
}

/// A site module's three cross-module lane fns against `pfx` (the
/// class module's qualified head) plus the `go` that sums them. The
/// fn names carry `tag` (fn names are package-global — two site
/// modules in one world must not collide). The literal head is a bare
/// ident, so that lane rides a `use` of the class; every fn body
/// type-checks even unpublished, so one compile reports each lane's
/// verdict and one run sums 41 + 42 + 41.
fn box_site(pfx: &str, tag: &str) -> String {
    format!(
        "use {pfx}::{{ Box }};\n\
         pub fn {tag}_read() -> i32 {{ let b = {pfx}::mk(41); return b.v; }}\n\
         pub fn {tag}_write() -> i32 {{ let mut b = {pfx}::mk(41); b.v = 42; return b.get(); }}\n\
         pub fn {tag}_lit() -> i32 {{ let b = Box {{ v: 41 }}; return b.get(); }}\n\
         pub fn {tag}_go() -> i32 {{ return {tag}_read() + {tag}_write() + {tag}_lit(); }}\n"
    )
}

#[test]
fn member_visibility_fields_matrix() {
    let dep = modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]);

    // the declaring module's own cells, every dial: the law is the
    // dial, never the class — same-module reads, writes, literals,
    // and the `::` path form all run (41 + 42 + 41 + 41)
    for dial in ["", "pub ", "pub(pkg) ", "pub(super) "] {
        let pkgs = vec![
            dep.clone(),
            modpkg(
                "pkg",
                "mod a;\nentry fn main() -> i32 { return a::inside_read() + a::inside_write() + a::inside_lit() + a::inside_colon(); }\n",
                vec![("a", Vis::Self_, box_mod(dial).as_str())],
            ),
        ];
        assert_eq!(run_main_pkgs(&pkgs, "pkg"), 165, "same-module `{dial}v`");
    }

    // plain field from the ROOT: all three lanes refuse with the
    // phase-1 wording family — the owner prefixed, the free-fn hint
    // after the dash
    let pkgs = vec![
        dep.clone(),
        modpkg(
            "pkg",
            "use a::{ Box };\nmod a;\n\
             fn r() -> i32 { let b = a::mk(41); return b.v; }\n\
             fn w() -> i32 { let mut b = a::mk(41); b.v = 42; return b.get(); }\n\
             fn l() -> i32 { let b = Box { v: 41 }; return b.get(); }\n\
             entry fn main() -> i32 { return r() + w() + l(); }\n",
            vec![("a", Vis::Self_, box_mod("").as_str())],
        ),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    assert_eq!(
        diags.iter().filter(|d| d.contains("`Box.v` — `v` is private to module `a`")).count(),
        3,
        "read, write, and literal each diagnose: {diags:?}"
    );
    assert!(
        diags.iter().any(|d| d.contains("declare it `pub` (or `pub(pkg)`) to widen")),
        "{diags:?}"
    );

    // plain field from a DESCENDANT (site in `a/sub`, class in `a`):
    // the declaring module's subtree sees it — all three lanes run
    let pkgs = vec![
        dep.clone(),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { return a::sub::sub_go(); }\n",
            vec![
                ("a", Vis::Self_, format!("pub mod sub;\n{}", box_mod("")).as_str()),
                ("a/sub", Vis::Pub, box_site("a", "sub").as_str()),
            ],
        ),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 124);

    // plain field from a SIBLING subtree (class in `a/c`, site in
    // `a/d`): refused — `a/d` is no descendant of `a/c`
    let pkgs = vec![
        dep.clone(),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { return a::d::d_go(); }\n",
            vec![
                ("a", Vis::Self_, "pub mod c;\npub mod d;\n"),
                ("a/c", Vis::Pub, box_mod("").as_str()),
                ("a/d", Vis::Pub, box_site("a::c", "d").as_str()),
            ],
        ),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    assert_eq!(
        diags.iter().filter(|d| d.contains("`Box.v` — `v` is private to module `a/c`")).count(),
        3,
        "{diags:?}"
    );

    // `pub` and `pub(pkg)`: cross to the root, a sibling subtree, and
    // another top-level module alike
    for dial in ["pub ", "pub(pkg) "] {
        let pkgs = vec![
            dep.clone(),
            modpkg(
                "pkg",
                "mod a;\nmod b;\nentry fn main() -> i32 { return a::b::ab_go() + b::b_go(); }\n",
                vec![
                    ("a", Vis::Self_, format!("pub mod b;\n{}", box_mod(dial)).as_str()),
                    ("a/b", Vis::Pub, box_site("a", "ab").as_str()),
                    ("b", Vis::Pub, box_site("a", "b").as_str()),
                ],
            ),
        ];
        assert_eq!(run_main_pkgs(&pkgs, "pkg"), 124 + 124, "`{dial}v` crosses");
    }

    // `pub(super)` (class in `a/c`): the parent module's subtree —
    // `a` and `a/d` reach the field, the root does not
    let pkgs = vec![
        dep.clone(),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { return a::a_go() + a::d::d_go(); }\n",
            vec![
                ("a", Vis::Self_, format!("pub mod c;\npub mod d;\n{}", box_site("a::c", "a")).as_str()),
                ("a/c", Vis::Pub, box_mod("pub(super) ").as_str()),
                ("a/d", Vis::Pub, box_site("a::c", "d").as_str()),
            ],
        ),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 124 + 124);

    let pkgs = vec![
        dep.clone(),
        modpkg(
            "pkg",
            "use a::c::{ Box };\nmod a;\n\
             fn r() -> i32 { let b = a::c::mk(41); return b.v; }\n\
             fn w() -> i32 { let mut b = a::c::mk(41); b.v = 42; return b.get(); }\n\
             fn l() -> i32 { let b = Box { v: 41 }; return b.get(); }\n\
             entry fn main() -> i32 { return r() + w() + l(); }\n",
            vec![
                ("a", Vis::Self_, "pub mod c;\n"),
                ("a/c", Vis::Pub, box_mod("pub(super) ").as_str()),
            ],
        ),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    assert_eq!(
        diags.iter().filter(|d| {
            d.contains("`Box.v` — `v` is `pub(super)` — visible only in `a`'s subtree")
        }).count(),
        3,
        "the read, the write, and the literal each diagnose: {diags:?}"
    );

    // the `::`-spelled path form rides the same dial (the path-chain
    // lanes: a `b::v` read, a `b::v = ..` write)
    let pkgs = vec![
        dep,
        modpkg(
            "pkg",
            "mod a;\n\
             fn r() -> i32 { let b = a::mk(41); return b::v; }\n\
             fn w() -> i32 { let mut b = a::mk(41); b::v = 42; return b.get(); }\n\
             entry fn main() -> i32 { return r() + w(); }\n",
            vec![("a", Vis::Self_, box_mod("").as_str())],
        ),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    assert_eq!(
        diags.iter().filter(|d| d.contains("`Box.v` — `v` is private to module `a`")).count(),
        2,
        "the path-form read and write each diagnose: {diags:?}"
    );
}

#[test]
fn struct_fields_stay_public_cross_module() {
    // the always-pub law pinned: `DataKind::Struct` has no dial — a
    // struct field reads, writes, and literal-spells from another
    // module exactly as before this phase
    let point = "pub struct Point { x: i32; y: i32 }\n\
                 pub fn mk(x: i32) -> Point { return Point { x: x, y: 0 }; }\n";
    let pkgs = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg(
            "pkg",
            "use a::{ Point };\nmod a;\nentry fn main() -> i32 {\n\
             \x20   let mut p = a::mk(40);\n\
             \x20   p.y = 2;\n\
             \x20   let q = Point { x: 40, y: 2 };\n\
             \x20   return p.x + p.y + q.x + q.y;\n\
             }\n",
            vec![("a", Vis::Self_, point)],
        ),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 84);
}

/// The repro's `imp` module with its members' dials spelled: `pubf`
/// fronts the two methods, `pubfld` the two fields.
fn todolist_mod(pubf: &str, pubfld: &str) -> String {
    format!(
        "pub class TodoList {{ {pubfld}_id: i32; {pubfld}tasks: str }}\n\
         impl TodoList {{\n\
         \x20   {pubf}fn toggle_task(mut self, id: i32) {{ self._id += id; }}\n\
         \x20   {pubf}fn append_task(mut self, t: str) {{ self.tasks = t; }}\n\
         }}\n\
         pub fn mk() -> TodoList {{ return TodoList {{ _id: 0, tasks: \"\" }}; }}\n"
    )
}

#[test]
fn the_todolist_shape_private_fields_are_errors() {
    // the repro's FIELD/LITERAL half, methods already `pub`: the
    // root's `list.tasks` read and the literals' `_id`/`tasks`
    // spellings refuse with the member diagnostic (each probe fn
    // binds its own list — a failed literal poisons its name)
    let imp = todolist_mod("pub ", "");
    let main_src = "use imp::{ TodoList };\nmod imp;\n\
                    fn peek() -> i32 { let l = imp::mk(); return l.tasks.len(); }\n\
                    fn lit_b() -> i32 { let l = TodoList { tasks: \"\", _id: 0 }; return l.tasks.len(); }\n\
                    entry fn main() -> i32 {\n\
                    \x20   let list = TodoList { _id: 5, tasks: \"\" };\n\
                    \x20   return peek() + lit_b() + list.tasks.len();\n\
                    }\n";
    let pkgs = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg("pkg", main_src, vec![("imp", Vis::Self_, imp.as_str())]),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    assert!(
        diags.iter().any(|d| d.contains("`TodoList.tasks` — `tasks` is private to module `imp`")),
        "{diags:?}"
    );
    assert!(
        diags.iter().any(|d| d.contains("`TodoList._id` — `_id` is private to module `imp`")),
        "{diags:?}"
    );

    // spelling `pub` on the fields turns the same program green and
    // correct: the root reads and writes the fields, appends through
    // the (already pub) method ("Shutdown" = 8 codepoints, _id 5 + 3)
    let pub_imp = todolist_mod("pub ", "pub ");
    let main_src = "use imp::{ TodoList };\nmod imp;\n\
                    fn peek(list: TodoList) -> i32 { return list.tasks.len(); }\n\
                    entry fn main() -> i32 {\n\
                    \x20   let mut list = TodoList { _id: 5, tasks: \"\" };\n\
                    \x20   list.append_task(\"Shutdown\");\n\
                    \x20   list.toggle_task(3);\n\
                    \x20   return peek(list) + list._id;\n\
                    }\n";
    let pkgs = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg("pkg", main_src, vec![("imp", Vis::Self_, pub_imp.as_str())]),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 16);
}

#[test]
fn the_todolist_shape_private_members_full_two_file_shape() {
    // the repro's FULL two-file shape — the phase-1 methods PLUS this
    // phase's fields and literal: the root's toggle_task/append_task
    // calls, its `list.tasks` read, and the literal's `_id`/`tasks`
    // spellings ALL refuse in one compile (each site is its own fn —
    // the sibling-recovery walk reports every lane)
    let imp = todolist_mod("", "");
    let main_src = "use imp::{ TodoList };\nmod imp;\n\
                    fn peek() -> i32 { let l = imp::mk(); return l.tasks.len(); }\n\
                    fn lit_b() -> i32 { let l = TodoList { tasks: \"\", _id: 0 }; return 0; }\n\
                    entry fn main() -> i32 {\n\
                    \x20   let list = TodoList { _id: 5, tasks: \"\" };\n\
                    \x20   let other = imp::mk();\n\
                    \x20   other.toggle_task(1);\n\
                    \x20   other.append_task(\"Shutdown\");\n\
                    \x20   return peek() + lit_b();\n\
                    }\n";
    let pkgs = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg("pkg", main_src, vec![("imp", Vis::Self_, imp.as_str())]),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    for want in [
        "`TodoList.toggle_task` — `toggle_task` is private to module `imp`",
        "`TodoList.append_task` — `append_task` is private to module `imp`",
        "`TodoList.tasks` — `tasks` is private to module `imp`",
        "`TodoList._id` — `_id` is private to module `imp`",
    ] {
        assert!(diags.iter().any(|d| d.contains(want)), "missing `{want}`: {diags:?}");
    }

    // spelling `pub` on all four members turns the program green with
    // the correct output ("Shutdown" = 8 codepoints, _id 5 + 3)
    let pub_imp = todolist_mod("pub ", "pub ");
    let main_src = "use imp::{ TodoList };\nmod imp;\n\
                    entry fn main() -> i32 {\n\
                    \x20   let mut list = TodoList { _id: 5, tasks: \"\" };\n\
                    \x20   list.append_task(\"Shutdown\");\n\
                    \x20   list.toggle_task(3);\n\
                    \x20   return list.tasks.len() + list._id;\n\
                    }\n";
    let pkgs = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg("pkg", main_src, vec![("imp", Vis::Self_, pub_imp.as_str())]),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 16);
}

#[test]
fn use_through_pub_mod_chain_two_levels() {
    // `use dep::alpha::beta::{cell}` — the two-level pub mod walk,
    // then the leaf binds and the bare call resolves
    let dep = modpkg(
        "dep",
        "pub mod alpha;\n",
        vec![
            ("alpha", Vis::Pub, "pub mod beta;\npub fn top() -> i32 { return 10; }\n"),
            ("alpha/beta", Vis::Pub, "pub fn cell() -> i32 { return 2; }\n"),
        ],
    );
    let consumer = modpkg(
        "pkg",
        "use dep::alpha::beta::{cell};\nuse dep::alpha::{top};\nentry fn main() -> i32 { return cell() + top(); }\n",
        vec![],
    );
    assert_eq!(run_main_pkgs(&[dep, consumer], "pkg"), 12);
}

#[test]
fn use_of_a_non_pub_mod_is_rejected() {
    let diags = compile_with_diags(
        &[
            modpkg(
                "dep",
                "pub mod alpha;\nmod secret;\n",
                vec![
                    ("alpha", Vis::Pub, "pub fn top() -> i32 { return 10; }\n"),
                    ("secret", Vis::Self_, "pub fn whisper() -> i32 { return 1; }\n"),
                ],
            ),
            modpkg(
                "pkg",
                "use dep::secret::{whisper};\nentry fn main() -> i32 { return whisper(); }\n",
                vec![],
            ),
        ],
        "pkg",
    );
    assert!(
        diags.iter().any(|d| d.contains(
            "`secret` is not visible from outside the package (`mod`, not `pub mod`)"
        )),
        "{diags:?}"
    );
}

#[test]
fn an_inherent_impl_in_the_wrong_module_is_rejected() {
    let pkgs = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg(
            "pkg",
            "mod a;\nimpl Counter { pub fn bump(self) -> i32 { return self.n + 1; } }\nentry fn main() -> i32 { return 0; }\n",
            vec![("a", Vis::Self_, "pub struct Counter { n: i32 }\n")],
        ),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    assert!(
        diags.iter().any(|d| d.contains(
            "an inherent impl for `Counter` must live in `Counter`'s module (a/mod.rut)"
        )),
        "{diags:?}"
    );
    // the impl in ITS module compiles and runs
    let ok = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { return a::bumped(); }\n",
            vec![(
                "a",
                Vis::Self_,
                "pub struct Counter { n: i32 }\nimpl Counter { pub fn bump(self) -> i32 { return self.n + 1; } }\npub fn bumped() -> i32 { let c = Counter { n: 41 }; return c.bump(); }\n",
            )],
        ),
    ];
    assert_eq!(run_main_pkgs(&ok, "pkg"), 42);
}

#[test]
fn a_package_head_in_a_position_names_the_use_fix() {
    // dep is use-bound in pkg; `dep::alpha::top()` written directly in a
    // position refuses — uses are the only cross-package door
    let pkgs = vec![
        modpkg(
            "dep",
            "pub mod alpha;\n",
            vec![("alpha", Vis::Pub, "pub fn top() -> i32 { return 10; }\n")],
        ),
        modpkg(
            "pkg",
            "use dep::{};\nentry fn main() -> i32 { return dep::alpha::top(); }\n",
            vec![],
        ),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    assert!(
        diags.iter().any(|d| d.contains("uses are the only cross-package door")
            && d.contains("use dep::alpha::{ top }")),
        "{diags:?}"
    );
}

#[test]
fn use_bindings_are_per_file() {
    // the root binds `cell` from dep; a's file does NOT — its bare
    // `cell()` stays unknown, naming where the binding lives
    let pkgs = vec![
        modpkg(
            "dep",
            "pub mod alpha;\n",
            vec![("alpha", Vis::Pub, "pub fn cell() -> i32 { return 2; }\n")],
        ),
        modpkg(
            "pkg",
            "use dep::alpha::{cell};\nmod a;\nentry fn main() -> i32 { return cell() + a::go(); }\n",
            vec![("a", Vis::Self_, "pub fn go() -> i32 { return cell(); }\n")],
        ),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    assert!(
        diags.iter().any(|d| d.contains("`cell` is use-bound in the root module")
            && d.contains("needs its own `use`")),
        "{diags:?}"
    );
}

#[test]
fn cross_file_methods_and_members_collect() {
    // a's class carries its methods; the ROOT constructs and calls
    // them through the qualified path — the type's module owns the
    // surface, the compiling file is just the entry
    let pkgs = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { let c: a::Acc = a::mk(40); return c.bump2(); }\n",
            vec![(
                "a",
                Vis::Self_,
                "pub struct Acc { n: i32 }\nimpl Acc {\n    pub fn bump2(self) -> i32 { return self.n + 2; }\n}\npub fn mk(n: i32) -> Acc { return Acc { n: n }; }\n",
            )],
        ),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 42);
}

#[test]
fn duplicate_names_across_modules_are_loud() {
    let pkgs = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg(
            "pkg",
            "mod a;\nmod b;\nentry fn main() -> i32 { return 0; }\n",
            vec![
                ("a", Vis::Self_, "pub struct Thing { v: i32 }\n"),
                ("b", Vis::Self_, "pub struct Thing { v: i32 }\n"),
            ],
        ),
    ];
    let diags = compile_with_diags(&pkgs, "pkg");
    assert!(
        diags.iter().any(|d| d.contains("duplicate type name `Thing`")
            && d.contains("already declared in module `a`")),
        "{diags:?}"
    );
}

#[test]
fn qualified_iface_and_generic_types_resolve() {
    // a's interface as a qualified param type; a's generic class
    // instantiated through the qualified spelling
    let pkgs = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 {\n    let b: a::Box<i32> = a::mkbox(40);\n    return a::widen(b) + 2;\n}\n",
            vec![(
                "a",
                Vis::Self_,
                "pub interface Sized { fn size(self) -> i32; }\npub class Box<T> { item: T }\npub fn mkbox(v: i32) -> Box<i32> { return Box { item: v }; }\npub fn widen(b: Box<i32>) -> i32 { return b.item; }\n",
            )],
        ),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 42);
}

#[test]
fn qualified_static_call_through_the_mod_path() {
    // `a.Counter.make(41).bump()` — the static form on the walked
    // module's type (`a.Counter`), then the instance call
    let pkgs = vec![
        modpkg("dep", "entry fn main() -> i32 { return 0; }\n", vec![]),
        modpkg(
            "pkg",
            "mod a;\nentry fn main() -> i32 { let c = a::Counter::make(41); return c.bump(); }\n",
            vec![(
                "a",
                Vis::Self_,
                "pub struct Counter { n: i32 }\nimpl Counter {\n    pub fn make(n: i32) -> Counter { return Counter { n: n }; }\n    pub fn bump(self) -> i32 { return self.n + 1; }\n}\n",
            )],
        ),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 42);
}
