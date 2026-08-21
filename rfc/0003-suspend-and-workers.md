# RFC 0003: Concurrency — Coroutines & Workers

- **Status:** Draft
- **Date:** 2026-08-21
- **Author:** hpp2334
- **Depends on:** RFC 0001 (pillars P2/P6), RFC 0002 (types), RFC 0004 (memory)

## Summary

rut's concurrency is **pull-based**: a suspend fn compiles to a
resumable state machine (a *cold future*) that only progresses when polled —
Rust semantics with Kotlin ergonomics (the modifier is literally `suspend`,
like Kotlin; `await`/`spawn` read like Kotlin/JS). Workers are separate VM
isolates connected by typed channels; `bytes`/`array<T>` buffers can be
*transferred* (zero-copy) between them.

There are no promises, no microtask queue, and no implicit scheduling:
**the host owns time**, exactly like tur's frame-driven flush loop.

## 1. Model: pull, not push

| | JS Promise (push) | rut Future (poll) |
|---|---|---|
| created | starts executing immediately on next microtask | cold — nothing runs until awaited or spawned |
| completion | pushes `.then` callbacks via job queue | nobody polls ⇒ nobody pays |
| allocation | promise + reactions per await | one coroutine frame per *task*, frames reused across awaits |
| cancellation | abort flag / never settles | drop the task — state machine and its resources are released **at the suspension point** |
| host integration | host must drive a job queue | host polls the VM: `run_until_idle()`, `next_deadline()` |

Cancellation-by-drop is the property tur currently emulates by aborting the
driver future (`TaskHandle::abort`); in rut it falls out of the model for free.

## 2. `suspend fn` and `await`

See **`examples/concurrency/countdown.rut`** — cold future, `await` as the
only suspension point.

Rules:

- `suspend fn f(..): T` describes a function returning `Future<T>`.
  Calling it **does not run it** (cold). It runs when the future is `await`ed
  or `spawn`ed.
- `await` is only legal inside `suspend fn`. There is no implicit yield
  anywhere else — no function is ever preempted mid-expression.
- `?` composes: `const body = await fs.read(path)?;` awaits, then propagates
  `Err` in one expression.
- A future may be awaited by exactly one task (`await` consumes it). Share a
  result by `spawn`ing a `Task<T>` and awaiting the task.

### 2.1 What the compiler emits

The body is split at each `await` into states; every local live across an
`await` is stored in the coroutine frame's register file, which lives in a
heap object (refcounted, RFC 0004). `fetch_page` from
**`examples/concurrency/fetch-page.rut`** becomes (illustrative
pseudo-bytecode):

```text
fetch_page$suspend(frame):
entry:
    call    http$get frame.url           -> f0
    await   f0                           -> raw      ; suspend: state=1
    chk     raw                          -> raw  ?   ; `?` propagates Err
state1:                                             ; resume jumps here
    call    decode raw                   -> t1  ?
    ret     Result.ok(t1)
```

Core representation — `CoroutineFrame { func, state, regs }`, the `Flow`
enum, the `Await` opcode, and `Vm::resume` — is specified with Rust sketches
in RFC 5003 §2.

## 3. Tasks: `spawn`, `cancel`, `select`

See **`examples/concurrency/spawn-cancel.rut`** (cancellation-by-drop) and
**`examples/concurrency/select.rut`** (racing with `select`/`select_all`).

- `spawn(fut): Task<T>` schedules the future on the current VM. `Task<T>` is
  itself a `Future<Result<T, Cancelled>>`: `await task` joins it.
- `cancel()` marks the task cancelled and **drops the coroutine frame at its
  next suspension point** — pending `sleep`s die with it, and every local with
  a destructor (RFC 0004 §3) runs deterministically. Cancellation never
  interrupts mid-expression.
- `await select { .. }` races futures; the winner's value is produced by its
  arm expression, the losers are dropped (i.e. cancelled). Arms use `->`
  like `when`; `fut -> expr` discards the resolved value, `fut as x -> expr`
  binds it.
- `select_all(futs)` (stdlib, built on `select`) resolves with the first
  ready value — used in the worker example below.
