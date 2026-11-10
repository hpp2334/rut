//! The injected, READ-ONLY host environment — the capability contract
//! the dep walk (the dep-walk-into-crate plan) reads through.
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
//! [`Io`] is the Rust-side spelling of that contract and [`JsEnv`] is
//! its JS impl: the host object kept alive plus the four functions
//! CACHED at `set_env`, plain `js_sys::Function` calls per read —
//! synchronous, no `JsFuture` anywhere. The grant is read-only BY
//! CONSTRUCTION: no write, no delete, no fetch exists on the trait,
//! so no handler can spell one. Paths are rooted strings the crate
//! joins with `/` (`std::path` never enters — the host normalizes).
//!
//! [`JsEnv`] is constructed only through the `set_env` bridge; native
//! tests never build one (they hand the loop their own `Io` through
//! the native `set_env_io` seam). The dep walk (`crate::walk`) is the
//! capability's one reader.
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

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};

/// the loop's wake source — the closure the `SetEnv` arm installs on
/// the env: a clone of the loop's own inbox sender captured inside, so
/// firing it posts one identity-free `Msg::Wake` (`crate::event_loop`).
/// Pure machinery; no business fact crosses it.
pub type WakeFn = Rc<dyn Fn()>;

/// the host's read-only world — the four calls the walk needs. A miss
/// is `None`/`false`, never an error and never a throw.
pub trait Io {
    /// one file's bytes — `None` on a miss
    fn read(&self, path: &str) -> Option<Vec<u8>>;
    /// one directory's entry names — `None` when there is no dir
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

/// the JS host object behind [`Io`]: the object itself (kept alive —
/// its functions may close over it) plus the functions cached at
/// construction. A missing or non-callable method degrades to its
/// miss shape (`read` → `None`, `read_dir` → `None`, `is_dir` →
/// `false`, `notify` → a no-op, `wakeAfter` → no clock) — the
/// capability stays total, never a throw at the trait boundary.
pub struct JsEnv {
    obj: js_sys::Object,
    read: Option<js_sys::Function>,
    read_dir: Option<js_sys::Function>,
    is_dir: Option<js_sys::Function>,
    notify: Option<js_sys::Function>,
    /// the host's clock, when the object carries one — `wakeAfter(ms,
    /// wake)`, cached like the four business reads
    wake_after: Option<js_sys::Function>,
    /// the ONE wake `Closure` — built at the single `set_wake` call
    /// and kept for the session, never rebuilt (a rebuild would drop
    /// the closure a live JS timer may still hold). `RefCell` because
    /// `Io` methods take `&self`; the write happens once, at install.
    wake: RefCell<Option<Closure<dyn FnMut()>>>,
}

impl JsEnv {
    /// cache the functions off the host object — the ONLY
    /// constructor, called by the `set_env` bridge (native tests never
    /// build one; there is no JS to reach)
    pub(crate) fn new(obj: js_sys::Object) -> JsEnv {
        let method = |name: &str| {
            js_sys::Reflect::get(&obj, &JsValue::from_str(name))
                .ok()
                .and_then(|v| v.dyn_ref::<js_sys::Function>().cloned())
        };
        let read = method("read");
        let read_dir = method("readDir");
        let is_dir = method("isDir");
        let notify = method("notify");
        let wake_after = method("wakeAfter");
        JsEnv {
            obj,
            read,
            read_dir,
            is_dir,
            notify,
            wake_after,
            wake: RefCell::new(None),
        }
    }
}

impl Io for JsEnv {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        let f = self.read.as_ref()?;
        let v = f.call1(&self.obj, &JsValue::from_str(path)).ok()?;
        if v.is_null() || v.is_undefined() {
            return None; // the miss shape — null, never a throw
        }
        // a non-Uint8Array return degrades to the miss shape, never a
        // panic: the cast is CHECKED (`dyn_ref`), not assumed — reads
        // begin at the dep walk, so a hostile/sloppy host object must
        // not take the core down
        v.dyn_ref::<js_sys::Uint8Array>().map(|a| a.to_vec())
    }

    fn read_dir(&self, path: &str) -> Option<Vec<String>> {
        let f = self.read_dir.as_ref()?;
        let v = f.call1(&self.obj, &JsValue::from_str(path)).ok()?;
        if v.is_null() || v.is_undefined() {
            return None; // no dir
        }
        // same law: a non-Array return degrades to no-dir, and a
        // non-string entry skips (never a guessed name)
        let arr = v.dyn_ref::<js_sys::Array>()?;
        Some(arr.iter().filter_map(|e| e.as_string()).collect())
    }

    fn is_dir(&self, path: &str) -> bool {
        let Some(f) = self.is_dir.as_ref() else {
            return false;
        };
        // a non-boolean return degrades to false — the total-function
        // law every method of this impl keeps
        f.call1(&self.obj, &JsValue::from_str(path))
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    fn notify(&self, message: &str, is_error: bool) {
        if let Some(f) = self.notify.as_ref() {
            let _ = f.call2(
                &self.obj,
                &JsValue::from_str(message),
                &JsValue::from_bool(is_error),
            );
        }
    }

    fn set_wake(&self, wake: WakeFn) {
        // ONE closure for the whole session: the `Rc` moves inside, and
        // every later `arm_wake` hands JS the SAME callback object —
        // the host's `setTimeout` never sees a rebuilt closure.
        let c = Closure::new(move || (*wake)());
        *self.wake.borrow_mut() = Some(c);
    }

    fn arm_wake(&self, ms: u32) -> bool {
        // no clock on the host object, or the wake source never
        // installed (SetEnv not run) ⇒ false ⇒ the caller degrades to
        // eager — the total-function law every method of this impl keeps
        let Some(f) = self.wake_after.as_ref() else {
            return false;
        };
        let guard = self.wake.borrow();
        let Some(c) = guard.as_ref() else {
            return false;
        };
        f.call2(
            &self.obj,
            &JsValue::from_f64(f64::from(ms)),
            c.as_js_value(),
        )
        .is_ok()
    }
}
