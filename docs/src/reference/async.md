# Async and await

rut's concurrency is **pull-based**. An `async fn` compiles into an
`impl Future<T>` for a hidden frame type — a **cold future** that only
progresses when driven. There are no promises, no microtask queue, and
no implicit scheduling: **the host owns time**. One noun (`Future<T>`),
one consume law (`await` **or** launch — exactly one), one pender
(`sleep`; an embedder may mount its own).

| | Push (promises) | rut (poll) |
|---|---|---|
| created | starts executing on the next microtask | **cold** — nothing runs until awaited or launched |
| completion | pushes callbacks through a job queue | nobody drives ⇒ nobody pays |
| allocation | promise + reactions per `then` | one hidden frame record per call; locals are cell-backed |
| cancellation | abort flag / never settles | `abort()` flags the frame; the probe at its checkpoint runs the drop path |
| host integration | the host must drain a job queue | the host polls the VM: `run_ready()`, `next_deadline()` |

## `async fn` and `await`

```rut
use ink::{ Logger };
use async_host::{ launch_future, sleep };

async fn countdown(cx: RunContext, n: u32) -> u32 {
    let log = Logger.new("count");
    let mut i = n;
    while (i > 0) {
        await sleep(1000);            // the ONLY suspension point
        i -= 1;
        log.info(f"tick {i}");
    }
    return i;
}

pub fn main() {
    launch_future(countdown(3));      // trigger; receipt ignored
}
```

```text
tick 2
tick 1
tick 0
```

Rules:

- The **first parameter is the cx**: `async fn f(cx: RunContext, ..)`.
  The engine mints it at call sites and per drive — call sites do not
  pass it.
- `async fn f(..) -> T` describes a value that widens to `Future<T>`.
  **Calling it runs nothing** (cold). It runs when the future is
  `await`ed or launched.
- `await` is legal only inside an `async fn`. In this build it targets
  **engine-woven futures** — async-fn results and `sleep`.
- A future is awaited or launched by **exactly one driver**; each path
  consumes it. Driving one frame from two paths is a disclosed misuse,
  not a soundness hole.
- `launch_future(launch_future(f))` is a **type error**: the receipt is
  not a `Future` ([tasks](tasks.md)).
- Async **methods** are not woven in this build — async points are free
  fns. A sync method may mint the future internally and return it typed
  `Future<T>` (the standard HTTP face does exactly this).
- Cancellation-by-drop: the probe at the resumed checkpoint branches to
  a drop path that releases the frame's cell-backed locals (their
  `on_drop` callbacks fire, [the Rc heap](rc-heap.md)), retires the
  state, and returns.

## The cx protocol

`RunContext` carries two reads, usable as plain data inside a future
impl ([the host futures bridge](host-futures.md)):

| Read | Answers |
|---|---|
| `cx.checkpoint()` | this frame's current resume state (a `u32`) |
| `cx.cancelled()` | the frame's abort flag |

Cancellation is data the frame reads — never an exception it catches.

## The standard launcher set

Launchers are **host surface** — ordinary rut code over the open
`Future` surface, provided by the `rut/async_host` package (users may
write their own the same way):

```rut
pub fn launch_future<T>(f: Future<T>) -> LaunchedFutureHandle<T>;
pub fn sleep(ms: u32) -> Future<nil>;
// LaunchedFutureHandle<T>: the receipt — one member, abort() -> bool
```

Each embedder mounts `rut/async_engine` (the rows `__launch`, `__abort`,
`__sleep`, `__sleep_yield`) plus `rut/async_host`, and installs the row
bodies. A session that mounts neither simply has no launcher; `await`
still works inline.

The crossings cross as **`opaque`** — there is no `any` in the
vocabulary: `opaque(f)` seals a frame on the way out,
`opaque.downcast<Future<nil>>(b) -> ?Future<nil>` recovers it on the
way in ([opaque — erasure and downcast](opaque.md)). `sleep(ms)` is
literally that downcast over the engine's minted sleep frame.

Host async fns declared in a declaration file —
`pub host async fn fetch(url: str) -> bytes;` — mint the same cold
woven frames; the embedder backs them with a `Completer`
([the host futures bridge](host-futures.md)).

## What the compiler emits

One `async fn` compiles into **one** code body over existing ops only —
no new vocabulary:

- a hidden **frame record** is the future:

  | field | content |
  |---|---|
  | `state` | the checkpoint enum's member singleton; **null = completed or dropped** |
  | `cancelled` | the abort flag |
  | `awaiter` | the frame awaiting this one |
  | `pending` | the future this one parks on |
  | `[4..]` | locals — every binding, cell-backed |

- a per-fn **checkpoint enum** whose member index *is* the resume
  state, dispatched with a branch table — **the state field is the pc**;
- each await site: read the callee's state; not done → drive one step
  (a plain vtable call of `Future::yield`); still pending → store the
  awaiter/pending edges, set the state, and `ret` — suspension is a
  plain return whose caller reads the state field for the answer
  (`Done | Parked`);
- the drop path: clear the pending edges, release ref locals in binding
  order, retire.

Locals are mirrored into the frame cell uniformly, so no liveness
analysis decides which locals survive a park. A completed frame re-drive
answers at the dispatch default (a silent retire). The `awaiter`/`pending`
pair is one-directional — a driven frame re-enqueues its awaiter, and no
cycle forms.

## The driving loop

The VM owns the queues; the **embedder owns the loop**:

| Engine API | Meaning |
|---|---|
| `vm.launch(fut)` | enqueue a future (the queue owns one reference) |
| `vm.cancel(fut) -> bool` | flag cancellation + re-enqueue; `false` when already retired |
| `vm.drive(fut) -> Drive` | one re-entrant call of `Future::yield`; reads the state field |
| `vm.arm_timer(deadline_ms, fut)` | arm a sleep deadline on the virtual clock |
| `vm.now_ms()` / `vm.set_now(t)` | read / advance the deterministic virtual clock |
| `vm.next_deadline() -> Option<u64>` | expire due timers into the ready queue; answer the earliest remaining deadline |
| `vm.run_ready() -> usize` | drain the ready queue, then poll parked host futures, until a spin produces no completion |
| `vm.pending_tasks() -> usize` | ready + armed timers + host futures parked on Completers — the idle test |

Two embedder pumps:

```rust
// wall-clock (a CLI or service): settle from real threads
vm.call::<_, ()>("boot", (args,))?;
while vm.pending_tasks() > 0 {
    vm.run_ready()?;
    std::thread::sleep(Duration::from_millis(2));
}

// virtual clock (tests): fully deterministic
while vm.pending_tasks() > 0 {
    vm.run_ready()?;
    if let Some(d) = vm.next_deadline() { vm.set_now(d + 1); }
}
```

Fuel rides the existing per-op budget
([resource limits](resource-limits.md)): a drive that runs out parks
the frame and propagates `OutOfFuel`; a re-drive re-enters at the
frame's checkpoint.

See [tasks](tasks.md) for receipts, cancellation, and the join/select
tier; [the host futures bridge](host-futures.md) for backing a host
async fn with Rust; [workers and channels](workers-and-channels.md) for
isolate parallelism. A worked user-defined future lives in
[the custom-async example](../examples/04-custom-async.md).
