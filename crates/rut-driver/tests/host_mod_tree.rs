//! The host `.d.rut` MOD TREE (the host-mod-tree plan, phase 1): a
//! `type = "host"` pkg organizes its surface as a module tree —
//! `pub mod net;` in the root `.d.rut`, the children in
//! `NAME/mod.d.rut` beside the declaring file, recursively,
//! cycle-guarded. Every row carries the mod path it lives under
//! ([`rut_driver::HostRow`]); the registration string is
//! `pkg::path::name` through `full_name` (flat rows are the
//! `path = ""` case and spell today's shape). The phase-1 gates:
//!
//! - the FS mount (the Decl flavor of the loader's mod mount);
//! - the loud missing-child error (a `mod NAME;` whose child isn't
//!   mounted refuses — the parse-only silent drop is fixed);
//! - path-qualified rows + the async row family expansion;
//! - the bundle lane (the tree rides the `rut.mods` rows envelope;
//!   a FLAT host pkg packs byte-identically — no rows entry);
//! - flat compatibility: root rows still register (and bind through a
//!   consumer's `use`) exactly as before.
//!
//! Phase 2 (NOT here): qualified-use BINDING (`use myhost::net::connect`
//! resolving through the externs gate) — a consumer spells the root
//! rows' flat use paths only.

use std::path::Path;

use rut_driver::{PkgBody, HostRow};
use rut_native::{load_path_session, pack_dir};
use rut_core::types::{TY_BOOL, TY_OPAQUE, TY_STR};

const MYHOST_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/myhost");

