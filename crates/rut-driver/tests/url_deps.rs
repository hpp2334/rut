//! url deps — `[deps]` rows naming a remote `.rutbundle` url, the
//! rut.lock pin, and the injected remote. The layer split under test:
//! the CALL SITE owns HOW bytes arrive (here: the shared ready-map
//! fixture — no network, no runtime dep), the LOADER owns WHAT they
//! are — the pin is lock law, read from `<root>/rut.lock` and
//! verified at the mount door on every load.
//!
//! Fixtures are real packed bundles (`pack_dir`, no net); the consumer
//! worlds mount them through the fetcher, and their `rut.lock` pins
//! the fixture urls (`common::write_lock`). Bundles stay CLOSED: no
//! test fetches at bundle load, and the inert-url-row case proves the
//! loader refuses rather than fetches.

use std::collections::BTreeMap;
use std::path::Path;

mod common;
use common::{block_on, dep_dir, write_lock, Table};

use rut_driver::sha256_hex;
use rut_native::{load_dir, pack_dir, pack_dir_zip};

// ------------------------------------------------------------------
// the fixture world: real dirs, real packed bundles

/// One source pkg over the auto core — the chain's graph. Closure and
/// shape refusals come back as one span-0 diagnostic.
#[allow(dead_code)] // not every suite in this file needs both lanes
fn compiled(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(
        rut_driver::RutRun::new()
            .mount(rut_driver::Mounted::source(spec, src))
            .entrypoint(spec)
            .compile(),
    )
}

/// [`compiled`] with calc offered: core auto-rides, `calc` is an
/// ordinary pkg).
#[allow(dead_code)]
fn compiled_std(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(
        rut_driver::RutRun::new()
            // the root DECLARES its calc use (the structural form; the
            // offer stays the walked tree pkg)
            .mount(
                rut_driver::Mounted::source(spec, src)
                    .manifest(rut_driver::Manifest::lib(spec, &["calc_host"])),
            )
            .mount(rut_native::tree_pkg("calc_host").expect("the toolchain tree's rut/calc_host"))
            .entrypoint(spec)
            .compile(),
    )
}

#[allow(dead_code)]
fn graph_of(c: Result<rut_driver::Compiled, rut_driver::RunError>) -> rut_driver::GraphOutput {
    match c {
        Ok(c) => c.graph,
        Err(e) => rut_driver::GraphOutput {
            diags: vec![rut_lexer::diag::Diag::new(
                rut_lexer::span::Span::new(0, 0),
                e.msg,
            )],
            program: None,
        },
    }
}

fn scratch(tag: &str) -> rut_testkit::Scratch {
    rut_testkit::Scratch::new(&format!("url-deps-{tag}"))
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, text).unwrap();
}

/// A temp `$RUT_HOME` for the test's lifetime — the store doors never
/// touch the real `~/.rut`. Mutating the process env here is safe
/// under nextest's one-process-per-test model (the suite's runner),
/// which is the only runner this repo uses.
struct Home(rut_testkit::Scratch);
impl Home {
    fn new(tag: &str) -> Home {
        let d = scratch(&format!("{tag}-home"));
        std::env::set_var("RUT_HOME", &*d);
        Home(d)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        std::env::remove_var("RUT_HOME");
    }
}

fn manifest(name: &str, _entry: &str, extra: &str) -> String {
    // `extra` is the fragment after the name row (`, "deps": { … }`) —
    // the entry keys are repealed: the body is `src/mod.rut` in the
    // package's `src/` shell (the `_entry` arg names the file the
    // caller writes)
    format!(r#"{{"name": "{name}"{extra}}}"#)
}

/// A plain linkable leaf pkg — packs to a one-group compiled bundle.
fn leaf_world(tag: &str, name: &str, body: &str) -> rut_testkit::Scratch {
    let root = scratch(tag);
    let dir = root.join(name);
    write(&dir, "rut.jsonc", &manifest(name, "mod.rut", ""));
    write(&dir, "src/mod.rut", body);
    root
}

const UTIL_BODY: &str = "pub fn twice(v: i64) -> i64 {\n    return v * 2;\n}\n";

/// A deep util tree: the root declares `pub mod layout;`, layout
/// declares `pub mod grid;` — two levels of declared children — and
/// the consumer walks `use util::layout::grid::{deep}` to the leaf.
const DEEP_ROOT: &str = "pub mod layout;\npub fn twice(v: i64) -> i64 {\n    return v * 2;\n}\n";
const DEEP_LAYOUT: &str = "pub mod grid;\npub fn span() -> i64 { return 5; }\n";
const DEEP_GRID: &str = "pub fn deep() -> i64 { return 6; }\n";
const DEEP_APP: &str = "use util::layout::grid::{deep};\nuse util::{twice};\nentry fn go() -> i64 {\n    return twice(21) + deep();\n}\n";

/// Rewrite one archive entry (the manifest, say) and re-seal the
/// container — the doctored-closure fixtures.
fn resealed(bytes: &[u8], key: &str, text: &str) -> Vec<u8> {
    let mut entries: Vec<(String, Vec<u8>)> = rut_driver::bundle::parse_bundle(bytes).unwrap();
    for (n, b) in entries.iter_mut() {
        if n == key {
            *b = text.as_bytes().to_vec();
        }
    }
    rut_driver::bundle::write_bundle(&entries).unwrap()
}

fn linked_binary(loaded: &rut_driver::Walk) -> Vec<u8> {
    let g = loaded
        .pkgs
        .iter()
        .fold(rut_driver::RutRun::new(), |r, p| r.mount(p.clone()))
        .entrypoint(&loaded.root)
        .compile()
        .expect("compile the walk");
    assert!(
        g.graph.diags.is_empty(),
        "{:?}",
        g.graph
            .diags
            .iter()
            .map(|d| d.msg.clone())
            .collect::<Vec<_>>()
            .join("\n")
    );
    rut_core::binary::encode(&g.graph.program.expect("linked program"))
}

/// Mount (calc offered) + compile + flatten + run one entry.
fn run_entry<R: rut_vm::interp::Ret>(loaded: rut_driver::Walk, entry: &str) -> R {
    let g = loaded
        .pkgs
        .iter()
        .fold(rut_driver::RutRun::new(), |r, p| r.mount(p.clone()))
        .mount(rut_native::tree_pkg("calc_host").expect("the toolchain tree's rut/calc_host"))
        .entrypoint(&loaded.root)
        .compile()
        .expect("compile the walk");
    assert!(
        g.graph.diags.is_empty(),
        "{:?}",
        g.graph
            .diags
            .iter()
            .map(|d| d.msg.clone())
            .collect::<Vec<_>>()
            .join("\n")
    );
    let flat = rut_core::link::flatten(g.graph.program.expect("linked program"));
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::builder()
        .program(std::rc::Rc::new(flat))
        .limits(limits)
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("vm");
    vm.call::<_, R>(entry, ()).expect("run")
}

// ------------------------------------------------------------------
// the tests

#[test]
fn url_dep_mounts_compiles_and_equals_the_dir_twin() {
    let root = leaf_world("happy", "util", UTIL_BODY);
    let bytes = pack_dir(&root.join("util")).expect("pack util");
    let url = "https://fixtures.test/util.rutbundle";

    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "mod.rut",
            &format!(r#", "deps": {{"util": {{"url": "{url}"}}}}"#),
        ),
    );
    write(
        &app,
        "src/mod.rut",
        "use util::{twice};\n\nentry fn go() -> i64 {\n    return twice(21);\n}\n",
    );

    let mut table = BTreeMap::new();
    table.insert(url.to_string(), bytes.clone());
    write_lock(&app, &table);
    let loaded = block_on(rut_native::load_dir_with(&app, &Table::from(table))).expect("url load");
    assert_eq!(loaded.root, "app");
    // the url dep is a leaf: mounted, not walked — its body is the
    // bundle's compiled root
    assert!(matches!(
        loaded.pkg("util").unwrap().body,
        rut_driver::MountedBody::Compiled(_)
    ));
    assert_eq!(run_entry::<i64>(loaded, "go"), 42);

    // the pinned equivalence: the url lane and the path lane link to
    // the identical binary
    let twin = root.join("app_path");
    write(
        &twin,
        "rut.jsonc",
        &manifest(
            "app",
            "mod.rut",
            r#", "deps": {"util": {"path": "../util"}}"#,
        ),
    );
    write(
        &twin,
        "src/mod.rut",
        "use util::{twice};\n\nentry fn go() -> i64 {\n    return twice(21);\n}\n",
    );
    let dir_loaded = load_dir(&twin).expect("dir load");
    assert_eq!(
        linked_binary(&dir_loaded),
        {
            let mut table = BTreeMap::new();
            table.insert(url.to_string(), bytes);
            let s = block_on(rut_native::load_dir_with(&app, &Table::from(table))).unwrap();
            linked_binary(&s)
        },
        "dir and url lanes compile identically"
    );
}

