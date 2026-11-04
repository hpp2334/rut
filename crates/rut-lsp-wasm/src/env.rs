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

use wasm_bindgen::{JsCast, JsValue};

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
}

/// the JS host object behind [`Io`]: the object itself (kept alive —
/// its functions may close over it) plus the four functions cached at
/// construction. A missing or non-callable method degrades to its
/// miss shape (`read` → `None`, `read_dir` → `None`, `is_dir` →
/// `false`, `notify` → a no-op) — the capability stays total, never a
/// throw at the trait boundary.
pub struct JsEnv {
    obj: js_sys::Object,
    read: Option<js_sys::Function>,
    read_dir: Option<js_sys::Function>,
    is_dir: Option<js_sys::Function>,
    notify: Option<js_sys::Function>,
}

impl JsEnv {
    /// cache the four functions off the host object — the ONLY
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
        JsEnv {
            obj,
            read,
            read_dir,
            is_dir,
            notify,
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
}
