# RFC 0005: Builtin Generic Types — `Option`, `Result`, `Vec`

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0004 (primitives)
- **Supersedes:** RFC 0002 §3 (pre-restructure)
- **Part:** B — Language surface

## Summary

## 8. Pointers — `*T` and `nil` (v1.1)

`*T` is a nil-able, rc-backed pointer. `nil` is its null literal;
dereferencing `nil` (field access through it) is the `NilDeref` trap —
never a silent read. Sharing is explicit: `make_ptr(v)` boxes `v` into a
fresh one-slot cell (the literal-forwarding case costs nothing), and
writes through the pointer hit the shared cell.

- `p.x` / `p.m(..)` auto-deref: field and method access through a
  pointer reads the pointee.
- `p == nil` / `p != nil` compare against the null pointer; `*T == *T`
  is cell identity.
- `on_drop<T>(p: *T, cleanup: fn(*T))` (RFC 0016 §3) attaches a cleanup
  that runs when the cell's refcount reaches zero — one callback per
  pointer, a second attach is an error.

## 9. The bracket spelling — `[T]` (v1.1)

`[T]` is the accepted spelling of the heap array `Array<T>` (runtime
length, non-growable): `let xs: [i32] = [1, 2, 3];`.

## 10. Removals — `Option`, `Result`, `own` (v1.1)

The builtin sums are gone from `std:core` — no `Option<T>`, no
`Result<T, E>`, no constructors or `unwrap` family. Their jobs moved to
the language's own shapes:

- **Absence** is `nil` on a pointer type (`*T`): a lookup returns
  `*V`, and `nil` means "not found" (§8).
- **Errors** are records: `(T, err)` with a user-chosen `err` type —
  an empty str / `false` / `nil` second element is success
  (RFC 0004 §4).
- **Type-erased recovery** returns `(T, bool)`: `downcast<T>(o)` is a
  test-plus-unbox, `false` on a mismatch (RFC 0014).
- `checked_add`/`checked_sub`/`checked_mul` return `(value, ok)` —
  `false` on overflow, `value` the wrapped result.

`own(x)` is removed with them: under copy-by-value (RFC 0016 §1) every
binding already owns its cell, so `own` would be the identity. Sharing
is `make_ptr` (§8); a use site that still spells `own` (or the removed
sums) diagnoses with the removal and its replacement — nothing else is
kept for compatibility.


`Option<T>`, `Result<T, E>` are **builtin (VM-native)** types — they cannot
be user-defined because user enums carry no data (RFC 0006). No sugar
operators (`?.`, `??`); the API is explicit snake_case methods plus the `?`
propagation operator. Their NAMES live in `std:core` like every prelude
name: `import { Option, Result } from "std:core"` (RFC 0028) — the types
are builtin, the names are imported, never ambient.

| `Option<T>` | `Result<T, E>` |
|---|---|
| `is_some() / is_none() -> bool` | `is_ok() / is_err() -> bool` |
| `value: T` (traps on `None`) | `value: T` (traps on `Err`) |
| `unwrap_or(d: T) -> T` | `error: E` (traps on `Ok`) |
| `expect(msg: str) -> T` | `unwrap_or(d: T) -> T` |

`Vec<T>` is **std-lib rut code**, not a VM builtin: a `pub class` in
`std:collection` over the non-growable `Array<T>`, with the mutable,
growable, handle-shared sequence API — `push`/`pop`, indexing, `.len()`
(RFC 0016 §4). Construction is the class's own methods —
`Vec.new()` (empty), `Vec.with_capacity(n)` (reserve), `Vec.zeroed(n)`
(n zeroed live elements), `Vec.from(arr)` (copy an `Array<T>`). There is
no `Vec<T>(..)` type-call: class construction is always a method call
(RFC 0010 §1). Element access (`v[i]`, `v[i] = x`, `.len()`) and
`for (x of v)` lower through its `impl Index<T> for Vec<T>` (below) to
the fused `arrget`/`arrset` ops on the backing `buf` (RFC 0032 §1.1 R2)
— never a per-element call; `push`/`pop` are ordinary rut methods
compiled per instantiation (RFC 0013 §2). A cursor,
`VecIter<T>` (`impl Iterator<T>`), backs `for (x of v.iter())` — `Vec`
is rut code, so it ships its own iterator.