#[test]
fn the_manifest_pin_retires_loudly_even_with_a_fetcher() {
    // the pins moved to rut.lock: the manifest url row lost its
    // `sha256`, and the retirement is LOUD — the key's existence is
    // the crime (the pin here is value-shaped RIGHT), never a silent
    // ignore that would surface as an unpinned fetch later. The pin
    // law itself (mismatch naming both hashes) re-points at rut.lock
    // when the walk learns to read it.
    let root = leaf_world("pin", "util", UTIL_BODY);
    let bytes = pack_dir(&root.join("util")).expect("pack util");
    let url = "https://fixtures.test/util.rutbundle";
    let pin = sha256_hex(&bytes);

    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "mod.rut",
            &format!(r#", "deps": {{"util": {{"url": "{url}", "sha256": "{pin}"}}}}"#),
        ),
    );
    write(
        &app,
        "src/mod.rut",
        "entry fn go() -> i64 {\n    return 1;\n}\n",
    );

    let mut table = BTreeMap::new();
    table.insert(url.to_string(), bytes.clone());
    let err = block_on(rut_native::load_dir_with(&app, &Table::from(table.clone())))
        .unwrap_err()
        .to_string();
    assert!(err.contains("deps.util: 'sha256' retired"), "{err}");
    assert!(err.contains("rut.lock"), "{err}");

    // the same refusal on the vendored-map lane: the parse is the
    // door, not the fetcher
    let err = rut_native::load_dir_fetched(&app, &table)
        .unwrap_err()
        .to_string();
    assert!(err.contains("'sha256' retired"), "{err}");
}

#[test]
fn name_vs_key_refuses() {
    let root = leaf_world("namekey", "util", UTIL_BODY);
    let bytes = pack_dir(&root.join("util")).expect("pack util");
    let url = "https://fixtures.test/util.rutbundle";

    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "mod.rut",
            &format!(r#", "deps": {{"not_util": {{"url": "{url}"}}}}"#),
        ),
    );
    write(
        &app,
        "src/mod.rut",
        "entry fn go() -> i64 {\n    return 1;\n}\n",
    );

    let mut table = BTreeMap::new();
    table.insert(url.to_string(), bytes);
    write_lock(&app, &table);
    let err = block_on(rut_native::load_dir_with(&app, &Table::from(table)))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains(&format!("dep `not_util` points at {url}")),
        "{err}"
    );
    assert!(err.contains("the bundle names itself `util`"), "{err}");
}

#[test]
fn bundle_stays_closed_no_fetch_at_bundle_load() {
    // a url row inside an already-packed bundle is inert metadata: the
    // closure law demands a group, the loader refuses — it NEVER
    // fetches at bundle load (load_bundle_bytes owns no fetcher at all)
    let root = leaf_world("closed", "util", UTIL_BODY);
    let bytes = pack_dir(&root.join("util")).expect("pack util");
    // the resealed manifest rides INSIDE the bundle — it carries the
    // wire pair the pack door stamps (the version gate reads it)
    let doctored = resealed(
        &bytes,
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 13, "name": "util", "deps": {"ghost": {"url": "https://fixtures.test/ghost.rutbundle"}}}"#,
    );

    // the direct bundle lane
    let err = rut_driver::Bundle::parse(&doctored)
        .and_then(|b| b.walk())
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("the bundle is missing its `ghost` dependency group"),
        "{err}"
    );

    // the same law through the url mount door (gate 7): the consumer's
    // fetcher only knows `util`'s bytes — a fetch for `ghost` would be
    // the loudest possible failure, and the refusal names the group
    let url = "https://fixtures.test/util.rutbundle";
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "mod.rut",
            &format!(r#", "deps": {{"util": {{"url": "{url}"}}}}"#),
        ),
    );
    write(
        &app,
        "src/mod.rut",
        "entry fn go() -> i64 {\n    return 1;\n}\n",
    );
    let mut table = BTreeMap::new();
    table.insert(url.to_string(), doctored);
    write_lock(&app, &table);
    let err = block_on(rut_native::load_dir_with(&app, &Table::from(table)))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("the bundle is missing its `ghost` dependency group"),
        "{err}"
    );
}

