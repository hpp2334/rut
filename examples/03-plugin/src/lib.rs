//! 03-plugin — the host side of the chat-server example, as a small SDK.
//!
//! The layering (see README): rut's business logic is a plain class
//! (`Moderator`), the crossing protocol lives in ~15 lines of `entry fn`
//! shims, and this side mirrors it — [`Plugin`] is the typed embedder
//! surface; `vm.call` happens in exactly one place (`Plugin::fire`),
//! plus the `emit` host fn's nested render call (RFC 0022 §1).
//!
//! Both Opaque directions meet here: rut's `Moderator` state comes back
//! as a rut-constructed `Opaque` (RFC 0014), the host's event bus goes
//! in as a host-constructed [`OpaqueBox`]`<EventBus>` — `subscribe` and
//! `emit` are that box's callbacks, reached only through the RFC 0023
//! borrow guards.

use std::collections::HashMap;
use std::rc::Rc;

use rut_vm::interp::{HostHooks, Limits, Vm};
use rut_vm::{OpaqueBox, Trap, TrapKind, Value};

/// The host's server object — handed to rut as an Opaque box. All host
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
    bus: OpaqueBox<EventBus>,
    state: Value, // rut's `Moderator`, opaque to us
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
        // core+calc (the embedder's mount); `server` and `pouch` resolved
        // from the plugin manifest's `[deps]` — the server surface is
        // server/server.d.rut, no hand-written Rust surface
        rut_driver::mount_std(&mut session);
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
        let mut vm = Vm::new(Rc::new(prog), limits, HostHooks::default())?;
        install(&mut vm);

        // the handshake: bus in, state out
        let bus = OpaqueBox::alloc(
            &mut vm,
            EventBus { subscriptions: HashMap::new(), lines: Vec::new(), emits: 0, renders: 0 },
        )?;
        let bus_val = bus.clone().into_value();
        let Value::Opaque(state) = vm.call("init", &[bus_val])? else {
            return Err(Trap::new(TrapKind::Invalid, "init must return the plugin state handle"));
        };
        let table = bus.with(|b| b.subscriptions.clone())?;
        Ok(Plugin { vm, bus, state: Value::Opaque(state), table })
    }

    /// Dispatch one event to its subscribed rut handler. The only
    /// `vm.call` site for events; unknown topics are dropped, not
    /// trapped — a server keeps running when a plugin didn't subscribe.
    pub fn fire(&mut self, topic: &str, event_args: &[Value]) -> Result<(), Trap> {
        let Some(handler) = self.table.get(topic).cloned() else {
            println!("[server] no handler for `{topic}` — dropped");
            return Ok(());
        };
        let mut args = vec![self.state.clone()];
        args.extend_from_slice(event_args);
        self.vm.call(&handler, &args)?;
        Ok(())
    }

    // ---- the typed surface: what a driver or test actually uses ----

    pub fn join(&mut self, name: &str) -> Result<(), Trap> {
        self.fire("join", &[Value::Str(name.into())])
    }

    pub fn leave(&mut self, name: &str) -> Result<(), Trap> {
        self.fire("leave", &[Value::Str(name.into())])
    }

    pub fn msg(&mut self, user: &str, text: &str) -> Result<(), Trap> {
        self.fire("msg", &[Value::Str(user.into()), Value::Str(text.into())])
    }

    pub fn tick(&mut self, n: i64) -> Result<(), Trap> {
        self.fire("tick", &[Value::I64(n)])
    }

    /// Final stats emit; returns the plugin's total message count.
    pub fn shutdown(&mut self) -> Result<i64, Trap> {
        match self.vm.call("shutdown", &[self.state.clone()])? {
            Value::I64(n) => Ok(n),
            v => Err(Trap::new(TrapKind::Invalid, format!("shutdown: {v:?}"))),
        }
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
/// from their receiver argument — the bus IS the state.
fn install(vm: &mut Vm) {
    vm.register_host_fn("server::subscribe", |_vm, args| {
        let bus = OpaqueBox::<EventBus>::from_value(&args[0])?;
        let Value::Str(topic) = &args[1] else {
            return Err(Trap::new(TrapKind::Invalid, "subscribe: str topic"));
        };
        let Value::Str(handler) = &args[2] else {
            return Err(Trap::new(TrapKind::Invalid, "subscribe: str handler"));
        };
        bus.with_mut(|b| b.subscriptions.insert(topic.clone(), handler.clone()))?;
        Ok(Value::Nil)
    });

    vm.register_host_fn("server::emit", |vm, args| {
        let bus = OpaqueBox::<EventBus>::from_value(&args[0])?;
        // The bus stays MUTABLY BORROWED across the nested vm.call
        // (RFC 0022 §1 re-entrancy + RFC 0023 guard): `render_line` runs
        // on a fresh frame stack while the emitting handler is parked
        // mid-op, and a second `emit` fired from inside `render_line`
        // would trap on the guard instead of racing.
        bus.with_mut(|b| -> Result<(), Trap> {
            let Value::Str(line) = vm.call("render_line", &[args[1].clone(), args[2].clone()])?
            else {
                return Err(Trap::new(TrapKind::Invalid, "render_line must return str"));
            };
            b.lines.push(line);
            b.emits += 1;
            b.renders += 1;
            Ok(())
        })??;
        Ok(Value::Nil)
    });
}
