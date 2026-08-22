# RFC 0021: Workers & Channels — Isolate Concurrency

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0018 (suspend), RFC 0019 (tasks), RFC 0016 (heaps),
  RFC 0025 (host types `send`)
- **Supersedes:** RFC 0003 §5 + RFC 5003 §3 (pre-restructure)
- **Part:** D — Concurrency

## Summary

Workers are **separate VMs on separate threads** with separate heaps (RFC
0016). No shared memory; all data crosses typed channels. See
**`examples/workers/image-pipeline.rut`** (main side) and
**`examples/workers/image-worker.rut`** (worker side); a larger server-shaped
variant lives in `examples/network/echo-server.rut` + `echo-worker.rut`.

## 1. Channel semantics (v1)

- `Channel<T>()` returns a channel value with two endpoints, `sender` and
  `receiver` (unbounded, MPSC). `send(v)` is sync and cheap;
  `recv(): Future<Option<T>>` suspends until a message arrives, resolves
  `None` when every sender is gone. (Bounded/suspend-send and MPMC are OQ-1.)
- Channel endpoints are **transferable** values: pass one to a worker through
  `spawn_worker` args or through another channel.

## 2. What may cross an isolate boundary

| Type | Crossing rule |
|---|---|
| ints/floats/bool/char | copy |
| `string` | copy (immutable) |
| `bytes`, `Vec<T>` (T numeric/bool/char) | **transfer** if refcount == 1, else deep copy (zero-copy fast path is the common case) |
| `Array<T, N>` (fixed arrays are inline values) | deep copy, like a class instance — every element must itself be crossable |
| `dyn Slice<T>` **owned** cells (boxed `Array<T, N>`) | transfer if refcount == 1, else deep copy — same rule as `bytes` |
| `dyn Slice<T>` **views** (`Vec.as_slice()`) | **not transferable** — the view names a Vec cell in the sender's heap (compile-time error at the send site) |
| `class` instances / builtin `Option`/`Result` | deep copy; every field must itself be crossable |
| `Sender` / `Receiver` | transfer |
| closures | **not transferable** in v1 — compile-time error at the send/`spawn_worker` site |
| host opaque types | transfer **only** if registered `send` by the host (RFC 0025) — checked at the transfer, by `TypeId` |
| anything else (e.g. `Task`, wakers) | compile-time error at the `send`/`spawn_worker` call site |

Enforcement is static at the send site (checker knows `T`) plus a runtime
`transfer_into` check on the receiving heap (unique-rc detection) — §4.

- A trap inside a worker kills **only that worker**; `worker.join()` returns
  `Err(Trap)` and the host decides what to do (restart it, surface UI).
- The main VM **blocks on nothing**: pure rut code is always single-threaded
  per VM, which keeps the RC heap (RFC 0016) lock-free.

## 3. Worker lifecycle

`spawn_worker(module, args)` is a host hook (RFC 0035 §2): construct the
child VM, load the module, transfer args, run the worker's entry to
completion. Worker entry points receive transferred args as their
parameters (RFC 0003-style entry — the worker example pair shows the shape).

## 4. Internals: spawn & cross-isolate transfer

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

## Open questions

- OQ-1: bounded channels and MPMC.
- OQ-2: cross-isolate sharing of immutable data (interned strings, type
  descriptors) — safe read-only page sharing vs strict copies (RFC 0001).
- OQ-3: `chrecv` fairness across multiple awaiting receivers — FIFO per
  channel proposed.
- OQ-4: per-worker heap quotas (`vm.set_heap_budget`) so a runaway worker
  traps instead of OOM-ing the host — likely yes, cheap to add.
