# 04 — Custom async

A **parse-only** example: `examples/04-custom-async/custom_async.rut`
is a single rut file with no Cargo harness — you cannot run it today,
and that is a deliberate disclosure, not an oversight. What it shows
is the async vocabulary from the *user* side: a hand-written future
type, an `impl` of the engine's built-in `Future` trait, and a
user-written launcher with per-checkpoint stats and cancellation
audits. The runnable harness that would drive it end to end is the
follow-up work; the file is kept in the repo now because the parser's
conformance suite compiles every example, so this vocabulary is
checked to stay parseable
([Async and await](../reference/async.md)).

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

`CustomFuture` is a stage machine, but the *point* of the file is the
accounting around it: every launch, resumption, cancellation, and
completion lands in one module-owned record the embedder can read
after the run:

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

### The machine — what `async fn` weaves, spelled out

When you write `async fn`, the engine weaves a hidden frame whose
shape is exactly this: a stage counter, the payload the stages carry,
and a done flag. `CustomFuture` spells that shape out in source, so
you can see what the keyword hides. The machine itself is plain rut —
only the *driving harness* is missing — so the block below adds a
`main` that advances it by hand; that is as far as one can run it
today:

```rut
use ink::{ Logger };

struct CustomFuture {
    steps: u32;      // checkpoints booked before the run finishes
    stage: str;      // what the current stage does (the audit's label)
    done: bool;      // the last stage ran
}

impl CustomFuture {
    fn new(steps: u32, stage: str) -> Self {
        return Self { steps: steps, stage: stage, done: false };
    }

    /// One stage of work — what one resumption advances. `false` when
    /// no stages are left: the runner's completion signal.
    fn advance(mut self) -> bool {
        if (self.done) {
            return false;
        }
        if (self.steps == 0) {
            self.done = true;
            return false;
        }
        self.steps -= 1;
        return true;
    }
}

pub fn main() {
    let log = Logger.new("future");
    let mut f = CustomFuture.new(3, "poll");
    let mut polls = 0;
    while (f.advance()) {
        polls += 1;
    }
    log.info(f"3 steps took {polls} advancing polls, then advance() answered false (done={f.done})");
}
```

```text
3 steps took 3 advancing polls, then advance() answered false (done=true)
```

### The trait surface: a user impl of a built-in trait

`Future` is an engine-named built-in trait — the engine weaves it for
every `async fn`'s hidden frame — but built-in traits are engine
*named*, not engine *closed*. A hand-written machine registers
through the ordinary nominal path, one impl block, and rides the same
drive law ([Traits and dispatch](../core-concepts/traits-and-dispatch.md)):

```rut
impl Future<nil> for CustomFuture {
    fn yield(self, cx: RunContext) {
        // the ledger view: the frame's resume state, read as data
        let at = cx.checkpoint();
        // the cancellation probe: a cancelled frame stops advancing —
        // the audit sees it as "died here, did not advance" — and runs
        // its drop path instead of the next stage
        if (cx.cancelled()) {
            self.done = true;
            self.stage = f"{self.stage}@{at}:cancelled";
        }
    }
}
```

The receiver **is** the frame — the machine's fields are its state —
and the context is the only handle a resumption needs. The frozen
context protocol reads as data: `checkpoint` answers this frame's
resume state, `cancelled` answers the task's abort flag. Cancellation
is a value the frame inspects, not an exception it catches.

### The user launcher

The engine's standard launcher set (`launch_future`,
`LaunchedFutureHandle`, `sleep`) is ordinary code any module may
re-spell — that is the point of `launch_custom`. It supplies the
driving and the audit over the same `Future` surface, so the machine
never knows which launcher started it:

```rut
use async_engine::{ __abort, __launch };

fn launch_custom(f: Future<nil>, log: ?AuditLog) -> CustomLaunched {
    __launch(opaque(f));
    log.note_launch();
    return CustomLaunched { task: f, log: log, alive: true };
}
```

The returned `CustomLaunched` receipt is deliberately **not** a
future — it cannot be awaited, and it is not re-launchable (that is
a type error, never a runtime check). Its one real member is the
cancel edge, which flags the task and lets the loop's re-drive run
the probe:

```rut
    fn cancel(mut self) -> bool {
        if (!self.alive) {
            return false;
        }
        self.alive = false;
        let t = self.task;
        if (t == nil) {
            return false;
        }
        let log = self.log;
        if (log != nil) {
            log.note_cancel();
        }
        return __abort(opaque(t));
    }
```

Alongside sits `audit_drive`, the audit twin of the engine loop's
drive columns: run a stage, count the resumption, stop on completion.

## Scope, honestly

Two boundaries to keep straight when you read this file:

- **User futures are launcher-drivable** — the driving loop finds the
  `Future::yield` row in the vtable, exactly as it does for woven
  frames.
- **`await` targets engine-woven futures only** in the current build
  (async-fn results and `sleep`); awaiting an arbitrary user impl,
  and joining concurrent futures, land with the next async milestone.

So today the file is a parse-checked design corpus and a teaching
document: the shape of the weave, the shape of a launcher, and the
proof that the future trait is a user-extensible surface — not a
runnable demo. When you want the runnable version of the same story,
read [06 — GitHub viewer CLI](06-github-viewer-cli.md), and for the
model behind it, [the async model](../core-concepts/async-model.md).

## Takeaways

- `async fn` compiles to a hidden frame implementing `Future` — this
  file spells that frame out in user source.
- Built-in traits are engine-named, not engine-closed: `impl
  Future<nil> for YourType` registers through the ordinary path.
- Cancellation is data the frame reads (`cx.cancelled()`), and every
  checkpoint is observable (`cx.checkpoint()`).
- A launcher is just code: driving + audit over the public `Future`
  surface, re-spellable by any module.
- The file is parse-only by disclosure; the runnable harness is
  follow-up work — the machine itself is plain rut (it ran above),
  but nothing launches or drives it as a future yet.
