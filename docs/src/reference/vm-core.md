# VM core

One struct, one thread, one heap. The VM (`rut-vm`) interprets
[typed bytecode](typed-bytecode.md) over an RC heap with inline
destruction — no JIT, no garbage-collection pass, no background threads.

## The machine

```rust
pub struct Vm {
    pub prog: Rc<Program>,        // the linked program
    heap: Heap,                   // the RC heap — [The VM heap](vm-heap.md)
    frames: Vec<SavedFrame>,      // saved frames (the call stack)
    cur_func: u32, cur_pc: u32,   // the ACTIVE frame
    cur_regs: Vec<Slot>, cur_ret_dst: Option<Reg>,
    fuel: Option<u64>,            // per-op budget
    interrupt_every: u32,         // budget check cadence (default 1024)
    hooks: HostHooks,             // embedder surface
    ready: VecDeque<Slot>,        // launched/parked frames to (re)drive
    timers: BTreeMap<u64, Vec<Slot>>,  // sleep deadlines -> wake lists
    now_ms: u64,                  // the VM's virtual clock
    ...
}
```

A `Vm` is single-threaded by construction. Saved frames are plain
records `{ func, pc, regs, ret_dst }`; calls push, returns pop. Register
files are recycled through a pool, and per-function side tables
(reference-typed register indices, per-type field layouts, baked type
representations) keep the hot loop free of type-table lookups.

## The interpreter loop

The loop is a `match` over decoded ops — `decode` is a table index, not a
byte scan. On native targets the default engine is a **threaded**
dispatcher (`rut-vm-threaded`): every op ends by tail-calling the next
op's handler, so dispatch is a jump and hot state stays in registers. A
portable loop over the same handler methods backs wasm and other targets;
behavior is identical, only the dispatch shape differs. Ops the threaded
engine does not absorb *bail* to the match interpreter with the pc pinned
at the op.

- Direct calls are an index + jump; interface (itable) calls are two loads
  (itable row) + an indirect jump.
- Refcount retain/release are inline in the loop; a release to zero
  nulls the weak list, queues the type's `dispose` body (the
  `[disposal]` contract), and frees
  ([The Rc heap and destructors](rc-heap.md)).
- Fuel is accounted per op, with the fuller budget check amortized every
  `interrupt_every` ops.

## The host boundary

The embedder's host-fn binding table (`HostRegistry`) is built **before**
the `Vm` exists and is consumed at construction: every host thunk the
program declares is resolved against the registry immediately, so a
declared-but-unbound host fn is a *construction error*, never a mid-run
trap. The join produces one dense dispatch row per function — an adapter
code pointer, the body's state word, the declared return type, and its
is-reference verdict — so calling a host fn costs one table copy, no name
work, no allocation.

