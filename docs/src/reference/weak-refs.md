# Weak references and the cycle collector

Reference counting frees an object when its strong count reaches zero
([the Rc heap](rc-heap.md)). A strong cycle never reaches zero — so
**strong cycles leak until the `Vm` is dropped**. rut states this as a
rule instead of hiding it behind a collector: the heap stays trivially
simple, destructors stay exactly deterministic, and `Weak<T>` gives the
two shapes that cause accidental cycles — back-pointers and caches — a
first-class answer.

## Cycle policy

| Rule | Content |
|---|---|
| No collector | No mark bits, no colors, no pauses beyond destructor chains. What leaks leaks wholly and predictably. |
| Resource holders | Classes holding resources (drop cleanups, host boxes) must **not** participate in strong cycles. |
| Back-pointers | Parent/child and observer shapes take a `Weak` back-pointer. |
| Pure-data cycles | Harmless — memory only, freed wholesale at teardown. |
| Lints | The compiler may warn on obvious self-reference (a value stored into its own field through a handle path); general cycle detection stays out of scope. |

Cycles are possible through ordinary dataclass fields (`next: ?Node`
back-pointers) as well as through collections.

## `Weak<T>`

`Weak` is a builtin generic class with one member. A weak box is a side
cell holding the referent's slot word, **unretained** — a weak never
keeps anything alive, and a path through a `Weak` does not close a
strong cycle.

```rut
use ink::{ Logger };

class Node {
    value: u32;
    next: ?Node;          // strong — keeps the tail alive
}

impl Node {
    pub fn new(value: u32) -> Self { return Self { value: value, next: nil }; }
}

fn use_node(n: Node) {
    let log = Logger.new("t");
    log.info(f"node {n.value}");
}

pub fn main() {
    let n = Node.new(1);
    let w = Weak.new(n);        // Weak<Node>; T infers from n
    let b = w.upgrade();        // ?Node — a live handle
    if (b != nil) {
        use_node(b);
    }
}
```

```text
node 1
```

### Construction — `Weak.new(v)`

- Construction is a **class method** with admission at the
  instantiation: `T` must be a **reference type**. `Weak<i32>` and
  `Weak.new(some_fn)` diagnose; primitives and `fn` values refuse.
  (The retired type-call spelling — a bare call of the type name —
  does not compile; the diagnostic names `Weak.new(v)`.)
- Works over **any cell**: a class, dataclass, `Vec`, `[T]`, enum, `str`,
  `bytes`, a user `opaque` box, or a host box.
- `Weak.new(nil)` traps ("weak on nil").
- **`Weak.new(v)` consumes the argument's temporary** and nulls its
  register (the engine's one consuming op): the weak observes the
  *binding's* lifetime, never a temporary's. `Weak.new(make())` watches
  a referent that dies as soon as the temporary is released —
  `upgrade()` answers `nil`. Bind the value first:

  ```rut
  use ink::{ Logger };

  struct Payload { n: i32; }

  fn make() -> Payload { return Payload { n: 1 }; }

  pub fn main() {
      let log = Logger.new("t");
      let v = make();
      let w = Weak.new(v);  // watches the binding v — lives as long as v does
      log.info(f"{w.upgrade() != nil}");
  }
  ```

  ```text
  true
  ```

- Two `Weak.new(v)` of one `v` are distinct boxes; `==` on weak boxes is
  identity, and two weaks of the same referent are not equal to each
  other.

### Upgrade — `w.upgrade() -> ?T`

- A dead-check plus retain: a live referent is retained into a fresh
  `?T` handle; a dead one answers the **null slot** — true `nil`, never a
  box containing nil.
- On `Weak<?U>` (legal) the answer is `??U` — the sticky-`?` law
  ([by-reference and nullable](by-reference-and-nullable.md)).
- `upgrade()` on nil traps ("upgrade on nil"); on a non-weak value it
  traps ("upgrade on non-weak").

## Deterministic ordering

The referent's death **nulls every weak box before anything that runs
user code** — before the `on_drop` pin check, before payload teardown.
A cleanup or dispose body that calls `upgrade()` sees `nil`, with no
window:

```rut
class Edge { back: ?Weak<Node>; }

on_drop(p, fn (q: ?Node) {
    // the parent died before this cleanup ran:
    // every weak pointed at it already reads nil
    if (q.back.upgrade() == nil) { /* always taken here */ }
});
```

Weak references are **not** destructors — they observe; they never run
code. They exist for caches, observers, and back-pointers.

## Leak reporting

- **rc discipline** is asserted in debug builds: retain/release pairing,
  no underflow, no double destruction.
- **Shutdown leak report**: at `Vm::drop`, surviving non-immortal
  objects are reported grouped by type, with allocation sites in debug
  builds — the primary tool for finding leaked cycles. Deterministic
  execution means a leak reproduces exactly.
- The host can read live usage at any time
  (`vm.heap_usage()`, [resource limits](resource-limits.md)).
- Fuzz targets assert no crash and that destructor counts match
  allocations minus reported survivors.

Status: the `Weak<T>` surface above is shipped; the shutdown report and
per-type live counts are the specified diagnostics tier.

If real code ever demands more, a collector could return only as a
separate, opt-in module — the heap assumes nothing of it today.