#[test]
fn embedder_mount_outranks_the_url_dep() {
    // first-mount-wins: the fetch may satisfy the row, but the MOUNT
    // never overwrites the embedder's util — the walk skips a name
    // already in the session
    let root = leaf_world("firstwin", "util", UTIL_BODY);
    let bytes = pack_dir(&root.join("util")).expect("pack util");
    let url = "https://fixtures.test/util.rutbundle";
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "mod.rut",
            &format!(r#", "deps": {{"util": {{"url": "{url}"}}}}"#),
        ),
    );
    write(
        &app,
        "src/mod.rut",
        "entry fn go() -> i64 {\n    return 1;\n}\n",
    );

    let mut chain = rut_driver::RutRun::new()
        // the embedder's own mount, offered first
        .mount(rut_driver::Mounted::source("util", "// the embedder's"));
    let mut table = BTreeMap::new();
    table.insert(url.to_string(), bytes);
    write_lock(&app, &table);
    let walked = block_on(rut_native::dir_pkgs_with(&app, &Table::from(table))).expect("mount");
    assert_eq!(walked.root, "app");
    chain = walked.pkgs.iter().fold(chain, |r, p| r.mount(p.clone()));
    // the compile closes the world; the closure check would name a
    // wrong winner, so just compile the offered world's root
    let g = chain
        .entrypoint(&walked.root)
        .compile()
        .expect("compile the offered world");
    assert!(g.graph.program.is_some(), "{:?}", g.graph.diags);
}

#[test]
fn embedder_offer_mounts_the_store_unpack_tree() {
    // the offer lane's SOURCE transport: `dir_pkgs_with` mounts a
    // source-zip url dep through the STORE's unpack tree — the same
    // address `rut install` publishes (`<deps>/<spec>@<pin16>/unpack/`).
    // The project dir gains NO `.rut`; the mounted pkg's text is the
    // STORE tree's bytes (the marker comment below rides only there).
    let root = leaf_world("zipstore", "util", UTIL_BODY);
    let zip = pack_dir_zip(&root.join("util")).expect("pack util as a source zip");
    let url = "https://fixtures.test/util.zip";
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "mod.rut",
            &format!(r#", "deps": {{"util": {{"url": "{url}"}}}}"#),
        ),
    );
    write(
        &app,
        "src/mod.rut",
        "use util::{twice};\nentry fn go() -> i64 {\n    return twice(21);\n}\n",
    );

    // the store entry: the walk's ONE derivation law lays it where
    // install would — the pin is the bytes' sha256, the tree carries
    // the marker the fixture zip never had
    let home = Home::new("zipstore");
    let pin = sha256_hex(&zip);
    let unpack = home
        .path()
        .join("deps")
        .join(rut_driver::store_dir("util", &pin))
        .join(rut_driver::UNPACK_DIR);
    std::fs::create_dir_all(unpack.join("src")).unwrap();
    std::fs::write(unpack.join("rut.jsonc"), manifest("util", "mod.rut", "")).unwrap();
    std::fs::write(
        unpack.join("src/mod.rut"),
        format!("// mounted from the store\n{UTIL_BODY}"),
    )
    .unwrap();

    let mut table = BTreeMap::new();
    table.insert(url.to_string(), zip.clone());
    write_lock(&app, &table);
    let walked =
        block_on(rut_native::dir_pkgs_with(&app, &Table::from(table))).expect("mount the zip dep");
    assert_eq!(walked.root, "app");
    let util = walked
        .pkgs
        .iter()
        .find(|p| p.spec == "util")
        .expect("the zip dep mounted");
    match &util.body {
        rut_driver::MountedBody::Source { text, .. } => assert!(
            text.contains("// mounted from the store"),
            "the pkg mounts the STORE tree, not the fixture bytes: {text}"
        ),
        b => panic!("the store mount is a source pkg, got {b:?}"),
    }
    // the project law: no `.rut` beside the project — not even the
    // legacy plant's shape
    assert!(
        !app.join(".rut").exists(),
        "the project dir stays rut.jsonc + rut.lock + src/"
    );
    let answer: i64 = run_entry(walked, "go");
    assert_eq!(answer, 42);
    assert!(
        unpack.exists(),
        "the store entry outlives the run untouched — the walk never deletes"
    );
}

#[test]
fn the_temp_fallback_unpacks_when_the_store_tree_is_absent() {
    // the store entry's `unpack/` missing while the artifact vouches
    // (partial eviction, hand deletion): the walk unpacks ONCE into a
    // run-scoped temp dir and mounts that — the mount still succeeds,
    // the project gains NO `.rut`, and the temp tree dies with the
    // walk (zero traces).
    let root = leaf_world("ziptmp", "util", UTIL_BODY);
    let zip = pack_dir_zip(&root.join("util")).expect("pack util as a source zip");
    let url = "https://fixtures.test/util.zip";
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "mod.rut",
            &format!(r#", "deps": {{"util": {{"url": "{url}"}}}}"#),
        ),
    );
    write(
        &app,
        "src/mod.rut",
        "use util::{twice};\nentry fn go() -> i64 {\n    return twice(21);\n}\n",
    );

    let _home = Home::new("ziptmp"); // a COLD store — no entry for util
    let mut table = BTreeMap::new();
    table.insert(url.to_string(), zip.clone());
    write_lock(&app, &table);
    let walked =
        block_on(rut_native::dir_pkgs_with(&app, &Table::from(table))).expect("mount the zip dep");
    assert_eq!(walked.root, "app");
    assert!(
        walked.pkgs.iter().any(|p| p.spec == "util"),
        "the fallback mount succeeds"
    );
    let answer: i64 = run_entry(walked, "go");
    assert_eq!(answer, 42);
    assert!(
        !app.join(".rut").exists(),
        "the fallback never writes the project"
    );
    let leftovers: Vec<String> = std::fs::read_dir(rut_testkit::temp_root())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(&format!("rut-unpack-{}-", std::process::id())))
        .collect();
    assert!(
        leftovers.is_empty(),
        "the run-scoped temp dirs die with the walk: {leftovers:?}"
    );
}

