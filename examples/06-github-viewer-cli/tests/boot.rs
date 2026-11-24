//! The gate that was missing: the binary must BOOT under test.
//!
//! The example's world used to be spelled twice — the suite's copy
//! (right, and the one the tests ran) and the binary's copy (wrong,
//! and the one nothing ran) — so `cargo run -p rgh` panicked at the
//! eager decl↔bodies join (exit 101: `host fn … is declared by a
//! mounted package but never bound`) while every offline test stayed
//! green. This gate runs the BUILT binary with no args: a working rgh
//! answers the brain's usage text and exits 2 (the brain's usage
//! law); a boot panic is exit 101 and fails here.
//!
//! Skips — never fails — when the global dep store is cold for the
//! pinned bundle (the walk would reach for the network); `rut
//! install` in the example dir warms it, and on a warm store the
//! walk is cache-only, so the gate still networks at nothing.

#[path = "../src/world.rs"]
mod world;

#[test]
fn the_binary_boots_no_args_is_usage_exit_two() {
    if !world::store_warm() {
        eprintln!(
            "the_binary_boots_no_args_is_usage_exit_two: skipped (the global \
             dep store is cold for the pinned bundle — `rut install` in \
             examples/06-github-viewer-cli warms it)"
        );
        return;
    }
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_rgh"))
        .output()
        .expect("run the built rgh");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(2),
        "the binary must boot to the brain's usage law (a boot panic is \
         exit 101) — stderr:\n{err}"
    );
    assert!(
        err.contains("usage: rgh --repo="),
        "no args answers the usage text — stderr:\n{err}"
    );
}
