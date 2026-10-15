//! Dep hygiene — the nameability law (D5). Mounting stays transitive:
//! A's deps still pull B into the closure and A still compiles against
//! B's mounted rows. What is declaration-scoped is NAMEABILITY: a pkg
//! may `use` exactly the heads it declares itself — `deps` ∪
//! (`dev-deps` only when it is the program root) ∪
//! `core`. A head that is MOUNTED but UNDECLARED by the consumer is
//! D5, naming the mount edge and the fix; unmounted heads keep their
//! shapes (NoModule — pinned unchanged here).
//!
//! The law's doors, each pinned end to end:
//! - the walked world (the run chain's closure check + the graph's
//!   `ensure_use`/`validate_qualified_uses` — one door,
//!   `Session::resolve_use`, feeds both);
//! - the dev arm (the root's dev-deps nameable at the root only);
//! - the bundle lane (a source group's in-archive dep: declared → ok,
//!   stripped → D5; the pack lane refuses through the same resolver);
//! - the hand-mounted world (the structural manifest is the one
//!   mechanism — `Manifest::lib(..)` / `parse_manifest(..)`; a missing
//!   row renders D5's host clause).

use std::path::{Path, PathBuf};

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-dep-hygiene-{tag}-{}", std::process::id()));
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

/// One manifest with the given `[deps]` rows (`"b": {"path": "../b"}`).
fn manifest(name: &str, deps: &[(&str, &str)]) -> String {
    let rows: Vec<String> = deps
        .iter()
        .map(|(k, v)| format!(r#""{k}": {{"path": "{v}""#).to_string() + "}")
        .collect();
    let table = if rows.is_empty() {
        String::new()
    } else {
        format!(", \"deps\": {{{}}}", rows.join(", "))
    };
    format!(r#"{{"name": "{name}"{table}}}"#)
}

/// A three-pkg walked world: `app` (the root: `[deps]` = `app_deps`,
/// `[dev-deps]` = `dev`), `a` (deps = `a_deps`), `b` (a leaf exporting
/// `pub fn x()`). Tests overwrite `app/mod.rut` (and sometimes `a`'s)
/// before compiling.
fn world(
    tag: &str,
    app_deps: &[(&str, &str)],
    a_deps: &[(&str, &str)],
    dev: &[(&str, &str)],
) -> PathBuf {
    let root = scratch(tag);
    let app = root.join("app");
    let mut m = manifest("app", app_deps);
    if !dev.is_empty() {
        let rows: Vec<String> = dev
            .iter()
            .map(|(k, v)| format!(r#""{k}": {{"path": "{v}""#).to_string() + "}")
            .collect();
        m.insert_str(m.len() - 1, &format!(", \"dev-deps\": {{{}}}", rows.join(", ")));
    }
    write(&app, "rut.jsonc", &m);
    write(&app, "mod.rut", "entry fn main() -> i32 { return 1; }\n");
    let a = root.join("a");
    write(&a, "rut.jsonc", &manifest("a", a_deps));
    write(&a, "mod.rut", "pub fn mid() -> i32 { return 2; }\n");
    let b = root.join("b");
    write(&b, "rut.jsonc", &manifest("b", &[]));
    write(&b, "mod.rut", "pub fn x() -> i32 { return 3; }\n");
    root
}

fn load(dir: &Path) -> rut_driver::Loaded {
    rut_native::load_dir(dir).expect("the walk mounts")
}

/// Compile the world's `app` root AS WRITTEN (tests overwrite
/// `app/mod.rut` before this) — the run chain's verdict.
fn compile_root(dir: &Path, src: &str) -> Result<rut_driver::GraphOutput, String> {
    write(&dir.join("app"), "mod.rut", src);
    let loaded = load(&dir.join("app"));
    rut_driver::RutRun::new()
        .pkgs(&loaded)
        .entrypoint(&loaded.root)
        .compile()
        .map(|c| c.graph)
        .map_err(|e| e.msg)
}

/// [`compile_root`], refusing unless the run refused — the Err leg.
fn refuse_root(dir: &Path, src: &str) -> String {
    match compile_root(dir, src) {
        Err(e) => e,
        Ok(g) => panic!(
            "expected a refusal, got diags: {:?}",
            g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>()
        ),
    }
}

// ---------------------------------------------------------------------
// D5 — mounted but undeclared (the walked world)
// ---------------------------------------------------------------------

#[test]
fn transitive_head_is_mounted_but_undeclared_is_d5() {
    // app deps {a}; a deps {b}; app uses b — b IS in the closure
    // (transitive mounting is untouched), but app never declared it.
    // D5, byte-for-byte: the head, the mount edge ("pulled by `a`"),
    // the consumer, and the fix.
    let root = world("d5-text", &[("a", "../a")], &[("b", "../b")], &[]);
    let err = refuse_root(
        &root,
        "use b::{x};\nentry fn main() -> i32 { return x(); }\n",
    );
    assert_eq!(
        err,
        "cannot resolve `b` — `b` is mounted in this program (pulled by `a`) but `app` does not declare it; add \"b\": { \"path\": \"..\" } to `app`'s `rut.jsonc` `deps`",
    );
}

#[test]
fn declaring_the_transitive_head_compiles() {
    // the same world with the row added — the fix works: `b` joins
    // app's `deps`, the use is declared, everything compiles
    let root = world(
        "d5-fix",
        &[("a", "../a"), ("b", "../b")],
        &[("b", "../b")],
        &[],
    );
    let g = compile_root(
        &root,
        "use b::{x};\nentry fn main() -> i32 { return x(); }\n",
    )
    .expect("the declared use compiles");
    assert!(
        g.diags.is_empty(),
        "{}",
        g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn the_use_law_is_the_consumers_own_word_at_every_depth() {
    // `a` (a NON-root pkg) using the mounted `b` it never declared is
    // the same D5, with `a` as the consumer — the closure check reads
    // every mounted source's uses, not just the root's. (`b` mounts
    // through the ROOT's row here; `a` still may not name it.)
    let root = world("d5-mid", &[("a", "../a"), ("b", "../b")], &[], &[]);
    write(&root.join("a"), "mod.rut", "use b::{x};\npub fn mid() -> i32 { return x(); }\n");
    let err =
        refuse_root(&root, "use a::{mid};\nentry fn main() -> i32 { return mid(); }\n");
    assert_eq!(
        err,
        "cannot resolve `b` — `b` is mounted in this program (pulled by `app`) but `a` does not declare it; add \"b\": { \"path\": \"..\" } to `a`'s `rut.jsonc` `deps`",
    );
}

// ---------------------------------------------------------------------
// unmounted heads keep their shapes
// ---------------------------------------------------------------------

#[test]
fn unmounted_head_stays_the_bare_nomodule() {
    // D5 is MOUNTED-but-undeclared ONLY: a name nothing mounts is the
    // unchanged NoModule text (never a false D5)
    let root = world("nomodule", &[("a", "../a")], &[], &[]);
    let err = refuse_root(
        &root,
        "use nosuch::{thing};\nentry fn main() -> i32 { return 1; }\n",
    );
    assert_eq!(
        err,
        "cannot resolve `nosuch` — no module with that name is mounted; declare it in your `rut.jsonc` `deps`",
    );
}

// ---------------------------------------------------------------------
// the dev arm — dev-deps are nameable at the root only
// ---------------------------------------------------------------------

#[test]
fn root_dev_deps_are_usable_at_the_root() {
    // app's `dev-deps` mount for the self-build, and the root may NAME
    // them: `use b::` at the root compiles
    let root = world("dev-root", &[("a", "../a")], &[], &[("b", "../b")]);
    let g = compile_root(
        &root,
        "use b::{x};\nentry fn main() -> i32 { return x(); }\n",
    )
    .expect("the root's dev use compiles");
    assert!(
        g.diags.is_empty(),
        "{}",
        g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn the_same_dev_name_used_by_a_non_root_is_d5() {
    // the flip side: `a` (not the root) naming app's dev-dep is D5 —
    // dev tables mount only for a self-build, and nameability follows
    // the same root-only law (the puller edge is app's dev pass)
    let root = world("dev-nonroot", &[("a", "../a")], &[], &[("b", "../b")]);
    write(&root.join("a"), "mod.rut", "use b::{x};\npub fn mid() -> i32 { return x(); }\n");
    let err =
        refuse_root(&root, "use a::{mid};\nentry fn main() -> i32 { return mid(); }\n");
    assert_eq!(
        err,
        "cannot resolve `b` — `b` is mounted in this program (pulled by `app`) but `a` does not declare it; add \"b\": { \"path\": \"..\" } to `a`'s `rut.jsonc` `deps`",
    );
}

// ---------------------------------------------------------------------
// the bundle lane
// ---------------------------------------------------------------------

/// A packable world: `app` deps {u, g}; `g` deps {u} and USES it; `u`
/// a leaf. Packed, `u` and `g` ride as source groups under the
/// compiled root.
fn bundle_world(tag: &str) -> (PathBuf, Vec<u8>) {
    let root = scratch(tag);
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 11, "name": "app", "deps": {"u": {"path": "../u"}, "g": {"path": "../g"}}}"#,
    );
    write(&app, "mod.rut", "use u::{leaf};\nentry fn go() -> i32 { return leaf(); }\n");
    let u = root.join("u");
    write(&u, "rut.jsonc", r#"{"name": "u"}"#);
    write(&u, "mod.rut", "pub fn leaf() -> i32 { return 5; }\n");
    let g = root.join("g");
    write(&g, "rut.jsonc", r#"{"name": "g", "deps": {"u": {"path": "../u"}}}"#);
    write(&g, "mod.rut", "use u::{leaf};\npub fn twice() -> i32 { return leaf() * 2; }\n");
    let bytes = rut_native::pack_dir(&app).expect("pack");
    (root, bytes)
}

#[test]
fn bundle_group_names_its_declared_in_archive_dep() {
    // the packed manifest is byte-for-byte the directory's: `g`
    // declared `u`, the archive rides both, and compiling the bundle
    // (the compiled root's ledger walk re-ensures the source groups)
    // takes g's `use u::` through the same allowlist — declared, green
    let (_root, bytes) = bundle_world("bundle-ok");
    let loaded = rut_driver::Pkg::from_bundle(&bytes).expect("load");
    let c = rut_driver::RutRun::new()
        .pkgs(&loaded)
        .entrypoint(&loaded.root)
        .compile()
        .expect("the declared bundle compiles");
    assert!(
        c.graph.diags.is_empty(),
        "{}",
        c.graph.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn bundle_group_with_the_row_stripped_is_d5() {
    // the same archive, but the group's declaration row stripped
    // post-load (the pkg is pure data — the offer is the test's):
    // the source group's use of `u` is now mounted-but-undeclared →
    // D5, and the mount edge names the archive root
    let (_root, bytes) = bundle_world("bundle-d5");
    let mut loaded = rut_driver::Pkg::from_bundle(&bytes).expect("load");
    for p in loaded.pkgs.iter_mut() {
        // groups mount under their content identity — match by the
        // bare name half
        if rut_driver::bundle::mount_name(&p.spec) == "g" {
            p.manifest.deps.remove("u");
        }
    }
    let err = rut_driver::RutRun::new()
        .pkgs(&loaded)
        .entrypoint(&loaded.root)
        .compile()
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "cannot resolve `u` — `u` is mounted in this program (pulled by `app`) but `g` does not declare it; add \"u\": { \"path\": \"..\" } to `g`'s `rut.jsonc` `deps`",
    );
}

#[test]
fn the_pack_lane_refuses_through_the_same_resolver() {
    // the pack walk has no run chain — its closure is `compile_units`,
    // whose use loops gate on the same `resolve_use`: an undeclared
    // transitive use is the D5 PackError, `pack: ` prefixed
    let root = world("pack-d5", &[("a", "../a")], &[("b", "../b")], &[]);
    write(
        &root.join("app"),
        "mod.rut",
        "use b::{x};\nentry fn main() -> i32 { return x(); }\n",
    );
    // bundle keys: the pack lane needs them
    write(
        &root.join("app"),
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 11, "name": "app", "deps": {"a": {"path": "../a"}}}"#,
    );
    write(
        &root.join("a"),
        "rut.jsonc",
        r#"{"name": "a", "deps": {"b": {"path": "../b"}}}"#,
    );
    let err = rut_native::pack_dir(&root.join("app")).unwrap_err().to_string();
    assert!(
        err.contains("cannot resolve `b`")
            && err.contains("is mounted in this program (pulled by `a`)")
            && err.contains("`app` does not declare it"),
        "{err}"
    );
}

// ---------------------------------------------------------------------
// the hand-mounted world — the structural manifest is the one mechanism
// ---------------------------------------------------------------------

const B_SRC: &str = "pub fn x() -> i32 { return 3; }\n";
const C_USES_B: &str = "use b::{x};\nentry fn main() -> i32 { return x(); }\n";

#[test]
fn hand_world_without_a_row_is_d5_host_clause() {
    // two hand offers, no manifest on the consumer: `b` is mounted
    // (offered by the host — no walk edge exists) and `c` never
    // declared it → D5's STRUCTURAL rendering
    let err = rut_driver::RutRun::new()
        .pkg(rut_driver::Pkg::source("b", B_SRC))
        .pkg(rut_driver::Pkg::source("c", C_USES_B))
        .entrypoint("c")
        .compile()
        .unwrap_err()
        .to_string();
    assert_eq!(
        err,
        "cannot resolve `b` — `b` is mounted in this program (offered by the host) but `c` does not declare it; declare it in the pkg's manifest — `Manifest::lib(\"c\", &[\"b\"])` or a `\"deps\"` row through `parse_manifest(..)`",
    );
}

#[test]
fn manifest_lib_fills_the_tables_structurally() {
    // the builder door: `Pkg::manifest(Manifest::lib(..))` stamps the
    // structural form — no json text anywhere — and the use is
    // declared
    let c = rut_driver::Pkg::source("c", C_USES_B)
        .manifest(rut_driver::Manifest::lib("c", &["b"]));
    assert!(c.manifest.deps.contains_key("b"), "the structural row rides");
    let compiled = rut_driver::RutRun::new()
        .pkg(rut_driver::Pkg::source("b", B_SRC))
        .pkg(c)
        .entrypoint("c")
        .compile()
        .expect("the declared hand world compiles");
    assert!(
        compiled.graph.diags.is_empty(),
        "{:?}",
        compiled.graph.diags
    );
}

#[test]
fn parse_manifest_stamps_the_same_law() {
    // the parsing door: `parse_manifest(embedded)` + `.manifest(..)` —
    // a real manifest text with a real descriptor — declares exactly
    // the same way
    let m = rut_driver::parse_manifest(r#"{"name": "c", "deps": {"b": {"path": "../b"}}}"#)
        .expect("the manifest parses");
    let compiled = rut_driver::RutRun::new()
        .pkg(rut_driver::Pkg::source("b", B_SRC))
        .pkg(rut_driver::Pkg::source("c", C_USES_B).manifest(m))
        .entrypoint("c")
        .compile()
        .expect("the parsed declaration declares");
    assert!(
        compiled.graph.diags.is_empty(),
        "{:?}",
        compiled.graph.diags
    );
}

#[test]
fn over_declaration_stays_legal() {
    // the law bounds uses by declarations, never declarations by
    // uses: a row naming a head the source never spells is inert —
    // declared-unused is legal (D4 guards the bad shape, deps ∩ dev-deps)
    let compiled = rut_driver::RutRun::new()
        .pkg(rut_driver::Pkg::source("b", B_SRC))
        .pkg(
            rut_driver::Pkg::source("c", "entry fn main() -> i32 { return 1; }\n")
                .manifest(rut_driver::Manifest::lib("c", &["b"])),
        )
        .entrypoint("c")
        .compile()
        .expect("declared-unused is legal");
    assert!(
        compiled.graph.diags.is_empty(),
        "{:?}",
        compiled.graph.diags
    );
}

// ---------------------------------------------------------------------
// core rides unconditionally
// ---------------------------------------------------------------------

#[test]
fn core_names_no_row() {
    // `use core::{ .. }` is legal with no declaration anywhere — the
    // graph rides the prelude into every unit, and the resolver passes
    // the head unconditionally
    let compiled = rut_driver::RutRun::new()
        .pkg(rut_driver::Pkg::source(
            "solo",
            "use core::{panic};\nentry fn main() -> i32 { return 1; }\n",
        ))
        .entrypoint("solo")
        .compile()
        .expect("core is free");
    assert!(
        compiled.graph.diags.is_empty(),
        "{:?}",
        compiled.graph.diags
    );
}
