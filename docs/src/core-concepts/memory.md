# Memory: the Rc heap, weak refs, and cycles

rut's memory management is **reference counting with deterministic
destructors and no collector**. An object is destroyed the moment its
last strong reference goes away — at a knowable statement, not "at some
future GC". The cost is honesty about cycles: a strong cycle never
reaches zero, so it lives until the VM is torn down. rut makes that a
stated rule and ships `Weak<T>` as the first-class answer, instead of
hiding it behind a collector.

## Everything is a cell

Every non-primitive value — strings, byte buffers, arrays, records,
enums, interface objects, erasure boxes, nullable boxes, coroutine frames —
is a heap cell. Each cell starts with a header: a refcount, a type id,
and a few flags. The counts are plain integers: a VM runs on one thread
with one heap, workers are separate VMs, and nothing is ever shared
between them, so there are no atomics anywhere in the heap.

Because assignments and passes copy *handles*, not payloads, the
refcount traffic is exactly the aliasing the program performs:

```rut
use ink::{ Logger };

struct Node { next: ?Node }

entry fn main() {
    let log = Logger.new("cells");
    let a: ?Node = Node { next: nil };
    let b = a;              // one retain
    log.info(f"a == b: {a == b}");   // one cell — it dies when the LAST of a, b goes away
}
```

```text
a == b: true
```

The compiler knows every register's static type, so it emits
ref-aware moves only where references flow; scalar code pays nothing.
Function boundaries do the paired inc/dec; a flat primitive buffer's
element traffic is plain slot moves.

## Deterministic destruction

When a count reaches zero, destruction runs in a fixed order:

1. **Weak boxes null first, then the `[disposal]` member.** A type
   carrying a `[disposal]`-marked member gets it called —
   `(mut self, cx: DisposalContext)`, the engine minting the cx — the
   engine pins the dying cell, queues the call, and the interpreter
   drains the queue at call boundaries. One marked member per class;
   `use core::{ DisposalContext }` brings the cx type in. Fields
   release after the body returns.
2. **Fields release in declaration order**, recursively — a dying
   record's strings, arrays, and boxes release their own references, so
   nothing is pinned until VM shutdown just because its owner died.
3. **Host opaques run their Rust `Drop` at the same point.** A host
   handle registered by the embedder releases its socket, texture, or
   thread when the last script reference disappears — never "someday at
   GC" (see [the host boundary](host-boundary.md)).

```rut
use core::{ DisposalContext };
use ink::{ Logger };

class Connection {
    url: str;
    log: Logger;
}

impl Connection {
    fn open(url: str, log: Logger) -> Connection {
        return Connection { url: url, log: log };
    }
}

impl Connection {
    [disposal] fn close(mut self, cx: DisposalContext) {
        self.log.info(f"closed {self.url}");
    }
}

entry fn main() {
    let log = Logger.new("rc");
    let conn = Connection.open("tcp://edge", log);
    log.info("main is done — the count hits zero at the boundary");
}
```

```text
main is done — the count hits zero at the boundary
closed tcp://edge
```

Compare this with finalizer-based designs: there is no finalizer that
"may run later or never", no flush phase, and no resource whose release
time you cannot point at in your own code.

Two edge rules worth knowing:

- **Refcount overflow pins.** If a counter would exceed its maximum,
  the object is deliberately immortalized (and logged) rather than
  wrapping — the same rule Swift uses.
- **Weak boxes null first.** When a cell with weak watchers dies, every
  `Weak` box to it is nulled *before* any user code runs — a dispose
  body that calls `upgrade()` sees `nil`, deterministically.

## Weak references

`Weak.new(v)` mints a `Weak<T>` over `v`'s cell that **never keeps
anything alive**; `w.upgrade()` answers `?T` — the retained referent, or
`nil` once it died. Construction is the class-method form, admission is
checked (reference types only — `Weak<i32>` diagnoses), and
`Weak.new(nil)` traps. The name is **import-gated** — spell
`use core::{ Weak };`, or the bare `Weak` diagnoses
`` `Weak` is not in scope — `use core::{ Weak }` ``.

