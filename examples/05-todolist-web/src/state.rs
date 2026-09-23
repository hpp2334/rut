//! The turn law (survey §5.1): there is no microtask queue, nobody
//! polls, the host drives. Each DOM event or timer fire is ONE
//! `vm.call("on_event", …)` turn; between turns rut is inert and the
//! host holds only the event queue. `drain_queue` is that pump — the
//! same code the twin's tests and the wasm32 page run.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use rut_vm::interp::Vm;
use rut_vm::{OpaqueRef, Trap, TrapKind};

use crate::backend::DomBackend;

/// The frozen numeric event kinds (the survey §4.2 re-entry contract).
/// rut reads them as data; the host is the only writer.
pub const EV_DOM: i32 = 1;
pub const EV_TIMER: i32 = 2;

/// One asynchronous fact, host → rut: an `on_event` row.
pub struct WebEvent {
    pub kind: i32,
    pub subject: String,
    pub detail: String,
}

/// What fired: a DOM listener (by id) or a timer (by tag). The sink's
/// currency — closures push these, never callables (the closure law,
/// RFC 0025).
pub enum Ev {
    Dom(i64),
    Timer(String),
}

/// The enqueue side the backends hold: `Rc<dyn Fn(Ev)>` — pushing a row
/// of data, never a callable crossing into rut.
pub type EvSink = Rc<dyn Fn(Ev)>;

/// One registered listener: the element it watches and the event name.
pub struct ListenerRow<El> {
    pub el: El,
    pub event: String,
}

/// The host's whole page state: the backend, the queue, the listener
/// registry, the re-entrancy guard, and the app container. Single thread
/// (RFC 0034) — the guard is a plain bool.
pub struct WebState<D: DomBackend> {
    pub dom: D,
    pub queue: VecDeque<WebEvent>,
    pub listeners: HashMap<i64, ListenerRow<D::El>>,
    /// The app container: the ONE opaque the boot turn returned, handed
    /// back on every `on_event` turn (phase 2's shape — RFC 0003 §1's
    /// own law: rut has no mutable module state, so the store lives in
    /// the container the host passes back, the 00-todolist pattern).
    pub app: Option<OpaqueRef>,
    /// The pump's report surface (err-channel phase 3): every
    /// `on_event` turn's non-empty err component lands here — DATA the
    /// host reads and acts on (the wasm lane forwards each into its
    /// `rut_web_last_error` slot). A returned err is never a poison
    /// pill; a TRAPPED turn still aborts the pump loudly through its
    /// own channel.
    pub turned_errs: Vec<String>,
    next_listener: i64,
    pub in_turn: bool,
}

impl<D: DomBackend> WebState<D> {
    pub fn new(dom: D) -> WebState<D> {
        WebState {
            dom,
            queue: VecDeque::new(),
            listeners: HashMap::new(),
            app: None,
            turned_errs: Vec::new(),
            next_listener: 1, // listener ids are from 1 (the spec)
            in_turn: false,
        }
    }

    /// Allocate the next listener id — rut's dispatch key.
    pub fn next_listener_id(&mut self) -> i64 {
        let id = self.next_listener;
        self.next_listener += 1;
        id
    }

    /// A DOM listener fired: classify the row, attach the listened
    /// input's current value when the element IS an input, enqueue. A
    /// stale id is listener drift — a host-side internal error, panic,
    /// never a rut diagnostic (the same class as the RFC 0025 boot
    /// panics).
    pub fn push_dom_event(&mut self, listener: i64) {
        let el = match self.listeners.get(&listener) {
            Some(row) => row.el.clone(),
            None => panic!(
                "web: listener {listener} is not registered — host registry drift (an embedding bug, not a rut diagnostic)"
            ),
        };
        let detail = self.dom.current_value(&el).unwrap_or_default();
        self.queue.push_back(WebEvent { kind: EV_DOM, subject: listener.to_string(), detail });
    }

    /// A timer fired: the tag is the whole payload.
    pub fn push_timer(&mut self, tag: &str) {
        self.queue.push_back(WebEvent { kind: EV_TIMER, subject: tag.to_string(), detail: String::new() });
    }

    /// The sink's body: classify and enqueue.
    pub fn route(&mut self, ev: Ev) {
        match ev {
            Ev::Dom(id) => self.push_dom_event(id),
            Ev::Timer(tag) => self.push_timer(&tag),
        }
    }