- v1 tasks are unstructured (no automatic child cancellation). Structured
  scopes (`scope { .. }` cancelling children on exit) are OQ-3.

## 4. Host futures bridge both ways

Native code may hand rut any Rust future; rut futures are pollable from
Rust. The only crossing point is a waker that enqueues a `Wake(task)` into
the owning VM's queue — the direct replacement for tur's
`WorkerMsg::Wake`-on-settle. The `RutAwaitable` adapter, `RutFuture`
(std::future impl), and the `TaskWaker` are specified in RFC 5003 §4.

Consequences for the embedder:

- `sleep(ms)` is a native fn returning a future whose waker posts into the
  timer wheel — no promise objects, no job executor to drain.
- IO written with plain `suspend`/`await` Rust plugs in unchanged.
- Tests can virtualize time by faking the timer wheel (tur's test clock).

## 5. Workers & channels

Workers are **separate VMs on separate threads** with separate heaps (RFC
0004). No shared memory; all data crosses typed channels. See
**`examples/workers/image-pipeline.rut`** (main side) and
**`examples/workers/image-worker.rut`** (worker side); a larger server-shaped
variant lives in `examples/network/echo-server.rut` + `echo-worker.rut`.

### 5.1 Channel semantics (v1)

- `Channel<T>()` returns a channel value with two endpoints, `sender` and
  `receiver` (unbounded, MPSC). `send(v)` is sync and cheap;
  `recv(): Future<Option<T>>` suspends until a message arrives, resolves
  `None` when every sender is gone. (Bounded/suspend-send and MPMC are OQ-4.)
- Channel endpoints are **transferable** values: pass one to a worker through
  `spawn_worker` args or through another channel.

### 5.2 What may cross an isolate boundary

| Type | Crossing rule |
|---|---|
| ints/floats/bool/char | copy |
| `string` | copy (immutable) |
| `bytes`, `Array<T>` (T numeric/bool/char) | **transfer** if refcount == 1, else deep copy (zero-copy fast path is the common case) |
| `class` instances / builtin `Option`/`Result` | deep copy; every field must itself be crossable |
| `Sender` / `Receiver` | transfer |
| closures | **not transferable** in v1 — compile-time error at the send/`spawn_worker` site |
| host opaque types | transfer **only** if registered `send` by the host (RFC 0005 §5) |
| anything else (e.g. `Task`, wakers) | compile-time error at the `send`/`spawn_worker` call site |

Enforcement is static at the send site (checker knows `T`) plus a runtime
`transfer_into` check on the receiving heap (unique-rc detection) —
implementation sketch in RFC 5003 §3.

- A trap inside a worker kills **only that worker**; `worker.join()` returns
  `Err(Trap)` and the host decides what to do (restart it, surface UI).
- The main VM **blocks on nothing**: pure rut code is always single-threaded
  per VM, which keeps the RC heap (RFC 0004) lock-free.

## 6. Replacing tur's flush loop

tur today: boa promise jobs + `WorkerMsg::Wake` + flush-driven sleep futures
(`core/async_/*`). With rut the frame loop becomes
`process_host_events()` → `rut.run_until_idle()` → park until
`next_deadline()` → render (Rust sketch in RFC 5003 §5). No job executor, no
`promise_to_future`, no `.then` under `Context`, no tslib `_ts_generator`
downleveling — `await` is an opcode.

## Open questions

- OQ-1: cancellation observation — should `Task<T>` await resolve to
  `Result<T, Cancelled>`, or should awaiting a cancelled task be a trap?
- OQ-2: priorities/fairness — round-robin ready queue in v1; do we need
  task priorities for UI responsiveness before M4?
- OQ-3: structured concurrency scopes with child cancellation.
- OQ-4: bounded channels and MPMC.
- OQ-5: cross-isolate sharing of immutable data (interned strings, type
  descriptors) — safe read-only page sharing vs strict copies (RFC 0001 OQ-5).
- OQ-6: `await` inside `catch`-style recovery after `?` — needs a
  `try { .. } catch`-over-Result sugar or stays explicit `match`.
