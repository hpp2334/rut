# Rc, dispose, and identity

One memory regime: every non-primitive value is a shared, refcounted
heap cell; destruction is deterministic; `==` is identity for cells.

## Reference semantics

Every non-primitive value — struct and class records, `str`, `bytes`,
`Vec`, `[T]`, enums, `opaque` boxes, interface-typed values, `?T` boxes,
closures' captured cells — is a **heap cell handle**:

- assignment, argument passing, and returning copy the handle
  (retain/release), an O(1) move;
- **mutation is visible through every alias**;
- writing is gated by the `mut`-binding law (see
  [Modules and visibility](modules-and-visibility.md)), never by the
  sharing.

There is no eager copy of any composite — bindings share by reference.
**`bytes.clone()` is the one copy escape hatch**: a one-shot deep copy
of a buffer's octets. Every other type shares on binding; a divergent
value of any other type is unreachable — build a new one instead.

Recursive shapes (`next: ?Node`, trees, lists) are legal: composite
fields and elements are pointer-sized handle slots, and the refcount
walk sees every handle field via the compile-time field table (see
[Reified types and layout](reified-types.md)).

## Disposal — the cell-death contract (the `[disposal]` marker)

Destructors are implemented, not attached. A type opts in with one
`[disposal]`-marked member on its inherent impl (the name is free —
the bracket designates):

```rut
use core::{ DisposalContext };
use ink::{ Logger };

struct Conn { log: Logger; url: str; }

impl Conn {
    [disposal] fn close(mut self, cx: DisposalContext) {
        self.log.info(f"closed {self.url}");
    }
}

entry fn main() {
    let log = Logger("t");
    let c = Conn { log: log, url: "tcp://edge" };
    log.info("main is done");
}   // c's refcount reaches zero here; dispose runs at the call boundary
```

```text
main is done
closed tcp://edge
```

- A `[disposal]` member runs when the cell's refcount reaches
  **zero** — deterministic destruction, not a collector callback. The
  dying value arrives as `mut self`; the fields release after the body
  returns. The member's NAME IS FREE — the bracket designates, the
  spelling never does (one `[disposal]` member per class).
- `cx: DisposalContext` is **engine-minted**, one per call. It is
  empty today — the parameter exists so the context can grow
  additively without ever touching the marker's signature.
- `DisposalContext` is the **import-gated** builtin class:
  `use core::{ DisposalContext };` brings it in
  ([core and the swappable packages](stdlib.md)). Using it without the
  use line diagnoses
  `` `DisposalContext` is not in scope — `use core::{ DisposalContext }` ``.
  The marker stands alone — there is no `Disposal` interface behind it
  and no import for the bracket word itself.
- One marked member per class, by the marker's own law — there is no
  per-value attach and nothing to attach twice.
- The body runs at a call boundary, not re-entrantly inside the
  release; the full sequence — weak boxes nulled first, then
  `dispose`, then the recursive field walk — is
  [the Rc heap](rc-heap.md)'s destruction order.
- The host side of the same law: a host payload's finalizer runs at
  cell death, before the payload's own Rust `Drop`.

Because every shared value now has a single knowable death — its
refcount reaching zero — a struct can carry its own destructor: no
wrapper class and no mint-site bookkeeping, just the impl.

## Weak references

`Weak<T>` (see [Builtin generic types](builtin-generic-types.md))
demotes any handle to a non-keeping reference:

```rut
use core::{ Weak };
use ink::{ Logger };

struct Tile { v: i32; }

entry fn main() {
    let log = Logger("t");
    let tile = Tile { v: 7 };
    let w = Weak.new(tile);        // does NOT keep the cell alive
    let got: ?Tile = w.upgrade();  // the live referent, or nil once dead
    log.info(f"{got.v}");
}
```

```text
7
```

The referent's death nulls every weak box before any user code runs.
A weak edge closes no cycle: strong cycles (two records holding each
other, a container that holds its own observer) still leak to engine
shutdown — the memory law is the program's; break cycles with `Weak`.

## Identity — `==`

`a == b` is a builtin operator: no dispatch, no opting in, no
element-wise story. `a != b` is its negation.

| Operand type | `==` means | Lowering |
|---|---|---|
| numeric / `bool` primitives | value | compare, IEEE 754 for floats (`NaN != NaN`, `-0.0 == 0.0`) |
| `str` | content (codepoints) | content compare |
| `bytes` | content (octets) | content compare |
| everything else — records, arrays, `Vec`, enums, closures, interface objects, `opaque`, `?T` | **cell identity** | the raw slot compare |

- Identity is the only `==` sharing can defend: with aliasing
  everywhere, structural equality of two independently built cells is
  ambiguous, and identity is O(1) with no deep walk.
  `[1, 2] == [1, 2]` is **false** — two cells.
- Enum members are immortal singletons, so `Flavor.Sour == Flavor.Sour`
  is `true` — the one place identity quietly behaves as value.
- `?T == ?T` is slot identity: two `nil`s are equal, a null and a box
  are not; `p == nil` derefs the nullable side and compares against the
  null slot.
- An **identity-compare lint** flags `==` between two obviously fresh
  composites (`Vec.from([..]) == Vec.from([..])`): "always false —
  compare fields". Asserting distinctness is legitimate and
  suppressible.
- Field-wise comparison is a library interface contract (`hash` + `eq`
  spelled per type) — the mechanism value-keyed maps ride. It is
  not connected to `==`.
- `when` literal patterns are unaffected: arms match compile-time
  values, never runtime `==`.
