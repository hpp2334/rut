# RFC 0020: The Host-Futures Bridge

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0018 (suspend & await), RFC 0019 (tasks)
- **Supersedes:** RFC 0003 §4 + RFC 5003 §4 (pre-restructure)
- **Part:** D — Concurrency

## Summary

Native code may hand rut any Rust future; rut futures are pollable from
Rust. The only crossing point is a waker that enqueues a `Wake(task)` into
the owning VM's queue — the direct replacement for tur's
`WorkerMsg::Wake`-on-settle.

Consequences for the embedder:

- `sleep(ms)` is a native fn returning a future whose waker posts into
  the timer wheel — no promise objects, no job executor to drain.
- IO written with plain `suspend`/`await` Rust plugs in unchanged.
- Tests can virtualize time by faking the timer wheel (tur's test clock).

## Internals

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

## Open questions

- OQ-1: host-side suspend construction (a native fn like `newCanvas`
  returning a future of a handle — RFC 0025) — just a future resolving to a
  `Host` value; needs an example.
