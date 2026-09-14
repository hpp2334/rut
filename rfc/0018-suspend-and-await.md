# RFC 0018: `suspend` & `await` — Poll-Based Coroutines

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0016 (heap), RFC 0001 (pillar P6)
- **Supersedes:** RFC 0003 §1–2 + RFC 5003 §1–2 (pre-restructure)
- **Part:** D — Concurrency

## Summary

rut's concurrency is **pull-based**: a suspend fn compiles to a
resumable state machine (a *cold future*) that only progresses when polled —
Rust semantics with Kotlin ergonomics (the modifier is literally `suspend`,
like Kotlin; `await`/`spawn` read like Kotlin/JS). There are no promises, no
microtask queue, and no implicit scheduling: **the host owns time**, exactly
like tur's frame-driven flush loop.

## 1. Model: pull, not push

| | JS Promise (push) | rut Future (poll) |
|---|---|---|
| created | starts executing immediately on next microtask | cold — nothing runs until awaited or spawned |
| completion | pushes `.then` callbacks via job queue | nobody polls ⇒ nobody pays |
| allocation | promise + reactions per await | one coroutine frame per *task*, frames reused across awaits |
| cancellation | abort flag / never settles | drop the task — state machine and its resources are released **at the suspension point** (RFC 0019) |
| host integration | host must drive a job queue | host polls the VM: `run_until_idle()`, `next_deadline()` (RFC 0035) |

Cancellation-by-drop is the property tur currently emulates by aborting the
driver future (`TaskHandle::abort`); in rut it falls out of the model for free.

## 2. `suspend fn` and `await`

A `suspend fn` returns a **cold** `Future<T>` — calling it runs nothing;
`await` is the only suspension point:

```rut
suspend fn countdown(n: u32) -> unit {
    let log = Logger.new("countdown");
    for (let i = n; i > 0; i -= 1) {
        log.info(f"{i}");
        await sleep(1000);             // <- the ONLY way to suspend
    }
    log.info("done");
}
```

Rules:

- `suspend fn f(..) -> T` describes a function returning `Future<T>`.
  Calling it **does not run it** (cold). It runs when the future is `await`ed
  or `spawn`ed (RFC 0019).
- `await` is only legal inside `suspend fn`. There is no implicit yield
  anywhere else — no function is ever preempted mid-expression.
- `?` composes: `let body = await fs.read(path)?;` awaits, then propagates
  `Err` in one expression.
- A future may be awaited by exactly one task (`await` consumes it). Share a
  result by `spawn`ing a `Task<T>` and awaiting the task (RFC 0019).

## 3. What the compiler emits

The body is split at each `await` into states; every local live across an
`await` is stored in the coroutine frame's register file, which lives in a
heap object (refcounted, RFC 0016). This `fetch_page` — `await` and `?`
composing in one expression —

```rut
suspend fn fetch_page(url: str) -> Result<bytes, HttpError> {
    let raw = await get(url)?;       // await + `?` propagate in one expr
    let html = decode(raw)?;         // stays in the resumed state
    return Result.ok(html);
}
```

becomes (illustrative pseudo-bytecode):

```text
fetch_page$suspend(frame) ->
entry:
    call    http$get frame.url           -> f0
    await   f0                           -> raw      ; suspend: state=1
    chk     raw                          -> raw  ?   ; `?` propagates Err
state1:                                             ; resume jumps here
    call    decode raw                   -> t1  ?
    ret     Result.ok(t1)
```

## 4. Internals: the coroutine frame & the `Await` opcode

```rust
/// One resumable suspend invocation. The register file persists across
/// suspensions, so `state` only selects the resume block.
pub struct CoroutineFrame {
    pub func: Rc<RutFunction>,   // the monomorphized suspend instance
    pub state: u8,               // resume index (set at each suspension)
    pub regs: Box<[Slot]>,       // typed per FnType (RFC 0015 §5)
    pub stack_base: u32,
}

/// Result of running a frame to its next stop.
pub enum Flow {
    Ret,       // returned; value in ret slot
    Suspend,   // hit `await` on Pending; frame saved on the task
    Trap(Trap),
}
```

The `await` opcode — poll the inner future once, suspend on Pending:

```rust
Op::Await { fut, dst } => {
    let f = self.heap.future(regs[fut].ptr)?;
    match f.poll_rut(self)? {                    // poll inner future ONCE
        Poll::Ready(v) => {
            self.heap.release(regs[fut].ptr);    // future consumed (RFC 0016)
            regs[dst] = v;
        }
        Poll::Pending => {
            f.set_waker(self.current_task_waker()); // wake => resume this task
            return Ok(Flow::Suspend);
        }
    }
}
```

Resuming re-enters at the recorded state:

```rust
impl Vm {
    pub fn resume(&mut self, task: TaskId) -> Result<Flow, Trap> {
        let frame = self.tasks.frame_mut(task);
        self.pc = frame.resume_pc();             // stored at suspension
        self.run(frame)
    }
}
```

## Open questions

- OQ-1: `await` inside `catch`-style recovery after `?` — needs a
  `try { .. } catch`-over-Result sugar or stays explicit `match`.
