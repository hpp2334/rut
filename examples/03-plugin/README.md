# 03-plugin — the server one

A Rust chat-room **server** with a rut **moderator plugin**, in the shape
of [00-todolist](../00-todolist), [01-sort](../01-sort), and
[02-digest](../02-digest): `plugin.rut` is the plugin, `src/lib.rs` is the
embedder SDK, `src/main.rs` drives a scripted session. This is the first
example built on **re-entrant `vm.call`** (RFC 0022 §1) — and the first
where **both Opaque directions** meet:

- rut's moderator state lives rut-side behind a rut-constructed `Opaque`
  (`Opaque.new(Moderator.new(bus))`, RFC 0014) — the host holds the
  handle and hands it back on every event.
- the host's event bus lives Rust-side behind a **host-constructed**
  `OpaqueBox<EventBus>` (RFC 0023/0026) — handed to `init` as the plugin's
  view of the server. `subscribe` and `emit` are that box's callbacks.

## The layering — callbacks at the edges, classes in the middle

```
RUST biz (main / tests)      →  typed Plugin methods, no vm.call
RUST adapter (src/lib.rs)    →  the ONLY vm.call sites
        ↓ bus box in · export names out
RUT adapter (plugin.rut)     →  the ONLY entry fns; one-line forwards
RUT biz (class Moderator)    →  pure logic + emit, no entry fn
```

Neither business layer touches the crossing protocol. `Moderator` holds
the bus handle and speaks through one private `say(topic, payload)`;
`Plugin` exposes `join`/`msg`/`tick`/`shutdown` and dispatches through
one `fire`.

## The round trip

1. `Plugin::load` constructs the bus box and calls `init(bus)`; the
   plugin registers callback **names** (`join`→`on_join`, …) — closures
   cannot cross the boundary in either direction (RFC 0023 §2), so
   name-registration is the honest callback contract — and returns its
   state handle.
2. Each event dispatches the subscribed export with the state handle.
3. The plugin's decisions flow back through `emit(bus, topic, payload)`,
   whose host body **re-enters rut**: a nested `vm.call("render_line")`
   formats the wire line while the emitting handler is still parked
   mid-op (RFC 0022 §1). The bus stays mutably borrowed across that
   nested call — the RFC 0023 guard is what makes that sound; a second
   `emit` fired from inside `render_line` would trap instead of race.

The moderator rules themselves are plain rut: `!stats` command answers,
flood control (three consecutive messages from one sender → mute), muted
users rejected on rejoin, tick heartbeats, and a shutdown stats line.

## Run it

```
cargo run -p plugin
```

The transcript prints at the end; `tests/session.rs` asserts it exactly
and proves every line crossed the nested render (`emits == renders`).
