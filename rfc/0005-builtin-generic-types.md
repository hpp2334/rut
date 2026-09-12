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
`.len()` lower through its `impl Slice<T> for Vec<T>` (below) to the
fused `arrget`/`arrset` ops on the backing `buf` (RFC 0032 §1.1 R2) —
never a per-element call; `push`/`pop` are ordinary rut methods compiled
per instantiation (RFC 0013 §2).

**`bytes` — the immutable binary primitive** (RFC 0004): a non-generic
builtin cell holding a contiguous octet buffer, compared by content.
Construction: `bytes(n)` (n zeroed octets), `bytes.from(a)` (copies an
`Array<u8, N>` or `Vec<u8>`), and `v.freeze()` on a mutable `Vec<u8>`
builder. Reading: `.len()`, `b[i]: u8` (bounds trap), `for (let b of b)`,
`==`/`!=` by content. `string.encode() -> bytes` (UTF-8) and
`bytes.decode() -> string` (UTF-8, lossy) bridge text and binary.
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
  `Slice<T>` impl.

**`Slice<T>` — the sequence contract**: builtin and resolved by the
compiler, not dispatchable by name. `x[i]` get/set, `x[i] = v`,
`.len()`, and `for (x of s)` lower through the receiver's `Slice`
impl. Implementations:

- `Array<T>` — the builtin (native) impl, lowering to the fused
  `arrget`/`arrset`/`arrlen` ops (RFC 0032 §1.1 R2).
- `Vec<T>` — a real `impl Slice<T> for Vec<T>` in `std:collection`
  (`len`/`get`/`set`); the compiler inlines the one-line accessors under
  the concrete instantiation, so element access is the fused
  `arrget`/`arrset` on the backing `buf`, not a per-element call.
  `string`/`bytes` are **not** `Slice`: their indexing and iteration are
  language primitives (`strcharat`/`bytesget`, RFC 0004).

The `dyn Slice<T>` object form (view cells over the owner, RFC 0016 §4)
is the follow-up; today `Slice` is resolved statically per receiver.

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
