# RFC 0005: Builtin Generic Types — `Option`, `Result`, `Vec`

- **Status:** Draft
- **Date:** 2026-08-23
- **Revised:** 2026-09 — the byref-nullable amendment (RFC 0044): §8's
  pointer surface `*T`/`&v` is REMOVED and replaced by the nullable
  `?T` (prefix-only, `nil` the null, auto-deref at every use); §9's
  generic zero storage moves from `[*T]` to `[?T]`; §10's `own` removal
  reason becomes the sharing law (`bytes.clone()` is the one copy
  escape hatch); the `Vec` backing is a `buf: [?T]`.
- **Author:** hpp2334
- **Depends on:** RFC 0004 (primitives)
- **Supersedes:** RFC 0002 §3 (pre-restructure)
- **Part:** B — Language surface

## Summary

## 8. The nullable — `?T` and `nil` (RFC 0044)

`?T` is a nil-able cell — the same one-slot rc-backed box the old `*T`
pointer was, spelled to say what it means. `nil` is its null literal —
and the empty type's one value (RFC 0004 §4): a context-free `nil` has
type `nil`, while nullable positions (`let p: ?T = nil`, `p == nil`,
`left: nil` in a literal) type it as `?T`. Dereferencing `nil` (any
value use through it) is the `NilDeref` trap — never a silent read.
Sharing needs no spelling: **every binding shares its cell**
(RFC 0044 §1) — a `?T` binding IS the cell reference, and writes
through it hit the shared cell.

The spelling is **prefix-only and binds tightest**: `?` applies to the
type term that follows — `[?T]` is `[T | nil]` (nullable elements),
`?[T]` is `[T] | nil` (a nullable array), `??T` chains (the same
runtime box, unwrapped transitively at use sites). The removed
spellings diagnose: `*T` and postfix `T?` point at `?T`; expression
`*x`/`&x` point at the sharing law ("pass `x` directly").

