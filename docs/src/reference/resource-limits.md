# Resource limits and fuel

Two knobs make rut safe for third-party code: a **heap budget** and
**fuel**. Both are enforced as **resumable traps**, both are set at
construction, and both bind on the VM's own accounting
([the VM heap](vm-heap.md)) — not on the OS.

```rust
pub struct Limits {
    pub fuel: Option<u64>,             // None = unbounded ops
    pub heap_limit_bytes: Option<u64>, // None = host-enforced only
    pub interrupt_every: u32,          // check period; default 1024
}
```

## The heap budget

| | |
|---|---|
| **What counts** | everything the VM heap tracks: cell headers + payloads, buffer blocks, string blocks, frames and register blocks, `opaque` store entries |
| **What does not** | module binaries, the shared type table, host-side Rust state |
| **Check points** | every `Heap::alloc` route: cell mint, `Vec` growth (`push` reallocation), string concat, frame-pool growth, host crossing mints |
| **On failure** | `Trap::OutOfMemory` — the check runs **before any write**, so the heap is byte-identical to its pre-op state; nothing is half-initialized |
| **Resumption** | the frame is parked: raise the limit (`Heap::set_limit`), drop references and retry, or drop the VM |
| **Observability** | `vm.heap_usage()` (live), `vm.heap_peak()` (high-water) |

`Vec` growth charges the budget before the write; shrinking is never
refunded (the accounting overcounts rather than undercounts).

## Fuel

- One unit per executed op; `fuel` counts **down**. The counter is
  decremented and checked on **every** op — `interrupt_every` meters
  the separate interrupt lane, not fuel.
- `Trap::OutOfFuel` parks the frame exactly like any resumable stop:
  nothing is unwound. Resumption is `vm.add_fuel(n)` then
  `vm.resume()` — the frame *is* the loop state. Fuel is chosen at
  construction: `add_fuel` is a no-op on an unbounded machine, and the
  runtime budget is read-only — by design.
- Fuel is the **deterministic** budget: the same program with the same
  fuel dies at the same op, every run — reproducible reports and hang
  proofs in tests.
- **Native fns run outside fuel.** A hanging native is a host bug: keep
  native bodies non-blocking, or push blocking work to a thread through
  the host-futures lane ([the host futures bridge](host-futures.md)).

## The trap contract

| Trap | Raised by | Frame state | Resumption |
|---|---|---|---|
| `OutOfMemory` | heap budget exceeded at an allocation check | parked before the write | raise limit / free refs → `resume()` |
| `OutOfFuel` | fuel reached 0 at a check point | parked at the exact `pc` | `add_fuel(n)` → `resume()` |

```rust
let limits = Limits { fuel: Some(1_000_000),
                      heap_limit_bytes: Some(64 * 1024 * 1024),
                      interrupt_every: 1024 };
let mut vm = Vm::builder().compiled(compiled).limits(limits).build()?;

match vm.call::<_, ()>("main", ()) {
    Err(t) if t.name() == "OutOfFuel" => {
        vm.add_fuel(1_000_000);
        vm.resume::<()>()?;           // re-enters at the parked pc
    }
    r => { r?; }
}
```

The async layer's resume granularity is the **checkpoint**, not the op:
a drive that runs out of fuel parks the future's frame, and a re-drive
re-enters at the frame's checkpoint state
([async and await](async.md)).

## Hang detection

The host owns time; the recipes differ by embedder:

| Host | Recipe |
|---|---|
| UI / game | drain the ready queue inside the frame loop; grant fuel per frame so a runaway script starves at the next check |
| wasm page | grant finite fuel; a watchdog stops refueling — the parked frame dies at its next check |
| desktop service | a watchdog thread flips a flag the host honors between `resume()` calls; no joins, no signals |
| tests | finite fuel + the virtual clock (`vm.set_now`) → fully deterministic hang proofs |

Anything blocking that cannot be bounded by fuel (IO, locks) belongs on
a worker thread answering a `Completer` — the VM thread never blocks on
it ([the host futures bridge](host-futures.md)).

## Fairness

Within one VM, the ready ring is round-robin: one greedy launched
future cannot starve the rest of a queue drain indefinitely — each
frame runs to its next park or completion, and fuel bounds each of
those runs.
