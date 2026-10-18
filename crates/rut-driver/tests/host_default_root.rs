//! `mod.d.rut` as the host ROOT — lib-parity discovery (the
//! host-root amendment, phase 7): `mod.d.rut` beside the manifest is
//! the auto-discovered default root for a `type = "host"` pkg, the
//! twin of a body pkg's `mod.rut`. The gates:
//!
//! - default-root discovery: no `entry.type`, a `mod.d.rut` root
//!   loads (flat and tree shapes), from the directory AND from a
//!   packed bundle;
//! - precedence: a spelled `entry.type` still wins when both sit
//!   beside the manifest (existing flat pkgs keep their
//!   `calc.d.rut`-style roots byte-identically);
//! - neither present is the loud no-entry error naming BOTH doors;
//! - a `mod.rut` beside a host manifest stays refused — a host pkg is
//!   pure surface, with or without a spelled entry;
//! - reader compat: an OLD host bundle (manifest `entry.type` + the
//!   custom root key — the pre-amendment pack shape) loads unchanged
//!   through the override lane;
//! - the pack lane mirrors the discovery (it verifies the exact
//!   layout the reader mounts back) and refuses the both-missing dir.

use std::path::Path;

use rut_driver::{HostRow, MountedBody};
use rut_native::{load_path_session, pack_dir};
use rut_core::types::{TY_BYTES, TY_I64, TY_STR};

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
    let d = std::env::temp_dir().join(format!("rut-host-default-root-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn rows_of(pkg: &rut_driver::Mounted) -> Vec<HostRow> {
    let MountedBody::Host { host_funcs, .. } = &pkg.body else { panic!("a host body") };
    host_funcs.clone()
}

#[test]
fn the_default_root_is_discovered_without_an_entry_type() {
    // the lib-parity law: `mod.d.rut` beside the manifest is the root
    // surface — flat first
    let dir = make_pkg(
        &scratch("flat"),
        &[
            ("rut.jsonc", r#"{"name": "flat_host", "type": "host"}"#),
            ("mod.d.rut", "pub host fn ping() -> bool;\n"),
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
            ("mod.d.rut", "pub mod net;\npub host fn legacy_ping() -> bool;\n"),
            ("net/mod.d.rut", "pub host fn connect(url: str) -> opaque;\n"),
        ],
    );
    let loaded = load_path_session(&dir).expect("loads");
    let m = loaded.pkg("myhost").unwrap();
    assert_eq!(m.mods.keys().collect::<Vec<_>>(), vec!["net"]);
    assert_eq!(
        rows_of(m).iter().map(|r| r.full_name("myhost")).collect::<Vec<_>>(),
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
            ("duo.d.rut", "pub host fn spelled() -> bool;\n"),
            ("mod.d.rut", "pub host fn default_root() -> bool;\n"),
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
        &[
            (
                "rut.jsonc",
                r#"{"format": "rutbundle", "format_version": 11, "name": "h", "type": "host"}"#,
            ),
        ],
    );
    let err = pack_dir(&dir).unwrap_err().to_string();
    assert!(err.contains("host pkg with no entry"), "{err}");
    assert!(err.contains("mod.d.rut"), "{err}");
}

#[test]
fn a_body_beside_a_host_manifest_stays_refused() {
    // the body refusal predates the amendment and rides unchanged —
    // a `mod.rut` beside a host manifest refuses whether the entry is
    // spelled or the default root sits there
    for manifest in [
        r#"{"name": "h", "type": "host", "entry": {"type": "./h.d.rut"}}"#,
        r#"{"name": "h", "type": "host"}"#,
    ] {
        let dir = make_pkg(
            &scratch("body"),
            &[
                ("rut.jsonc", manifest),
                ("h.d.rut", "pub host fn ping() -> bool;\n"),
                ("mod.d.rut", "pub host fn root_row() -> bool;\n"),
                ("mod.rut", "entry fn main() {}\n"),
            ],
        );
        let err = load_path_session(&dir).unwrap_err().to_string();
        assert!(err.contains("pure surface"), "{err}");
        assert!(err.contains("mod.d.rut"), "{err}");
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
                r#"{"format": "rutbundle", "format_version": 11, "name": "flat_host", "type": "host"}"#,
            ),
            ("mod.d.rut", "pub host fn ping() -> bool;\n"),
        ],
    );
    let bytes = pack_dir(&dir).expect("pack");
    let bundle = rut_driver::bundle::Bundle::parse(&bytes).unwrap();
    let names: Vec<&str> = bundle.entries().iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["rut.jsonc", "mod.d.rut"], "the root rides its default key");
    assert_eq!(bytes, pack_dir(&dir).unwrap(), "deterministic");
    let loaded = rut_driver::Mounted::from_bundle(&bytes).expect("load");
    assert_eq!(
        rows_of(loaded.pkg("flat_host").unwrap())[0].full_name("flat_host"),
        "flat_host::ping",
    );

    // tree: the rows envelope rides beside the default root, and the
    // round trip re-mounts path-qualified
    let dir = make_pkg(
        &scratch("pack-tree"),
        &[
            (
                "rut.jsonc",
                r#"{"format": "rutbundle", "format_version": 11, "name": "myhost", "type": "host"}"#,
            ),
            ("mod.d.rut", "pub mod net;\npub host fn legacy_ping() -> bool;\n"),
            ("net/mod.d.rut", "pub host fn connect(url: str) -> i64;\n"),
        ],
    );
    let bytes = pack_dir(&dir).expect("pack");
    let bundle = rut_driver::bundle::Bundle::parse(&bytes).unwrap();
    let names: Vec<&str> = bundle.entries().iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["rut.jsonc", "mod.d.rut", rut_driver::mods::ROWS_NAME]);
    let loaded = rut_driver::Mounted::from_bundle(&bytes).expect("load");
    let m = loaded.pkg("myhost").unwrap();
    assert_eq!(m.mods.keys().collect::<Vec<_>>(), vec!["net"]);
    assert_eq!(
        rows_of(m).iter().map(|r| r.full_name("myhost")).collect::<Vec<_>>(),
        vec!["myhost::legacy_ping", "myhost::net::connect"],
    );
}