#[test]
fn a_gutted_store_tree_falls_back_to_the_temp_unpack() {
    // the store entry's `unpack/` carries the manifest but the entry
    // module's files are GUTTED (hand deletion, partial eviction):
    // mounting it would die at the entry mount — and the state is
    // unhealable (`store_hit` answers from the artifact bytes, the
    // idempotent publish skips a present dir). The gate probes the
    // entry files and takes the temp fallback instead: the validated
    // bytes unpack, the dep loads and runs, the project stays clean.
    //
    // (The reviewer's one-url-two-specs shape needs no pin: it is
    // UNREACHABLE — `[deps]` keys are bare names at parse
    // (`descriptor.rs`), and a second name that disagrees with the
    // zip's own pkg refuses at the name-vs-key gate
    // (`name_vs_key_refuses` pins the refusal). One url mounts under
    // exactly one name — the zip's own — so the address always
    // matches the mount site.)
    let root = leaf_world("zipgut", "util", UTIL_BODY);
    let zip = pack_dir_zip(&root.join("util")).expect("pack util as a source zip");
    let url = "https://fixtures.test/util.zip";
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "mod.rut",
            &format!(r#", "deps": {{"util": {{"url": "{url}"}}}}"#),
        ),
    );
    write(
        &app,
        "src/mod.rut",
        "use util::{twice};\nentry fn go() -> i64 {\n    return twice(21);\n}\n",
    );

    // the store entry: the manifest rides, `src/` does not
    let home = Home::new("zipgut");
    let pin = sha256_hex(&zip);
    let unpack = home
        .path()
        .join("deps")
        .join(rut_driver::store_dir("util", &pin))
        .join(rut_driver::UNPACK_DIR);
    std::fs::create_dir_all(&unpack).unwrap();
    std::fs::write(unpack.join("rut.jsonc"), manifest("util", "mod.rut", "")).unwrap();

    let mut table = BTreeMap::new();
    table.insert(url.to_string(), zip.clone());
    write_lock(&app, &table);
    let walked = block_on(rut_native::dir_pkgs_with(&app, &Table::from(table)))
        .expect("the gutted store tree falls back, the dep mounts");
    assert_eq!(walked.root, "app");
    let util = walked
        .pkgs
        .iter()
        .find(|p| p.spec == "util")
        .expect("the dep mounted");
    match &util.body {
        rut_driver::MountedBody::Source { text, .. } => assert_eq!(
            text, UTIL_BODY,
            "the mount is the validated bytes' tree, not the gutted store tree"
        ),
        b => panic!("the fallback mount is a source pkg, got {b:?}"),
    }
    let answer: i64 = run_entry(walked, "go");
    assert_eq!(answer, 42);
    assert!(
        !app.join(".rut").exists(),
        "the fallback never writes the project"
    );
    // and the gutted entry stays as-it-was — the walk is a pure store
    // reader (install's own lanes own any repair)
    assert!(unpack.join("rut.jsonc").is_file());
    assert!(!unpack.join("src").exists());
}

#[test]
fn a_gutted_store_tree_falls_back_in_the_pack_lane_too() {
    // the compiled-bundle pack lane consumes the world (and with it
    // the fallback's temp guards) BEFORE the emission re-reads the
    // unpacked trees through its `read` closure — the guards ride the
    // lane's scope instead, so the gutted-store fallback packs clean
    // (the crash this pins: `cannot read /tmp/rut-unpack-*/rut.jsonc`
    // from the emission's re-read).
    let root = leaf_world("zipgutpack", "util", UTIL_BODY);
    let zip = pack_dir_zip(&root.join("util")).expect("pack util as a source zip");
    let url = "https://fixtures.test/util.zip";
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "mod.rut",
            &format!(r#", "deps": {{"util": {{"url": "{url}"}}}}"#),
        ),
    );
    write(
        &app,
        "src/mod.rut",
        "use util::{twice};\nentry fn go() -> i64 {\n    return twice(21);\n}\n",
    );

    let _home = Home::new("zipgutpack"); // manifest rides, src/ gutted
    let pin = sha256_hex(&zip);
    let unpack = _home
        .path()
        .join("deps")
        .join(rut_driver::store_dir("util", &pin))
        .join(rut_driver::UNPACK_DIR);
    std::fs::create_dir_all(&unpack).unwrap();
    std::fs::write(unpack.join("rut.jsonc"), manifest("util", "mod.rut", "")).unwrap();

    let mut table = BTreeMap::new();
    table.insert(url.to_string(), zip.clone());
    write_lock(&app, &table);
    let a = block_on(rut_native::pack_dir_opts_with(
        &app,
        &rut_driver::PackOpts::default(),
        &Table::from(table.clone()),
    ))
    .expect("the fallback packs (guards outlive the emission)");
    let b = block_on(rut_native::pack_dir_opts_with(
        &app,
        &rut_driver::PackOpts::default(),
        &Table::from(table),
    ))
    .expect("deterministic");
    assert_eq!(a.0, b.0, "the packed bytes are deterministic");
    assert!(
        !app.join(".rut").exists(),
        "the pack never writes the project"
    );
}

