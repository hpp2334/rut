# RFC 0005: Builtin Generic Types — `Option`, `Result`, `Vec`

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0004 (primitives)
- **Supersedes:** RFC 0002 §3 (pre-restructure)
- **Part:** B — Language surface

## Summary

`Option<T>`, `Result<T, E>` are **builtin (VM-native)** types — they cannot
be user-defined because user enums carry no data (RFC 0006). No sugar
operators (`?.`, `??`); the API is explicit snake_case methods plus the `?`
propagation operator. See **`examples/basic/option-result.rut`**.

| `Option<T>` | `Result<T, E>` |
|---|---|
| `is_some() / is_none() -> bool` | `is_ok() / is_err() -> bool` |
| `value: T` (traps on `None`) | `value: T` (traps on `Err`) |
| `unwrap_or(d: T) -> T` | `error: E` (traps on `Ok`) |
| `expect(msg: string) -> T` | `unwrap_or(d: T) -> T` |

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

**`bytes` — the immutable binary primitive** (RFC 0004): a non-generic
builtin cell holding a contiguous octet buffer, compared by content.
Construction: `bytes(n)` (n zeroed octets), `bytes_from(a)` (copies an
`Array<u8>`), and `freeze()` on a mutable `Vec<u8>` builder. Reading:
`bytes_len(b): i32`, `b[i]: u8` (bounds trap), `for (let b of b)`,
`==`/`!=` by content. `string_encode(s) -> bytes` (UTF-8) and
`bytes_decode(b) -> string` (UTF-8, lossy) bridge text and binary.
`Vec<u8>` itself is only a mutable builder; `bytes` is the binary type
that crosses the host boundary (RFC 0023 §2).

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
- `string` — the builtin (native) impl, element `char`; iteration and
  indexing use `strcharat`/`strlen`.
- `bytes` — the builtin (native) impl, element `u8`; `bytesget`.
- `Vec<T>` — a real `impl Index<T> for Vec<T>` in `std:collection`
  (`len`/`get`/`set`); the compiler inlines the one-line accessors under
  the concrete instantiation, so element access is the fused
  `arrget`/`arrset` on the backing `buf`, not a per-element call.

`string`/`bytes` have **no method syntax** — their operations are
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
0028) — the example spans three files: `examples/host/plugin/my_map.d.rut`
(decl), `examples/host/my-map.rut` (consumer) +
`examples/host/my_map.rs` (implementation).

`==` on `Option<T>` / `Result<T, E>` is a **compile error**: there is no
element-wise equality in v1 (no `Equal` interface — RFC 0012 §4; `==`
compares primitives by value and everything else by cell identity, which
is almost never what an Option comparison wants). Compare structurally:
`when`, `.is_some()` / `.is_ok()`, or the payload (`.value == d`).

Traps (`Option.value` on `None` included) unwind to the host boundary only
(RFC 0034 §2) — errors as values, bugs as traps (RFC 0001 P4).
