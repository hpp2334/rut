# By-reference and nullable

The sharing regime, the nullable `?T`, identity `==`, and the one copy
escape hatch. These laws landed together and they are not optional:
every other page assumes them.

## The sharing regime

- **Primitives and `fn` values copy** (immediate slots): `u8..u64`,
  `i8..i64`, `f32`/`f64`, `bool`, and closures' function values.
- **Every cell type shares its cell**: `str`, `bytes`, struct/class
  records, `[T]` arrays, enums, interface objects, `opaque` boxes, closures'
  captured cells, and `?T` boxes.
- `let b = a` — any cell type — is an O(1) handle move (retain new,
  release old). Mutation through any alias is visible through all of
  them.

```rut
use ink::{ Logger };

struct Point { x: i32; y: i32; }

entry fn main() {
    let log = Logger("t");
    let mut p = Point { x: 1, y: 2 };
    let q = p;              // SHARE: one cell, two names — no copy
    p.x = 4;                // q.x is 4 now
    log.info(f"{q.x}");
}
```

```text
4
```

- Writing is gated by the **`mut`-binding law**, never by the sharing:
  a `let` binding is a read-only view; `let mut` grants write access to
  the object it holds (see
  [Modules and visibility](modules-and-visibility.md)). Parameters
  declare the same intent: `fn step(mut p: Point)` writes the caller's
  point; `fn area(p: Point)` promises not to.
- A `T` stored into a `?T` slot boxes once — the box **aliases** the
  payload's cell (a share); primitives and `nil` copy the bits. There
  is no deref-on-load/box-on-store special case anywhere.
- Primitive elements in a `[T]` stay flat slots; composite elements are
  handle slots. `for (let x of xs)` yields the **shared** element, not
  a per-element copy — a cell element writes through to the sequence.
- The repeat `[v; n]` retains the handle `n` times: every slot aliases
  the one cell. The repeat never copies.

Copy-by-value is unobservable except through the two legal effects:
**aliasing** (writes visible through every binding) and `==` (below).

## `?T` — the nullable type

`?T` is a nil-able cell — a one-slot box. `nil` is the null literal;
dereferencing it traps `NilDeref` — never a silent read.

### Grammar — prefix only, binds tightest

| Spelling | Type | Reading |
|---|---|---|
| `?T` | `T \| nil` | the nullable |
| `[?T]` | `[T \| nil]` | array of nullables |
| `?[T]` | `[T] \| nil` | nullable array |
| `??T` | chained | the same runtime box, unwrapped transitively |

There is no postfix `T?` and no `*T` — each is a diagnosed error naming
`?T`. In expression position, `*x`/`&x` diagnose: "bindings share by
reference now — pass `x` directly" (binary `*`/`&` operators are
untouched).

### Coercions and uses

**`T → ?T` boxes; `?T → T` derefs** — both implicit at the
expected-type position, transitive through `??T`:

- Auto-deref covers every value position: `p.x`, `p.m(..)`, `p[i]`,
  `for (x of p)`, arithmetic on `p`'s payload.
- `p == nil` / `p != nil` compare against the null slot; guard before
  use. The trap is the bug-catcher, not the semantics.
- A `?T` binding IS the cell reference: writes through it hit the
  shared cell (a `nudge(mut pt: ?Point)` moves the caller's point).
- `nil` typing: a context-free `nil` has type `nil`; in an
  expected-`?T` position it types as that `?T` — so
  `let p: ?Node = nil` and a `left: nil` field in a literal just work.
  Absence reads plainly: a lookup returns `?V`, and `nil` means "not
  found".
- When the cell's refcount reaches zero, the engine runs the type's
  `[disposal]` member — the cell-death hook
  (see [Rc, dispose, and identity](rc-dispose-identity.md)).
- Across the host boundary `?T` crosses nil-flattened when its element
  crosses — an `entry fn -> (?T, err)` is a first-class host answer.

## `==` — identity for cells, content for text

| Operand type | `==` means | Lowering |
|---|---|---|
| numeric / `bool` primitives | value | IEEE 754 for floats (`NaN != NaN`, `-0.0 == 0.0`) |
| `str` | content (codepoints) | content compare |
| `bytes` | content (octets) | content compare |
| everything else — records, arrays, enums, closures, interface objects, `opaque`, `?T` | **cell identity** | the raw slot compare |

`[1, 2] == [1, 2]` is **false** — two cells. Identity is O(1) with no
deep walk; compare content where content is the contract (a loop, or a
hash/eq interface contract for keys). `?T == ?T` is slot identity: two
`nil`s are equal, a null and a box are not. See
[Rc, dispose, and identity](rc-dispose-identity.md) for the full law
and the lint on obviously-fresh composites.

## `bytes.clone()` — the one copy escape hatch

`b.clone() -> bytes` mints a fresh buffer with `b`'s octets — a one-shot
deep copy, the **only copy syntax in the language**. `bytes.from(a)`
also deep-copies. Every other type shares on binding, and a divergent
value of any other type is unreachable — build a new one instead.

```rut
use ink::{ Logger };

entry fn main() {
    let log = Logger("t");
    let header = bytes.from([1, 2, 3]);
    let alias = header;              // shares: one buffer, two names
    let diverged = header.clone();   // a fresh buffer, same octets
    log.info(f"{alias == header} {diverged == header}");
}
```

```text
true true
```

## Containers under sharing

- `Vec<T>` backs on `buf: [?T]` — `[nil; cap]` is the generic zero
  (`nil` is the slot's zero); `push` takes the `T → ?T`-boxed handle;
  loads yield the `?T` (uses auto-deref). See
  [Builtin generic types](builtin-generic-types.md).
- Keyed collections: `get(k) -> ?V` — `nil` = absent, a hit answers the
  **stored cell** (the aliasing law: two gets of one key name one cell
  until a replace re-stores).
- Array windows (`v.slice(a, b) -> ?Vec<T>`) are the shared cell: writes
  through the window hit the parent (see
  [String slicing and views](string-views.md)).
- Erasure rides the same laws: `opaque(v)` aliases the payload's cell,
  and `opaque.downcast<T>(o)`'s match is the box's own inner cell (see
  [opaque — erasure and downcast](opaque.md)).

## The answer channel

**`(?T, err)` — concretely `(value, ok)` / `(T, str)` — is THE answer
channel.** The convention is law: empty err + a value = success; empty
err + `nil` = "not found"; non-empty err = "failed". The err channel
disambiguates a legitimately-absent value from a failure. The
exactly-one-non-nil invariant is the caller's convention — the engine
types the components and decodes them; it does not police the
combination. A return-position destructure (`let (v, ok) = f();`) is
optimized so the pair mints no cell where it is used as a pair.
