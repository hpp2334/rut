# Workers and channels

Workers are **separate VMs on separate threads** with separate heaps.
There is no shared memory: everything that crosses is a value on a
typed channel, moved or deep-copied under the rules below. Pure rut
code inside one VM is always single-threaded, which is what keeps the
RC heap lock-free ([the Rc heap](rc-heap.md)).

Status: this page specifies the isolate surface; it is not yet wired
into the engine. The async surface it builds on — futures, launch,
await — is live ([async and await](async.md)).

## Channels

`Channel<T>()` produces a channel value with two endpoints:

| Endpoint | Type | Behavior |
|---|---|---|
| `sender.send(v)` | sync | unbounded buffer; `send` never blocks and is cheap |
| `receiver.recv()` | `Future<?T>` | suspends until a message arrives; answers **`nil` once every sender is gone** |

Semantics:

- Channels are **MPSC** (many senders, one receiver), unbounded.
- Endpoints are ordinary values and **transferable**: pass one to a
  worker through `spawn_worker` args, or through another channel.
- Receiving `nil` is the shutdown signal — dropping the sender half is
  how a parent tells a worker to finish.
- Closing semantics pair with the nullable answer, not an error type
  ([by-reference and nullable](by-reference-and-nullable.md)).

Typical shape:

```rut
use core::{ RunContext };

async fn worker(cx: RunContext, rx: Receiver<u32>, tx: Sender<u32>) -> nil {
    while (true) {
        let msg = await rx.recv();
        if (msg == nil) { return; }       // all senders gone
        tx.send(1);                       // acknowledge (send is sync)
    }
}
```

## What may cross an isolate boundary

| Type | Crossing rule |
|---|---|
| primitives (ints, floats, `bool`) | copy |
| `str` | copy (immutable) |
| every other cell — `Vec<T>`, `[T]`, class/struct instances, enums, `?T` boxes | **transfer** if `rc == 1`, else deep copy — the zero-copy fast path is the common case; every element/field must itself be crossable |
| `Slice<T>` view | same rule as its owner cell; provenance (which `Vec`/`[T]` it views) is invisible across the boundary |
| `Sender` / `Receiver` | transfer |
| `Template` | copy — a builtin carrier; its `opaque` args must themselves be crossable ([templates](templates.md)) |
| host opaque types | transfer **only** if the host registered `send` for them — checked at the transfer, by type |
| closures | **not transferable** — compile-time error at the `send`/`spawn_worker` site |
| anything else (receipts, wakers) | compile-time error at the call site |

Enforcement is two-layered:

1. **Static** at the send/spawn site — the checker knows `T` and
   rejects non-crossable types (closures, unregistered host types).
2. **Runtime** on the receiving heap — `transfer_into` detects
   uniqueness: an `rc == 1` buffer is *adopted* (the raw block moves,
   no copy); anything shared is deep-copied through a descriptor-driven
   walk.

Transfers move **accounting**, not just pointers: a moved buffer's
bytes are charged to the child heap at the transfer
([the VM heap](vm-heap.md)).

## Worker lifecycle

```rut
let w = spawn_worker("workers/fetch", (url, ch.sender()));
```

`spawn_worker(module, args)` is a host hook:

1. construct a child `Vm` sharing the **type table and loader — never
   the heap** ([the VM heap](vm-heap.md));
2. load the module;
3. transfer the args into the child heap — charged to the **child**
   budget before the worker starts, so a worker cannot OOM its parent
   ([resource limits](resource-limits.md));
4. run the worker's entry to completion on its own thread; entry
   parameters **are** the transferred args;
5. return a `Worker` handle; `worker.join() -> Future<Result<(), Trap>>`
   completes when the entry returns.

## Isolation laws

- **A trap inside a worker kills only that worker**: `join()` answers
  the trap and the host decides — restart it, surface it, ignore it.
  The parent VM and its other workers are untouched.
- **The main VM blocks on nothing**: `join()` is a future, channel
  receives are awaits; the parent's driving loop keeps running while
  workers compute.
- **No data races by construction**: nothing is shared, so there is
  nothing to lock. Two isolates communicate only through transferred
  values and channel messages.
- A worker's exit does not close channels it received senders from —
  the shutdown signal is the *drop* of the sender endpoints, which
  falls out of the worker's heap teardown.

Open surface: bounded channels and MPMC; cross-isolate sharing of
immutable data; receive fairness across multiple awaiting receivers
(FIFO per channel proposed).
