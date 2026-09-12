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
(RFC 0010 §1). Element access (`v[i]`, `v[i] = x`, `for (x of v)`) and
`.len()` lower through its `impl Iter for Vec<T>` (below) to the fused
`arrget`/`arrset` ops on the backing `buf` (RFC 0032 §1.1 R2) —
never a per-element call; `push`/`pop` are ordinary rut methods compiled
per instantiation (RFC 0013 §2).

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
  `Iter` impl.

**`Iter` — the sequence contract** (RFC 0012): a read-only trait with an
associated element type — `type Target`, `len`, `get`. `x[i]`,
`.len()`, and `for (x of s)` lower through the receiver's `Iter` impl.
Implementations:

- `Array<T>` — the builtin (native) impl, lowering to the fused
  `arrget`/`arrlen` ops (RFC 0032 §1.1 R2), `Target = T`.
- `string` — the builtin (native) impl, `Target = char`; iteration and
  indexing use `strcharat`/`strlen`.
- `bytes` — the builtin (native) impl, `Target = u8`; `bytesget`.
- `Vec<T>` — a real `impl Iter for Vec<T> { type Target = T; .. }` in
  `std:collection` (`len`/`get`); the compiler inlines the one-line
  accessors under the concrete instantiation, so element access is the
  fused `arrget`/`arrset` on the backing `buf`, not a per-element call.

`Iter` is read-only: mutable element write (`a[i] = v`) stays on the
concrete `Array`/`Vec` fused `arrset` path (the impl's optional `set`
hook). `string`/`bytes` have **no method syntax** — their operations are
free functions (`string_len`, `string_encode`, `bytes_len`,
`bytes_decode`, `bytes_from`, `bytes_zeroed`, RFC 0012); `string_len`
counts characters, `bytes_len` counts octets. The `dyn Iter` object form
(RFC 0012 §2) is the follow-up; today `Iter` is resolved statically per
receiver.

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
element-wise equality in v1 (no `Equal` trait — RFC 0012 §4; `==`
compares primitives by value and everything else by cell identity, which
is almost never what an Option comparison wants). Compare structurally:
`when`, `.is_some()` / `.is_ok()`, or the payload (`.value == d`).

Traps (`Option.value` on `None` included) unwind to the host boundary only
(RFC 0034 §2) — errors as values, bugs as traps (RFC 0001 P4).
