# 03-plugin — the server one

A Rust chat-room **server** with a rut **moderator plugin**, in the shape
of [00-todolist](../00-todolist), [01-sort](../01-sort), and
[02-digest](../02-digest) — but loaded as a **module directory**:
`plugin/rut.json` names the module (`plugin`), `plugin/plugin.rut` is
the whole plugin (adapter + business logic), `src/lib.rs` is the
embedder SDK, and `src/main.rs` drives a scripted session through **both
load forms** — the directory and a v5 **compiled** `.rutbundle` packed
from it at runtime
([bundles](../../docs/src/reference/bundles.md)). This is the first
example built on **re-entrant `vm.call`**
([embedding](../../docs/src/reference/embedding.md)) — and the first
where **both opaque directions** meet:

- rut's moderator state lives rut-side behind a rut-constructed `opaque`
  (`opaque.new(Moderator.new(bus))`, [opaque](../../docs/src/reference/opaque.md)) —
  the host holds the
  handle and hands it back on every event.
- the host's event bus lives Rust-side behind a **host-constructed**
  `Opaque<EventBus>` (the
  [value boundary](../../docs/src/reference/value-boundary.md) and
  [native containers](../../docs/src/reference/native-containers.md)) —
  handed to `init` as the plugin's
  view of the server. `subscribe` and `emit` are that box's callbacks.

## The layering — callbacks at the edges, classes in the middle

```
RUST biz (main / tests)      →  typed Plugin methods, no vm.call
RUST adapter (src/lib.rs)    →  the ONLY vm.call sites
        ↓ bus box in · export names out
RUT adapter (plugin/plugin.rut) →  the ONLY entry fns; one-line forwards
RUT biz (plugin/moderator.rut)  →  pure logic + emit, no entry fn
```

Neither business layer touches the crossing protocol. `Moderator` holds
the bus handle and speaks through one private `say(topic, payload)`;
`Plugin` exposes `join`/`msg`/`tick`/`shutdown` and dispatches through
one `fire`. The plugin stays free of generic packages, so it publishes
compiled: the manifest is bundle-shaped (v5) and the same directory
packs unchanged — linkable pkgs ride `.rutc`, host pkgs ride source.

## The round trip

1. `Plugin::load` constructs the bus box and calls `init(bus)`; the
   plugin registers callback **names** (`join`→`on_join`, …) — closures
   cannot cross the boundary in either direction (the
   [value boundary](../../docs/src/reference/value-boundary.md)), so
   name-registration is the honest callback contract — and returns its
   state handle.
2. Each event dispatches the subscribed export with the state handle.
3. The plugin's decisions flow back through `emit(bus, topic, payload)`,
   whose host body **re-enters rut**: a nested `vm.call("render_line")`
   formats the wire line while the emitting handler is still parked
   mid-op ([embedding](../../docs/src/reference/embedding.md)). The bus
   stays mutably borrowed across that
   nested call — the [value boundary](../../docs/src/reference/value-boundary.md)
   guard is what makes that sound; a second
   `emit` fired from inside `render_line` would trap instead of race.

The moderator rules themselves are plain rut: `!stats` command answers,
flood control (three consecutive messages from one sender → mute), muted
users rejected on rejoin, tick heartbeats, and a shutdown stats line.

## Run it

```
cargo run -p plugin
```

The transcript prints at the end, followed by the packed-form run: the
same directory packed to a v5 compiled `.rutbundle` in temp, loaded
back, and driven through the identical session — the printed
"identical" line is the round-trip proof. `tests/session.rs` asserts the transcript exactly,
proves every line crossed the nested render (`emits == renders`), and
gates the bundle form: round-trip equality, byte-determinism, and the
refusal gates (unknown `format_version`, CRC corruption).

The same loader drives the CLI for any self-contained module (this
example's host fns live in the embedder, so it needs `cargo run`):

```
rut run path/to/mod            # a module directory (rut.json)
rut pack path/to/mod           # -> mod.rutbundle
rut run path/to/mod.rutbundle  # the packed form
```
