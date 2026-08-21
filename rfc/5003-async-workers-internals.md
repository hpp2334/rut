# RFC 5003: rut — suspend & Workers Internals (implementation)

- **Status:** Draft
- **Date:** 2026-08-21
- **Author:** hpp2334
- **Implements:** RFC 0003 (concurrency semantics) — this file holds the
  Rust/VM-side sketches; RFC 0003 holds the language-facing contract and the
  `examples/` references. Numbering: 5xxx = implementation RFCs.

## 1. Coroutine lowering

`suspend fn` bodies are split at each `await` into states; locals live across
suspensions in the coroutine frame's register file, itself a heap object
(refcounted, RFC 5004 §1). See RFC 0003 §2.1 for the pseudo-bytecode of
`examples/concurrency/fetch-page.rut`.

## 2. Core representation

```rust
/// One resumable suspend invocation. The register file persists across
/// suspensions, so `state` only selects the resume block.
pub struct CoroutineFrame {
    pub func: Gc<RutFunction>,   // the monomorphized suspend instance
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

The `await` opcode — poll the inner future once, suspend on Pending:

```rust
Op::Await { fut, dst } => {
    let f = self.heap.future(regs[fut].ptr)?;
    match f.poll_rut(self)? {                    // poll inner future ONCE
        Poll::Ready(v) => {
            self.heap.release(regs[fut].ptr);    // future consumed (RFC 5004)
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

## 3. Worker spawn & cross-isolate transfer

Static checks at the send site (the checker knows `T`), plus a runtime
`transfer_into` on the receiving heap (unique-rc detection):

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

## 4. Host-future bridging

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

## 5. The tur frame loop

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
