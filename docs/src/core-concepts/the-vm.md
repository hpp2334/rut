# The bytecode VM

Everything a rut program becomes is a **typed bytecode** executed by a
register VM. There is no JIT, no interpreter tiering, no runtime
speculation: the compiler does all optimization ahead of time, and what
runs is exactly what was compiled. This chapter is the tour of that
artifact — the instruction set, the module binary, the load-time
verification, and the interpreter's execution and budgeting model.

## One struct, one thread, one heap

```rust
pub struct Vm {
    types:  TypeTable,     // runtime type descriptors + vtables
    heap:   Heap,          // the refcounted, self-managed heap
    mods:   Vec<Module>,   // linked binaries
    frames: Vec<Frame>,    // the call stack
    ready:  Deque<Slot>,   // woken async frames
    timers: BTreeMap<u64, Vec<Slot>>,  // the virtual clock's wheel
    lim:    Limits,        // fuel, heap budget, interrupt interval
    host:   HostHooks,     // registered native bodies, clock, loader
}
```

A VM is single-threaded by construction (`!Send`); workers are separate
VM instances communicating by messages. The host holds one and drives
it with methods: `call` an entry point, `run_ready` to drain woken
frames, `next_deadline` for the timer wheel, `drive` for a single
resumption. See [the host boundary](host-boundary.md) for the embed
loop and [the async model](async-model.md) for the queues' role in
scheduling.

## Typed bytecode

The instruction set is register-based: operands are `u16` indices into
a per-frame slot file, and **every register has a static type recorded
in the function's signature**. That single decision is what lets the
hot path stay tag-free (see [reified types](reified-types.md)) and what
makes refcounting precise: the compiler emits plain moves for scalars
and retain/release moves for references, with no runtime tag checks.

Ops are small and fixed-size, with variadic operand lists (call
arguments, branch tables) living in per-function pools. A representative
sample:

```text
mov / movref   rD, rS        ; move; the ref form retains new, releases old
i32add, f64mul, ...          ; typed arithmetic — the opcode selects the
                             ; kind, the width rides the op
icmp, fcmp     rD, a, b      ; compares -> bool
call / callm   f, args       ; direct call / direct method call
calli          slot, recv    ; trait vtable call (dynamic dispatch)
callnat        nat, args     ; internal natives (string concat, ...)
newcell / getf / setf        ; mint a record, read/write a field
arrget / arrset              ; indexing — bounds checked, trap on miss
tidof          rD, rO        ; read a value's runtime type id
brtable        rIdx, arms    ; enum matching, resume dispatch
```

The type system also decides what is *not* an op:

- **Nothing static.** What the type table can answer at compile time
  becomes a constant — no "get type id" instruction, no length load for
  a fixed-size array.
- **Nothing polymorphic and nothing named.** A trait-typed receiver has
  exactly one op (the vtable call); string concatenation is a native
  call, not an opcode.
- **Concrete memory, control flow, and two readbacks.** Ops touch
  memory only through compile-time-known layouts, plus the two reads
  reification needs: "what is it" (`tidof`) and the guarded payload
  extract behind a downcast.

## The module binary

Compilation produces a versioned, self-contained binary per module:
types, constants, functions (with their typed register signatures),
imports, and a symbol table for backtraces. Its properties matter more
than its layout:

- **Deterministic.** The same sources plus the same dependency versions
  produce byte-identical binaries, so artifacts are cacheable by
  content hash and shippable instead of sources.
- **Self-describing but strippable.** Names and source spans are
  retained for traces by default; a release build can strip them, and a
  sidecar map file (keyed by content hash) can restore symbolication
  later. See the reference on
  [diagnostics](../reference/diagnostics.md).
- **Link-time identity.** Type ids are module-local at rest and rebased
  into the VM's global table at link, where impl registries merge and
  duplicates are rejected. Cross-module trait calls specialize per
  concrete argument here too — the static-dispatch rule of
  [traits and dispatch](traits-and-dispatch.md) holds across module
  boundaries.
- **Little-endian everywhere**, by law — every serialized word, so
  artifacts are platform-independent by construction rather than by
  luck.

## Verification at load

Loading re-checks each function before anything executes: register
operands against the signature, operand pools and jump targets in
range, type operands present in the type table, native slots declared,
and async state tables closed. A failed verification is a load error
naming the module and function — **corrupted binaries never execute**.
This is also where crossing rules are enforced for host-facing
signatures, so a non-crossing shape is rejected before any script runs.

Module-level initialization is folded at compile time: literals, enum
members, constant expressions, and interned string literals become
constants in the binary. A user function call in a module initializer
is a compile error — there is no load-time execution at all. Loading a
module runs nothing.

## The interpreter loop and traps

Execution is a decode-dispatch loop over frames. A frame is the
function's code, a program counter, its typed register file, and (for
async frames) the resume state. Calls push frames; returns pop them;
direct calls are an index plus jump, and trait calls are two loads plus
an indirect jump through the vtable.

Failure is a **trap**, not an exception and not a Rust panic: integer
overflow on the plain operators, a `nil` deref, an out-of-bounds index,
a failed `assert` or `panic(..)`, or an exhausted budget. A trap
captures the raw frame chain (rendered lazily — symbolication happens
only if someone prints it), runs destructors on the way out, and
unwinds as an `Err(Trap)` to the host call. Rut code cannot catch one;
the boundary is the catch point (see
[the host boundary](host-boundary.md)).

## Budgets and interrupts

Two knobs make it safe to run third-party code, both set at
construction and mutable while running:

```rust
Limits { fuel: Option<u64>,            // one unit per op, counts down
         heap_limit_bytes: Option<u64>,// checked at every allocation
         interrupt_every: u32 }        // default: check every 1024 ops
```

- Exhaustion is **resumable**: the trap parks the frame exactly where
  it was — the frame *is* the loop state — so the host can add fuel,
  raise the heap limit, or flip the interrupt flag and resume. A UI
  host yields mid-frame; a wasm page grants finite fuel; a test sets
  finite fuel plus a virtual clock and gets a deterministic hang proof.
- The heap check runs *before* any write, so an out-of-memory failure
  leaves the heap byte-identical (see
  [memory](memory.md)).
- Native functions run outside the fuel budget — a hanging native is a
  host bug by contract — and the interrupt hook is the host's lever at
  native return points.

## Why no JIT is viable

A JS engine's speed is its JIT, and the JIT exists to speculate about
polymorphic property loads. rut has no polymorphic loads: field access
is by known index, calls bind statically whenever the site names one
concrete type, generics are monomorphized, and the remaining dynamic
dispatch is an honest checked vtable hop. The optimizer's currency is
**type stability** — what the checker proves is everything the
optimizer gets — and its passes (constant folding, inlining,
monomorphization, register-level cleanup) all run ahead of time. The
result is a runtime with no warmup curve, no deoptimization
asymmetries, and per-op costs you can reason about from the bytecode
alone.

The full tables and laws live in the reference: [typed
bytecode](../reference/typed-bytecode.md), [the module binary and
verification](../reference/module-binary.md), [VM
core](../reference/vm-core.md), and [resource limits and
fuel](../reference/resource-limits.md).
