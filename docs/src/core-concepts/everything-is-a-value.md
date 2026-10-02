# Everything is a value

rut's type system has one spine: **every name binds a value**, and there
are exactly two ways a value can exist. Primitives and `fn` values are
*immediate* — moved by copying their bits. Every other type is a
*cell* — a heap object whose **handle** is what gets copied. That single
split explains assignment, parameter passing, equality, and what
`bytes.clone()` is for.

## The two regimes

| | immediate | cells |
|---|---|---|
| types | `u8..u64`, `i8..i64`, `f32`, `f64`, `bool`, `fn` values | `str`, `bytes`, `struct` and `class` records, `[T]` arrays, enums, trait objects, `opaque` boxes, `?T` boxes, closures' captured cells |
| assignment | copies the bits | copies the handle (O(1)) |
| mutation | n/a — write the variable | visible through every alias |
| `==` | by value | `str`/`bytes`: by content; everything else: identity |

There is no third regime. No type is "sometimes copied, sometimes
shared": a `struct` value never silently deep-copies, a `str` never
silently aliases. The regime is a property of the *type*, known at
compile time, and the compiler emits the right move for it.

## Sharing is the law for cells

`let b = a` on any cell type is one handle move: the new binding retains
the cell, the old binding releases it. Passing a million-element array
to a function retains once. A loop binding iterates the *stored*
elements, not copies of them.

```rut
use ink::{ Logger };

struct Point { x: f32; y: f32 }

entry fn main() {
    let log = Logger.new("values");
    let mut p = Point { x: 1, y: 2 };
    let q = p;          // q and p name ONE cell
    p.x = 4;            // q.x is 4 now — sharing is the law
    log.info(f"q.x = {q.x}");   // read through the other alias
}
```

```text
q.x = 4
```

Mutation through an alias is visible through all of them, in both
directions. What gates *writing* is the `mut`-binding rule — a binding
must be declared `mut` to be written through — never the sharing itself.
Sharing is always safe: a handle keeps its referent alive, so nothing
dangles, and there is no borrow checker because there are no loans.

If you want a value that aliases nothing, you build one: a literal, a
constructor, a copy of the fields you need. The language has exactly one
copy escape hatch — `bytes.clone()`, a one-shot deep copy of a binary
buffer, for interop hand-offs. There is no generic `clone`: a divergent
value of any other type is unreachable by construction, which is what
makes aliasing something you can reason about locally.

## `?T` — the nullable

Absence is `nil` on a nullable type, spelled as a prefix on the type:
`?T` is "a `T` or `nil`". `?` applies to the type term that follows, so
`[?T]` is an array of nullables while `?[T]` is a nullable array, and
`??T` chains (and unwraps transitively at use).

The coercions are one line: **`T → ?T` boxes; `?T → T` derefs.** The
box is a fresh one-slot cell that *aliases* the value's cell — boxing a
record shares it, boxing a primitive copies its bits. The deref is
implicit at every value position: field access, method calls, indexing,
iteration, arithmetic all read through the box.

```rut
use ink::{ Logger };

struct User { id: i64; name: str }

fn lookup(id: i64) -> ?User {
    if (id == 7) { return User { id: 7, name: "ada" }; }
    return nil;             // a miss is nil, nothing else
}

entry fn main() {
    let log = Logger.new("values");
    let u = lookup(7);                // ?User
    log.info(f"u.name = {u.name}");   // auto-deref when non-nil
    let missing = lookup(8);
    log.info(f"miss is nil: {missing == nil}");   // guard where absence is expected
}
```

```text
u.name = ada
miss is nil: true
```

A `nil` reaching a value use traps (`NilDeref`) — never a silent read.
Guard with `u == nil` where absence is expected; the trap is the
bug-catcher, not the semantics. A miss is `nil` and nothing else: a
lookup that finds nothing returns `?V`, and callers compare against
`nil`.

## `==` is honest about the regime

- Primitives compare **by value**.
- `str` compares **by content** (codepoints); `bytes` by octets.
- Everything else compares **by identity** — two bindings are equal
  exactly when they name the same cell.

Identity is the only equality full sharing can defend: two separately
built `[1, 2]` arrays are two cells, so `[1,2] == [1,2]` is `false`.
Content comparison is a loop over fields, written where content is the
contract. The one place identity quietly behaves like value equality:
enum variants are immortal singletons, so `Flavor.Sour == Flavor.Sour`
is `true`.

## The answer channel: a pair

There is no exception type. Failures are data in
the second element of a record — `(value, err)` — with one documented
convention:

- an **empty err plus a value** is success;
- an **empty err plus `nil`** means "not found";
- a **non-empty err** means "failed".

```rut
use ink::{ Logger };

fn parse_hex(s: str) -> (bytes, str) {
    if (s.len() % 2 != 0) {
        return (bytes.zeroed(0), "hex: odd-length input");
    }
    let n = s.len() / 2;
    let mut buf: [u8] = [0u8; n];
    for (let i = 0; i < n; i += 1) {
        let hi = hex_digit(s.code_at(i * 2));
        let lo = hex_digit(s.code_at(i * 2 + 1));
        if (hi < 0 || lo < 0) {
            return (bytes.zeroed(0), "hex: not a digit");
        }
        buf[i] = ((hi * 16) + lo) as u8;
    }
    return (bytes.from(buf), "");       // (data, "") on success
}

fn hex_digit(c: u32) -> i64 {
    if (c >= 48 && c <= 57) { return (c - 48) as i64; }   // '0'..'9'
    if (c >= 97 && c <= 102) { return (c - 87) as i64; }  // 'a'..'f'
    return -1;
}

entry fn main() {
    let log = Logger.new("values");
    let (raw, err) = parse_hex("4869");
    if (err != "") { log.info(f"err: {err}"); return; }   // propagate
    log.info(f"ok: {raw.decode()} ({raw.len()} bytes)");
    let (raw, err) = parse_hex("486");
    if (err != "") { log.info(f"err: {err}"); }           // the failure channel
}
```

```text
ok: Hi (2 bytes)
err: hex: odd-length input
```

The pair destructure in return position is free — the compiler fuses the
mint away — so this shape costs nothing on hot paths. At the host
boundary the same pair crosses field by field, which makes
`entry fn -> (?T, err)` the standard answer a Rust embedder reads.

## Consequences you feel day to day

- **Reassignment is cheap.** Moving containers around — returning them,
  storing them, restructuring them — is handle traffic, not payload
  traffic. Algorithms that churn records and arrays pay aliasing, not
  copying.
- **Loop variables share elements; the binding is one variable.** A
  `for (let x of xs)` loop hands you the stored element; writes through it
  mutate the sequence. The loop variable is ONE binding reassigned per
  iteration — `for..of` is sugar for a `while`-shaped loop, and the
  capture law treats it like any other ref-headed binding.
- **Containers of primitives stay flat.** A fixed `[i32]` is a packed
  `i32` buffer; growable sequences of primitives store raw payloads with
  a one-byte nil tag — no per-element heap box.
- **Erasure is a box.** `opaque(v)` mints an erasure box; recovery is
  `opaque.downcast<T>(o)`, checked against the runtime type. See
  [reified types](reified-types.md) and the reference on
  [`opaque`](../reference/opaque.md).

The memory machinery under all of this — refcounts, deterministic
destructors, weak references — is the next chapter's subject:
[memory](memory.md). The exact rules live in the reference on
[by-reference and nullable](../reference/by-reference-and-nullable.md).
