# opaque — erasure and downcast

`opaque` is the erasure-box engine primitive: a concrete type whose
values box any value — **forgetting the static type and keeping the
runtime type for downcast only**. Erasure is a call you write; the
static type system never loosens, and rut has no cast syntax for it
(`as` is the numeric cast only).

## The surface

`opaque` is a declared engine primitive, **ambient** — no `use` gates
it. Its entire API:

| Member | Meaning |
|---|---|
| `opaque(v)` | erasure: box any value; answers the `opaque` box |
| `opaque.downcast<T>(o) -> ?T` | checked recovery: the box's inner cell on a match, `nil` on a mismatch |
| `x is opaque` | the ordinary concrete test for the box type itself |

```rut
use ink::{ Logger };

struct Point { x: i32; y: i32; }
enum Flavor { Sour, Sweet }

entry fn main() {
    let log = Logger.new("t");
    let box1 = opaque(Point { x: 1, y: 2 });   // erasure = call of the type name
    let p = opaque.downcast<Point>(box1);      // ?Point — the nullable
    when (p != nil) {
        true -> { log.info(f"point {p.x} {p.y}"); },
        else  -> { log.info("point: nil"); },
    }
    let wrong = opaque.downcast<i32>(opaque(Flavor.Sour));  // nil, no trap
    log.info(f"wrong == nil: {wrong == nil}");
}
```

```text
point 1 2
wrong == nil: true
```

## The laws

- **Construction is erasure.** Every value can become `opaque`. Under
  the all-cells regime, boxing is **zero-copy**: the box stores the
  payload's cell handle, so mutations through the original name are
  visible through the box (the alias law, see
  [By-reference and nullable](by-reference-and-nullable.md)). A
  primitive payload copies the bits — value semantics where aliasing is
  unobservable:

  ```rut
  use ink::{ Logger };

  entry fn main() {
      let log = Logger.new("t");
      let mut n = 5;
      let b = opaque(n);    // a prim payload COPIES the bits
      n = 9;                // ...so the source moving stays out
      log.info(f"{opaque.downcast<i32>(b)} vs {n}");
  }
  ```

  ```text
  5 vs 9
  ```

- **A match is the box's own inner cell.** The recovered `?T` shares the
  box: writes through the recovery are the source's, in both directions,
  through every holder. A primitive payload reads the copied value.
- **A mismatch is `nil`.** There is no flag and no zero value — the miss
  is the nullable's null; an unguarded dereference traps `NilDeref`,
  never a silent zero.
- **The type argument must be concrete.** An interface-typed type argument
  (`opaque.downcast<Drawable>`) is a compile error — interface-typed values
  have no recovery path by design.
- **The opaque-is law: `is` names the box, never the payload.**
  `o is T` and `o is I` answer **by the box** — false for every payload
  type, concrete and interface alike. The one check that names what it is —
  `o is opaque` — is `true`. Recovery is `downcast<T>` only. On a
  non-box receiver, `x is opaque` is the ordinary concrete test for the
  box type.

  ```rut
  use ink::{ Logger };

  entry fn main() {
      let log = Logger.new("t");
      let b = opaque("hello");
      log.info(f"{b is str} {opaque.downcast<str>(b) != nil}");
  }
  ```

  ```text
  false true
  ```

- **Boxes are never equal unless identical**: `opaque(v) == opaque(v)`
  is `false` — two boxes, two cells. Box once, compare boxes.
- **Interface-typed values cannot be boxed**: erasure takes a concrete
  value. Fixed arrays box fine; their identity covers the element type.

## Composition and costs

`opaque` is a concrete primitive, not a lattice node: it composes in
every type position trivially — `Vec<opaque>`, `[opaque]`, fields,
returns, parameters.

```rut
use ink::{ Logger };
use pouch::{ Vec };

struct Point { x: i32; y: i32; }

entry fn main() {
    let log = Logger.new("t");
    let boxes: [opaque] = [opaque(Point { x: 3, y: 4 }), opaque("two")];
    let vec: Vec<opaque> = Vec.from([opaque(5)]);
    log.info(f"{boxes.len()} {vec.len()}");
}
```

```text
2 1
```

- The rut-side box is one small cell: the erased value rides inline in
  the box, and cell churn is recycled by the heap arena. Heap accounting
  charges the box's cell, as always (see
  [Reified types and layout](reified-types.md)).
- The host side of the same surface: an embedder can mint the box over
  host data with no static rut shape. rut sees only the box —
  `o is opaque` is `true`, `o is T` misses for every rut `T`, and
  `opaque.downcast<T>` answers `nil` for every `T` (the miss is checked,
  never a trap). The host borrows the payload back typed, call-scoped
  and borrow-guarded.
- Crossing isolates is allowed iff the boxed value's type is crossable,
  checked at runtime via the type descriptor.
- An `opaque` box can do nothing until it is recovered — no methods, no
  fields, no format-string rendering. That is the difference from
  gradual typing: the erased-storage type for a hot loop is a smell;
  lints flag downcasts inside loop bodies and `opaque`
  parameters/returns on non-storage functions.
