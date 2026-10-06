# Async and await

rut's concurrency is **pull-based**. An `async fn` compiles into the
driven half of a hidden frame type — a **cold future** that only
progresses when driven. There are no promises, no microtask queue, and
no implicit scheduling: **the host owns time**. One noun (`Future<T>`,
a closed builtin class), one consume law (`await` **or** launch —
exactly one), one pender (`sleep`; an embedder may mount its own).

| | Push (promises) | rut (poll) |
|---|---|---|
| created | starts executing on the next microtask | **cold** — nothing runs until awaited or launched |
| completion | pushes callbacks through a job queue | nobody drives ⇒ nobody pays |
| allocation | promise + reactions per `then` | one hidden frame record per mint; locals are cell-backed |
| cancellation | abort flag / never settles | `abort()` flags the frame; the probe at its checkpoint runs the drop path |
| host integration | the host must drain a job queue | the host polls the VM: `run_ready()`, `next_deadline()` |

## `Future<T>` — the one async type

Every async producer answers the **same** type: an `async fn` call, an
`async { }` block, `sleep`, `select2`, `completer` — all `Future<..>`,
so `await`/`select2`/`launch_future` never ask "which kind".
`Future<T>` is a **closed** `builtin class`: no constructor, no impl
lane, no user-callable members — a user type cannot BE a future
(structural satisfaction cannot see engine identity, so nominal
closure is the only spelling of "engine-minted only"). The hidden
frame is representation; the handle is the surface.

## `async fn`, `async { }`, and `await`

```rut
use ink::{ Logger };
use futures::{ launch_future, sleep };

async fn countdown(n: u32) -> u32 {
    let log = Logger("count");
    let mut i = n;
    while (i > 0) {
        await sleep(1000);            // the ONLY suspension point
        i -= 1;
        log.info(f"tick {i}");
    }
    return i;
}

entry fn main() {
    launch_future(countdown(3));      // trigger; receipt ignored
}
```

```text
tick 2
tick 1
tick 0
```

`async fn` is genuine **sugar** — both sides of the law are writable
programs:

```rut
async fn work() -> nil { BODY }
// ≡
fn work() -> Future<nil> {
    return async { BODY };
}
```

The `async { }` **block** is the primitive: one expression form
(keyword before a block). Evaluating it **mints the frame** — the mint
is pure, so an async block is legal in sync code (`launch_future(async
{ .. })`, a `select2` arm); the frame runs when driven. Its type is
`Future<T>` with `T` inferred from the block's `return` statements
(fn-body rules); an annotated position pins it
(`let f: Future<str> = async { .. }`). The block's reads capture the
enclosing locals **by value** at the mint (the capture law: scalars
copy, ref-headed handles share their cell), and a `return` inside the
block answers **the future** — not any enclosing fn.

Rules:

- **The cx is injected, never spelled.** The weave binds `cx` (a
  `RunContext`) in every async body; an async fn's parameters are
  ordinary values — a spelled `cx: RunContext` diagnoses.
- `async fn f(..) -> T` describes a value of type `Future<T>`.
  **Calling it runs nothing** (cold). It runs when the future is
  `await`ed or launched.
- `await` is legal only inside an async body (an `async fn` or an
  `async { }` block), and its operand must be a `Future<..>` — plain
  type identity, since every producer mints the closed class.
- A future is awaited or launched by **exactly one driver**; each path
  consumes it. Driving one frame from two paths is a disclosed misuse,
  not a soundness hole.
- `launch_future(launch_future(f))` is a **type error**: the receipt is
  not a `Future` ([launched futures](launched-futures.md)).
- Async **methods** are not woven in this build — async points are
  free fns or `async { }` blocks inside sync methods (the standard
  HTTP face mints inside its methods).
- Cancellation-by-drop: the probe at the resumed checkpoint branches to
  a drop path that releases the frame's cell-backed locals (their
  `[disposal]` members run, [the Rc heap](rc-heap.md)), retires the
  state, and returns.

## The cx protocol

`RunContext` — a closed `builtin class`, injected by the weave —
carries two reads, usable as plain data inside any async body:

| Read | Answers |
|---|---|
| `cx.checkpoint()` | this frame's current resume state (a `u32`) |
| `cx.cancelled()` | the frame's abort flag |

Cancellation is data the frame reads — never an exception it catches.
(The weave's own `next_checkpoint` edge is unspelled: the weave
records each await's resume state.)

## The standard launcher set

Launchers are **host surface** — ordinary rut code over the closed
`Future` class, provided by the `rut/futures` package (users may
write their own the same way):

```rut
pub fn launch_future<T>(f: Future<T>) -> LaunchedFutureHandle<T>;
pub fn sleep(ms: u32) -> Future<nil>;
// LaunchedFutureHandle<T>: the receipt — one member, abort() -> bool
```

Each embedder offers `rut/async_host` (the rows `__launch`, `__abort`,
`__sleep`, `__sleep_yield`) plus `rut/futures`, and installs the row
bodies. A world that offers neither simply has no launcher; `await`
still works inline.

The crossings cross as **`opaque`**: `opaque(f)` seals a frame on the
way out, `opaque.downcast<Future<nil>>(b) -> ?Future<nil>` recovers it
on the way in ([opaque — erasure and downcast](opaque.md)). `sleep(ms)` is
literally that downcast over the engine's minted sleep frame.

Host async fns declared in a declaration file —
`pub host async fn fetch(url: str) -> bytes;` — mint the same cold
frames; the embedder backs them with a `Completer`
([the host futures bridge](host-futures.md)).

## What the compiler emits

One async producer compiles into **one** code body over existing ops
only — no new vocabulary:

- a hidden **frame record** is the future:

  | field | content |
  |---|---|
  | `state` | the checkpoint enum's member singleton; **null = completed or dropped** |
  | `cancelled` | the abort flag |
  | `awaiter` | the frame awaiting this one |
  | `pending` | the future this one parks on |
  | `[4..]` | locals — every binding, cell-backed (an async block's captures included) |

- a per-producer **checkpoint enum** whose member index *is* the resume
  state, dispatched with a branch table — **the state field is the pc**;
- each await site: read the callee's state; not done → drive one step
  (a call through the `Future<T>` class's designated yield slot — the
  engine ABI, unspelled); still pending → store the
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
| `vm.drive(fut) -> Drive` | one drive step through the future's designated yield slot; reads the state field |
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

See [launched futures](launched-futures.md) for receipts, cancellation, and the
race/completer tier (`select2` / `select_all` / `completer`);
[the host futures bridge](host-futures.md) for backing a host
async fn with Rust.
