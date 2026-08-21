# RFC 0003: Concurrency — Coroutines & Workers

- **Status:** Draft
- **Date:** 2026-08-21
- **Author:** hpp2334
- **Depends on:** RFC 0001 (pillars P2/P6), RFC 0002 (types), RFC 0004 (memory)

## Summary

rut's concurrency is **pull-based**: an async function compiles to a
resumable state machine (a *cold future*) that only progresses when polled —
Rust semantics with Kotlin ergonomics (`suspend` is spelled `async fn`, and
`await`/`spawn` read like Kotlin/JS). Workers are separate VM isolates
connected by typed channels; `bytes`/`array<T>` buffers can be *transferred*
(zero-copy) between them.

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

## 2. `async fn` and `await`

```rut
import { sleep } from "std:time";

async function countdown(n: u32): void {
    for (let i = n; i > 0; i -= 1) {
        console.log(i);
        await sleep(1000);          // <- the ONLY way to suspend
    }
    console.log("done");
}
```

Rules:

- `async function f(..): T` describes a function returning `Future<T>`.
  Calling it **does not run it** (cold). It runs when the future is `await`ed
  or `spawn`ed.
- `await` is only legal inside `async function`. There is no implicit yield
  anywhere else — no function is ever preempted mid-expression.
- `?` composes: `const body = await fs.read(path)?;` awaits, then propagates
  `Err` in one expression.
- A future may be awaited by exactly one task (`await` consumes it). Share a
  result by `spawn`ing a `Task<T>` and awaiting the task.

### 2.1 What the compiler emits

The body is split at each `await` into states; every local live across an
`await` is stored in the coroutine frame's register file, which lives in a
heap object (refcounted, RFC 0004). The function becomes:

```text
async function fetchPage(url: string): Result<bytes, HttpError> {
    const body = await http.get(url);    // state 0 -> 1
    const html = decode(body)?;          // stays in state 1
    return Result.ok(html);
}
```

lowers to (illustrative pseudo-bytecode):

```text
fetchPage$async(frame):
entry:
    call    http$get frame.url           -> f0
    await   f0                           -> body     ; suspend: state=1
state1:                                              ; resume jumps here
    call    decode body                  -> t1  ?    ; `?` propagates Err
    ret     Result.ok(t1)
```

Core representation (Rust sketch — illustrative, not final API):

```rust
/// One resumable async invocation. The register file persists across
/// suspensions, so `state` only selects the resume block.
pub struct CoroutineFrame {
    pub func: Gc<RutFunction>,   // the monomorphized async instance
    pub state: u32,              // resume index (set at each suspension)
    pub regs: Box<[Slot]>,       // typed per FnType (RFC 0002 §10)
    pub stack_base: u32,
}

/// Result of running a frame to its next stop.
pub enum Flow {
    Ret,       // returned; value in ret slot
    Suspend,   // hit `await` on Pending; frame saved on the task
    Trap(Trap),
}
```

The `await` opcode:

```rust
Op::Await { fut, dst } => {
    let f = self.heap.future(regs[fut].ptr)?;
    match f.poll_rut(self)? {                    // poll inner future ONCE
        Poll::Ready(v) => {
            self.heap.release(regs[fut].ptr);    // future consumed (RFC 0004)
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

## 3. Tasks: `spawn`, `cancel`, `select`

```rut
async function main(): void {
    const task = spawn(countdown(10));  // runs concurrently on this VM
    await sleep(2500);
    task.cancel();                      // stops at the NEXT suspension point
}
```

- `spawn(fut): Task<T>` schedules the future on the current VM. `Task<T>` is
  itself a `Future<Result<T, Cancelled>>`: `await task` joins it.
- `cancel()` marks the task cancelled and **drops the coroutine frame at its
  next suspension point** — pending `sleep`s die with it, and every local with
  a destructor (RFC 0004 §4) runs deterministically. Cancellation never
  interrupts mid-expression.
- `await select { .. }` races futures; the winner's value is produced by its
  arm expression, the losers are dropped (i.e. cancelled):

```rut
let outcome = await select {
    timeout(500)        => "gave up",
    button.onClick()    => "clicked",
};
```

- `selectAll(futs)` (stdlib, built on `select`) resolves with the first
  ready value — used in the worker example below.
- v1 tasks are unstructured (no automatic child cancellation). Structured
  scopes (`scope { .. }` cancelling children on exit) are OQ-3.

## 4. Host futures bridge both ways

Native code may hand rut any Rust future; rut futures are pollable from
Rust. The only crossing point is a waker that enqueues a `Wake(task)` into
the owning VM's queue — the direct replacement for tur's
`WorkerMsg::Wake`-on-settle.

```rust
/// Native awaitable: any `Pin<Box<dyn Future>>` adapts into a rut future.
pub trait RutAwaitable {
    fn poll(&mut self, vm: &mut Vm, waker: &Waker) -> Poll<Value>;
}

/// A rut task viewed from Rust (only usable on the VM's own thread).
impl std::future::Future for RutFuture {
    type Output = Result<Value, Trap>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let vm = self.vm();
        vm.wake_queue.register(cx.waker().clone(), self.task); // host wake -> task
        match vm.resume(self.task)? {
            Flow::Ret     => Poll::Ready(Ok(vm.take_ret(self.task))),
            Flow::Suspend => Poll::Pending,
            Flow::Trap(t) => Poll::Ready(Err(t)),
        }
    }
}