- Coercions: **`T → ?T` boxes** (the `MakeOpt` one-slot cell — the box
  ALIASES the payload's cell, a share; primitives copy bits), **`?T →
  T` derefs** (a field-0 read + nil check). Both are implicit at the
  expected-type position; the funnel is transitive through `??T`.
- `p.x` / `p.m(..)` / `p[i]` / `for (x of p)` auto-deref: every value
  position reads the payload.
- `p == nil` / `p != nil` compare against the null slot; `?T == ?T` is
  slot identity, like every cell (RFC 0044 §3).
- `?T` does not cross the host boundary — the rule `*T` had (RFC 0023).
- `on_drop<T>(p: ?T, cleanup: fn(?T))` (RFC 0016 §3) attaches a cleanup
  that runs when the cell's refcount reaches zero — one callback per
  nullable, a second attach is an error.

## 9. The bracket spelling — `[T]`, the repeat `[v; n]` (v1.1)

`[T]` is the spelling of the heap array (runtime length, non-growable):
`let xs: [i32] = [1, 2, 3];` — the literal allocates the cell. The type
is grammar, resolved directly: no surface name carries it, and a module
needs no `use` to spell it.

Construction is the **repeat expression** `[v; n]` — a VALUE and a
count (`[nil; n]`, `[0u8; cap]`); there is no type-in-expression form (a
type is not a value). A scalar/nil fill is the memset-class op (a nil
fill is exactly the zero-fill); a ref fill retains the cell handle n
times — every slot aliases the one cell (RFC 0044's sharing law: the
repeat never copies). Generic zero-initialized storage is structural:
nullable arrays `[?T]` with `[nil; n]` — `nil` is the slot's zero.

The `Array` NAME is removed: the `builtin class Array<T>` decl is gone
(`a[i]`, `a.len()` are compiler-lowered — no decl needed), and a use
site that still spells `[T]` / `[T](n)` diagnoses with the
removal and its replacement (`[T]` / `[v; n]`).

## 10. Removals — `Option`, `Result`, `own` (v1.1)

The builtin sums are gone from `core` — no `Option<T>`, no
`Result<T, E>`, no constructors or `unwrap` family. Their jobs moved to
the language's own shapes:

- **Absence** is `nil` on a nullable (`?T`): a lookup returns
  `?V`, and `nil` means "not found" (§8).
- **Errors** are records: `(T, err)` with a user-chosen `err` type —
  an empty str / `false` / `nil` second element is success
  (RFC 0004 §4).
- **Type-erased recovery** returns `(T, bool)`: `downcast<T>(o)` is a
  test-plus-unbox, `false` on a mismatch (RFC 0014).
- `checked_add`/`checked_sub`/`checked_mul` return `(value, ok)` —
  `false` on overflow, `value` the wrapped result.

`own(x)` is removed with them: under the sharing law (RFC 0044 §1)
every binding shares its cell — there is no eager copy for `own` to be
the inverse of, and no spelling to force one. `bytes.clone()` is the
one copy escape hatch (RFC 0044 §4); a use site that still spells
`own` (or the removed
sums) diagnoses with the removal and its replacement — nothing else is
kept for compatibility.


`Option<T>`, `Result<T, E>` are **builtin (VM-native)** types — they cannot
be user-defined because user enums carry no data (RFC 0006). No sugar
operators (`?.`, `??`); the API is explicit snake_case methods plus the `?`
propagation operator. Their NAMES live in `core` like every prelude
name: `use core::{ Option, Result };` (RFC 0028) — the types
are builtin, the names are used, never ambient.

| `Option<T>` | `Result<T, E>` |
|---|---|
| `is_some() / is_none() -> bool` | `is_ok() / is_err() -> bool` |
| `value: T` (traps on `None`) | `value: T` (traps on `Err`) |
| `unwrap_or(d: T) -> T` | `error: E` (traps on `Ok`) |
| `expect(msg: str) -> T` | `unwrap_or(d: T) -> T` |

`Vec<T>` is **std-lib rut code**, not a VM builtin: a `pub class` in
`pouch` over the non-growable `[T]`, with the mutable,
growable, handle-shared sequence API — `push`/`pop`, indexing, `.len()`
(RFC 0016 §4). Construction is the class's own methods —
`Vec.new()` (empty), `Vec.with_capacity(n)` (reserve), `Vec.filled(v, n)`
(n slots of `v` — the repeat `[v; n]` boxed for the caller),
`Vec.from(arr)` (copy a `[T]`). There is
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
a `u8` array cell** (`[u8]`), so it reuses the array ops — `b[i]` is
`ArrGet`, `b.len()` is `ArrLen`, `bytes.zeroed(n)`/`bytes(n)` are
`ArrNew`, `bytes.from(a)` is an array copy (`Own`), and `==` is the
content comparison `ArrayCmp`. Construction: `bytes(n)` (n zeroed
octets), `bytes.from(a)` (copies a `[u8]`), and `freeze()` on a
mutable `Vec<u8>` builder. Reading: `b.len(): i32`, `b[i]: u8`
(bounds trap), `for (let b of b)`. `s.encode() -> bytes` (UTF-8) and
`b.decode() -> str` (UTF-8, lossy) are lowered by the compiler as LIR
loops (no native). The retired free-fn spellings (`bytes_len(b)`,
`string_len(s)`, …) diagnose with the member replacement (RFC 0004 §4).
`Vec<u8>` itself is only a mutable builder; `bytes` is the binary type
that crosses the host boundary (RFC 0023 §2).

One more builtin is type syntax plus a member:

- **`[T]` — the heap array**: runtime length, non-growable (the
  backing `Vec<T>` grows); construction is the repeat `[v; n]` (§9).
  A **cell handle — shared like every
  non-primitive** (RFC 0016 §4): assignment aliases, mutation is
  visible through every handle. The literal is pure data:
  `[a, b, c] : [T]` (RFC 0007 §1) — it allocates the cell.
  Indexing, `for..of`, `.len()` (the runtime length via `arrlen`,
  RFC 0032 §1.1 R2); OOB traps. `[T]` has the builtin (native)
  `Index` impl.

**`Index<T>` — the random-access contract** (RFC 0012): a `len` +
`get`/`set` trait whose element is the trait's **type argument**
(`Index<T>`, not an associated type). `x[i]`, `x[i] = v`, `.len()`, and
`for (x of s)` lower through the receiver's `Index` impl.
Implementations:

- `[T]` — the builtin (native) impl, lowering to the fused
  `arrget`/`arrset`/`arrlen` ops (RFC 0032 §1.1 R2), element `T`.
- `str` — the builtin (native) impl, element `char`; iteration and
  indexing use `strcharat`/`strlen`.
- `bytes` — the builtin (native) impl, element `u8`; `bytesget`.
- `Vec<T>` — the `pouch` class (RFC 0028), a record with a
  `buf: [?T]` field (the nullable-handle backing: `[nil; cap]` is the
  only generic zero, stores take the `T → ?T`-boxed handle) and a
  `len: i32` field; `v[i]`,
  `v[i] = x`, `v.len()`, and `for (x of v)` lower to the fused element
  ops on those fields, loads yielding the `?T` (uses auto-deref) — no
  trait call,
  no accessor inlining. `for (x of v)` yields the shared element as
  the binding (RFC 0044 §1); indexed reads compute at `T`.

`str`/`bytes` carry **member contracts** declared per type in the
prelude (v1.1, RFC 0004 §4): `s.len()`/`s.code()`/`s.encode()`,
`b.len()`/`b.decode()`, and the `bytes.zeroed(n)`/`bytes.from(a)`
type-methods — each lowering to the same fused ops.

**`Iterator<E>` — the iteration protocol** (RFC 0012 §6): a type is
iterable when it declares
`fn __iterate(self, emit: fn(E) -> bool)`; `for (x of it)` desugars to
`it.__iterate(emit)`. The builtin sequences keep their fused index
loops (the fast
indexed lowering). Both contracts resolve through the impl registry per
the two-rule law (RFC 0012 §1) — static for single-origin receivers;
the trait-typed forms (`Vec<Index<T>>`, `Vec<Iterator<T>>`) spell the
bare trait name in type position.

`Vec<T>` and `[T]` implement `reflect`'s `Reflectable` and
`Deserializable` for **every instantiation** via the builtin-impl
registry (RFC 0037) — sequences are data: reflectable like records.

`Map<K, V>` / `Set<T>` are
deliberately absent from it: containers are **library types**, provided by
`pouch` as a declaration file + Rust bodies (RFC 0025, RFC 0026, RFC
0028) — the `my_map` declaration is inlined in RFC 0026 §1, and the
consumer-side wrapper class is the `Logger` pattern (RFC 0028).

`==` on `Option<T>` / `Result<T, E>` is a **compile error**: there is no
element-wise equality in v1 (no `Equal` trait — RFC 0012 §8; `==`
compares primitives by value and everything else by cell identity —
RFC 0044 §3 — which
is almost never what an Option comparison wants). Compare structurally:
`when`, `.is_some()` / `.is_ok()`, or the payload (`.value == d`).

Traps (`Option.value` on `None` included) unwind to the host boundary only
(RFC 0034 §2) — errors as values, bugs as traps (RFC 0001 P4).
