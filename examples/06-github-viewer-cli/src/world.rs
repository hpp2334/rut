//! The example's ONE world — the body installs and the declared
//! host-fn table, spelled once and consumed by the binary
//! (`src/main.rs`), the offline suite, and the live smoke (the test
//! crates include this file with `#[path = "../src/world.rs"]
//! mod world;`).
//!
//! The two-homes shape WAS the bug: the suite's world (right, and the
//! one the tests run) and the binary's world (wrong, and the one no
//! gate ran) drifted — the binary never bound `strbuild_host` (json's
//! writer) and never joined `calc_host` into its expected table, so
//! the eager decl↔bodies join panicked at boot (exit 101) while every
//! offline test stayed green. One home ends the drift class: the
//! suite varies ONLY the http lane (the fixture `pkg_with` vs the
//! real `pkg()`), passed in; the expected table reads the same tree
//! mounts for both.
//!
//! Each consumer uses a subset of the home (the bin:
//! `host_pkgs` + `expected`; the suite: `decl_world` + `expected` +
//! `host_pkgs`; the boot gate: `store_warm`), so a slice of the
//! surface is unused per compiling crate — that is the shape of one
//! shared home, not dead code:
#![allow(dead_code)]

use rut_vm::HostPkg;

/// The host-body installs: the calc Math surface, the nmap table,
/// strbuild (json's writer rides it — the `strbuild_host` rows are in
/// every consumer's declared set, so the bodies install beside it),
/// the async engine's launcher set, and the http lane the CALLER
/// names (the suite's fixture `pkg_with`, the binary's real `pkg()`).
pub fn host_pkgs(http: HostPkg) -> Vec<HostPkg> {
    vec![
        rut_std::math::pkg(),
        rut_std::nmap::pkg(),
        rut_std::strbuild::pkg(),
        rut_std::async_host::pkg(),
        http,
    ]
}

/// The declared world, as tree mounts: the std closure (pouch,
/// nmapset, json, the http pair) + rgh_host + the async pair + calc
/// (the Math bodies' decl surface) — every one a walked pkg riding
/// its real rut.jsonc manifest, so the declared closure covers the
/// deps' own host rows (json's strbuild pair included). The binary's
/// CHAIN still rides the manifest walk (the brain's source is the
/// walk's root; the pinned http zip offers the same names — the mount
/// door's sha256 law keeps the two spellings one world); its expected
/// table reads THIS, the same home the suite reads.
pub fn decl_world() -> Vec<rut_driver::Mounted> {
    let tree = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut");
    let mut pkgs = Vec::new();
    for d in ["pouch", "nmapset", "json", "http_host", "http"] {
        pkgs.extend(
            rut_native::dir_pkgs(&tree.join(d))
                .expect("mount tree pkg")
                .pkgs,
        );
    }
    pkgs.extend(
        rut_native::dir_pkgs(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("rgh_host"))
            .expect("mount rgh_host")
            .pkgs,
    );
    pkgs.extend(
        rut_native::dir_pkgs(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut/futures"),
        )
        .expect("the async pair walks")
        .pkgs,
    );
    pkgs.push(rut_native::tree_pkg("calc_host").expect("the toolchain tree's rut/calc_host"));
    pkgs
}

/// The declared host-fn table over [`decl_world`] — the raw rows'
/// contract check reads it (`HostRegistry::verify_against`).
pub fn expected() -> rut_vm::interp::ExpectedHostFns {
    rut_driver::declared_host_fns(&decl_world())
}

/// TRUE when the global dep store already holds every url row the
/// lock pins — the walk then resolves cache-only. A cold store would
/// push the mount to the network (or fail it offline); the boot gate
/// SKIPS on cold, it never fails for a cold store. (`rut install` in
/// the example dir warms it — the pin IS the address:
/// `<name>@<sha256[:16]>`.)
pub fn store_warm() -> bool {
    let lock_text =
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("rut.lock"))
            .expect("read rut.lock");
    let lock = rut_driver::parse_lock(&lock_text).expect("parse rut.lock");
    let deps = rut_driver::deps_dir();
    lock.deps().iter().all(|row| {
        std::path::Path::new(&deps)
            .join(rut_driver::store_dir(&row.pkg, &row.sha256))
            .is_dir()
    })
}
