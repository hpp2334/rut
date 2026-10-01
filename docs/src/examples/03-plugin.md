# 03 — Plugin

A Rust chat-room **server** with a rut **moderator plugin** — the
first example built on two things at once: **re-entrant `vm.call`**
(the host calls rut, and a host fn called *by* rut calls back into
rut while the first call is still parked) and **both `opaque`
directions** (rut hands the host a handle to its own state; the host
hands rut a handle to the server's event bus). It is also the
packaging example: the plugin loads from a **module directory** and
from a **`.rutbundle`** packed from that directory at runtime — the
two forms of one contract
([Module bundles](../reference/bundles.md)).

## Run it

```sh
cargo run -p plugin         # from the repo root
cargo test -p plugin        # transcript asserted + bundle round-trip gates
```

The demo drives one scripted session twice — once against the
directory, once against the packed bundle — and prints both
transcripts plus the round-trip proof. Abridged:

```text
== transcript (module directory) ==
[system] *** ada joined (1 online)
[broadcast] <ada> hello world
...
[system] shutdown: 0 online, 3 msgs, 1 mutes
-- 3 messages moderated --

== packed form (examples/03-plugin/plugin -> NNNN bytes, written to /tmp/...) ==
transcript identical to the directory form: true
```

The test suite asserts the transcript exactly, proves every line
crossed the nested render (`emits == renders`), and gates the bundle
form: round-trip equality, byte-determinism, and refusal on an
unknown format version or CRC corruption.

## Code tour

### The layering: callbacks at the edges, classes in the middle

Four layers, and neither business layer ever touches the crossing
protocol:

```text
RUST biz (main / tests)      ->  typed Plugin methods, no vm.call
RUST adapter (src/lib.rs)    ->  the ONLY vm.call sites
RUT adapter (plugin/plugin.rut) ->  the ONLY entry fns; one-line forwards
RUT biz (Moderator)          ->  pure logic + emit, no entry fn
```

The moderator rules themselves are plain rut — flood control (three
consecutive messages from one sender mutes them), muted users
rejected on rejoin, a `!stats` command, tick heartbeats:

```rut
pub fn on_msg(mut self, user: str, text: str) {
    if (self.is_muted(user)) {
        self.say("muted", user);
    } else if (text == "!stats") {
        self.say("direct", f"{user}: {self.stats()}");
    } else {
        if (user == self.last_sender) {
            self.streak += 1;
        } else {
            self.last_sender = user;
            self.streak = 1;
        }
        if (self.streak == 3) {
            self.muted.push(user);
            self.mutes += 1;
            self.say("system", f"{user} muted for flooding");
        } else {
            self.msgs += 1;
            self.say("broadcast", f"<{user}> {text}");
        }
    }
}
```

Every bus write funnels through one private `say(topic, payload)` —
the business logic never spells a crossing.

### The handshake: both opaque directions meet

Closures cannot cross the boundary in either direction, so callbacks
register **by name**: the plugin hands the host the export names it
wants wired to each topic, and returns the handle to its own state
(`init` is the entire handshake — bus box in, state handle out):

```rut
entry fn init(bus: opaque) -> opaque {
    subscribe(bus, "join", "on_join");
    subscribe(bus, "msg", "on_msg");
    subscribe(bus, "leave", "on_leave");
    subscribe(bus, "tick", "on_tick");
    let mod_: ?Moderator = Moderator.new(bus);
    return opaque(mod_);
}
```

- rut's moderator state comes back as a **rut-constructed** `opaque`
  — the host holds the handle and passes it back on every event.
- the host's `EventBus` goes in as a **host-constructed**
  `Opaque<EventBus>` — `subscribe` and `emit` are that box's
  callbacks, reached only through the borrow guards
  ([Value boundary and borrows](../reference/value-boundary.md),
  [Embedding and native modules](../reference/embedding.md)).

### The re-entrant call: `emit` re-enters rut mid-op

The host binds the two `server` rows over its bus. The `emit` body is
the interesting one — while the emitting handler is still parked
mid-op, it makes a **nested** `vm.call("render_line")` to format the
wire line:

```rust
let mut pkg = rut_vm::HostPkg::new("server");
rut_vm::pkg_fn!(pkg, "emit", (Opaque<EventBus>, &str, &str) -> (),
    |vm: &mut Vm, bus: Opaque<EventBus>, topic: &str, handler: &str| -> Result<(), Trap> {
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
hosts.install_host_pkg(&ctx, pkg.build());
```

(The comment above this code in the source is worth reading too.) The
bus stays mutably borrowed *across* the nested call — the value
boundary's borrow guard is what makes that sound: a second `emit`
fired from inside `render_line` would trap on the guard instead of
racing. On the rut side, `render_line` is the one-line entry the host
re-enters — pure string work, so it runs on its own (the line it
prints below is the transcript's broadcast row):

```rut
use ink::{ Logger };

entry fn render_line(topic: str, payload: str) -> str {
    return f"[{topic}] {payload}";
}

pub fn main() {
    let log = Logger.new("bus");
    log.info(render_line("broadcast", "<ada> hello world"));
}
```

```text
[broadcast] <ada> hello world
```

### The packaging: one directory, two load forms

The plugin is a **module directory** — a manifest naming the entry
plus its deps, loadable as-is and packable unchanged
([Project structure and rut.jsonc](../reference/project-structure.md)):

```jsonc
// plugin/rut.jsonc
{
  "format": "rutbundle",
  "format_version": 9,
  "name": "plugin",
  "entry": { "lib": "./plugin.rut" },

  "deps": {
    "server": { "path": "../server" }
  }
}
```

`server/` is the interesting dependency: a **pure declaration
surface** — a host package whose manifest points at a `.d.rut` decl
file, with the Rust embedder binding the bodies
([Host fns and declaration files](../reference/host-fns.md)):

```rut
pub host fn subscribe(bus: opaque, topic: str, handler: str);

pub host fn emit(bus: opaque, topic: str, payload: str);
```

`main.rs` packs the same directory with `rut_driver::pack_dir` — a
v9 **compiled** bundle: the plugin rides as a `.rutc` binary, its host
pkg `server` as a source group — writes `plugin.rutbundle` to temp, and
loads it back through the identical `Plugin::load`. The transcript
equality print is the proof. The CLI drives the same loader for any self-contained module:

```sh
rut run path/to/mod            # a module directory (rut.jsonc)
rut pack path/to/mod           # -> mod.rutbundle
rut run path/to/mod.rutbundle  # the packed form
```

The Rust side mirrors the rut layering: business code calls typed
methods (`p.join("ada")`, `p.msg(...)`), and `vm.call` happens in
exactly one dispatcher that looks up the subscribed export name per
topic — unknown topics are dropped, not trapped, so a server keeps
running when a plugin didn't subscribe.

## Takeaways

- **One directory is one module**; the manifest names the entry and
  the deps, and the same directory packs to a `.rutbundle` unchanged.
- **Registration by name, not by closure** — callbacks never cross
  the boundary as values, in either direction.
- **Re-entrancy is sound by construction**: a host fn can call back
  into rut mid-op, and the borrow guard turns a would-be data race
  into a loud trap.
- **Both `opaque` directions** compose: rut-state-out,
  host-state-in, and neither side can inspect what the other erased.
- The layering discipline — adapters own the crossing protocol,
  business logic stays pure — is what keeps a plugin auditable.

The crossing rules this example lives on are in
[the host boundary](../core-concepts/host-boundary.md); the load
forms are specified in [Module bundles](../reference/bundles.md).
