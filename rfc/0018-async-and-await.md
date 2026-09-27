# RFC 0018: `async` & `await` — the Future-Only Vocabulary

- **Status:** Landed (phase 2 of the async batch; the survey is
  `docs/async-survey.md`, the report `docs/async-report.md`)
- **Date:** 2026-08-22 · **Amended:** 2026-09-27 (the landing: the
  checkpoint desugaring replaces the `Await`-opcode sketch; `suspend`
  scrubbed from the vocabulary; the frozen surface is RFC 0012 §7's)
- **Author:** hpp2334
- **Depends on:** RFC 0016 (heap), RFC 0012 §7 (the frozen cx protocol),
  RFC 0001 (pillar P6)
- **Part:** D — Concurrency

## Summary

rut's concurrency is **pull-based**: an async fn compiles into the
engine-woven `impl Future<T>` for a hidden frame type — a *cold future*
that only progresses when driven. One noun (`Future<T>`), one consume
law (`await` **or** launch — exactly one), one pender (`sleep`, the
embedder's to mount). There are no promises, no microtask queue, and no
implicit scheduling: **the host owns time**, exactly like tur's
frame-driven flush loop. There is no `Task` concept anywhere, ever — the
launched future's receipt is its own type
(`LaunchedFutureHandle<T>`), and `suspend` is not part of the language.

## 1. Model: pull, not push

| | JS Promise (push) | rut Future (poll) |
|---|---|---|
| created | starts executing immediately on next microtask | cold — nothing runs until awaited or launched |
| completion | pushes `.then` callbacks via job queue | nobody drives ⇒ nobody pays |
| allocation | promise + reactions per await | one hidden frame record per call; its locals are cell-backed |
| cancellation | abort flag / never settles | abort flags the task; the probe at its checkpoint runs the drop path (RFC 0016 §3 order) |
| host integration | host must drive a job queue | host polls the VM: `run_ready()`, `next_deadline()` (RFC 0035) |

Cancellation-by-drop falls out of the model: the probe at the resumed
checkpoint branches to a drop path that releases the frame's cell-backed
locals (their `on_drop` callbacks fire by refcount), retires the state,
and returns.

## 2. `async fn` and `await`

An `async fn` returns a **cold** `Future<T>` — calling it runs nothing;
`await` is the only in-body suspension point; `launch_future` is the
only trigger:

```rut
async fn countdown(cx: RunContext, n: u32) -> nil {
    for (let i = n; i > 0; i -= 1) {
        rt.log(i);
        await sleep(1000);             // <- the ONLY suspension point
    }
}

fn main() -> nil {
    launch_future(countdown(3));       // trigger — receipt ignored
}
```

Rules (RFC 0012 §7's frozen surface):

- The first parameter IS the cx: `async fn f(cx: RunContext, ..)`.
  The engine mints it at call sites and per drive, the way it mints
  `self`.
- `async fn f(..) -> T` describes a value that widens to `Future<T>`.
  Calling it **does not run it** (cold). It runs when the future is
  `await`ed or launched.
- `await` is only legal inside an `async fn`, and in v1 it targets
  **engine-woven futures** (async-fn results, `sleep`) — the probe
  reads the engine-reserved state field a user `impl Future` does not
  have; user impls drive through launchers. Join lands with RFC 0019.
- `await select` keeps parsing; its semantics are RFC 0019's — the
  compile gate says "not in this build".
- A future may be awaited by exactly one driver (`await` consumes it;
  launch consumes it). rut does not move, so sharing a frame is a
  disclosed v1 misuse, not a soundness hole.
- `launch_future(launch_future(f))` is a **type error**: the receipt
  is not a `Future` (ruling 6).
- v1 crosses the launch boundary with raw slots (`any`) under the
  hood; the typed `Future<T> -> LaunchedFutureHandle<T>` arrow lives
  in rut code (the `async_host` inline package) where the type system
  holds it. User-written launchers over the same rows are first-class
  (RFC 0012 §7's "users may write their own launchers").

## 3. What the compiler emits — the checkpoint desugaring

One async fn compiles into ONE `FuncCode` — the woven `Future::yield` —
over existing ops only: a hidden `TyKind::Data` frame type, a per-fn
checkpoint `Enum` whose member index IS the resume state, and an
`Op::BrTable` dispatch. Layout (`rut_core::async_frame`):

```text
frame cell (TyKind::Data, hidden):  [0]=state   the checkpoint enum's
                                                member singleton; null =
                                                completed or dropped
                                    [1]=cancelled  the task's abort flag
                                    [2]=awaiter the frame awaiting THIS one
                                    [3]=pending the future THIS one parks on
                                    [4..]=locals  every binding, cell-backed

entry:   jmp l_dispatch
s0:      cancelled-probe -> drop | restore locals -> BODY
            … per await site:
               getf  st <- fut.state         ; the done probe (null = done)
               br    st -> drive, resume
            drive:  calli fut.yield(cx)      ; cold-poll one step
               getf  st <- fut.state
               br    st -> park, resume
            park:  setf fut.awaiter <- frame  ; the wake edge
               setf frame.pending <- fut
               enumnew sK ; setf frame.state <- sK
               ret                           ; suspension — the frame parks
            sK:  cancelled-probe -> drop | clear pending edges
               -> restore locals -> the continuation
            drop: setf state <- null ; release ref locals (binding order)
                  ret
done:    setf state <- null               ; completion discards the value
         ret
l_retire: ret                             ; a re-drive of a finished frame
l_dispatch: getf state <- frame ; brtable state [s0..sK] default l_retire
```

Laws the desugaring bakes in:

- **The state field is the pc.** The dispatch reads the frame's state
  and the `BrTable` lands in the arm — "the pc is the state" is
  literal. The DONE sentinel is the null slot, so a completed frame
  re-drive answers at the brtable's default arm (a silent retire).
- **Locals are cell-backed uniformly.** Every binding mirrors into the
  frame cell (`SetF` at binding, at assignment, and a restore at each
  arm), so a park's `Ret` releases only staging registers and no
  liveness analysis decides which locals survive.
- **The cx is stateless.** `checkpoint`/`cancelled` are field reads on
  the frame edge the cx carries; the driving loop mints a fresh cx per
  drive, so the argv pair is always fresher than any mirror.
- **Zero new vocabulary.** No new ops, no `Nat`/`Intrinsic` rows, no
  new `TyKind` — VERSION stays 13 (RFC 0032's note). The old `Await`
  opcode sketch and the `CoroutineFrame`/`Flow::Wait` machinery of the
  draft are superseded: the poll is a plain `CallI` through the
  future's vtable row, and suspension is a plain `Ret` whose caller
  (the driving loop) reads the state field for the answer
  (`Drive::Done | Drive::Parked`).

## 4. The driving loop — the VM's queues and the engine halves

The VM owns the queues; the embedder owns the loop (RFC 0035 §4):

- `vm.launch(slot)` — enqueue (the queue owns one reference).
- `vm.cancel(task) -> bool` — flag the task's cancellation and
  re-enqueue; `false` when already retired. `abort`'s engine half.
- `vm.drive(fut) -> Drive` — one re-entrant call of the future's
  `yield` (the `InterpCursor` stash/restore), a freshly minted cx over
  the frame edge; the answer reads off the state field. A completed
  frame re-enqueues its `awaiter` edge — the one-directional pair
  (awaiter edge + pending edge) never forms a cycle.
- `vm.arm_timer(deadline, slot)` / `vm.now_ms()` / `vm.set_now(t)` —
  the deterministic virtual clock; `sleep`'s engine-backed yield arms
  its deadline on the fresh arm and retires on the resumed one.
- `vm.next_deadline()` — expire due timers into the ready queue,
  answer the earliest remaining deadline; `vm.run_ready()` drains the
  queue; `vm.pending_tasks()` is the idle test.

Fuel accounting rides the existing per-op budget (RFC 0034/0040); a
drive that runs out of fuel propagates `OutOfFuel` with the frame
parked at its pc, and a re-drive re-enters at the frame's checkpoint
state — the checkpoint, not the op, is the async layer's resume
granularity (disclosed v1 semantics).

## Open questions

- OQ-1: `await` inside `catch`-style recovery after `?` — needs a
  `try { .. } catch`-over-Result sugar or stays explicit `match`.
- OQ-2 (new): `cx.next_checkpoint(v)` from user bodies — a checkpoint
  value is the checkpoint enum's member singleton and a runtime `u32`
  cannot name one without new vocabulary; the weave writes the states
  it owns. A user-spelled checkpoint algebra is RFC 0019 territory.
