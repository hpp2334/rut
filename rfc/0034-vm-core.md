# RFC 0034: VM Core — Interpreter Loop, Traps, Budgets

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0032 (LIR), RFC 0016 (heap), RFC 0018–0019 (tasks),
  RFC 0022 (host), RFC 0001 (M1)
- **Supersedes:** RFC 0008 §1–3, §6–7 (pre-restructure)
- **Implementation side:** heap internals in RFC 0016–0017, coroutine
  internals in RFC 0018, dispatch internals in RFC 0015.
- **Part:** F — Toolchain & artifacts

## Summary

One struct, one thread, one heap:

```rust
pub struct Vm {
    types:  TypeTable,               // global RutType descriptors + vtables
    heap:   Heap,                    // RFC 0016 §5 — RC + cycle collector
    mods:   Vec<Module>,             // linked images (RFC 0033 §1)
    frames: Vec<Frame>,              // the call stack
    ready:  Deque<TaskId>,           // woken coroutines (RFC 0018 §4)
    budget: Budget,                  // §4 — ops, deadline, interrupt
    host:   HostHooks,               // RFC 0035 §1 — loader, timers, natives
}
```

A `Vm` is `!Send` by construction (workers are separate VMs — RFC 0021).
Everything the host needs is methods on it; nothing else is public.

## 1. Frames & the interpreter loop

```rust
struct Frame {
    func: &'static FuncImage,        // linked code (RFC 0033 §1)
    pc: u32,
    regs: Vec<Slot>,                 // typed per the signature; verified
    state: u8,                       // suspend resume state (RFC 0018 §4)
    task: Option<TaskId>,            // frames may belong to a coroutine
}
```

The loop is a `match` over decoded ops (`decode` is a table index, not a
byte scan). Calls push frames; `ret` pops; ref discipline is emitted by the
compiler and asserted in debug builds (RFC 0016 §3). Direct calls are an
index + jump; interface calls are two loads + indirect jump (RFC 0015 §6);
native calls cross `Value` (RFC 0023). Every `callnat` additionally pushes
a one-word `Native { module, slot }` **marker frame** so traces can show
the host boundary (`at plugin:my_map.MyMap.set (host)`) — RFC 0036 §2.

## 2. Traps

Arithmetic overflow (RFC 0004 §3), `Option.value` on `None`, failed
assertions, `panic(...)`, stack/budget exhaustion, and verifier-impossible
states unwind as `Err(Trap)` — not Rust panics:

```rust
struct Trap { kind: TrapKind, msg: String, trace: RawTrace }  // RFC 0036 §2
```

The `RawTrace` (frame pcs + native-boundary markers) is captured at
unwind; **rendering is lazy** — names and source spans are looked up in
the image's `SymbolTable` only if someone prints it (RFC 0036). **Traps
are catchable only at the host boundary** (RFC 0001 P4): a
`vm.call(...)` returns `Result<Value, Trap>`; rut code never catches
one. Coroutine frames are dropped on the way out — destructors run
(RFC 0016 §3).

## 3. Coroutines & scheduling

- Every suspend entry runs as a `Task`: a root `CoroutineFrame` parked when
  `await` returns `Pending`. Wakers push `TaskId`s into `ready` (RFC 0020)
  — there is no microtask queue and no job executor.
- `spawn` creates a task; `cancel` marks it and drops the frame at its
  next suspension point; `select` is implemented in the VM because losers
  must be dropped atomically with the winner's resolution (RFC 0019).
- Timers (`sleep`) are host-provided (RFC 0035 §1): the VM never owns a
  clock. Tests virtualize time by faking the timer hook — full determinism.

## 4. Budgets & interrupts

- `Budget { max_ops: u64, deadline: Option<Instant>, interrupt_every: u32 }`
  checked every `interrupt_every` ops (default 1024): over budget →
  `Err(Trap::Interrupted)` with the frame parked, **resumable** — the loop
  state is the frame, so the host can continue it later (`vm.resume()`).
- `interrupt()` lets a UI host yield mid-frame (16 ms slices) and tests
  cut infinite loops; native fns run outside the budget and are expected
  to be fast (RFC 0022 §3).
- `run_until_idle()`: drain `ready` + all non-suspended frames until
  nothing is runnable. `poll(deadline)`: like `run_until_idle` but bounded
  by wall time. These are the only stepping verbs (RFC 0001 "Host
  stepping").

## 5. Heap & collector integration

RC ops are inline in the loop; a `release` to zero runs `dispose()` then
frees (RFC 0016 §3). Suspect-list growth triggers the cycle collector
between frames only — never mid-function (borrow guards, RFC 0023 §2, stay
trivially sound). `vm.collect_cycles()` lets the host force a pass
(RFC 0017 §2).

## Open questions

- OQ-1: frame reuse pool vs fresh `Vec<Slot>` per call — measure on
  recursive corpus code (quicksort, matrix-mul).
- OQ-2: `Trap::Interrupted` resumption granularity — mid-expression is
  fine (frames are resumable) but native-fn boundaries are not; document
  exact guarantees.
- OQ-3: `Vm::clone`-free snapshots for debugging/time-travel — defer.
