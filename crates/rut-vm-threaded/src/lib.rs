//! `rut-vm-threaded` — tail-call threaded dispatch for the rut VM.
//!
//! This is the **only** crate that enables the nightly `become` /
//! `rust-preserve-none` features; everything downstream (notably `rut-vm`)
//! stays free of target cfgs and unstable attributes.
//!
//! The backend is selected by the `rut_threaded` capability cfg emitted by
//! `build.rs` (native x86_64/aarch64 — see the module docs there):
//!
//! - [`native`] — `become`-threaded handlers: each op ends by replacing its
//!   frame with the next handler's, so dispatch is a direct/indirect
//!   tail-jump and hot state lives in registers.
//! - [`wasm`] — a portable loop over the same [`Machine`] methods, used on
//!   wasm and any other target without verified tail calls.
//!
//! Both backends call the **same** `Machine` op methods, so behavior is
//! identical; only the dispatch shape differs.

#![cfg_attr(rut_threaded, feature(explicit_tail_calls, rust_preserve_none_cc))]
#![cfg_attr(rut_threaded, allow(incomplete_features))]

mod api;
pub use api::Machine;

#[cfg(rut_threaded)]
mod native;
#[cfg(rut_threaded)]
pub use native::run;

#[cfg(not(rut_threaded))]
mod wasm;
#[cfg(not(rut_threaded))]
pub use wasm::run;
