//! todolist-web — phase 1 of the `todolist-web` batch: the HOST WEB-API
//! SURFACE only, no app logic (the todolist app is phase 2). The `web`
//! pkg is example-local (survey §4): `web.d.rut` declares the ten
//! crossings; this crate binds the bodies twice —
//!
//! - **wasm32** ([`web_dom`]): the real page, web_sys behind
//!   target-gating (the workspace's FIRST wasm-bindgen-family dep),
//!   raw exports `rut_web_boot` / `rut_web_pump` / `rut_web_last_error`
//!   / `rut_web_alloc` (the `rut-wasm` ABI pattern);
//! - **host** ([`fake_dom`], native only): the fake-DOM twin — the SAME
//!   surface over a `HashMap`-backed tree with the same trap shapes and
//!   a scripted timer queue — so `cargo test --workspace` stays a real
//!   gate on native.
//!
//! Shared, target-independent: the registry bindings
//! ([`hosts`], RFC 0025), the turn law ([`state`]: one door per event
//! fn, the queue, the re-entrancy guard), the session mount
//! ([`mount`]). Thinness law: no app names, no data shipping, no
//! scheduler — the host knows nothing about todos.

pub mod backend;
pub mod hosts;
pub mod mount;
pub mod state;

#[cfg(not(target_arch = "wasm32"))]
pub mod fake_dom;
#[cfg(not(target_arch = "wasm32"))]
pub mod host;
#[cfg(target_arch = "wasm32")]
pub mod web_dom;

pub use backend::DomBackend;
pub use state::{Ev, EvSink, WebEvent, WebState};
