# Builtin generic types

The built-in sequence and wrapper surfaces: the heap array `[T]`, the
nullable `?T`, the growable `Vec<T>`, `Weak<T>`, and the keyed
collections.

## `[T]` — the heap array

`[T]` is the spelling of the fixed array: **runtime length,
non-growable**. The type is grammar, resolved directly — no `use` names
it.

```rut
use ink::{ Logger };

struct Point { x: i32; y: i32; }

pub fn main() {
    let log = Logger.new("t");
    let xs: [i32] = [1, 2, 3];   // the literal allocates the cell
    let ys: [?Point] = [nil; 4]; // the repeat: a VALUE and a count
    log.info(f"{xs.len()} {ys.len()}");
}
```

```text
3 4
```

- Construction is the **repeat expression** `[v; n]` — a value and a
  count; there is no type-in-expression form. A scalar/`nil` fill is
  the memset-class op; a ref fill retains the cell handle `n` times —
  every slot aliases the one cell (the sharing law: the repeat never
  copies).
- `[T]` is a cell handle — shared like every non-primitive: assignment
  aliases, mutation is visible through every handle.
- `a[i]`, `a[i] = x`, `.len()`, and `for (x of a)` are
  compiler-lowered to the fused array ops — never a per-element call.
  Out-of-bounds traps.
- Fixed-length windows: `v.slice(from, to)` — see
  [String slicing and views](string-views.md).

The old `Array<T>` name is removed: the array type is spelled `[T]`,
construction is the repeat `[v; n]`.

## `?T` — the nullable

`?T` is a nil-able cell: a one-slot box whose payload is a `T` or the
null slot. `nil` is its null literal — and the empty type's one value: a
context-free `nil` has type `nil`, while nullable positions
(`let p: ?T = nil`, `p == nil`, `left: nil` in a literal) type it as
`?T`. Dereferencing `nil` is the `NilDeref` trap — never a silent read.

The spelling is **prefix-only and binds tightest** — `?` applies to the
type term that follows:

| Spelling | Type | Reading |
|---|---|---|
| `?T` | `T \| nil` | the nullable |
| `[?T]` | `[T \| nil]` | array of nullables |
| `?[T]` | `[T] \| nil` | nullable array |
| `??T` | chained | the same runtime box, unwrapped transitively at use sites |

- Coercions: **`T → ?T` boxes** (the box *aliases* the payload's cell —
  a share; primitives copy bits), **`?T → T` derefs** (a field-0 read
  plus nil check). Both are implicit at the expected-type position; the
  funnel is transitive through `??T`.
- Auto-deref covers every value position: `p.x`, `p.m(..)`, `p[i]`,
  `for (x of p)`, arithmetic on `p`'s payload.
- `p == nil` / `p != nil` compare against the null slot; `?T == ?T` is
  slot identity (see [Rc, dispose, and identity](rc-dispose-identity.md)).
- A `?T` binding IS the cell reference — writes through it hit the
  shared cell (gated by `mut`, see
  [Modules and visibility](modules-and-visibility.md)).
- When the cell's refcount reaches zero, the engine runs the type's
  `Disposal` impl (`dispose(self, cx)`) — the cell-death hook, not a
  per-value attach ([the Rc heap](rc-heap.md)).
- Across the host boundary `?T` crosses nil-flattened when its element
  crosses.

The removed pointer spellings diagnose: `*T` and postfix `T?` point at
`?T`; expression `*x`/`&x` point at the sharing law ("pass `x`
directly"). See [By-reference and nullable](by-reference-and-nullable.md).

## `Vec<T>` — the growable sequence

`Vec<T>` is a library class (package `pouch`) over the non-growable
`[T]`: a `buf: [?T]` backing plus a live `len`. Loads yield the `?T`
(uses auto-deref), stores take the coerced handle — reads and writes
alias the stored cells, and binding an element copies nothing.

| Construction | Meaning |
|---|---|
| `Vec.new()` | empty |
| `Vec.with_capacity(n)` | reserve `n` slots |
| `Vec.filled(v, n)` | `n` slots of `v` |
| `Vec.from(arr)` | copy a `[T]` |

Explicit type arguments may be spelled at the call:
`Vec<i32>.from([1, 2, 3])`. There is no `Vec<T>(..)` type-call —
construction is always a method call (see
[Classes and constructors](classes.md)).

| Member | Meaning |
|---|---|
| `push(v)` | append; amortized O(1) growth |
| `pop() -> T` | remove and return the last element; traps on empty — guard with `len() > 0` |
| `v[i]`, `v[i] = x` | element access; out-of-bounds traps |
| `len() -> i32` | live length |
| `for (x of v)` | iteration; `x` is the shared element |
| `slice(from, to) -> ?Vec<T>` | fixed-length window (compiler-lowered) — writes through it hit the parent |
| `as_array() -> [T]` | copy the live elements into a fresh, exactly-sized array |
| `freeze() -> bytes` | `Vec<u8>` only: copy the live octets into the immutable `bytes` |

`Vec<u8>` is the mutable binary builder; `bytes` is the binary type
that crosses the host boundary (see
[Primitive types](primitive-types.md)).

## `Weak<T>` — the weak reference

`Weak<T>` is a builtin class whose box holds an *unretained* word to a
referent — a weak never keeps anything alive.

```rut
use ink::{ Logger };

struct Tile { v: i32; }

pub fn main() {
    let log = Logger.new("t");
    let tile = Tile { v: 7 };
    let w = Weak.new(tile);        // the class-method construction
    let got: ?Tile = w.upgrade();  // the live referent, or nil once dead
    log.info(f"{got.v}");
}
```

```text
7
```

- `Weak.new(v)` traps on a `nil` `v`; `T` must be a reference type
  (`Weak<i32>` diagnoses — primitives move by value). `Weak<?U>` is
  legal and `upgrade()` answers `??U`.
- The referent's death nulls every weak box before any user code runs;
  `upgrade()` answers `nil` deterministically from then on.
- A weak edge closes no cycle: strong cycles still leak — see
  [Rc, dispose, and identity](rc-dispose-identity.md).

## Keyed collections

`HashMap<K, V>` and `HashSet<T>` (package `nmapset`) wrap a native key
table. Admission is the compile-time **union bound**
`K requires i8 | i16 | i32 | i64 | u8 | u16 | u32 | u64 | bool | str | bytes`
— a key type outside the set fails at the instantiation (escape hatch:
encode it canonically to `bytes`). Float keys are absent by design:
floats have no stable equality contract. The API is
`new`/`with_capacity`/`put`/`get`/`has`/`remove`/`len`; `get` answers
`?V` — `nil` is absent, and a hit returns the **stored cell**, not a
copy (the aliasing law).

## Absence and errors

There are no `Option`/`Result` builtins — the spellings are ordinary
identifiers, and an unresolved use diagnoses as the unknown name it
is. The [standard library](stdlib.md#what-core-does-not-have)'s
removed-surface table maps each retired spelling to its replacement:

- **Absence** is `nil` on a nullable: a lookup returns `?V`, and `nil`
  means "not found".
- **Errors** are the answer channel: `(?T, err)` — see
  [Primitive types](primitive-types.md).
- **Type-erased recovery** is `opaque.downcast<T>(o) -> ?T` — see
  [opaque — erasure and downcast](opaque.md).

`==` on the removed sum spellings is a compile error. Compare
structurally: `when`, a `nil`/`!= nil` guard, or the payload.