#[test]
fn a_gutted_mod_child_falls_back_to_the_temp_unpack() {
    // the round-1 shape one level deeper: the store entry's root and
    // level-1 files ride, but a declared mod grandchild's module file
    // is GUTTED (the `grid/` dir present, `grid/mod.rut` deleted).
    // The entry-only probe chose the store tree and the mount died at
    // the child lookup — a state unhealable by install. The deepened
    // probe walks the declared mod edges, finds the missing
    // descendant, and takes the temp fallback: the validated bytes
    // unpack once, the dep loads and runs at full depth, the project
    // stays clean, and the store entry is left as found.
    let root = scratch("zipdeepgut");
    let util = root.join("util");
    write(&util, "rut.jsonc", &manifest("util", "mod.rut", ""));
    write(&util, "src/mod.rut", DEEP_ROOT);
    write(&util, "src/layout/mod.rut", DEEP_LAYOUT);
    write(&util, "src/layout/grid/mod.rut", DEEP_GRID);
    let zip = pack_dir_zip(&util).expect("pack util as a source zip");
    let url = "https://fixtures.test/util.zip";
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "mod.rut",
            &format!(r#", "deps": {{"util": {{"url": "{url}"}}}}"#),
        ),
    );
    write(&app, "src/mod.rut", DEEP_APP);

    // the store entry: manifest + root + layout ride; the grid dir is
    // there, its module file is not
    let home = Home::new("zipdeepgut");
    let pin = sha256_hex(&zip);
    let unpack = home
        .path()
        .join("deps")
        .join(rut_driver::store_dir("util", &pin))
        .join(rut_driver::UNPACK_DIR);
    std::fs::create_dir_all(unpack.join("src/layout/grid")).unwrap();
    std::fs::write(unpack.join("rut.jsonc"), manifest("util", "mod.rut", "")).unwrap();
    std::fs::write(unpack.join("src/mod.rut"), DEEP_ROOT).unwrap();
    std::fs::write(unpack.join("src/layout/mod.rut"), DEEP_LAYOUT).unwrap();

    let mut table = BTreeMap::new();
    table.insert(url.to_string(), zip);
    write_lock(&app, &table);
    let walked = block_on(rut_native::dir_pkgs_with(&app, &Table::from(table)))
        .expect("the gutted grandchild falls back, the dep mounts");
    assert_eq!(walked.root, "app");
    let util = walked
        .pkgs
        .iter()
        .find(|p| p.spec == "util")
        .expect("the dep mounted");
    // the mount is the validated bytes' tree at FULL depth — the
    // declared edges resolved, from the fallback, not the gutted store
    assert!(
        util.mods.contains_key("layout") && util.mods.contains_key("layout/grid"),
        "the fallback mounts the whole tree: {:?}",
        util.mods.keys().collect::<Vec<_>>()
    );
    let answer: i64 = run_entry(walked, "go");
    assert_eq!(answer, 48);
    assert!(
        !app.join(".rut").exists(),
        "the fallback never writes the project"
    );
    // the store entry stays as found — the walk is a pure store
    // reader (install's own lanes own any repair)
    assert!(unpack.join("rut.jsonc").is_file());
    assert!(unpack.join("src/mod.rut").is_file());
    assert!(unpack.join("src/layout/mod.rut").is_file());
    assert!(unpack.join("src/layout/grid").is_dir());
    assert!(
        !unpack.join("src/layout/grid/mod.rut").exists(),
        "the walk never repairs the store tree"
    );
    // and the run-scoped temp dir died with the walk (zero traces)
    let leftovers: Vec<String> = std::fs::read_dir(rut_testkit::temp_root())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(&format!("rut-unpack-{}-", std::process::id())))
        .collect();
    assert!(
        leftovers.is_empty(),
        "the run-scoped temp dirs die with the walk: {leftovers:?}"
    );
}

#[test]
fn a_whole_deep_store_tree_mounts_from_the_store() {
    // the deepened probe's positive pin: a fully-present deep tree
    // (two levels of declared children) probes COMPLETE — the store
    // tree mounts, no needless temp unpack. The mounted pkg's files
    // read from the STORE path: the store copies carry a marker
    // comment the zip's bytes never had, root and grandchild both.
    let root = scratch("zipdeepstore");
    let util = root.join("util");
    write(&util, "rut.jsonc", &manifest("util", "mod.rut", ""));
    write(&util, "src/mod.rut", DEEP_ROOT);
    write(&util, "src/layout/mod.rut", DEEP_LAYOUT);
    write(&util, "src/layout/grid/mod.rut", DEEP_GRID);
    let zip = pack_dir_zip(&util).expect("pack util as a source zip");
    let url = "https://fixtures.test/util.zip";
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "mod.rut",
            &format!(r#", "deps": {{"util": {{"url": "{url}"}}}}"#),
        ),
    );
    write(&app, "src/mod.rut", DEEP_APP);

    // the store entry: the whole tree laid out, the store's own
    // copies marked where it matters (root + the deep grandchild)
    let home = Home::new("zipdeepstore");
    let pin = sha256_hex(&zip);
    let unpack = home
        .path()
        .join("deps")
        .join(rut_driver::store_dir("util", &pin))
        .join(rut_driver::UNPACK_DIR);
    std::fs::create_dir_all(unpack.join("src/layout/grid")).unwrap();
    std::fs::write(unpack.join("rut.jsonc"), manifest("util", "mod.rut", "")).unwrap();
    std::fs::write(
        unpack.join("src/mod.rut"),
        format!("// mounted from the store\n{DEEP_ROOT}"),
    )
    .unwrap();
    std::fs::write(unpack.join("src/layout/mod.rut"), DEEP_LAYOUT).unwrap();
    std::fs::write(
        unpack.join("src/layout/grid/mod.rut"),
        format!("// mounted from the store\n{DEEP_GRID}"),
    )
    .unwrap();

    let mut table = BTreeMap::new();
    table.insert(url.to_string(), zip);
    write_lock(&app, &table);
    let walked = block_on(rut_native::dir_pkgs_with(&app, &Table::from(table)))
        .expect("the whole deep tree mounts from the store");
    assert_eq!(walked.root, "app");
    let util = walked
        .pkgs
        .iter()
        .find(|p| p.spec == "util")
        .expect("the dep mounted");
    match &util.body {
        rut_driver::MountedBody::Source { text, .. } => assert!(
            text.starts_with("// mounted from the store"),
            "the pkg mounts the STORE tree, not the fixture bytes: {text}"
        ),
        b => panic!("the store mount is a source pkg, got {b:?}"),
    }
    let grid = util
        .mods
        .get("layout/grid")
        .expect("the grandchild mounted");
    assert!(
        grid.text.starts_with("// mounted from the store"),
        "the deep files read from the store path: {}",
        grid.text
    );
    // no temp fallback ran — the store tree probed complete
    let leftovers: Vec<String> = std::fs::read_dir(rut_testkit::temp_root())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(&format!("rut-unpack-{}-", std::process::id())))
        .collect();
    assert!(
        leftovers.is_empty(),
        "the probe spared the temp unpack: {leftovers:?}"
    );
    let answer: i64 = run_entry(walked, "go");
    assert_eq!(answer, 48);
    assert!(
        !app.join(".rut").exists(),
        "the project dir stays rut.jsonc + rut.lock + src/"
    );
    assert!(
        unpack.exists(),
        "the store entry outlives the run untouched — the walk never deletes"
    );
}

