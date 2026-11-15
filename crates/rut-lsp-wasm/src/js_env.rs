//! The JS impl of the core's [`rut_lsp::env::Io`] — the host object
//! kept alive plus its functions cached at `set_env`.
//!
//! A missing or non-callable method degrades to its miss shape (`read`
//! → `None`, `read_dir` → `None`, `is_dir` → `false`, `notify` → a
//! no-op, `wakeAfter` → no clock) — the capability stays total, never
//! a throw at the trait boundary. The full contract (the `HostEnv`
//! interface, the read-only law, the clock) is spelled on
//! `rut_lsp::env`; this file is the JS spelling of it, nothing more.

use std::cell::RefCell;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};

/// the JS host object behind [`rut_lsp::env::Io`]: the object itself
/// (kept alive — its functions may close over it) plus the functions
/// cached at construction.
pub(crate) struct JsEnv {
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
    /// constructor, called by the `set_env` bridge (native callers
    /// never build one; there is no JS to reach)
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

impl rut_lsp::env::Io for JsEnv {
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

    fn set_wake(&self, wake: rut_lsp::env::WakeFn) {
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
