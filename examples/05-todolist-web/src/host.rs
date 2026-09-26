//! The native page owner: the twin's `Vm` plus its [`WebState`], with
//! the pump (`drain_queue`), the fire/advance entries the tests drive,
//! and the guard simulation for the re-entrancy test. wasm32 never
//! compiles this module — the page's pump there is `web_dom::pump_page`
//! over thread-locals (single thread, RFC 0034, both shapes legal).

use std::cell::RefCell;
use std::rc::Rc;

use rut_vm::interp::{CallArgs, Ret, Vm};
use rut_vm::{OpaqueRef, Trap};

use crate::backend::DomBackend;
use crate::fake_dom::FakeDom;
use crate::state::{drain_queue, WebState};

pub struct WebHost<D: DomBackend> {
    pub state: Rc<RefCell<WebState<D>>>,
    vm: Vm,
}

impl<D: DomBackend> WebHost<D> {
    pub fn new(state: Rc<RefCell<WebState<D>>>, vm: Vm) -> WebHost<D> {
        WebHost { state, vm }
    }

    /// A typed call into the app — the boot turn (`main`) and the
    /// harness probes ride this.
    pub fn call<A: CallArgs, R: Ret>(&mut self, export: &str, args: A) -> Result<R, Trap> {
        self.vm.call(export, args)
    }

    /// The boot turn: `main` builds the static DOM, registers the
    /// listeners, and RETURNS the app container — the pump hands it back
    /// on every turn (RFC 0003 §1: the state crosses). Afterwards the
    /// page is purely event-driven.
    pub fn boot(&mut self) -> Result<OpaqueRef, Trap> {
        let app: OpaqueRef = self.vm.call::<_, OpaqueRef>("main", ())?;
        self.state.borrow_mut().app = Some(app.clone());
        Ok(app)
    }

    /// Drain the queue (the pump): one turn per row, guard up around
    /// every call, FIFO across turns.
    pub fn pump(&mut self) -> Result<(), Trap> {
        drain_queue::<D>(&self.state, &mut self.vm)
    }

    /// Fire a DOM listener now — the twin's `Closure` stand-in — then
    /// pump. A stale id panics in the registry (listener drift).
    pub fn fire_listener(&mut self, listener: i64) -> Result<(), Trap> {
        self.state.borrow_mut().push_dom_event(listener);
        self.pump()
    }

    /// The re-entrancy simulation: a synchronous DOM dispatch inside a
    /// live turn (the real DOM's `focus()`/`click()` class). While this
    /// wrapper holds the guard up, any pump is the forbidden
    /// stack-dispatch (guard panic) and events can only queue.
    pub fn in_sync_dom_dispatch<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.state.borrow_mut().begin_turn();
        let r = f(self);
        self.state.borrow_mut().end_turn();
        r
    }

    /// Read access into the backend for assertions.
    pub fn with_dom<T>(&self, f: impl FnOnce(&D) -> T) -> T {
        f(&self.state.borrow().dom)
    }
}

/// The twin-specific drive: the virtual timer clock lives on the
/// backend, so `advance` is concrete (wasm32 has no twin — the page's
/// timers come from real JS tasks).
impl WebHost<FakeDom> {
    /// Advance the twin's virtual clock, route every due timer, and
    /// pump: the tim_after/on_timer round trip, synchronously and
    /// deterministically.
    pub fn advance(&mut self, ms: i64) -> Result<(), Trap> {
        // collect under the borrow (the clock tick fires nothing), then
        // route with the borrow dropped — the same shape the real page
        // gets for free: JS tasks never run mid-statement
        let due = self.state.borrow_mut().dom.advance(ms);
        for ev in due {
            self.state.borrow_mut().route(ev);
        }
        self.pump()
    }
}