/// The archive source-group world: `codec` (interface + entries,
/// dev-deps for its own build) packed INSIDE `zeta` as a declared-but-
/// unused dep, so it rides the archive as a SOURCE group — the consumer
/// sees codec as an ARCHIVE SOURCE GROUP.
fn codec_world(tag: &str) -> (rut_testkit::Scratch, Vec<u8>, String) {
    let root = scratch(tag);
    write(
        &root.join("pouch"),
        "rut.jsonc",
        &manifest("pouch", "mod.rut", ""),
    );
    write(
        &root.join("pouch"),
        "src/mod.rut",
        "pub class Vec<T> {\n    items: [T];\n}\n\nimpl<T> Vec<T> {\n    pub fn filled(v: T, n: i32) -> Self {\n        let items: [T] = [v; n];\n        return Self { items: items };\n    }\n\n    pub fn first(self) -> T {\n        return self.items[0];\n    }\n}\n",
    );
    // codec — the json shape: the interface and public names in the
    // base, dev-deps for its own build
    write(
        &root.join("codec"),
        "rut.jsonc",
        &manifest(
            "codec",
            "mod.rut",
            r#", "dev-deps": {"pouch": {"path": "../pouch"}}"#,
        ),
    );
    write(
        &root.join("codec"),
        "src/mod.rut",
        "pub interface Coded {\n    fn coded(self) -> str;\n}\n\npub class CodedStr(str);\n\nimpl CodedStr {\n    pub fn coded(self) -> str {\n        return self.inner;\n    }\n}\n\npub fn encode(s: str) -> str {\n    return CodedStr(s).coded();\n}\n",
    );
    // zeta — packs codec as a declared-but-unused (source) group
    write(
        &root.join("zeta"),
        "rut.jsonc",
        &manifest(
            "zeta",
            "mod.rut",
            r#", "deps": {"codec": {"path": "../codec"}}"#,
        ),
    );
    write(
        &root.join("zeta"),
        "src/mod.rut",
        "pub fn ping() -> i32 {\n    return 7;\n}\n",
    );
    let bytes = pack_dir(&root.join("zeta")).expect("pack zeta");
    let url = "https://fixtures.test/zeta.rutbundle";
    (root, bytes, url.to_string())
}

#[test]
fn colliding_pack_numberings_namespace_per_archive() {
    // two independently packed bundles whose ledgers assign the SAME
    // scope number to DIFFERENT specs — the normal CDN case (every
    // per-package bundle numbers its own closure) — share one session:
    // each archive's rows shift into a fresh range and its binaries
    // rebase with that map, so neither argues about a number. The
    // consumer compiles over both, each dep resolving through its OWN
    // archive's rows.
    let a = leaf_world("leda", "util_a", UTIL_BODY);
    let b = leaf_world(
        "ledb",
        "util_b",
        "pub fn thrice(v: i64) -> i64 {\n    return v * 3;\n}\n",
    );
    let bytes_a = pack_dir(&a.join("util_a")).expect("pack a");
    let bytes_b = pack_dir(&b.join("util_b")).expect("pack b");

    let app = a.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "mod.rut",
            r#", "deps": {"util_a": {"url": "https://fixtures.test/a.rutbundle"}, "util_b": {"url": "https://fixtures.test/b.rutbundle"}}"#,
        ),
    );
    write(
        &app,
        "src/mod.rut",
        "use util_a::{twice};\nuse util_b::{thrice};\n\nentry fn go() -> i64 {\n    return twice(1) + thrice(2);\n}\n",
    );

    let mut table = BTreeMap::new();
    table.insert("https://fixtures.test/a.rutbundle".to_string(), bytes_a);
    table.insert("https://fixtures.test/b.rutbundle".to_string(), bytes_b);
    write_lock(&app, &table);
    let session = block_on(rut_native::load_dir_with(&app, &Table::from(table)))
        .expect("colliding numberings namespace per archive");
    assert!(matches!(
        session.pkg("util_a").unwrap().body,
        rut_driver::MountedBody::Compiled(_)
    ));
    assert!(matches!(
        session.pkg("util_b").unwrap().body,
        rut_driver::MountedBody::Compiled(_)
    ));
    // and the value semantics survive both rebases: 2*1 + 3*2 = 8
    assert_eq!(run_entry::<i64>(session, "go"), 8);
}

#[test]
fn sync_wrapper_refuses_url_dep_loudly() {
    // the old sync names keep their signatures: no fetcher, no future
    // — a url row is the loud error naming the FIX
    let root = leaf_world("nofetch", "util", UTIL_BODY);
    let url = "https://fixtures.test/util.rutbundle";
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "mod.rut",
            &format!(r#", "deps": {{"util": {{"url": "{url}"}}}}"#),
        ),
    );
    write(
        &app,
        "src/mod.rut",
        "entry fn go() -> i64 {\n    return 1;\n}\n",
    );

    let err = load_dir(&app).unwrap_err().to_string();
    assert!(err.contains("this loader has no fetcher"), "{err}");
    assert!(err.contains("`load_dir_with`"), "{err}");
    assert!(err.contains("vendor the dep"), "{err}");
    assert!(err.contains(url), "{err}");
}

