//! 03-plugin — the host side of the chat-server example, as a small SDK.
//!
//! The layering (see README): rut's business logic is a plain class
//! (`Moderator`), the crossing protocol lives in ~15 lines of `entry fn`
//! shims, and this side mirrors it — [`Plugin`] is the typed embedder
//! surface; `vm.call` happens in exactly one place (`Plugin::fire`),
//! plus the `emit` host fn's nested render call (RFC 0022 §1).
//!
//! Both opaque directions meet here: rut's `Moderator` state comes back
//! as a rut-constructed `opaque` (RFC 0014), the host's event bus goes
//! in as a host-constructed [`Opaque`]`<EventBus>` — `subscribe` and
//! `emit` are that box's callbacks, reached only through the RFC 0023
//! borrow guards.

use std::collections::HashMap;
use std::rc::Rc;

use rut_vm::interp::{CallArg, HostHooks, Limits, Vm};
use rut_vm::{Opaque, OpaqueRef, Trap, TrapKind};

/// The host's server object — handed to rut as an opaque box. All host
/// state lives here; the host fns reach it only through the `with`/
/// `with_mut` guards, so no host state is captured in closures at all.
struct EventBus {
    subscriptions: HashMap<String, String>, // topic -> rut export name
    lines: Vec<String>,                     // the transcript, in order
    emits: u64,
    renders: u64, // nested vm.call count — the re-entrancy proof
}

/// The embedder's handle to a loaded plugin — the only owner of the VM
/// and the two Opaques. Business code sees typed methods, never
/// `vm.call`.
pub struct Plugin {
    vm: Vm,
    bus: Opaque<EventBus>,
    state: OpaqueRef, // rut's `Moderator`, opaque to us
    table: HashMap<String, String>, // subscription snapshot from `init`
}

impl Plugin {
    /// Load the plugin module from a module directory (`rut.toml`) or a
    /// packed `.rutbundle` (RFC 0038) — the two forms of the same
    /// contract; the root spec comes from the manifest, not the caller.
    /// `init` registers the callback names; the table is frozen from
    /// then on.
    pub fn load(path: &std::path::Path, limits: &Limits) -> Result<Plugin, Trap> {
        let (mut session, root) =
            rut_driver::load_path_session(path).map_err(|e| Trap::new(TrapKind::Invalid, e))?;
        // the embedder mounts what the plugin uses: `core` only —
        // `server` and `pouch` resolved from the plugin manifest's
        // `[deps]` (the server surface is server/server.d.rut, no
        // hand-written Rust surface). Mounting `calc` would DECLARE its
        // host fns, and the load-time contract (below) would rightly
        // demand their bodies.
        rut_driver::mount_std_core(&mut session);
        let g = rut_driver::compile_graph(&session, &root);
        if !g.diags.is_empty() {
            return Err(Trap::new(
                TrapKind::Invalid,
                g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n"),
            ));
        }
        let prog = g
            .program
            .ok_or_else(|| Trap::new(TrapKind::Invalid, "compile produced no program"))?;
        rut_vm::verify::verify(&prog).map_err(|m| Trap::new(TrapKind::Invalid, m))?;
        // bindings BEFORE the Vm (RFC 0025): the .d.rut surface and the
        // bound bodies must agree — a mismatch panics HERE, never mid-run
        let mut hosts = rut_vm::interp::HostRegistry::new();
        install(&mut hosts);
        hosts.verify_against(&session.expected_host_fns());
        let mut vm = Vm::new(Rc::new(prog), limits, HostHooks::default(), hosts)?;

        // the handshake: bus in, state out
        let bus = Opaque::alloc(
            &mut vm,
            EventBus { subscriptions: HashMap::new(), lines: Vec::new(), emits: 0, renders: 0 },
        )?;
        let state: OpaqueRef = vm.call("init", (bus.handle().clone(),))?;
        let table = bus.with(|b| b.subscriptions.clone())?;
        Ok(Plugin { vm, bus, state, table })
    }

