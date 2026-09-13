//! Capability detection for the threaded backend.
//!
//! `cfg(rut_threaded)` is the single positive capability the crate keys off:
//! the target must have guaranteed tail calls (nightly `become`) and the
//! `rust-preserve-none` ABI we rely on for register-threaded dispatch.
//!
//! wasm has tail calls (`+tail-call`) but the JITs lower the threaded shape
//! 1.2–4.6× *slower* than a match loop, so wasm deliberately uses the
//! portable loop backend instead. Other architectures take the same loop
//! until `become` lowering is verified there.

fn main() {
    println!("cargo:rustc-check-cfg=cfg(rut_threaded)");

    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let family = std::env::var("CARGO_CFG_TARGET_FAMILY").unwrap_or_default();

    let threaded = family != "wasm" && (arch == "x86_64" || arch == "aarch64");
    if threaded {
        println!("cargo:rustc-cfg=rut_threaded");
    }

    println!("cargo:rerun-if-changed=build.rs");
}
