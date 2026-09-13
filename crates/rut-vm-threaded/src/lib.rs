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
//!   frame with the next handler's, so dispatch is a tail jump and hot state
//!   stays in registers.
//! - [`wasm`] — a portable loop over the same [`Machine`] methods, for wasm
//!   and any other target without verified tail calls.
//!
//! Both backends call the **same** `Machine::op_*` methods, so behavior is
//! identical; only the dispatch shape differs. Unthreaded ops return
//! [`ThreadOut::Bail`] with the pc left at that op so the VM's match
//! interpreter takes over.
//!
//! **Status:** the mechanism (Phase 0) is validated, but wiring it into
//! `rut-vm` (Phase 1) regressed every workload, so `rut-vm` still uses its
//! match loop. See `README.md` for the measurements and what a winning
//! design needs (thread hot state as arguments; full op coverage).

#![cfg_attr(rut_threaded, feature(explicit_tail_calls, rust_preserve_none_cc))]
#![cfg_attr(rut_threaded, allow(incomplete_features))]

mod api;
pub use api::{tag_of, Flow, Machine, Table, ThreadOut, NTAGS};

pub mod dispatch_bench;

#[cfg(rut_threaded)]
mod native;
#[cfg(rut_threaded)]
pub use native::{build_table, run};

#[cfg(not(rut_threaded))]
mod wasm;
#[cfg(not(rut_threaded))]
pub use wasm::{build_table, run};