    /// Dispatch one event to its subscribed rut handler. Unknown topics
    /// are dropped, not trapped — a server keeps running when a plugin
    /// didn't subscribe. (The old dynamic `fire(&[Value])` became these
    /// per-arity typed dispatchers — the plan's one dynamic call site.)
    fn dispatch(&mut self, topic: &str) -> Result<(), Trap> {
        match self.table.get(topic).cloned() {
            Some(handler) => self.vm.call::<_, ()>(&handler, (self.state.clone(),)),
            None => {
                println!("[server] no handler for `{topic}` — dropped");
                Ok(())
            }
        }
    }
    fn dispatch1<A: CallArg>(&mut self, topic: &str, arg: A) -> Result<(), Trap> {
        match self.table.get(topic).cloned() {
            Some(handler) => {
                self.vm
                    .call::<_, ()>(&handler, (self.state.clone(), arg))
            }
            None => {
                println!("[server] no handler for `{topic}` — dropped");
                Ok(())
            }
        }
    }
    fn dispatch2<A1: CallArg, A2: CallArg>(
        &mut self,
        topic: &str,
        a1: A1,
        a2: A2,
    ) -> Result<(), Trap> {
        match self.table.get(topic).cloned() {
            Some(handler) => self
                .vm
                .call::<_, ()>(&handler, (self.state.clone(), a1, a2)),
            None => {
                println!("[server] no handler for `{topic}` — dropped");
                Ok(())
            }
        }
    }

    // ---- the typed surface: what a driver or test actually uses ----

    pub fn join(&mut self, name: &str) -> Result<(), Trap> {
        self.dispatch1("join", name)
    }

    pub fn leave(&mut self, name: &str) -> Result<(), Trap> {
        self.dispatch1("leave", name)
    }

    pub fn msg(&mut self, user: &str, text: &str) -> Result<(), Trap> {
        self.dispatch2("msg", user, text)
    }

    pub fn tick(&mut self, n: i64) -> Result<(), Trap> {
        self.dispatch1("tick", n)
    }

    /// no handler: logged, no trap — the server survives a plugin that
    /// didn't subscribe (kept as its own entry point for the demo)
    pub fn unknown(&mut self) -> Result<(), Trap> {
        self.dispatch("bogus")
    }

    /// Final stats emit; returns the plugin's total message count.
    pub fn shutdown(&mut self) -> Result<i64, Trap> {
        self.vm.call("shutdown", (self.state.clone(),))
    }

    /// Everything the plugin emitted, in order.
    pub fn transcript(&self) -> Vec<String> {
        self.bus.with(|b| b.lines.clone()).expect("bus free after calls return")
    }

    /// `(emits, nested renders)` — equal counts prove every line crossed
    /// the re-entrant call.
    pub fn stats(&self) -> (u64, u64) {
        self.bus.with(|b| (b.emits, b.renders)).expect("bus free after calls return")
    }
}

/// Bind the `server` bodies. Stateless: both fns unwrap the bus
/// from their receiver argument — the bus IS the state. Typed per
/// server/server.d.rut; `Plugin::load` verifies the contract.
fn install(hosts: &mut rut_vm::interp::HostRegistry) {
    rut_vm::register!(
        hosts,
        "server::subscribe",
        (Opaque<EventBus>, &str, &str) -> (),
        |vm: &mut Vm, bus: Opaque<EventBus>, topic: &str, handler: &str| -> Result<(), Trap> {
            bus.with_mut(vm, |_vm, b| b.subscriptions.insert(topic.to_string(), handler.to_string()))?;
            Ok(())
        },
    );

    rut_vm::register!(
        hosts,
        "server::emit",
        (Opaque<EventBus>, &str, &str) -> (),
        |vm: &mut Vm, bus: Opaque<EventBus>, topic: &str, handler: &str| -> Result<(), Trap> {
            // The bus stays MUTABLY BORROWED across the nested vm.call
            // (RFC 0022 §1 re-entrancy + RFC 0023 guard): `render_line` runs
            // on a fresh frame stack while the emitting handler is parked
            // mid-op, and a second `emit` fired from inside `render_line`
            // would trap on the guard instead of racing.
            bus.with_mut(vm, |vm, b| -> Result<(), Trap> {
                let line: String =
                    vm.call("render_line", (topic.to_string(), handler.to_string()))?;
                b.lines.push(line);
                b.emits += 1;
                b.renders += 1;
                Ok(())
            })?
        },
    );
}
