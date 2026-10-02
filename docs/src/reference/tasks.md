# Tasks

A launched future's receipt —
`LaunchedFutureHandle<T>` — is its own type, and it is the entire
task-management surface. Everything in this chapter is spelled in that
vocabulary ([async and await](async.md)).

## Surface status

| Feature | Spelling | Status |
|---|---|---|
| launch | `launch_future(f)` | live |
| cancel | `h.abort()` | live |
| join | `await h` on a receipt | compile-gated — "not in this build" |
| race | `await select { .. }` | parses; semantics compile-gated |
| first-of | `select_all(futs)` (stdlib) | lands with `select` |
| structured scopes | `scope { .. }` | specified, not built |

## The receipt — `LaunchedFutureHandle<T>`

```rut
pub class LaunchedFutureHandle<T> {
    pub fn abort(mut self) -> bool;
}
```

Laws:

- The receipt is **not a `Future`**: `await h` diagnoses — the type
  system rejects it, never a runtime check.
- It is **not re-launchable**: `launch_future(h)` is a type error for
  the same reason. A future is consumed exactly once — by `await` or by
  `launch_future`, never both, never twice.
- One member: `abort()` — `true` if the frame was flagged and
  re-enqueued, `false` when it had already finished (a repeat abort is
  stable and harmless).

## Cancellation

`abort()` flags the frame's cancellation and re-enqueues it. **The
probe at its next checkpoint is the only place cancellation becomes
observable** — it never interrupts mid-expression.

At the probe, the frame runs its **drop path**:

1. the pending edge is cleared — a pending `sleep` dies with the frame;
2. every local releases **deterministically, in reverse declaration
   order** (`Disposal` impls run, [the Rc heap](rc-heap.md));
3. the state retires (null) — later `abort()` calls answer `false`.

Inside a future impl, `cx.cancelled()` reads the same flag as data, so
a hand-written frame can honor cancellation at its own pace
([the host futures bridge](host-futures.md)). Cancellation is
synchronous at the engine level (flag + re-enqueue); a frame parked on
a host future only observes the flag when the loop drives it again.

```rut
use core::{ Disposal, DisposalContext, RunContext };

class Drops {
    n: u32;
}

impl Disposal for Drops {
    fn dispose(mut self, cx: DisposalContext) { self.n += 1; }
}

async fn job(cx: RunContext, seen: ?Drops) -> nil {
    let buf = Drops { n: 0 };
    await sleep(5000);
    // if `job` is aborted while parked here, the probe at the sleep
    // checkpoint runs the drop path: buf's dispose fires, then the
    // locals release in reverse declaration order
}
```

## Join (specified)

The receipt is the join surface:
`await h` joins the launched future and produces its completion value.
Until the join tier lands, `await h` diagnoses with the join law and
the receipt stays non-awaitable.

Planned laws: joining an already-cancelled receipt yields the cancelled
case, never a trap; join re-enqueues the awaiter the same way an
in-body `await` parks, so the value surfaces on the driving loop, not
through a callback.

## `select` (specified)

```rut
await select {
    http.fetch(url)  resp  -> handle(resp),
    timeout(ms)             -> handle_timeout(),
}
```

- `await select { .. }` races futures; the **winner's value drives its
  arm**; the losers are dropped — which means **cancelled** (their drop
  paths run, pending sleeps die with them).
- Arms use `->` like `when`: `fut -> expr` discards the resolved value;
  `fut x -> expr` binds it to `x` (arm-local binding).
- `select_all(futs)` (stdlib, built on `select`) resolves with the
  first ready value.

The grammar parses today; the semantics are gated with
"`await select` is not in this build".

## Structure and fairness

- v1 tasks are **unstructured**: aborting a frame does not abort frames
  it awaits. Structured scopes — `scope { .. }` cancelling children on
  exit — are the specified remedy.
- The ready ring is **round-robin**: each drive runs a frame to its
  next park or completion, so one greedy future cannot starve the
  queue. Task priorities are not in the model.
- `vm.pending_tasks()` counts ready frames, armed timers, and host
  futures parked on Completers — the embedder's idle test
  ([async and await](async.md)).
- A trap inside a launched future propagates out of `run_ready()` /
  `drive()`; the frame is retired either way, and its locals ran their
  drop path.
