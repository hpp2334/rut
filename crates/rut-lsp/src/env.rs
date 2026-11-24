//! The injected, READ-ONLY host environment — the capability contract
//! the dep walk (the dep-walk-into-crate plan) and the mod-name
//! completion slot's disk probe read through.
//!
//! The host hands `set_env` ONE object carrying exactly four
//! functions — the JS face's `HostEnv` (`js/index.d.ts`, the
//! crate-owned wire contract law):
//!
//! ```ts
//! interface HostEnv {
//!     read(path: string): Uint8Array | null;   // miss = null, never throws
//!     readDir(path: string): string[] | null;  // null = no dir
//!     isDir(path: string): boolean;
//!     notify(message: string, isError: boolean); // the hint lane
//! }
//! ```
//!
//! [`Io`] is the Rust-side spelling of that contract; its JS impl
//! lives in the wasm face (`rut-lsp-wasm`'s `JsEnv` — the host object
//! kept alive plus the four functions CACHED at `set_env`, plain
//! `js_sys::Function` calls per read — synchronous, no `JsFuture`
//! anywhere). The grant is read-only BY CONSTRUCTION: no write, no
//! delete, no fetch exists on the trait, so no handler can spell one.
//! Paths are rooted strings the crate joins with `/` (`std::path`
//! never enters — the host normalizes). A path may also arrive as a
//! `file://` doc uri: the mod-name slot's probe hands the open
//! document's own directory VERBATIM (the uri minus its last segment,
//! scheme kept) — the host resolves it (`fileURLToPath`) or answers
//! `null`, which is that slot's honest empty list.
//!
//! Native callers never build a `JsEnv` (there is no JS to reach) —
//! they hand the loop their own `Io` through [`crate::Lsp::set_env_io`],
//! the same door the face's `set_env` posts its `JsEnv` through. The
//! capability has TWO readers, both pure reads: the dep walk
//! (`crate::load::dep_walk`) and the mod-name slot's disk probe
//! (`crate::feature::completion::mod_name::list_child_mods`).
//!
//! # The clock (machinery, the one addition)
//!
//! Beside the four business reads the env carries exactly ONE
//! machinery capability: a dumb clock. [`Io::set_wake`] installs the
//! loop's wake source (a closure that posts one identity-free
//! `Msg::Wake` on the loop's own inbox — the actor's self-address)
//! and [`Io::arm_wake`] schedules its call `ms` out; the JS spelling
//! is the host's plain `setTimeout` behind an optional `wakeAfter(ms,
//! wake)` method. The physics stay true — wasm32 cannot wake itself,
//! only the host has a timer — but the host supplies POWER while the
//! ACTOR owns every WHEN: toward the host the actor only ever
//! ANSWERS. Nothing business-coupled is added (no callback carries a
//! business fact), and an env without the clock keeps the loop
//! clock-less: every deferral degrades to eager, in-turn dispatch.

use std::rc::Rc;

/// the loop's wake source — the closure the `SetEnv` arm installs on
/// the env: a clone of the loop's own inbox sender captured inside, so
/// firing it posts one identity-free `Msg::Wake` (`crate::actor::event_loop`).
/// Pure machinery; no business fact crosses it.
pub type WakeFn = Rc<dyn Fn()>;

/// the host's read-only world — the four calls the walk and the
/// mod-name probe need. A miss is `None`/`false`, never an error and
/// never a throw.
pub trait Io {
    /// one file's bytes — `None` on a miss
    fn read(&self, path: &str) -> Option<Vec<u8>>;
    /// one directory's entry names — `None` when there is no dir. A
    /// `path` may spell a `file://` doc uri (the mod-name probe hands
    /// the open document's directory verbatim): resolve it or answer
    /// `None` — the unresolved shape is an empty answer, never an
    /// error.
    fn read_dir(&self, path: &str) -> Option<Vec<String>>;
    /// does the path name a directory?
    fn is_dir(&self, path: &str) -> bool;
    /// the hint lane — a message for the user, error-flagged or not
    fn notify(&self, message: &str, is_error: bool);
    // ---- the clock (machinery only — the one capability addition) ----
    /// store the loop's wake source — the dumb clock's trigger. The
    /// default is unsupported: an env that never overrides it keeps
    /// the loop clock-less (and `arm_wake` below answers `false`, so
    /// every deferral degrades to eager dispatch).
    fn set_wake(&self, _wake: WakeFn) {}
    /// schedule the stored wake's call `ms` milliseconds out — the
    /// host's plain `setTimeout`, nothing more. `true` = the clock
    /// exists and the window armed; `false` = no clock (the caller
    /// runs NOW — the degrade). Default: unsupported.
    fn arm_wake(&self, _ms: u32) -> bool {
        false
    }
}