#[test]
fn pack_url_dep_rode_along_and_deterministic() {
    let root = leaf_world("pack", "util", UTIL_BODY);
    let bytes = pack_dir(&root.join("util")).expect("pack util");
    let url = "https://fixtures.test/util.rutbundle";

    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "mod.rut",
            &format!(r#", "deps": {{"util": {{"url": "{url}"}}}}"#),
        ),
    );
    write(
        &app,
        "src/mod.rut",
        "use util::{twice};\n\nentry fn go() -> i64 {\n    return twice(21);\n}\n",
    );

    let mut map = BTreeMap::new();
    map.insert(url.to_string(), bytes);
    write_lock(&app, &map);
    let packed = rut_native::pack_dir_fetched(&app, &map).expect("pack");
    // determinism: the same world ⇒ the same bytes
    let packed_again = rut_native::pack_dir_fetched(&app, &map).expect("pack again");
    assert_eq!(
        packed, packed_again,
        "same inputs must pack byte-identically"
    );

    // the rode-along group: the output carries util as a compiled group
    // (re-encoded from THIS session's units, under its content
    // identity), and the url row rode byte-for-byte inside its manifest
    // (pinless — pins live in rut.lock, not in the manifest)
    let names: Vec<String> = rut_driver::bundle::parse_bundle(&packed)
        .unwrap()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert!(
        names.contains(&format!("{}/mod.rutc", dep_dir(&names, "util"))),
        "{names:?}"
    );
    assert!(names.contains(&"mod.rutc".to_string()), "{names:?}");
    let out_manifest = rut_driver::bundle::Bundle::parse(&packed)
        .unwrap()
        .read("rut.jsonc")
        .unwrap();
    assert!(
        out_manifest.contains(&format!(r#""url": "{url}""#)),
        "{out_manifest}"
    );
    assert!(!out_manifest.contains("sha256"), "{out_manifest}");

    // the packed artifact runs, straight from the output bytes
    let session = rut_driver::Bundle::parse(&packed)
        .and_then(|b| b.walk())
        .expect("load");
    assert_eq!(session.root, "app");
    assert_eq!(run_entry::<i64>(session, "go"), 42);

    // the async lane settles identically through the noop-waker driver
    let via_with = block_on(rut_native::pack_dir_opts_with(
        &app,
        &rut_driver::PackOpts::default(),
        &Table::from(map),
    ))
    .expect("pack_with")
    .0;
    assert_eq!(packed, via_with);
}

#[test]
fn pack_refuses_archive_source_group_with_dev_deps() {
    // v1 refusal: an archive-backed SOURCE group whose manifest still
    // declares [dev-deps] dirs — those paths never crossed the pack
    // boundary; the group's own-build closure cannot be reproduced
    //
    // ORDERING NOTE (the use-law enforcement phase): this app spells
    // `use codec::{encode}` with NO codec row — undeclared today (the
    // transitive zeta→codec edge), and the pack-side refusal fires
    // before any use-law diagnostic exists. When the walked law's D5
    // diagnostic lands, its refusal may fire FIRST here; the test's
    // subject is the PACK-side dev-deps refusal — if the use-law
    // precedes it, give `app` the `codec` path row (../codec, the
    // fixture's own layout) and keep this test's subject intact.
    let (root, bytes, url) = codec_world("devrefuse");
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "mod.rut",
            &format!(
                r#", "deps": {{"pouch": {{"path": "../pouch"}}, "zeta": {{"url": "{url}"}}}}"#
            ),
        ),
    );
    write(
        &app,
        "src/mod.rut",
        "use codec::{encode};\n\nentry fn go() -> str {\n    return encode(\"x\");\n}\n",
    );

    let mut map = BTreeMap::new();
    map.insert(url, bytes);
    write_lock(&app, &map);
    let err = rut_native::pack_dir_fetched(&app, &map)
        .unwrap_err()
        .to_string();
    // the dep mounts under its content identity — the message spells
    // the mount key
    assert!(
        err.contains("packed dep `codec@")
            && err.contains("` needs its own [dev-deps] directories"),
        "{err}"
    );
    assert!(err.contains("vendor this dep"), "{err}");
}

#[test]
fn pack_refuses_unused_compiled_url_dep() {
    // v1 refusal: a declared-but-unused url dep — its compiled root
    // cannot ride (the output's ledger would not carry its pack-time
    // scopes), and the archive keeps no source to re-emit
    let root = leaf_world("unused", "util", UTIL_BODY);
    let bytes = pack_dir(&root.join("util")).expect("pack util");
    let url = "https://fixtures.test/util.rutbundle";
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "mod.rut",
            &format!(r#", "deps": {{"util": {{"url": "{url}"}}}}"#),
        ),
    );
    write(
        &app,
        "src/mod.rut",
        "entry fn go() -> i64 {\n    return 1;\n}\n",
    );

    let mut map = BTreeMap::new();
    map.insert(url.to_string(), bytes);
    write_lock(&app, &map);
    let err = rut_native::pack_dir_fetched(&app, &map)
        .unwrap_err()
        .to_string();
    assert!(err.contains("compiled group `util`"), "{err}");
    assert!(err.contains("cannot ride unpackaged"), "{err}");
    assert!(err.contains("vendor this dep"), "{err}");
}

// ------------------------------------------------------------------
// the lock law: rut.lock is the pin's only source (phase 2)

#[test]
fn an_unpinned_url_row_refuses_with_the_install_recipe() {
    // the lock deleted / never committed: the `_with` lane refuses
    // BEFORE any fetch — the prefetch runs against an EMPTY table, so
    // a fetch attempt would answer "no fixture bytes"; the refusal is
    // the lock's, not the wire's
    let root = leaf_world("unpinned", "util", UTIL_BODY);
    let bytes = pack_dir(&root.join("util")).expect("pack util");
    let url = "https://fixtures.test/util.rutbundle";
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "mod.rut",
            &format!(r#", "deps": {{"util": {{"url": "{url}"}}}}"#),
        ),
    );
    write(
        &app,
        "src/mod.rut",
        "entry fn go() -> i64 {\n    return 1;\n}\n",
    );

    let err = block_on(rut_native::load_dir_with(&app, &Table::new()))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains(&format!(
            "dep `util` ({url}) is not pinned — rut.lock has no row for it: run `rut install`"
        )),
        "{err}"
    );

    // the vendored-map lane: the bytes are in hand, but the lock has
    // no row — the mount door refuses (the no-fetcher lane with an
    // empty map keeps ITS law: no bytes, no load, pin or no pin)
    let mut map = BTreeMap::new();
    map.insert(url.to_string(), bytes);
    let err = rut_native::load_dir_fetched(&app, &map)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("is not pinned — rut.lock has no row for it"),
        "{err}"
    );
    assert!(err.contains("run `rut install`"), "{err}");
}

