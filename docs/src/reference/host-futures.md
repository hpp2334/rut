# The host futures bridge

Native Rust code can back a rut async fn. The contract is one closure
per operation: spawn the work, hand out a **`Completer<T>`**, and settle
it from anywhere. The engine weaves the future; the embedder never
touches a frame.

## Declaring a host async fn

In a declaration package ([host fns and declaration files](host-fns.md)):

```rut
/// fetch `url`; the answer is the response body
pub host async fn fetch(url: str) -> bytes;
```

Rules:

- Concrete crossing signatures only — no generic host async fns;
  parameter and answer types come from the crossing set
  ([the host boundary](../core-concepts/host-boundary.md)).
- Arity 0–8.
- Calling one **runs nothing**: the call site mints a cold engine-woven
  `Future<T>` frame whose state field holds the host cell. The body is
  driven by `await` or `launch_future` exactly like any other future.
- The declared name itself is **never dispatched** — calling through it
  traps with a teaching message ("the weave mints the future at the
  call site").

## `Completer<T>`

The completion cell — the one helper a host async fn's body needs:

| Member | Thread | Meaning |
|---|---|---|
| `Completer::new() -> Completer<T>` | any | a fresh cell: atomic state + a mutex slot, one `Arc` |
| `.clone()` | any | another handle on the **same** cell — never a copy of the state |
| `.complete(v: T)` | any thread | settle with the answer |
| `.fail(msg: String)` | any thread | settle with a failure — `msg` **becomes the trap at the await** |
| `.poll() -> i32` | any | the driving loop's probe: `PENDING` (0) / `READY` (1) / `FAILED` (2) |
| `.take_result() -> Result<T, Trap>` | VM thread | drain the answer — one take; a second take traps; taking while pending traps |

`T` must be a crossing type (`Ret`). The completer itself crosses boxed
under `opaque` — it is what the woven frame holds in its state field
([opaque — erasure and downcast](opaque.md)).

**Thread law**: the VM stays single-threaded. Worker threads touch only
the completer's atomics + mutex; every rut cell is minted on the VM
thread when the answer is marshaled.

**Late answers are disclosed best-effort**: after a cancellation the
thread runs to its blocking completion and the result is simply never
taken.

## Registering — `pkg_async_fn!`

One closure (plus an optional abort hook) expands into the five rows
the weave drives. On the installer lane the row names are bare and the
pkg's scope prefixes at the install; the raw `register_async!` twin
(full `scope::name` strings on the bare registry) is the escape hatch —
same emitter, same family:

```rust
use rut_vm::{ HostPkg, Completer, Vm };

let mut pkg = HostPkg::new("mypkg");
rut_vm::pkg_async_fn!(pkg, "fetch", (String,) -> Vec<u8>,
    |url: String| -> Completer<Vec<u8>> {
        let c = Completer::new();
        let w = c.clone();
        std::thread::spawn(move || {
            match std::fs::read(url) {
                Ok(data) => w.complete(data),
                Err(e) => w.fail(e.to_string()),
            }
        });
        c                       // returned immediately: the future parks
    });
hosts.install_host_pkg(&ctx, pkg.build());
```

| Emitted row | Signature | Role |
|---|---|---|
| `mypkg::fetch` | `(args) -> T` | the decl row — a teaching trap, never dispatched |
| `mypkg::fetch__start` | `(args) -> opaque` | calls the closure, boxes the `Completer` — the state cell |
| `mypkg::fetch__yield` | `(state, cx) -> i32` | the resumption probe: 0 pending / 1 ready / 2 failed |
| `mypkg::fetch__take` | `(state) -> T` | marshals the answer onto the heap **on the VM thread**; a failure traps here |
| `mypkg::fetch__cancel` | `(state)` | the abort closure when one is given; otherwise the disclosed no-op |

The abort-closure form receives a `Completer<T>` clone (safe to move
anywhere); without it, cancellation is the documented best-effort law —
the thread finishes, the late result is discarded.

Rut side, the closure's own surface is invisible — callers see a
normal async fn (the cx is injected, never spelled):

```rut
use futures::{ launch_future, sleep };

// `fetch` is the host's async fn (the mypkg::fetch row family above);
// a stand-in body keeps the block runnable end to end:
async fn fetch(path: str) -> bytes {
    await sleep(5);
    return "hostname".encode();
}

async fn grab(path: str) -> bytes {
    let body = await fetch(path);
    return body;
}

entry fn main() {
    launch_future(grab("/etc/hostname"));
}
```

## The embedder's loop

The engine polls every future parked on a completer:

```rust
vm.call::<_, ()>("main", ())?;
while vm.pending_tasks() > 0 {
    vm.run_ready()?;                       // drain + poll + drain
    std::thread::sleep(Duration::from_millis(2));
}
```

`run_ready()` drains the ready queue, polls the parked host futures
(each poll runs the `__yield` probe; a ready one is driven through
`__take` and its awaiter re-enqueued), then drains again — one pass
settles a completed host future end to end. `vm.pending_tasks()` counts
ready frames, armed timers, and the host-pending set, so a worker that
never completes keeps the loop alive — the flip side of best-effort
cancellation.

The virtual clock works here too: drive with finite fuel and
`vm.set_now` for deterministic tests
([resource limits](resource-limits.md)).

## In-tree users

The standard HTTP lane is a `register_async!` client: `http_send`,
`http_body`, and `http_stream_next` are `pub host async fn` rows whose
bodies park `Completer`s answered from reqwest worker threads (or the
fixture lane's virtual clock). Mount it with `rut/http_host` +
`rut/http`; the full walk-through is
[the GitHub viewer example](../examples/06-github-viewer-cli.md).

Plain `host fn` rows (synchronous, run to completion inside the
crossing) are the other half of the boundary — see
[embedding and native modules](embedding.md).
