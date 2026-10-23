//! `src/mod.d.rut` as the host ROOT — lib-parity discovery (the
//! host-root amendment, phase 7): `src/mod.d.rut` in the package's
//! `src/` shell is the auto-discovered default root for a
//! `type = "host"` pkg, the twin of a body pkg's `src/mod.rut`. The
//! gates:
//!
//! - default-root discovery: no `entry.type`, a `mod.d.rut` root
//!   loads (flat and tree shapes), from the directory AND from a
//!   packed bundle;
//! - precedence: a spelled `entry.type` still wins when both sit in
//!   the shell (existing flat pkgs keep their `calc.d.rut`-style
//!   roots byte-identically);
//! - neither present is the loud no-entry error naming BOTH doors;
//! - a `mod.rut` beside a host manifest is an inert unread file (the
//!   stray-body refusal is repealed) — a host pkg is pure surface,
//!   with or without a spelled entry;
//! - reader compat: an OLD host bundle (manifest `entry.type` + the
//!   custom root key — the pre-amendment pack shape) loads unchanged
//!   through the override lane;
//! - the pack lane mirrors the discovery (it verifies the exact
//!   layout the reader mounts back) and refuses the both-missing dir.

use std::path::Path;

use rut_core::types::{TY_BYTES, TY_I64, TY_STR};
use rut_driver::{HostRow, MountedBody};
use rut_native::{load_path_session, pack_dir};

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

fn scratch(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "rut-host-default-root-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn rows_of(pkg: &rut_driver::Mounted) -> Vec<HostRow> {
    let MountedBody::Host { host_funcs, .. } = &pkg.body else {
        panic!("a host body")
    };
    host_funcs.clone()
}

