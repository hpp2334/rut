# The async model

rut's concurrency is **pull-based**, built on one noun: `Future<T>`.
Calling an `async fn` runs nothing — it produces a cold future. The
future runs when something *drives* it: an `await` inside another async
fn, or a launch into the VM's queue. There are no promises, no
microtask queue, and no implicit scheduling: **the host owns time**, and
nothing runs unless the driving loop runs it.

## Pull, not push

| | push (promises) | pull (rut futures) |
|---|---|---|
| created | starts on the next microtask | cold — nothing runs until awaited or launched |
| idle cost | reactions and queues exist whether or not anyone waits | nobody drives ⇒ nobody pays |
| per-await allocation | promise + callback lists | one hidden frame record per call |
| cancellation | abort flags layered on top | native: the frame is dropped at a checkpoint |
| host integration | the host must drain a job queue | the host polls: drain ready frames, ask for the next deadline |

## The surface

```rut
use async_host::{ launch_future, sleep };
use ink::{ Logger };

async fn countdown(cx: RunContext, log: Logger, n: u32) -> nil {
    for (let i = n; i > 0; i -= 1) {
        log.info(f"{i}");
        await sleep(1000);             // the only suspension spelling
    }
}

pub fn main() -> nil {
    let log = Logger.new("countdown");
    launch_future(countdown(log, 3));  // the other consume: launch
}
```

```text
3
2
1
```

The rules:

- **The first parameter is the context.** `async fn f(cx: RunContext,
  ..)` — the engine mints it at call sites and per drive, the way it
  mints `self`. It carries the frame edge: `checkpoint()` reads the
  resume state, `cancelled()` reads the task's abort flag.
- **Calling does not run.** `async fn f(..) -> T` describes a value
  that widens to `Future<T>`; it runs when awaited or launched.
- **One consume law.** A future is consumed by `await` *or* by
  `launch_future` — exactly one. The launch returns its own receipt
  type, which is not a future and cannot be launched or awaited again.
- **`await` targets engine-woven futures** (async-fn results and
  `sleep`). Hand-written futures are *launcher-drivable* — the driving
  loop finds their `Future::yield` in the vtable — which is how the
  standard `sleep` itself is built over the open trait surface.

## What the compiler emits

An `async fn` compiles to one hidden frame — an ordinary heap cell —
with a woven `Future::yield` that the driving loop calls once per
resumption:

```text
frame:  [0]=state      the checkpoint enum's member — the pc
        [1]=cancelled  the task's abort flag
        [2]=awaiter    the frame awaiting THIS one
        [3]=pending    the future THIS one parks on
        [4..]=locals   every binding, mirrored into the frame

entry:      dispatch on state
per await:  probe the parked future's state
              done    -> resume with its value
              pending -> link awaiter/pending edges, park, return
resume arm: cancelled-probe -> drop path | restore locals -> continue
```

The state field *is* the program counter: resume dispatch is a jump
table over the checkpoint enum. Because locals are mirrored into the
frame cell, parking releases only staging registers and no liveness
analysis decides which locals survive a suspension. The whole weave
rides existing bytecode — no new opcodes, no new runtime machinery.

## Cancellation is drop

`handle.abort()` flags the frame's cancellation and re-enqueues it. The
probe at its next checkpoint branches to the drop path: locals release
in reverse binding order, `on_drop` callbacks fire by refcount, and any
pending `sleep` dies with the frame. Cancellation **never interrupts
mid-expression** — the checkpoint probe is the only place a
cancellation becomes observable, so a cancelled function dies at a
known-clean boundary. This is the same machinery that frees any heap
object (see [memory](memory.md)); coroutine frames get no special case.

## The driving loop

The VM owns the queues; the embedder owns the loop:

```rust
vm.call::<_, ()>("main", ())?;              // script boots, launches futures
loop {
    vm.run_ready()?;                        // one drive per ready frame
    if let Some(d) = vm.next_deadline() {   // expire due timers into ready
        sleep_until(d);                     // the wall-clock stand-in
    }
    if vm.pending_tasks() == 0 { break; }   // idle
}
```

- `sleep(ms)` arms a deadline on the VM's **virtual clock**
  (`vm.now_ms()` / `vm.set_now()`). A test host advances the clock by
  hand and gets fully deterministic schedules — a UI host just maps it
  to wall time.
- Each queue entry owns one reference; a completed drive re-enqueues
  the awaiting frame through a one-directional edge pair, so wake
  wiring never forms a cycle.
- Fuel accounting rides the ordinary per-op budget: a drive that runs
  out parks the frame at its checkpoint and resumes there later.

## Host futures: `pub host async fn`

Native code joins the same model. A declaration file spells the row:

```rut
pub host async fn http_send(c: opaque, method: str, url: str,
                            headers: str, body: bytes) -> opaque;
pub host async fn http_stream_next(s: opaque) -> ?bytes;
```

and the embedder binds it with **one closure** that starts the work and
returns the future's state cell:

```rust
rut_vm::register_async!(hosts, "http_host::http_stream_next",
    (Opaque<HttpStream>,) -> Option<Vec<u8>>,
    move |s: Opaque<HttpStream>| -> Completer<Option<Vec<u8>>> {
        let c = Completer::new();
        let w = c.clone();
        std::thread::spawn(move || {
            w.complete(read_chunk(s));     // settle from any thread
        });
        c
    });
```

`Completer<T>` is a small shared cell: `complete(v)`/`fail(msg)` are
callable from any thread (atomics plus a mutex — the VM stays
single-threaded), `fail`'s message becomes the trap **at the await**,
and an optional abort closure runs when the script cancels a pending
future (late results are then simply discarded). The macro registers
the whole row family the weave drives — start, probe, take, cancel —
and the load-time check verifies the expansion against the declaration,
so a decl and its bodies cannot drift apart. IO written with plain
`async`/`await` Rust maps onto this naturally: the work leaves the VM,
and completion is one enqueue back into the ready queue.

## Structured concurrency, honestly

The language today keeps the vocabulary deliberately small: launch,
abort, and await. Racing (`await select { .. }`) and joining a launched
future (`await handle`) parse but are compile-gated — the diagnostics
name them as future work, and structured scopes (a block that cancels
its children on exit) are the same story. What exists now is already
enough to structure real programs — the launch/abort receipt gives you
explicit ownership of background work, and drop-based cancellation gives
it a clean off switch — but nothing in the model silently cancels
siblings on your behalf. See the reference on [async and
await](../reference/async.md), [tasks](../reference/tasks.md), and the
[host futures bridge](../reference/host-futures.md); the
[GitHub viewer CLI](../examples/06-github-viewer-cli.md) example shows a
full program living inside this loop.