#[test]
fn a_host_bundle_packs_its_surface_at_the_canonical_name() {
    // THE CANONICAL SURFACE LAW (wire v11): a manifest spelling
    // `entry.type` keeps the DIRECTORY key (the directory grammar is
    // untouched) — but the archive canonicalizes: the surface rides as
    // `mod.d.rut`, whatever the directory spelled. The override lane
    // is gone; the reader mounts `mod.d.rut` at both levels.
    let dir = make_pkg(
        &scratch("old-shape"),
        &[
            (
                "rut.jsonc",
                r#"{"format": "rutbundle", "format_version": 11, "name": "http_host", "type": "host", "entry": {"type": "./http_host.d.rut"}}"#,
            ),
            (
                "http_host.d.rut",
                "pub mod net;\npub host fn http_send(url: str) -> bytes;\n",
            ),
            ("net/mod.d.rut", "pub host fn http_status(r: opaque) -> i64;\n"),
        ],
    );
    let bytes = pack_dir(&dir).expect("pack");
    let bundle = rut_driver::bundle::Bundle::parse(&bytes).unwrap();
    let names: Vec<&str> = bundle.entries().iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec!["rut.jsonc", "mod.d.rut", rut_driver::mods::ROWS_NAME],
        "the surface rides at the ONE canonical name, whatever the directory spelled",
    );
    let loaded = rut_driver::Mounted::from_bundle(&bytes).expect("load");
    let names: Vec<String> = rows_of(loaded.pkg("http_host").unwrap())
        .iter()
        .map(|r| r.full_name("http_host"))
        .collect();
    assert_eq!(
        names,
        vec!["http_host::http_send", "http_host::net::http_status"],
        "the canonical surface's tree mounts (root row flat, child qualified)",
    );
    // and the surface rows carry their crossings (the readback keeps
    // the type law)
    let rows = rows_of(loaded.pkg("http_host").unwrap());
    assert_eq!(rows[0].params, vec![TY_STR]);
    assert_eq!(rows[0].ret, TY_BYTES);
    assert_eq!(rows[1].ret, TY_I64);
}