```rut
use core::{ Weak };
use ink::{ Logger };

class Model {
    name: str;
}

class View {
    model: ?Model;
    observer: ?Weak<Model>;      // a back-pointer that closes no cycle
}

entry fn main() {
    let log = Logger.new("rc");
    let mut m = Model { name: "doc" };
    let v = View { model: nil, observer: Weak.new(m) };   // observe without owning
    log.info(f"holding {m.name}; the view holds only a weak edge");
    m = Model { name: "next" };   // the old cell's last strong reference dies here
    log.info(f"upgrade() answers nil: {v.observer.upgrade() == nil}");
}
```

```text
holding doc; the view holds only a weak edge
upgrade() answers nil: true
```

The two shapes that cause accidental cycles — **back-pointers**
(child → parent) and **caches** (a table that should not extend
lifetimes) — are exactly what `Weak` is for. A path through a `Weak`
does not close a strong cycle.

## The cycle policy

**A strong cycle is a leak — deterministically.** Its members live
until the VM is dropped; no collector ever runs, no pass interrupts
execution, and destructor ordering never surprises you. Guidance is
lint-level, not runtime:

- resource-holding types (types with `[disposal]` members or host
  handles) must not participate in strong cycles;
- parent/child and observer shapes take `Weak` back-pointers;
- pure-data cycles are harmless — they cost only memory.

You are not left alone with the rule: debug builds assert refcount
discipline (retain/release pairing, no underflow, no double free), and
at teardown the VM emits a **leak report** — surviving objects grouped
by type with allocation sites — which is the primary tool for hunting
accidental cycles. Behavior is fully deterministic, so a leak
reproduces exactly.

## The self-managed heap

Host code may use `std` freely, but every byte backing a rut value is
allocated through the VM's own heap manager. The discipline is the
point, not the allocator:

- **Cells** come from fixed-size arena chunks with freelists; a dead
  slot is reused by the next mint.
- **Variable-size payloads** (string octets, array element runs) live
  in size-classed blocks that never move; in-place geometric growth
  keeps append loops linear, and freelists recycle the common sizes.
- **Immortal singletons** — interned string literals, enum variants —
  live in a slab freed only at VM drop, carrying the "no refcount"
  sentinel.
- **Nothing compacts.** The heap never moves cells, which is precisely
  what lets the boundary hand Rust zero-copy views into buffers
  (see [the host boundary](host-boundary.md)).

Why self-manage instead of using the host's allocator?

1. **Budgets.** An embedder can say "this script gets N bytes" and get
   a clean, resumable out-of-memory trap instead of an OS abort.
   `vm.heap_usage()` is exact because every allocation is accounted.
2. **Teardown.** `Vm::drop` frees the whole heap in slab units after
   the leak report; nothing per-object leaks past the VM — important
   for wasm pages and plugin hosts that spawn and kill scripts.
3. **Leak reporting** walks the VM's own headers, which requires the
   VM's own headers everywhere.

The budget check runs *before* any write — a failed allocation leaves
the heap byte-identical — and traps resumably: raise the limit and
continue, or drop references and retry. See the reference on
[resource limits](../reference/resource-limits.md) for the full knob
set, and [the bytecode VM](the-vm.md) for how budgets interleave with
execution.

## What this buys you

- **Deterministic resource release** — the property the whole design
  is organized around. If your script opened something, the closing
  statement is knowable.
- **No pauses.** There is no collection pass; destruction happens
  inline at release-to-zero, so nothing ever interrupts a frame.
- **A small, honest mental model.** Sharing is by reference, copies are
  built explicitly (see [everything is a value](everything-is-a-value.md)),
  lifetimes are refcounts, and the one thing refcounts cannot do —
  cycles — is a documented rule with a first-class tool.