    /// Pop the next queued row. THE RE-ENTRANCY GUARD: a pump while a
    /// turn is up would be a stack-dispatch — the one shape the turn
    /// law forbids. Events that fire mid-turn are QUEUE rows (phase 1's
    /// pick); reaching here through a live turn is our own glue bug,
    /// and the guard exists to make that choice visible.
    pub fn next_event(&mut self) -> Option<WebEvent> {
        if self.in_turn {
            panic!("web: event during a rut turn — events are queue, never stack");
        }
        self.queue.pop_front()
    }

    pub fn begin_turn(&mut self) {
        self.in_turn = true;
    }

    pub fn end_turn(&mut self) {
        self.in_turn = false;
    }
}

/// The pump, shared by the twin's [`crate::host::WebHost`] and the wasm32
/// page: drain the queue one turn at a time, guard up around every
/// `vm.call`, FIFO across turns. An event pushed mid-turn is absorbed by
/// the queue and runs as the NEXT turn — sequential, never stacked.
///
/// Every turn hands the app ITS container back (phase 2's entry shape,
/// `on_event(c, kind, subject, detail)`), and (err-channel phase 3) the
/// turn answers the entry-err pair `(?opaque, str)`:
///
/// * `Err` — a trapped turn: a BUG, wiring drift. The pump aborts LOUD
///   (`r?`), exactly as ever.
/// * `Ok((Some(c), err))` — a clean turn: the re-crossed container is
///   adopted, an empty err reported as nothing.
/// * `Ok((None, why))` — a SOFT FAILURE: the err is reported into
///   [`WebState::turned_errs`] and the pump KEEPS DRAINING. The nil
///   value channel does not poison the container — the host keeps the
///   one it holds (rut still owns it; the next turn runs), so the page
///   lives. The exactly-one-non-nil convention is the caller's law;
///   the pump reads it, never enforces it.
pub fn drain_queue<D: DomBackend>(
    state: &Rc<RefCell<WebState<D>>>,
    vm: &mut Vm,
) -> Result<(), Trap> {
    loop {
        let Some(ev) = state.borrow_mut().next_event() else {
            return Ok(());
        };
        let app = state.borrow().app.clone().ok_or_else(|| {
            Trap::new(
                TrapKind::Invalid,
                "web: no app container — the boot turn must return one (RFC 0003 §1: the state crosses, the host re-passes it)",
            )
        })?;
        state.borrow_mut().begin_turn();
        let r = vm.call::<_, (Option<OpaqueRef>, String)>("on_event", (app, ev.kind, ev.subject, ev.detail));
        state.borrow_mut().end_turn();
        let (crossed, err) = r?;
        if !err.is_empty() {
            // a returned err is DATA — report it and keep the queue live
            state.borrow_mut().turned_errs.push(err);
        }
        if let Some(c) = crossed {
            state.borrow_mut().app = Some(c);
        }
    }
}

/// A sink that routes through a weak handle back into the state — the
/// weak breaks the `state → dom → sink → state` cycle, so the page
/// state drops with its owner. The twin and the wasm32 boot both use
/// the slot (the wasm sink wraps this one with a pump).
pub fn weak_sink_slot<D: DomBackend>(
) -> (Rc<RefCell<Option<std::rc::Weak<RefCell<WebState<D>>>>>>, EvSink) {
    let slot: Rc<RefCell<Option<std::rc::Weak<RefCell<WebState<D>>>>>> =
        Rc::new(RefCell::new(None));
    let s = slot.clone();
    let sink: EvSink = Rc::new(move |ev: Ev| {
        if let Some(state) = s.borrow().as_ref().and_then(std::rc::Weak::upgrade) {
            state.borrow_mut().route(ev);
        }
    });
    (slot, sink)
}

/// Point a weak sink at its state (call once the `Rc` exists).
pub fn bind_weak_sink<D: DomBackend>(
    slot: &Rc<RefCell<Option<std::rc::Weak<RefCell<WebState<D>>>>>>,
    state: &Rc<RefCell<WebState<D>>>,
) {
    *slot.borrow_mut() = Some(Rc::downgrade(state));
}

/// Box a backend element as the opaque rut holds — `OpaqueBox<El>`: the
/// payload is invisible to rut (`o is opaque` holds, `downcast` misses),
/// released deterministically at rc 0 (RFC 0014/0016 §3).
pub fn box_element<D: DomBackend>(
    vm: &mut Vm,
    el: D::El,
) -> Result<OpaqueRef, Trap> {
    Ok(rut_vm::OpaqueBox::<D::El>::alloc(vm, el)?.handle().clone())
}

/// The uniform host-trap shape: `web::<fn>: <message>`.
pub fn web_trap(fn_name: &str, msg: impl std::fmt::Display) -> Trap {
    Trap::new(TrapKind::Invalid, format!("web::{fn_name}: {msg}"))
}