**`bytes` — the immutable binary primitive** (RFC 0004): at the language
level a distinct, immutable, content-compared type; **at the engine level
a `u8` array cell** (`Array<u8>`), so it reuses the array ops — `b[i]` is
`ArrGet`, `bytes_len(b)` is `ArrLen`, `bytes_zeroed(n)`/`bytes(n)` are
`ArrNew`, `bytes_from(a)` is an array copy (`Own`), and `==` is the
content comparison `ArrayCmp`. Construction: `bytes(n)` (n zeroed
octets), `bytes_from(a)` (copies an `Array<u8>`), and `freeze()` on a
mutable `Vec<u8>` builder. Reading: `bytes_len(b): i32`, `b[i]: u8`
(bounds trap), `for (let b of b)`. `string_encode(s) -> bytes` (UTF-8)
and `bytes_decode(b) -> str` (UTF-8, lossy) are lowered by the
compiler as LIR loops (no native). `Vec<u8>` itself is only a mutable
builder; `bytes` is the binary type that crosses the host boundary
(RFC 0023 §2).

One more builtin is type syntax plus a member:

- **`Array<T>` — the heap array**: runtime length, non-growable (the
  backing `Vec<T>` grows). A **cell handle — shared like every
  non-primitive** (RFC 0016 §4): assignment aliases, mutation is
  visible through every handle. The literal is pure data:
  `[a, b, c] : Array<T>` (RFC 0007 §1) — it allocates the cell.
  Indexing, `for..of`, `.len()` (the runtime length via `arrlen`,
  RFC 0032 §1.1 R2); OOB traps. `Array<T>` has the builtin (native)
  `Index` impl.

**`Index<T>` — the random-access contract** (RFC 0012): a `len` +
`get`/`set` interface whose element is the interface's **type argument**
(`Index<T>`, not an associated type). `x[i]`, `x[i] = v`, `.len()`, and
`for (x of s)` lower through the receiver's `Index` impl.
Implementations:

- `Array<T>` — the builtin (native) impl, lowering to the fused
  `arrget`/`arrset`/`arrlen` ops (RFC 0032 §1.1 R2), element `T`.
- `str` — the builtin (native) impl, element `char`; iteration and
  indexing use `strcharat`/`strlen`.
- `bytes` — the builtin (native) impl, element `u8`; `bytesget`.
- `Vec<T>` — a real `impl Index<T> for Vec<T>` in `std:collection`
  (`len`/`get`/`set`); the compiler inlines the one-line accessors under
  the concrete instantiation, so element access is the fused
  `arrget`/`arrset` on the backing `buf`, not a per-element call.

`str`/`bytes` have **no method syntax** — their operations are
free functions (`string_len`, `string_encode`, `bytes_len`,
`bytes_decode`, `bytes_from`, `bytes_zeroed`, RFC 0012); `string_len`
counts characters, `bytes_len` counts octets.

**`Iterator<T>` — the cursor contract** (RFC 0012): `next(mut self) ->
Option<T>` (no `len`). `for (x of it)` lowers to `next` until `None`.
The compiler inlines small `next` bodies at the use site, so a
generic cursor (`VecIter<T>`) needs no vtable entry. Indexable
sequences may also be iterated directly through `Index` (the fast
indexed lowering). Both contracts are resolved statically per receiver
today; the interface-object form (`Vec<Index<T>>`, `Vec<Iterator<T>>`)
is the follow-up — an interface name in type position *is* the object
type (there is no `dyn`).

`Vec<T>` and `Array<T>` implement `std:reflect`'s `Reflectable` and
`Deserializable` for **every instantiation** via the builtin-impl
registry (RFC 0037) — sequences are data: reflectable like records.

`Map<K, V>` / `Set<T>` are
deliberately absent from it: containers are **library types**, provided by
`std:collection` as a declaration file + Rust bodies (RFC 0025, RFC 0026, RFC
0028) — the `my_map` declaration is inlined in RFC 0026 §1, and the
consumer-side wrapper class is the `Logger` pattern (RFC 0028).

`==` on `Option<T>` / `Result<T, E>` is a **compile error**: there is no
element-wise equality in v1 (no `Equal` interface — RFC 0012 §4; `==`
compares primitives by value and everything else by cell identity, which
is almost never what an Option comparison wants). Compare structurally:
`when`, `.is_some()` / `.is_ok()`, or the payload (`.value == d`).

Traps (`Option.value` on `None` included) unwind to the host boundary only
(RFC 0034 §2) — errors as values, bugs as traps (RFC 0001 P4).
