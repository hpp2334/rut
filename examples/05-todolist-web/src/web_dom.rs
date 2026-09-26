//! The wasm32 half: the SAME `web.d.rut` surface bound over web_sys —
//! the real page. The backend is thin (one web call per crossing, the
//! survey §4.2 bodies verbatim); the page's brain is the shared state:
//! the queue, the turn law, the re-entrancy guard live in
//! [`crate::state`], identical to the twin the tests run.
//!
//! Raw exports (the `rut-wasm` ABI pattern; the loader is the page
//! shell's `loader.js`):
//!
//! ```text
//! rut_web_alloc(len) -> ptr                    hand the JS side a source buffer
//! rut_web_boot(ptr, len) -> i32                mount, bind, verify, boot `main` — 0 | -1
//! rut_web_pump() -> i32                        drain the event queue — 0 | -1
//! rut_web_last_error() -> ptr                  [u32 le length][bytes], once
//! ```
//!
//! The Vm and the page state live in thread-locals (single thread, RFC
//! 0034): DOM callbacks and timer callbacks fire from JS tasks, push
//! one event row through the sink, and pump only when no turn is up —
//! an event firing mid-turn is absorbed by the queue and runs as the
//! NEXT turn, never stacked.
//!
//! Documented limitation: listener/timer `Closure`s are owned by the
//! registry for the page's life — detaching is out of scope for phase 1.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use wasm_bindgen::prelude::{Closure, JsCast, JsValue};

use rut_vm::interp::{HostHooks, HostRegistry, Vm};
use rut_vm::OpaqueRef;

use crate::backend::DomBackend;
use crate::state::{bind_weak_sink, drain_queue, Ev, EvSink, WebState};

/// The boxed element handle rut holds — the web_sys handle itself, so
/// its `Drop` at rc 0 releases the JS reference deterministically
/// (RFC 0016 §3).
#[derive(Clone)]
pub struct El {
    pub el: web_sys::Element,
}

type StateRc = Rc<RefCell<WebState<WebDom>>>;
type WeakSlot = Rc<RefCell<Option<Weak<RefCell<WebState<WebDom>>>>>>;

/// The DOM exception's message, carried — never silently swallowed.
fn js_err(v: JsValue) -> String {
    js_sys::Error::from(v).to_string().into()
}

pub struct WebDom {
    sink: EvSink,
    window: web_sys::Window,
    document: web_sys::Document,
    /// the registry owns its Closures for the page's life
    listener_closures: RefCell<Vec<(i64, Closure<dyn FnMut()>)>>,
    timer_closures: RefCell<Vec<Closure<dyn FnMut()>>>,
}

impl WebDom {
    pub fn new(sink: EvSink, window: web_sys::Window, document: web_sys::Document) -> WebDom {
        WebDom {
            sink,
            window,
            document,
            listener_closures: RefCell::new(Vec::new()),
            timer_closures: RefCell::new(Vec::new()),
        }
    }
}

impl DomBackend for WebDom {
    type El = El;

    fn get(&self, id: &str) -> Result<El, String> {
        self.document
            .get_element_by_id(id)
            .map(|el| El { el })
            .ok_or_else(|| format!("no element '#{id}'"))
    }

    fn create(&mut self, tag: &str) -> Result<El, String> {
        self.document.create_element(tag).map(|el| El { el }).map_err(js_err)
    }

    fn set_text(&mut self, el: &El, text: &str) -> Result<(), String> {
        el.el.set_text_content(Some(text)); // infallible per WebIDL
        Ok(())
    }

    fn attr(&mut self, el: &El, name: &str, value: &str) -> Result<(), String> {
        el.el.set_attribute(name, value).map_err(js_err)
    }

    fn append(&mut self, parent: &El, child: &El) -> Result<(), String> {
        parent.el.append_child(&child.el).map(|_| ()).map_err(js_err)
    }

    fn remove(&mut self, parent: &El, child: &El) -> Result<bool, String> {
        // the DOM negative — NotFound maps to `false`, not a trap
        Ok(parent.el.remove_child(&child.el).is_ok())
    }

    fn clear(&mut self, el: &El) -> Result<(), String> {
        let node: &web_sys::Node = &el.el;
        while let Some(child) = node.first_child() {
            node.remove_child(&child).map_err(js_err)?;
        }
        Ok(())
    }

    fn current_value(&self, el: &El) -> Option<String> {
        el.el
            .dyn_ref::<web_sys::HtmlInputElement>()
            .map(|input| input.value())
    }

    fn set_input_value(&mut self, el: &El, value: &str) -> Result<(), String> {
        match el.el.dyn_ref::<web_sys::HtmlInputElement>() {
            Some(input) => {
                input.set_value(value);
                Ok(())
            }
            None => Err(format!(
                "boundary: got `{}` where `HtmlInputElement` binds",
                el.el.tag_name().to_lowercase()
            )),
        }
    }

    fn listen(&mut self, el: &El, event: &str, id: i64) -> Result<(), String> {
        let sink = self.sink.clone();
        let cb: Closure<dyn FnMut()> = Closure::new(move || sink(Ev::Dom(id)));
        el.el
            .add_event_listener_with_callback(event, cb.as_ref().unchecked_ref())
            .map_err(js_err)?;
        self.listener_closures.borrow_mut().push((id, cb));
        Ok(())
    }

    fn after(&mut self, ms: i64, tag: &str) -> Result<(), String> {
        let sink = self.sink.clone();
        let tag = tag.to_string();
        let cb: Closure<dyn FnMut()> = Closure::new(move || sink(Ev::Timer(tag.clone())));
        let wait = ms.clamp(0, i32::MAX as i64) as i32; // setTimeout's own clamp
        self.window
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                cb.as_ref().unchecked_ref(),
                wait,
            )
            .map_err(js_err)?;
        self.timer_closures.borrow_mut().push(cb);
        Ok(())
    }
}

