# Rc, dispose, and identity

One memory regime: every non-primitive value is a shared, refcounted
heap cell; destruction is deterministic; `==` is identity for cells.

## Reference semantics

Every non-primitive value — struct and class records, `str`, `bytes`,
`Vec`, `[T]`, enums, `opaque` boxes, trait-typed values, `?T` boxes,
closures' captured cells — is a **heap cell handle**:

- assignment, argument passing, and returning copy the handle
  (retain/release), an O(1) move;
- **mutation is visible through every alias**;
- writing is gated by the `mut`-binding law (see
  [Modules and visibility](modules-and-visibility.md)), never by the
  sharing.

There is no eager copy of any composite. `own(x)` and `make_ptr(v)` are
removed spellings — bindings share by reference. **`bytes.clone()` is
the one copy escape hatch**: a one-shot deep copy of a buffer's octets.
Every other type shares on binding; a divergent value of any other type
is unreachable — build a new one instead.

Recursive shapes (`next: ?Node`, trees, lists) are legal: composite
fields and elements are pointer-sized handle slots, and the refcount
walk sees every handle field via the compile-time field table (see
[Reified types and layout](reified-types.md)).

## Cleanups — `on_drop`

Destructors are attached, not implemented. The surface is the builtin

```rut
builtin fn on_drop<T>(p: ?T, cleanup: fn(?T)) -> nil;
```

- `cleanup(p)` runs when the referenced cell's refcount reaches **zero**
  — deterministic destruction, not a collector callback.
- Fields are released after the cleanup body returns.
- One callback per nullable; a **second attach is a compile error**.
- The host side of the same law: a host payload's finalizer runs at
  cell death, before the payload's own Rust `Drop`.

Because a struct shared everywhere has no single death to hook, structs
cannot carry destructors — if you need one, write a class whose handle
*is* the ownership, and attach the cleanup where you mint it.

## Weak references

`Weak<T>` (see [Builtin generic types](builtin-generic-types.md))
demotes any handle to a non-keeping reference:

```rut
use ink::{ Logger };

struct Tile { v: i32; }

pub fn main() {
    let log = Logger.new("t");
    let tile = Tile { v: 7 };
    let w = Weak(tile);            // does NOT keep the cell alive
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
| everything else — records, arrays, `Vec`, enums, closures, trait objects, `opaque`, `?T` | **cell identity** | the raw slot compare |

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
- Field-wise comparison is a library trait contract (`hash` + `eq`
  implemented per type) — the mechanism value-keyed maps ride. It is
  not connected to `==`.
- `when` literal patterns are unaffected: arms match compile-time
  values, never runtime `==`.