/// A temp module dir from a manifest + file map (the host_pkgs
/// fixture shape, nested paths included).
fn make_pkg(base: &Path, files: &[(&str, &str)]) -> std::path::PathBuf {
    let dir = base.join("pkg");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, text) in files {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

fn host_manifest(name: &str, surface: &str) -> String {
    format!(
        r#"{{"format": "rutbundle", "format_version": 10, "name": "{name}", "type": "host", "entry": {{"type": "./{surface}"}}}}"#
    )
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("rut-host-mod-tree-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The plan's target shape, in a temp dir: root + net + net/tls.
fn tree_world(tag: &str) -> std::path::PathBuf {
    make_pkg(
        &scratch(tag),
        &[
            ("rut.jsonc", &host_manifest("myhost", "myhost.d.rut")),
            (
                "myhost.d.rut",
                "pub mod net;\npub host fn legacy_ping() -> bool;\n",
            ),
            (
                "net/mod.d.rut",
                "pub mod tls;\npub host fn connect(url: str) -> opaque;\n",
            ),
            ("net/tls/mod.d.rut", "pub host fn handshake(c: opaque) -> opaque;\n"),
        ],
    )
}

fn rows_of(pkg: &rut_driver::Pkg) -> Vec<HostRow> {
    let PkgBody::Host { host_funcs, .. } = &pkg.body else { panic!("a host body") };
    host_funcs.clone()
}

#[test]
fn the_target_shape_loads_with_path_carrying_rows() {
    let loaded = load_path_session(Path::new(MYHOST_DIR)).expect("rut/myhost loads");
    assert_eq!(loaded.root, "myhost");
    let m = loaded.pkg("myhost").expect("myhost mounted");
    // the mounted tree rides the pkg (the phase-2 dep tables read it)
    assert_eq!(m.mods.keys().collect::<Vec<_>>(), vec!["net", "net/tls"]);
    assert_eq!(
        format!("{:?}", m.mods["net"].vis),
        "Pub",
        "the declaring edge's vis rides faithfully"
    );
    // every row carries the path it lives under — in declaration order
    assert_eq!(
        rows_of(m),
        vec![
            HostRow { path: String::new(), name: "legacy_ping".into(), params: vec![], ret: TY_BOOL, is_async: false },
            HostRow { path: "net".into(), name: "connect".into(), params: vec![TY_STR], ret: TY_OPAQUE, is_async: false },
            HostRow { path: "net/tls".into(), name: "handshake".into(), params: vec![TY_OPAQUE], ret: TY_OPAQUE, is_async: false },
        ],
    );
    // the registration strings: flat at the root, path-qualified under
    // the tree — the ONE source is `full_name`
    let rows = rows_of(m);
    assert_eq!(rows[0].full_name("myhost"), "myhost::legacy_ping");
    assert_eq!(rows[1].full_name("myhost"), "myhost::net::connect");
    assert_eq!(rows[2].full_name("myhost"), "myhost::net::tls::handshake");
}

#[test]
fn a_declared_child_without_its_file_is_the_loud_error() {
    // the latent silent drop is fixed: `mod net;` with no
    // `net/mod.d.rut` beside it refuses at load, naming the declaring
    // file and the expected spelling (the FS mount's loud half)
    let dir = make_pkg(
        &scratch("missing"),
        &[
            ("rut.jsonc", &host_manifest("myhost", "myhost.d.rut")),
            ("myhost.d.rut", "pub mod net;\npub host fn legacy_ping() -> bool;\n"),
        ],
    );
    let err = load_path_session(&dir).unwrap_err().to_string();
    assert!(err.contains("`mod net;` in mod.d.rut"), "{err}");
    assert!(err.contains("no `net/mod.d.rut` beside it"), "{err}");

    // the same law one level down: `net/mod.d.rut` declaring `mod
    // tls;` with no `net/tls/` names the DECLARING child file
    let dir = make_pkg(
        &scratch("missing-nested"),
        &[
            ("rut.jsonc", &host_manifest("myhost", "myhost.d.rut")),
            ("myhost.d.rut", "pub mod net;\npub host fn legacy_ping() -> bool;\n"),
            ("net/mod.d.rut", "pub mod tls;\npub host fn connect(url: str) -> opaque;\n"),
        ],
    );
    let err = load_path_session(&dir).unwrap_err().to_string();
    assert!(err.contains("`mod tls;` in net/mod.d.rut"), "{err}");

    // and a lower_decl_module hand-off whose mounted map disagrees
    // with the declarations errors the same way (the loader's mount is
    // not the only loud door — a hand-off is named, never guessed)
    let children = std::collections::BTreeMap::new();
    let err = rut_driver::lower_decl_module(
        "pub mod net;\npub host fn f();\n",
        &children,
        "root.d.rut",
    )
    .unwrap_err();
    assert!(err.contains("`mod net;` in root.d.rut"), "{err}");
    assert!(err.contains("no `net` module is mounted"), "{err}");
}

#[test]
fn a_child_that_does_not_parse_is_loud_too() {
    // the mount stays silent (the same law the lib tree has), but the
    // LOWERING parses every mounted child in Decl mode — a broken
    // surface file refuses with its syntax, never silently drops rows
    let dir = make_pkg(
        &scratch("broken-child"),
        &[
            ("rut.jsonc", &host_manifest("myhost", "myhost.d.rut")),
            ("myhost.d.rut", "pub mod net;\npub host fn legacy_ping() -> bool;\n"),
            ("net/mod.d.rut", "pub host fn broken( {{\n"),
        ],
    );
    let err = load_path_session(&dir).unwrap_err().to_string();
    assert!(err.contains("net/mod.d.rut"), "{err}");
    assert!(err.contains("does not parse"), "{err}");
}

#[test]
fn first_mount_wins_and_flat_rows_stay_in_declaration_order() {
    // a repeated `mod net;` mounts once (the mount core's law); the
    // root rows keep their flat path
    let dir = make_pkg(
        &scratch("repeated"),
        &[
            ("rut.jsonc", &host_manifest("myhost", "myhost.d.rut")),
            (
                "myhost.d.rut",
                "pub mod net;\npub mod net;\npub host fn legacy_ping() -> bool;\npub host fn second(x: str);\n",
            ),
            ("net/mod.d.rut", "pub host fn connect(url: str) -> opaque;\n"),
        ],
    );
    let loaded = load_path_session(&dir).expect("loads");
    let m = loaded.pkg("myhost").unwrap();
    assert_eq!(m.mods.keys().collect::<Vec<_>>(), vec!["net"], "one mount");
    assert_eq!(
        rows_of(m).iter().map(|r| r.full_name("myhost")).collect::<Vec<_>>(),
        vec!["myhost::legacy_ping", "myhost::second", "myhost::net::connect"],
        "root rows first (declaration order), then the tree's"
    );
}

#[test]
fn an_async_row_family_expands_path_qualified() {
    // `host async fn` under a mod: the ctx declares the family on the
    // QUALIFIED name, and the flatten spells `pkg::path::name<suffix>`
    let dir = make_pkg(
        &scratch("async"),
        &[
            ("rut.jsonc", &host_manifest("fixture", "fixture.d.rut")),
            (
                "fixture.d.rut",
                "pub mod net;\npub host fn sync_row(x: str) -> nil;\n",
            ),
            (
                "net/mod.d.rut",
                "pub host async fn probe(k: str) -> i64;\n",
            ),
        ],
    );
    let loaded = load_path_session(&dir).expect("loads");
    let ctx = rut_driver::host_pkg_ctx(&loaded.pkgs);
    let rows = ctx.rows_of("fixture").expect("fixture partitioned");
    // the root row stays bare; the tree row's family is path-qualified
    assert!(rows.contains_key("sync_row"));
    for suffix in ["", "__start", "__yield", "__take", "__cancel"] {
        assert!(rows.contains_key(&format!("net::probe{suffix}")), "net::probe{suffix}");
    }
    assert!(!rows.contains_key("probe__start"), "the family never lands on the bare name");
    let flat = rut_driver::declared_host_fns(&loaded.pkgs);
    for suffix in ["", "__start", "__yield", "__take", "__cancel"] {
        assert!(flat.contains_key(&format!("fixture::net::probe{suffix}")));
    }
    assert!(flat.contains_key("fixture::sync_row"));
}

#[test]
fn the_registration_contract_speaks_the_path_spelling() {
    // the install lane: the builder registers the scope-relative path
    // spelling, the mount table's keys are the same rows — the install
    // joins them row for row (a missing one would panic here), and a
    // consumer's FLAT use of a root row still binds (flat compatibility)
    use rut_vm::HostPkg;
    use rut_vm::interp::Vm;
    let dir = make_pkg(
        &scratch("register"),
        &[
            ("rut.jsonc", &host_manifest("myhost", "myhost.d.rut")),
            (
                "myhost.d.rut",
                "pub mod net;\npub host fn legacy_ping() -> bool;\n",
            ),
            (
                "net/mod.d.rut",
                "pub mod tls;\npub host fn connect(url: str) -> i64;\n",
            ),
            ("net/tls/mod.d.rut", "pub host fn handshake(c: i64) -> i64;\n"),
        ],
    );
    let loaded = load_path_session(&dir).expect("loads");
    let mut pkg = HostPkg::new("myhost");
    pkg.register::<_, (), bool, _>("legacy_ping", |_vm: &mut Vm| true);
    pkg.register::<_, (&str,), i64, _>("net::connect", |_vm: &mut Vm, _url: &str| 1);
    pkg.register::<_, (i64,), i64, _>("net::tls::handshake", |_vm: &mut Vm, c: i64| c);
    let compiled = rut_driver::RutRun::new()
        .pkgs(&loaded)
        .host_pkg(pkg.build())
        .pkg(
            // the app DECLARES its use (the myhost host surface)
            rut_driver::Pkg::source(
                "app",
                "use myhost::{legacy_ping};\n\nentry fn main() -> nil {\n    let _ = legacy_ping();\n}\n",
            )
            .manifest(rut_driver::Manifest::lib("app", &["myhost"])),
        )
        .entrypoint("app")
        .compile()
        .expect("the installer joins the path-qualified rows");
    assert!(compiled.graph.diags.is_empty(), "{:?}", compiled.graph.diags);
}

#[test]
fn flat_host_pkgs_pack_byte_identically() {
    // the back-compat guarantee: a flat host pkg writes NO rows entry
    // — the bundle stays the two-entry shape, byte-deterministic (same
    // input ⇒ same bytes), no version bump
    let dir = make_pkg(
        &scratch("flat"),
        &[
            ("rut.jsonc", &host_manifest("flat_host", "flat_host.d.rut")),
            ("flat_host.d.rut", "pub host fn ping() -> bool;\n"),
        ],
    );
    let bytes = pack_dir(&dir).expect("pack");
    let bundle = rut_driver::bundle::Bundle::parse(&bytes).unwrap();
    let names: Vec<&str> = bundle.entries().iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["rut.jsonc", "flat_host.d.rut"], "no rut.mods entry");
    let again = pack_dir(&dir).expect("repack");
    assert_eq!(bytes, again, "deterministic");
    // and it loads back exactly as before: one flat row
    let loaded = rut_driver::Pkg::from_bundle(&bytes).expect("load");
    let m = loaded.pkg("flat_host").unwrap();
    assert!(m.mods.is_empty());
    assert_eq!(
        rows_of(m),
        vec![HostRow { path: String::new(), name: "ping".into(), params: vec![], ret: TY_BOOL, is_async: false }],
    );
}

#[test]
fn a_host_tree_packs_into_the_rows_envelope_and_loads_back() {
    // the tree rides the additive `rut.mods` rows envelope (`""` = the
    // root surface, `<path>` = each child); the archive reader mounts
    // the decl children and the rows come back path-qualified
    let dir = tree_world("pack");
    let bytes = pack_dir(&dir).expect("pack");
    let bundle = rut_driver::bundle::Bundle::parse(&bytes).unwrap();
    let names: Vec<&str> = bundle.entries().iter().map(|(n, _)| n.as_str()).collect();
    assert!(
        names.contains(&rut_driver::mods::ROWS_NAME),
        "the tree rides the rows envelope: {names:?}"
    );
    let loaded = rut_driver::Pkg::from_bundle(&bytes).expect("load");
    let m = loaded.pkg("myhost").unwrap();
    assert_eq!(m.mods.keys().collect::<Vec<_>>(), vec!["net", "net/tls"]);
    assert_eq!(
        rows_of(m).iter().map(|r| r.full_name("myhost")).collect::<Vec<_>>(),
        vec!["myhost::legacy_ping", "myhost::net::connect", "myhost::net::tls::handshake"],
    );
    // and the pack is deterministic
    assert_eq!(bytes, pack_dir(&dir).unwrap(), "deterministic");
}

#[test]
fn the_same_leaf_name_in_two_mods_is_legal() {
    // the mod path is the namespace: two mods may each declare a leaf
    // of the same NAME — the rows differ by their paths, both register
    // (`pkg::a::probe` vs `pkg::b::probe`), neither collides
    let dir = make_pkg(
        &scratch("same-leaf"),
        &[
            ("rut.jsonc", &host_manifest("duo", "duo.d.rut")),
            ("duo.d.rut", "pub mod a;\npub mod b;\n"),
            ("a/mod.d.rut", "pub host fn probe(x: str) -> i64;\n"),
            ("b/mod.d.rut", "pub host fn probe(k: i64) -> i64;\n"),
        ],
    );
    let loaded = load_path_session(Path::new(&dir)).expect("loads");
    let rows = rows_of(loaded.pkg("duo").unwrap());
    assert_eq!(
        rows.iter().map(|r| r.full_name("duo")).collect::<Vec<_>>(),
        vec!["duo::a::probe", "duo::b::probe"],
        "the paths disambiguate the leaves"
    );
    assert_eq!(rows[0].params, vec![TY_STR]);
    assert_eq!(rows[1].params, vec![rut_core::types::TY_I64]);
    // and the embedder registers BOTH through the pkg scope
    use rut_vm::HostPkg;
    use rut_vm::interp::Vm;
    let mut pkg = HostPkg::new("duo");
    pkg.register::<_, (&str,), i64, _>("a::probe", |_vm: &mut Vm, s: &str| s.len() as i64);
    pkg.register::<_, (i64,), i64, _>("b::probe", |_vm: &mut Vm, k: i64| k + 1);
    let compiled = rut_driver::RutRun::new()
        .pkgs(&loaded)
        .host_pkg(pkg.build())
        .pkg(
            // the app DECLARES its use (the duo host surface)
            rut_driver::Pkg::source(
                "app",
                "use duo::a::{ probe };\n\nentry fn main() -> i64 {\n    return probe(\"hey\");\n}\n",
            )
            .manifest(rut_driver::Manifest::lib("app", &["duo"])),
        )
        .entrypoint("app")
        .compile()
        .expect("both leaves install");
    assert!(compiled.graph.diags.is_empty(), "{:?}", compiled.graph.diags);
}

#[test]
fn ensure_host_synthesizes_the_path_qualified_surface() {
    // the compiler's host arm: a mod row's SurfaceFn spells the
    // mod-path name (`net::connect`) with the REGISTRATION name riding
    // `host` (`myhost::net::connect`); a flat root row stays bare on
    // both. Read off the linked program's bodyless host funcs — the
    // keys the boot join resolves.
    let dir = tree_world("surface");
    let loaded = load_path_session(Path::new(&dir)).expect("loads");
    let compiled = rut_driver::RutRun::new()
        .pkgs(&loaded)
        .pkg(
            // the app DECLARES its use (the myhost host surface)
            rut_driver::Pkg::source(
                "app",
                "use myhost::{ legacy_ping };\nuse myhost::net::tls::handshake;\n\nentry fn main() -> bool {\n    return legacy_ping();\n}\n",
            )
            .manifest(rut_driver::Manifest::lib("app", &["myhost"])),
        )
        .entrypoint("app")
        .compile()
        .expect("compiles");
    assert!(compiled.graph.diags.is_empty(), "{:?}", compiled.graph.diags);
    let prog = rut_core::link::flatten(compiled.graph.program.expect("linked"));
    let mut host_rows: Vec<(String, String)> = prog
        .funcs
        .iter()
        .filter_map(|f| {
            let host = f.host_id?;
            Some((prog.interner.name(f.name).to_string(), prog.interner.name(host).to_string()))
        })
        .collect();
    host_rows.sort();
    assert_eq!(
        host_rows,
        vec![
            ("legacy_ping".to_string(), "myhost::legacy_ping".to_string()),
            ("net::connect".to_string(), "myhost::net::connect".to_string()),
            ("net::tls::handshake".to_string(), "myhost::net::tls::handshake".to_string()),
        ],
        "func name = the mod-path spelling (`net::connect`; root rows bare), \
         host id = the registration spelling (`myhost::net::connect`)"
    );
}

#[test]
fn the_host_tree_mount_refuses_a_body_beside_the_manifest() {
    // the host pkg has no body: a `mod.rut` beside the manifest still
    // refuses, and the message points at the tree's `mod.d.rut` home
    let dir = make_pkg(
        &scratch("body"),
        &[
            ("rut.jsonc", &host_manifest("myhost", "myhost.d.rut")),
            ("myhost.d.rut", "pub mod net;\npub host fn legacy_ping() -> bool;\n"),
            ("net/mod.d.rut", "pub host fn connect(url: str) -> opaque;\n"),
            ("mod.rut", "entry fn main() {}\n"),
        ],
    );
    let err = load_path_session(&dir).unwrap_err().to_string();
    assert!(err.contains("pure surface"), "{err}");
    assert!(err.contains("mod.d.rut"), "{err}");
}

#[test]
fn the_lib_surface_law_still_refuses_root_host_rows() {
    // the lib-surface law rides unchanged: host fn text outside a
    // `type = "host"` pkg refuses (root-level probe — the leaf half is
    // phase 2)
    let dir = make_pkg(
        &scratch("lib-law"),
        &[
            ("rut.jsonc", r#"{"name": "s", "type": "lib", "entry": {"type": "./s.d.rut"}}"#),
            ("s.d.rut", "pub host fn ping() -> bool;\n"),
        ],
    );
    let err = load_path_session(&dir).unwrap_err().to_string();
    assert!(err.contains("`host fn ping`"), "{err}");
    assert!(err.contains("`type = \"host\"`"), "{err}");
}
