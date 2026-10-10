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
    let loaded = Pkg::from_bundle(&bytes).expect("load");
    let bundled = loaded.pkg("pkg").expect("pkg mounted");
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
        r#"{"name": "pkg", "format": "rutbundle", "format_version": 10}"#,
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
fn the_published_std_v8_bundles_still_load_through_reader_compat() {
    // the pin is law: every PUBLISHED std-v8 artifact (the OLD flat
    // envelope — manifests spelling the repealed keys, sources at the
    // legacy names) keeps loading through the reader's compat lane.
    // The published bytes are committed at `dist/std-v8/` so the gate
    // never networks; `dist/std` (the working envelope) is the
    // freshness gate's business.
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
    for path in &names {
        let bytes = std::fs::read(path).expect("read the bundle");
        let entries = rut_driver::bundle::parse_bundle(&bytes).unwrap();
        let entry_names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
        assert!(
            !entry_names.iter().any(|n| n.ends_with("rut.mods")),
            "{} is old-envelope — it carries no rows section",
            path.display()
        );
        // the OLD manifest grammar rides: the archive's own rut.jsonc
        // spells the repealed keys, and the compat parse answers them
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
        r#"{"name": "app", "format": "rutbundle", "format_version": 10, "deps": {"pkg": {"path": "../pkg"}}}"#,
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
    assert!(names.contains(&"pkg/rut.mods".to_string()), "{names:?}");
    let loaded = Pkg::from_bundle(&bytes).expect("load");
    let pkg = loaded.pkg("pkg").expect("pkg group mounted");
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
    let legacy = r#"{"format": "rutbundle", "format_version": 10, "name": "pouch", "entry": {"lib": "./pouch.rut", "libs": ["./extra.rut"]}}"#;
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
    let loaded = Pkg::from_bundle(&legacy_bytes).expect("the old envelope loads");
    let pkg = loaded.pkg("pouch").expect("pouch mounted");
    // the spliced riding source: base, then libs in listed order
    let gen = pkg.gen_source.as_ref().expect("the legacy splice rides");
    assert_eq!(
        gen.text.as_str(),
        format!("{pouch_src}\n// the legacy splice tail\n"),
        "the splice is base, then libs in manifest order, '\n'-joined"
    );
}

// =====================================================================
// phase 3 — lir/resolution: visibility tiers, qualified positions,
// use-through-mods
//
// Spelling note: positions qualify with `.` (`a.Pair`, `layout.mk(3)`,
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
                    ModSource { path: p.to_string(), vis: v, text: t.to_string() },
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
    for p in pkgs {
        run = run.pkg(p.clone());
    }
    let c = run.entrypoint(entry).compile().expect("compile carries the diags");
    c.graph.diags.iter().map(|d| d.msg.clone()).collect()
}

fn run_main_pkgs(pkgs: &[rut_driver::Pkg], entry: &str) -> i64 {
    let mut run = rut_driver::RutRun::new();
    for p in pkgs {
        run = run.pkg(p.clone());
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
            "mod a;\nentry fn main() -> i32 {\n    let r: a.Pair = a.pair(3);\n    return r.x + r.y;\n}\n",
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
            "mod a;\npub fn root_side() -> i32 { return 30; }\nentry fn main() -> i32 { return a.c.c2.seven(); }\n",
            vec![
                ("a", Vis::Self_, "pub mod c;\npub fn five() -> i32 { return 5; }\n"),
                ("a/c", Vis::Pub, "pub mod c2;\n"),
                (
                    "a/c/c2",
                    Vis::Pub,
                    "pub fn seven() -> i32 {\n    return pkg.root_side() + pkg.a.five() + 7;\n}\n",
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
            "mod a;\nentry fn main() -> i32 { return a.ghost(); }\n",
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
            "mod a;\nentry fn main() -> i32 { return a.inner(); }\n",
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
            "mod a;\nentry fn main() -> i32 { return a.uses_inner(); }\n",
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
            "mod a;\nentry fn main() -> i32 { return a.go(); }\n",
            vec![
                ("a", Vis::Self_, "pub mod c;\npub fn go() -> i32 { return c.sup(); }\n"),
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
            "mod a;\nentry fn main() -> i32 { return a.c.sup(); }\n",
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
            "mod a;\nentry fn main() -> i32 { return a.go(); }\n",
            vec![
                ("a", Vis::Self_, "pub mod c;\npub mod d;\npub fn go() -> i32 { return d.reach(); }\n"),
                ("a/c", Vis::Pub, "pub(super) fn sup() -> i32 { return 11; }\n"),
                ("a/d", Vis::Pub, "pub fn reach() -> i32 { return a.c.sup(); }\n"),
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
        "pub mod alpha;\nentry fn main() -> i32 { return dep.alpha.inside(); }\n",
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
            "mod a;\nentry fn main() -> i32 { return a.bumped(); }\n",
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
    // dep is use-bound in pkg; `dep.alpha.top()` written directly in a
    // position refuses — uses are the only cross-package door
    let pkgs = vec![
        modpkg(
            "dep",
            "pub mod alpha;\n",
            vec![("alpha", Vis::Pub, "pub fn top() -> i32 { return 10; }\n")],
        ),
        modpkg(
            "pkg",
            "use dep::{};\nentry fn main() -> i32 { return dep.alpha.top(); }\n",
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
            "use dep::alpha::{cell};\nmod a;\nentry fn main() -> i32 { return cell() + a.go(); }\n",
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
            "mod a;\nentry fn main() -> i32 { let c: a.Acc = a.mk(40); return c.bump2(); }\n",
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
            "mod a;\nentry fn main() -> i32 {\n    let b: a.Box<i32> = a.mkbox(40);\n    return a.widen(b) + 2;\n}\n",
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
            "mod a;\nentry fn main() -> i32 { let c = a.Counter.make(41); return c.bump(); }\n",
            vec![(
                "a",
                Vis::Self_,
                "pub struct Counter { n: i32 }\nimpl Counter {\n    pub fn make(n: i32) -> Counter { return Counter { n: n }; }\n    pub fn bump(self) -> i32 { return self.n + 1; }\n}\n",
            )],
        ),
    ];
    assert_eq!(run_main_pkgs(&pkgs, "pkg"), 42);
}