#[test]
fn the_default_root_is_discovered_without_an_entry_type() {
    // the lib-parity law: `src/mod.d.rut` in the shell is the root
    // surface — flat first
    let dir = make_pkg(
        &scratch("flat"),
        &[
            ("rut.jsonc", r#"{"name": "flat_host", "type": "host"}"#),
            ("src/mod.d.rut", "pub host fn ping() -> bool;\n"),
        ],
    );
    let loaded = load_path_session(&dir).expect("loads");
    assert_eq!(loaded.root, "flat_host");
    assert!(loaded.pkg("flat_host").unwrap().mods.is_empty());
    assert_eq!(
        rows_of(loaded.pkg("flat_host").unwrap())[0].full_name("flat_host"),
        "flat_host::ping",
    );

    // then the tree: the root declares children, the rows ride
    // path-qualified — indistinguishable from a spelled entry's mount
    let dir = make_pkg(
        &scratch("tree"),
        &[
            ("rut.jsonc", r#"{"name": "myhost", "type": "host"}"#),
            (
                "src/mod.d.rut",
                "pub mod net;\npub host fn legacy_ping() -> bool;\n",
            ),
            (
                "src/net/mod.d.rut",
                "pub host fn connect(url: str) -> opaque;\n",
            ),
        ],
    );
    let loaded = load_path_session(&dir).expect("loads");
    let m = loaded.pkg("myhost").unwrap();
    assert_eq!(m.mods.keys().collect::<Vec<_>>(), vec!["net"]);
    assert_eq!(
        rows_of(m)
            .iter()
            .map(|r| r.full_name("myhost"))
            .collect::<Vec<_>>(),
        vec!["myhost::legacy_ping", "myhost::net::connect"],
    );
}

#[test]
fn a_spelled_entry_type_wins_over_the_default_root() {
    // both doors present: the spelled `entry.type` is the root — the
    // precedence law keeps every existing flat pkg (calc, engine,
    // nmap, …) loading byte-identically
    let dir = make_pkg(
        &scratch("precedence"),
        &[
            (
                "rut.jsonc",
                r#"{"name": "duo", "type": "host", "entry": {"type": "./duo.d.rut"}}"#,
            ),
            ("src/duo.d.rut", "pub host fn spelled() -> bool;\n"),
            ("src/mod.d.rut", "pub host fn default_root() -> bool;\n"),
        ],
    );
    let loaded = load_path_session(&dir).expect("loads");
    let names: Vec<String> = rows_of(loaded.pkg("duo").unwrap())
        .iter()
        .map(|r| r.full_name("duo"))
        .collect();
    assert_eq!(names, vec!["duo::spelled"], "the spelled entry is the root");
}

#[test]
fn neither_door_is_the_loud_no_entry_error() {
    // the both-missing refusal names BOTH spellings (the lib arm's
    // ambiguity refusal, d-flavored)
    let dir = make_pkg(
        &scratch("neither"),
        &[("rut.jsonc", r#"{"name": "h", "type": "host"}"#)],
    );
    let err = load_path_session(&dir).unwrap_err().to_string();
    assert!(err.contains("host pkg with no entry"), "{err}");
    assert!(err.contains("mod.d.rut"), "{err}");

    // the pack lane mirrors the discovery and refuses the same dir —
    // the pack verifies the exact layout the reader mounts back (the
    // pack lane gates on the bundle keys first, so spell them)
    let dir = make_pkg(
        &scratch("neither-pack"),
        &[(
            "rut.jsonc",
            r#"{"format": "rutbundle", "format_version": 12, "name": "h", "type": "host"}"#,
        )],
    );
    let err = pack_dir(&dir).unwrap_err().to_string();
    assert!(err.contains("host pkg with no entry"), "{err}");
    assert!(err.contains("mod.d.rut"), "{err}");
}

#[test]
fn a_stray_body_beside_a_host_manifest_is_inert() {
    // the stray-body refusal is REPEALED (the `src/` shell cutover): a
    // `mod.rut` beside a host manifest is an inert unread file — the
    // pkg loads whether the entry is spelled (the spelled surface
    // wins) or the default root sits in the shell
    for (manifest, want) in [
        (
            r#"{"name": "h", "type": "host", "entry": {"type": "./h.d.rut"}}"#,
            vec!["h::ping"],
        ),
        (r#"{"name": "h", "type": "host"}"#, vec!["h::root_row"]),
    ] {
        let dir = make_pkg(
            &scratch("body"),
            &[
                ("rut.jsonc", manifest),
                ("src/h.d.rut", "pub host fn ping() -> bool;\n"),
                ("src/mod.d.rut", "pub host fn root_row() -> bool;\n"),
                ("mod.rut", "entry fn main() {}\n"),
            ],
        );
        let loaded = load_path_session(&dir).expect("the stray body is unread, the pkg loads");
        let names: Vec<String> = rows_of(loaded.pkg("h").unwrap())
            .iter()
            .map(|r| r.full_name("h"))
            .collect();
        assert_eq!(names, want, "the entry law holds; the stray is inert");
    }
}

#[test]
fn the_default_root_packs_and_loads_back() {
    // flat: the bundle stays the two-entry shape (manifest + the
    // `mod.d.rut` root key) — no rows entry, no version bump
    let dir = make_pkg(
        &scratch("pack-flat"),
        &[
            (
                "rut.jsonc",
                r#"{"format": "rutbundle", "format_version": 12, "name": "flat_host", "type": "host"}"#,
            ),
            ("src/mod.d.rut", "pub host fn ping() -> bool;\n"),
        ],
    );
    let bytes = pack_dir(&dir).expect("pack");
    let bundle = rut_driver::bundle::Bundle::parse(&bytes).unwrap();
    let names: Vec<&str> = bundle.entries().iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec!["rut.jsonc", "mod.d.rut"],
        "the root rides its default key"
    );
    assert_eq!(bytes, pack_dir(&dir).unwrap(), "deterministic");
    let loaded = rut_driver::Mounted::from_bundle(&bytes).expect("load");
    assert_eq!(
        rows_of(loaded.pkg("flat_host").unwrap())[0].full_name("flat_host"),
        "flat_host::ping",
    );

    // tree: the ONE consolidated entry (wire v12 — the tree normalized
    // into the inline form, no rows envelope), and the round trip
    // re-lowers path-qualified through the inline grammar
    let dir = make_pkg(
        &scratch("pack-tree"),
        &[
            (
                "rut.jsonc",
                r#"{"format": "rutbundle", "format_version": 12, "name": "myhost", "type": "host"}"#,
            ),
            (
                "src/mod.d.rut",
                "pub mod net;\npub host fn legacy_ping() -> bool;\n",
            ),
            (
                "src/net/mod.d.rut",
                "pub host fn connect(url: str) -> i64;\n",
            ),
        ],
    );
    let bytes = pack_dir(&dir).expect("pack");
    let bundle = rut_driver::bundle::Bundle::parse(&bytes).unwrap();
    let names: Vec<&str> = bundle.entries().iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["rut.jsonc", "mod.d.rut"]);
    // the entry IS the tree: the child's declaration spliced inline
    let entry = bundle.read("mod.d.rut").unwrap();
    assert!(entry.contains("mod net {"), "the inline form: {entry}");
    assert!(
        entry.contains("connect"),
        "the child's rows ride inside: {entry}"
    );
    assert_eq!(bytes, pack_dir(&dir).unwrap(), "deterministic");
    let loaded = rut_driver::Mounted::from_bundle(&bytes).expect("load");
    let m = loaded.pkg("myhost").unwrap();
    // the tree lives in the grammar now — no file modules mount
    assert!(m.mods.is_empty(), "{:?}", m.mods.keys().collect::<Vec<_>>());
    assert_eq!(
        rows_of(m)
            .iter()
            .map(|r| r.full_name("myhost"))
            .collect::<Vec<_>>(),
        vec!["myhost::legacy_ping", "myhost::net::connect"],
    );
}

#[test]
fn a_host_bundle_packs_its_surface_at_the_canonical_name() {
    // THE CANONICAL SURFACE LAW (wire v11, unchanged): a manifest
    // spelling `entry.type` keeps the DIRECTORY key (the directory
    // grammar is untouched) — but the archive canonicalizes: the ONE
    // consolidated entry rides as `mod.d.rut`, whatever the directory
    // spelled. The override lane is gone; the reader mounts `mod.d.rut`
    // at both levels.
    let dir = make_pkg(
        &scratch("old-shape"),
        &[
            (
                "rut.jsonc",
                r#"{"format": "rutbundle", "format_version": 12, "name": "http_host", "type": "host", "entry": {"type": "./http_host.d.rut"}}"#,
            ),
            (
                "src/http_host.d.rut",
                "pub mod net;\npub host fn http_send(url: str) -> bytes;\n",
            ),
            (
                "src/net/mod.d.rut",
                "pub host fn http_status(r: opaque) -> i64;\n",
            ),
        ],
    );
    let bytes = pack_dir(&dir).expect("pack");
    let bundle = rut_driver::bundle::Bundle::parse(&bytes).unwrap();
    let names: Vec<&str> = bundle.entries().iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec!["rut.jsonc", "mod.d.rut"],
        "the ONE consolidated entry rides at the canonical name, no rows envelope",
    );
    let loaded = rut_driver::Mounted::from_bundle(&bytes).expect("load");
    let names: Vec<String> = rows_of(loaded.pkg("http_host").unwrap())
        .iter()
        .map(|r| r.full_name("http_host"))
        .collect();
    assert_eq!(
        names,
        vec!["http_host::http_send", "http_host::net::http_status"],
        "the consolidated entry's tree lowers (root row flat, child qualified)",
    );
    // and the surface rows carry their crossings (the readback keeps
    // the type law)
    let rows = rows_of(loaded.pkg("http_host").unwrap());
    assert_eq!(rows[0].params, vec![TY_STR]);
    assert_eq!(rows[0].ret, TY_BYTES);
    assert_eq!(rows[1].ret, TY_I64);
}