#[test]
fn a_lock_pin_mismatch_names_both_hashes() {
    // the poisoned-cache law, lock-sourced: the "sha256 pin mismatch
    // for " phrase survives (the CLI's eviction matches on it), the
    // pin's source phrase is rut.lock's now
    let root = leaf_world("lockpin", "util", UTIL_BODY);
    let bytes = pack_dir(&root.join("util")).expect("pack util");
    let url = "https://fixtures.test/util.rutbundle";
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "mod.rut",
            &format!(r#", "deps": {{"util": {{"url": "{url}"}}}}"#),
        ),
    );
    write(
        &app,
        "src/mod.rut",
        "entry fn go() -> i64 {\n    return 1;\n}\n",
    );

    let mut table = BTreeMap::new();
    table.insert(url.to_string(), bytes);
    let stale = "2f6e6a20c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e";
    write(
        &app,
        "rut.lock",
        &rut_driver::render_lock(&rut_driver::LockFile::from_rows(vec![
            rut_driver::LockRow {
                pkg: "util".into(),
                url: url.to_string(),
                sha256: stale.into(),
            },
        ])),
    );

    let err = rut_native::load_dir_fetched(&app, &table)
        .unwrap_err()
        .to_string();
    assert!(err.contains("sha256 pin mismatch for "), "{err}");
    assert!(
        err.contains(&format!("dep `util` — sha256 pin mismatch for {url}")),
        "{err}"
    );
    assert!(err.contains(&format!("rut.lock pins {stale}")), "{err}");
    let got = rut_driver::sha256_hex(table.get(url).unwrap());
    assert!(
        err.contains(&format!("the fetched bytes hash to {got}")),
        "{err}"
    );
}

#[test]
fn a_malformed_lock_refuses_loudly_naming_the_file() {
    // present-but-malformed: the loud parse error — rut.lock's path
    // plus the path-targeted value law (the manifest's shapes)
    let root = leaf_world("badlock", "util", UTIL_BODY);
    let app = root.join("app");
    write(&app, "rut.jsonc", &manifest("app", "mod.rut", ""));
    write(
        &app,
        "src/mod.rut",
        "entry fn go() -> i64 {\n    return 1;\n}\n",
    );
    write(
        &app,
        "rut.lock",
        r#"{"format": "rutlock", "format_version": 1, "deps": [{ "pkg": "util", "url": "https://x/u.rutbundle", "sha256": "nothex" }]}"#,
    );

    let err = load_dir(&app).unwrap_err().to_string();
    assert!(err.contains("rut.lock"), "{err}");
    assert!(
        err.contains("deps[0].sha256: must be 64 hex digits, found 'nothex'"),
        "{err}"
    );
}

#[test]
fn the_resolved_lane_validates_computed_pins_without_a_lock_file() {
    // install's chicken-and-egg lane: mount-validate against COMPUTED
    // pins (the map install is about to write as rut.lock) — no lock
    // file on disk, no fetcher, the same mount-door laws
    let root = leaf_world("resolved", "util", UTIL_BODY);
    let bytes = pack_dir(&root.join("util")).expect("pack util");
    let url = "https://fixtures.test/util.rutbundle";
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "mod.rut",
            &format!(r#", "deps": {{"util": {{"url": "{url}"}}}}"#),
        ),
    );
    write(
        &app,
        "src/mod.rut",
        "use util::{twice};\n\nentry fn go() -> i64 {\n    return twice(21);\n}\n",
    );

    let mut map = BTreeMap::new();
    map.insert(url.to_string(), bytes);
    let mut pins = BTreeMap::new();
    pins.insert(
        url.to_string(),
        rut_driver::sha256_hex(map.get(url).unwrap()),
    );
    let loaded = rut_native::load_dir_resolved(&app, &map, &pins).expect("computed pins load");
    assert_eq!(run_entry::<i64>(loaded, "go"), 42);

    // and the same door refuses a wrong computed pin
    pins.insert(
        url.to_string(),
        "2f6e6a20c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e".into(),
    );
    let err = rut_native::load_dir_resolved(&app, &map, &pins)
        .unwrap_err()
        .to_string();
    assert!(err.contains("sha256 pin mismatch for "), "{err}");
}

#[test]
fn url_dep_rows_collect_the_closure_deduped_by_url() {
    // the install-side collector: root + path-dep manifests (deps AND
    // dev-deps tables; a dev table is walked for PINS, never mounted),
    // url deps are leaves — deduped by url (the identity), the FIRST
    // spec in collect order wins
    let root = scratch("rows");
    let url_a = "https://fixtures.test/a.rutbundle";
    let url_b = "https://fixtures.test/b.rutbundle";
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        &manifest(
            "app",
            "mod.rut",
            &format!(r#", "deps": {{"util": {{"url": "{url_a}"}}, "sub": {{"path": "../sub"}}}}"#),
        ),
    );
    write(
        &app,
        "src/mod.rut",
        "entry fn go() -> i64 {\n    return 1;\n}\n",
    );
    write(
        &root.join("sub"),
        "rut.jsonc",
        &manifest(
            "sub",
            "mod.rut",
            &format!(
                r#", "deps": {{"twin": {{"url": "{url_a}"}}}}, "dev-deps": {{"other": {{"path": "../other"}}}}"#
            ),
        ),
    );
    write(
        &root.join("other"),
        "rut.jsonc",
        &manifest(
            "other",
            "mod.rut",
            &format!(r#", "deps": {{"deep": {{"url": "{url_b}"}}}}"#),
        ),
    );

    let rows = rut_native::url_dep_rows(&app).expect("collect");
    assert_eq!(
        rows,
        vec![
            ("twin".to_string(), url_a.to_string()),
            ("deep".to_string(), url_b.to_string()),
        ],
        "the url is the identity, the FIRST spec in collect order wins: the dep tables are \
         ordered maps, so `sub` (and its twin row) collect before `util`; sub's dev table is \
         walked for pins"
    );
}
