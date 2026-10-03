# Launched futures

A launched future's receipt —
`LaunchedFutureHandle<T>` — is its own type, and it is the
entire management surface. Everything in this chapter is spelled in that
vocabulary ([async and await](async.md)).

## Surface status

| Feature | Spelling | Status |
|---|---|---|
| launch | `launch_future(f)` | live |
| cancel | `h.abort()` | live |
| join | `await h` on a receipt | compile-gated — "not in this build" |
| race | `select2(a, b)` (stdlib) | live |
| first-of | `select_all(futs)` (stdlib) | live |
| manual futures | `completer<T>()` (stdlib) | live |
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
   order** (`[disposal]` members run, [the Rc heap](rc-heap.md));
3. the state retires (null) — later `abort()` calls answer `false`.

Inside an async body, the injected `cx.cancelled()` reads the same
flag as data, so a frame can honor cancellation at its own pace.
Cancellation is synchronous at the engine level (flag + re-enqueue); a
frame parked on a host future only observes the flag when the loop
drives it again.

```rut
use core::{ DisposalContext };

class Drops {
    n: u32;
}

impl Drops {
    [disposal] fn release(mut self, cx: DisposalContext) { self.n += 1; }
}

async fn job() -> nil {
    let buf = Drops { n: 0 };
    await sleep(5000);
    // if `job` is aborted while parked here, the probe at the sleep
    // checkpoint runs the drop path: buf's `[disposal]` member fires,
    // then the locals release in reverse declaration order
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

## Racing — `select2` / `select_all` (stdlib)

```rut
use ink::{ Logger };
use async_host::{ launch_future, sleep, select2, Either2 };

async fn slow_fetch() -> str {            // the stand-in for any slow producer
    await sleep(200);
    return "data";
}

async fn fetch_or_timeout() -> str {
    let winner = await select2(slow_fetch(), sleep(50));
    if (winner.is_a()) {
        return winner.a_value();
    }
    return "timeout";
}

async fn run_race(log: Logger) -> nil {
    log.info(await fetch_or_timeout());   // "timeout" — sleep(50) wins the race
}

entry fn main() {
    let log = Logger.new("race");
    launch_future(run_race(log));
}
```

- `select2(a, b)` races two futures; the call **mints the race** —
  nothing runs until `await` (or `launch_future`) drives it. The
  answer is `Either2<T, U>`: `is_a()`/`a_value()` read the first
  future's side, `is_b()`/`b_value()` the second's. The winner's type
  is the fn signature's `Either2<T, U>` — computed at the call site,
  never a cast.
- The **losers are cancelled** the moment a winner is settled: their
  drop paths run (pending sleeps die with them, `[disposal]` members
  fire). A cancelled race (`h.abort()` on the race future) cascades
  through its children.
- `select_all<T>(futs)` races a whole `[Future<T>]` cohort and
  resolves with `(i, v)` — the winner's index and value — cancelling
  every other still-live member. Ties (two members already retired)
  break by index order.

`Either2` is an ordinary generic class (`left: ?A; right: ?B`) —
rut's enums are the C-style member set, so the two-sided answer is
private fields plus accessors, not pattern arms.

## Manual futures — `completer<T>()`

`completer<T>()` mints a cold `Future<T>` **and** the resolution right
beside it — the primitive that turns a plain callback into a future the
whole `await`/`select2` vocabulary drives:

```rut
use core::{ Future };
use ink::{ Logger };
use async_host::{ completer, Completer, launch_future, sleep };

// a PLAIN fn returning a Future; no async block needed:
fn fetch_like(tag: str) -> Future<str> {
    let (f, done) = completer<str>();
    launch_future(settle_later(done, tag));   // the "callback" side
    return f;
}

async fn settle_later(done: Completer<str>, tag: str) -> nil {
    await sleep(10);
    done.resolve(tag);
}

async fn consume(log: Logger) -> nil {
    let v = await fetch_like("tag!");         // parks until the resolve
    log.info(v);
}

entry fn main() {
    let log = Logger.new("completer");
    launch_future(consume(log));
}
```

- `done.resolve(v)` settles the future and wakes its awaiter; it
  answers `false` when the future already retired — resolved,
  cancelled, or abandoned (the same stability `abort()` has). The
  right consumes itself: one resolve.
- `Completer<T>` is only the **right**, not the future — a
  `LaunchedFutureHandle` sibling. `await` diagnoses on it; the engine
  minted future it settles is the awaitable.

## Structure and fairness

- v1 launched futures are **unstructured**: aborting a frame does not abort frames
  it awaits. Structured scopes — `scope { .. }` cancelling children on
  exit — are the specified remedy.
- The ready ring is **round-robin**: each drive runs a frame to its
  next park or completion, so one greedy future cannot starve the
  queue. Priorities are not in the model.
- `vm.pending_tasks()` counts ready frames, armed timers, and host
  futures parked on Completers — the embedder's idle test
  ([async and await](async.md)).
- A trap inside a launched future propagates out of `run_ready()` /
  `drive()`; the frame is retired either way, and its locals ran their
  drop path.