/// A waker that does exactly one thing: re-enqueue a task.
struct TaskWaker { vm: VmHandle, task: TaskId, queue: Arc<WakeQueue> }
impl std::task::Wake for TaskWaker {
    fn wake(self: Arc<Self>) { self.queue.push((self.vm, self.task)); }
}
```

Consequences for the embedder:

- `sleep(ms)` is a native fn returning a future whose waker posts into the
  timer wheel — no promise objects, no job executor to drain.
- IO written with plain `async`/`await` Rust plugs in unchanged.
- Tests can virtualize time by faking the timer wheel (tur's test clock).

## 5. Workers & channels

Workers are **separate VMs on separate threads** with separate heaps (RFC
0004). No shared memory; all data crosses typed channels.

```rut
import { channel } from "std:channel";
import { type Receiver } from "std:channel";
import { Job, type Image } from "imaging";

async function main(): void {
    const urls = ["a.png", "b.png", "c.png", "d.png"];
    const ports: array<Receiver<Image>> = [];

    for (let i = 0; i < urls.length; i += 1) {
        const jobCh = channel<Job>();        // main -> worker (MPSC)
        const imgCh = channel<Image>();      // worker -> main
        spawnWorker("imaging.rut", [jobCh.receiver, imgCh.sender]); // transfer
        jobCh.sender.send(new Job(urls[i]));
        ports.push(imgCh.receiver);
    }

    let left = urls.length;
    while (left > 0) {
        const img = await selectAll(ports);  // first decoded wins
        blit(img);
        left -= 1;
    }
}   // senders dropped -> each worker's recv() resolves None -> workers exit
```

```rut
// imaging.rut — a whole separate VM; its trap cannot hurt the main VM
import { type Receiver, type Sender } from "std:channel";
import { type Job, type Image, fetch, decode } from "imaging";

async function main(jobs: Receiver<Job>, out: Sender<Image>): void {
    while (true) {
        const job = await jobs.recv();       // Future<Option<Job>>
        if (job.isNone()) { break; }         // senders gone; exit, dropping `out`
        out.send(decode(fetch(job.value.url)));
    }
}
```

### 5.1 Channel semantics (v1)

- `channel<T>()` returns a channel value with two endpoints, `sender` and
  `receiver` (unbounded, MPSC). `send(v)` is sync and cheap;
  `recv(): Future<Option<T>>` suspends until a message arrives, resolves
  `None` when every sender is gone. (Bounded/async-send and MPMC are OQ-4.)
- Channel endpoints are **transferable** values: pass one to a worker through
  `spawnWorker` args or through another channel.

### 5.2 What may cross an isolate boundary

| Type | Crossing rule |
|---|---|
| ints/floats/bool/char | copy |
| `string` | copy (immutable) |
| `bytes`, `array<T>` (T numeric/bool/char) | **transfer** if refcount == 1, else deep copy (zero-copy fast path is the common case) |
| `class` instances / builtin `Option`/`Result` | deep copy; every field must itself be crossable |
| `Sender` / `Receiver` | transfer |
| closures | **not transferable** in v1 — compile-time error at the send/`spawnWorker` site |
| host opaque types | transfer **only** if registered `send` by the host (RFC 0005 §5) |
| anything else (e.g. `Task`, wakers) | compile-time error at the `send`/`spawnWorker` call site |

Enforcement is static at the send site (checker knows `T`) plus a runtime
`transfer_into` check on the receiving heap (unique-rc detection):

```rust
pub fn spawn_worker(&mut self, module: &str, args: Vec<Value>) -> Result<Worker, Error> {
    let child = Vm::with_runtime(self.runtime()); // shares type table + loader,
                                                  // NOT the heap
    child.load(module)?;
    for a in &mut args { a.transfer_into(&child.heap)?; } // move or deep-copy
    let join = child.spawn_main(args)?;
    let thread = std::thread::spawn(move || child.run_until_idle());
    Ok(Worker { join })       // Worker::join() -> Future<Result<(), Trap>>
}

impl Value {
    fn transfer_into(&mut self, dst: &mut Heap) -> Result<(), Error> {
        if let Value::Ref(p) = self {
            match dst_uses(dst, p) { /* numeric buffer + rc==1 */ true => {
                dst.adopt(p);          // steal the raw buffer, no copy
            } _ => {
                *self = deep_copy_into(self, dst)?;  // descriptor-driven walk
            }}
        }
        Ok(())
    }
}
```

- A trap inside a worker kills **only that worker**; `worker.join()` returns
  `Err(Trap)` and the host decides what to do (restart it, surface UI).
- The main VM **blocks on nothing**: pure rut code is always single-threaded
  per VM, which keeps the RC heap (RFC 0004) lock-free.

## 6. Replacing tur's flush loop

tur today: boa promise jobs + `WorkerMsg::Wake` + flush-driven sleep futures
(`core/async_/*`). With rut the same frame becomes:

```rust
// one engine frame (native side)
fn on_vsync(&mut self) {
    self.process_host_events();               // input, clipboard, ...
    self.rut.run_until_idle()?;               // resume ready tasks to completion
    if let Some(d) = self.rut.next_deadline() { // earliest timer
        self.worker_ctx.schedule_wake(d);     // host parks until then
    }
    self.render_frame();
}
```

No job executor, no `promise_to_future`, no `.then` under `Context`, no
tslib `_ts_generator` downleveling — `await` is an opcode.

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
