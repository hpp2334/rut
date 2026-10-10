# 04 — Custom async

A **parse-only** example: `examples/04-custom-async/mod.rut`
is a single rut file with no Cargo harness — you cannot run it today,
and that is a deliberate disclosure, not an oversight. What it shows
is the async vocabulary from the *user* side: `async { }` blocks and
`async fn`s as the producers (the engine mints their frames; a user
type cannot BE a future — `Future<T>` is a closed class), the injected
`cx` read as data inside the async bodies, and a **user-written
launcher** with per-checkpoint stats and cancellation audits. The
runnable harness that would drive it end to end is the follow-up work;
the file is kept in the repo now because the parser's conformance
suite compiles every example, so this vocabulary is checked to stay
parseable ([Async and await](../reference/async.md)).

## "Run" it

There is no harness. The file is exercised by the parser conformance
test, which walks every `.rut` file in `examples/`:

```sh
cargo test -p rut-parser --test corpus    # parses every example, incl. this one
```

To *read* the same machinery running, see
[06 — GitHub viewer CLI](06-github-viewer-cli.md): its brain is an
`async fn` driven by the standard launcher this file re-spells by
hand.

## Code tour

### The audit log — the example's reason to exist

The point of the file is the accounting around the futures: every
launch, resumption, cancellation, and completion lands in one
module-owned record the embedder can read after the run:

```rut
struct AuditLog {
    launches: u32;      // futures this log saw started
    yields: u32;        // resumptions driven — one per checkpoint
    cancels: u32;       // cancellation probes that answered true
    checkpoints: u32;   // high-water mark of the context ledger
    finished: u32;      // runs that walked every step they booked
}
```

The launcher takes a shared `?AuditLog`, so the embedder holds one
cell and reads the whole run's story out of it afterwards — `summary`
renders the one-line audit.

### The producers — `async { }` blocks and the injected cx

Every async producer answers the ONE closed class: an `async fn` call,
an `async { }` block, `sleep` — all `Future<..>`, so the whole
await/select/launch vocabulary never asks "which kind". The
**`async { }` block is the primitive**: one expression form that mints
the frame where it stands (the mint is pure — legal in sync code); its
`return` answers the FUTURE, and its reads capture the enclosing
locals by value. `async fn` is genuine sugar over it — both sides of
that law are writable programs.

Inside an async body the weave **injects** the resume context as `cx`
(an async fn's parameters are ordinary values — a spelled `cx`
diagnoses). The frozen cx protocol reads as data: `checkpoint()`
answers this frame's resume state, `cancelled()` answers the frame's
abort flag — cancellation is a value the frame inspects, not an
exception it catches:

```rut
fn cancellable(log: ?AuditLog) -> Future<nil> {
    return async {
        while (true) {
            if (cx.cancelled()) {      // the probe, as data
                if (log != nil) {
                    log.note_cancel();
                }
                return;                // answers the FUTURE
            }
            await sleep(20);
        }
    };
}
```

### The user launcher

The engine's standard launcher set (`launch_future`,
`LaunchedFutureHandle`, `sleep`) is ordinary code any module may
re-spell — that is the point of `launch_custom`. It supplies the
driving and the audit over the same `Future` surface, so the producer
never knows which launcher started it:

```rut
use async_host::{ __abort, __launch };

fn launch_custom(f: Future<nil>, log: ?AuditLog) -> CustomLaunched {
    __launch(opaque(f));
    log.note_launch();
    return CustomLaunched { fut: f, log: log, alive: true };
}
```

The returned `CustomLaunched` receipt is deliberately **not** a
future — it cannot be awaited, and it is not re-launchable (that is
a type error, never a runtime check). Its one real member is the
cancel edge, which flags the frame and lets the loop's re-drive run
the probe.

The crossings cross as `opaque` — the engine rows' fixed ABI
([opaque](../reference/opaque.md)) — and the driving loop owns every
launched frame from there.

## Scope, honestly

Two boundaries to keep straight when you read this file:

- **Futures are engine-minted only.** `Future<T>` is a CLOSED builtin
  class: no constructor, no impl lane. The walls are the point —
  structural satisfaction cannot see engine identity, so nominal
  closure is the only spelling of "engine-minted only".
- **The file is parse-only** — nothing launches or drives it here.
  The runnable version of the same story is
  [06 — GitHub viewer CLI](06-github-viewer-cli.md), and the model
  behind it is [the async model](../core-concepts/async-model.md).

## Takeaways

- Every async producer has ONE type — `Future<T>`, the closed class;
  the hidden frame is representation, the handle is the surface.
- `async { }` is the primitive and `async fn` its sugar; a block mints
  where it stands, its `return` answering the future.
- The cx is injected, never spelled: `cx.cancelled()` and
  `cx.checkpoint()` are data reads inside any async body.
- A launcher is just code: driving + audit over the public `Future`
  surface, re-spellable by any module.
- The file is parse-only by disclosure; the runnable harness is
  follow-up work.