- Host bodies are plain Rust functions over `&mut Vm` and slots; arity is
  capped at 8 (the boundary's tuple law), and signatures are derived from
  the body's Rust shape, not written by hand.
- A host body records a trap in a VM-side channel rather than returning
  `Result`; the trap fires before any destination register is written.
- **Re-entrant calls** (a host fn calling back into rut) run the callee
  on a fresh frame stack under the same budget. The outer cursor is
  stash/restore: on return or trap the outer frame is exactly as it was,
  so a propagated nested trap parks the outer frame *at the host op* and
  `resume()` re-runs the host fn. A host fn that prefers to handle the
  situation itself can catch the trap, add fuel, and retry its nested
  call.
- The async host-fn lane (`register_async!`) binds a Rust async body to
  the minted `{scope}::{name}__start/__yield/__take/__cancel` rows the
  compiler weaves for `host async fn` declarations
  ([The host futures bridge](host-futures.md)).

## Traps

Bugs are traps, errors are values. Arithmetic overflow, division by
zero, indexing out of bounds, nil dereference,
`panic(...)`, a failed downcast guard, and budget exhaustion all unwind as
`Err(Trap)` — never a Rust panic:

```rust
pub struct Trap { pub kind: TrapKind, pub msg: String }

pub enum TrapKind {
    OutOfFuel, OutOfMemory, Interrupted,
    Overflow, DivByZero, IndexOutOfBounds,
    Panic, BadUnbox, Invalid, NilDeref,
}
```

**Traps are catchable only at the host boundary.** `vm.call(...)` returns
`Result`; rut code never catches one — errors in rut are `(T, err)` tuples
([Errors and optionality](../tutorial/errors.md)). On the way out,
frames are dropped and destructors run: `dispose` bodies fire, borrow
guards release. The `Vm` itself survives a trap — the next call starts
clean.

## Budgets and interrupts

```rust
pub struct Limits {
    pub fuel: Option<u64>,          // per-op countdown
    pub heap_limit_bytes: Option<u64>,
    pub interrupt_every: u32,       // default 1024
}
```

- **Fuel** counts ops down. Exhaustion parks the frame at its pc — the
  loop state *is* the frame — so `add_fuel(n)` + `resume()` continues it;
  nothing is lost or restarted. Loop back-edges carry `LoopHead`
  markers so tight loops still park.
- **Heap budget** is checked at every allocation; over-limit allocation
  traps `OutOfMemory` the same parkable way.
- **Heap accounting** is live: `heap_usage()` reports current bytes,
  `fuel_used` accumulates across resumes.
- **Interrupts** are cooperative: the budget check cadence bounds how
  long any host-requested stop can take. Native (host) bodies run outside
  the fuel budget and are expected to be fast.
- `Vm::builder().compiled(c).limits(l).build()` takes the limits
  (`.limits(..)` on the builder, or the parts a `Compiled` hands
  over); unset limits mean unbounded fuel and heap — embedders that
  need budgets pass them explicitly
  ([Resource limits and fuel](resource-limits.md)).

## Coroutines and the driving loop

Every async entry runs as a root frame, parked when `await`
returns pending. There is no microtask queue and no job executor — two
queues and a virtual clock:

- `ready: VecDeque<Slot>` — launched or woken frames; each entry owns one
  reference (retained on enqueue, released on pop).
- `timers: BTreeMap<u64, Vec<Slot>>` — sleep deadlines against the VM's
  **virtual clock** (`now_ms` / `set_now` — deterministic; the host
  advances it explicitly, or maps it to wall time).
- `drive(fut)` — one drive step through the future's designated yield
  slot (the `Future<T>` class's engine ABI): a fresh
  engine context over the frame edge, answering `Done` or `Parked` off
  the frame's state field. Fuel accounting rides the per-op budget
  unchanged: an exhausted drive parks the frame at its pc, and a re-drive
  re-enters at the checkpoint state.
- `run_ready()` drains the ready queue one drive per entry;
  `next_deadline()` expires due timers into `ready` and answers the
  earliest remaining deadline; `pending_tasks()` is the idle test.
- The wake pair (awaiter edge on the awaited frame, pending edge on the
  awaiter) is one-directional and cleared on resume — no cycle forms.
- A completed drive re-enqueues the frame's awaiter edge. A parked async
  frame is simply not in `ready` until its wake edge or timer fires.

The engine never owns a wall clock: sleeps are virtual-clock deadlines,
and tests virtualize time by advancing the clock — full determinism
([Async and await](async.md), [Launched futures](launched-futures.md)).

## Heap integration

Allocation, retain, and release are inline loop work. There is **no
collection pass at all**: destruction happens at release-to-zero, so
nothing ever interrupts a frame. Strong reference cycles leak by design;
the weak-reference machinery and the shutdown leak report are the
diagnostic surface ([Weak references and the cycle
collector](weak-refs.md), [The Rc heap and destructors](rc-heap.md)).
Heap layout, cell formats, and the bump allocator are [The VM
heap](vm-heap.md).