// ---- the page's thread-locals (single thread, RFC 0034) ----

thread_local! {
    static VM: RefCell<Option<Vm>> = const { RefCell::new(None) };
    static STATE: RefCell<Option<StateRc>> = const { RefCell::new(None) };
    static LAST_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn record_error(msg: &str) {
    LAST_ERROR.with(|e| *e.borrow_mut() = Some(msg.to_string()));
}

/// The sink's pump half: drain now when idle; absorb when a turn is up
/// (the live pump owns the queue — events are queue, never stack).
fn pump_if_idle(state: &StateRc) {
    if state.borrow().in_turn {
        return;
    }
    VM.with(|cell| {
        // try_borrow: if a turn holds the Vm, the guard above already
        // routed us here never — this is belt over suspenders
        if let Ok(mut slot) = cell.try_borrow_mut() {
            if let Some(vm) = slot.as_mut() {
                if let Err(t) = drain_queue::<WebDom>(state, vm) {
                    record_error(&t.msg);
                }
            }
        }
    });
    // soft-fail turns land in `turned_errs` (the pump keeps draining);
    // this lane's one error surface is the last_error slot, so each
    // reported err rides it too — read-once, the envelope pattern
    let errs: Vec<String> = std::mem::take(&mut state.borrow_mut().turned_errs);
    for e in errs {
        record_error(&e);
    }
}

/// Build the page state (sink → dom → state, weakly closed) and stash
/// it; the sink pushes AND pumps — the wasm32 shape of the turn law.
fn page_state() -> Result<StateRc, String> {
    let window = web_sys::window().ok_or("no global `window`")?;
    let document = window.document().ok_or("no `document`")?;
    let slot: WeakSlot = Rc::new(RefCell::new(None));
    let pump_sink: EvSink = {
        let slot = slot.clone();
        Rc::new(move |ev: Ev| {
            if let Some(state) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                state.borrow_mut().route(ev);
                pump_if_idle(&state);
            }
        })
    };
    let dom = WebDom::new(pump_sink, window, document);
    let state = Rc::new(RefCell::new(WebState::new(dom)));
    bind_weak_sink(&slot, &state);
    STATE.with(|s| *s.borrow_mut() = Some(state.clone()));
    Ok(state)
}

/// Mount (std core + pouch inline + the `web` surface), compile, bind,
/// `verify_against`, boot `Vm::new` + the `main` turn. After this the
/// page is purely event-driven — the host owns the loop.
fn boot_page(src: &str) -> Result<(), String> {
    let mut session = rut_driver::Session::new();
    // THE MIRROR (rut/rut.toml by hand — the Session is I/O-free):
    // every package the manifest names; the loader hands over the biz
    // module's spliced source (base + entry.libs, RFC 0041 §5)
    crate::mount::mount_app_session(&mut session)?;
    let prog = crate::mount::compile_app(&mut session, src)?;
    let expected = session.expected_host_fns();

    let state = page_state()?;
    let mut hosts = HostRegistry::new();
    crate::hosts::install_web_hosts(&mut hosts, &state);
    // the app session mounts `nmap_host` (the listener table rides
    // the val-column row `HashMap<str, i64>`) — its bodies bind here,
    // same join, before verify
    rut_std::nmap::install_std_nmap(&mut hosts);
    hosts.verify_against(&expected);

    let mut vm = Vm::new(
        Rc::new(prog),
        &crate::mount::limits(),
        HostHooks::default(),
        hosts,
    )
    .map_err(|t| format!("vm boot: {}", t.msg))?;
    // the boot turn returns the app container; the pump re-passes it on
    // every on_event turn (RFC 0003 §1 — no mutable module state)
    let app: OpaqueRef = vm
        .call::<_, OpaqueRef>("main", ())
        .map_err(|t| format!("the boot turn trapped: {}", t.msg))?;
    state.borrow_mut().app = Some(app);
    VM.with(|c| *c.borrow_mut() = Some(vm));
    Ok(())
}

// ---- the raw ABI ----

/// Hand the loader a buffer for the app source (boot is one-shot; the
/// allocation leaks with the page).
#[no_mangle]
pub extern "C" fn rut_web_alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// Compile + boot the page with the app source at `[ptr, ptr+len)`.
/// Returns 0, or -1 with `rut_web_last_error` filled.
#[no_mangle]
pub extern "C" fn rut_web_boot(src_ptr: *const u8, src_len: usize) -> i32 {
    let bytes = unsafe { core::slice::from_raw_parts(src_ptr, src_len) };
    let src = match core::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => {
            record_error(&format!("the app source is not utf-8: {e}"));
            return -1;
        }
    };
    match boot_page(src) {
        Ok(()) => 0,
        Err(e) => {
            record_error(&e);
            -1
        }
    }
}

/// Drain the event queue now (the loader never needs this — callbacks
/// pump themselves; exported for tests and for a polling loader).
#[no_mangle]
pub extern "C" fn rut_web_pump() -> i32 {
    let Some(state) = STATE.with(|s| s.borrow().clone()) else {
        record_error("pump before boot");
        return -1;
    };
    pump_if_idle(&state);
    0
}

/// The last error as `[u32 le length][bytes]` at the returned pointer
/// (the `rut-wasm` envelope pattern); read once, then null.
#[no_mangle]
pub extern "C" fn rut_web_last_error() -> *mut u8 {
    let msg = LAST_ERROR.with(|e| e.borrow_mut().take()).unwrap_or_default();
    let bytes = msg.into_bytes();
    let mut buf: Vec<u8> = Vec::with_capacity(4 + bytes.len());
    buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(&bytes);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}
