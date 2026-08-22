# RFC 0005: Builtin Generic Types — `Option`, `Result`, `Vec`

- **Status:** Draft
- **Date:** 2026-08-22
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
| `is_some() / is_none(): bool` | `is_ok() / is_err(): bool` |
| `value: T` (traps on `None`) | `value: T` (traps on `Err`) |
| `unwrap_or(d: T): T` | `error: E` (traps on `Ok`) |
| `expect(msg: string): T` | `unwrap_or(d: T): T` |

`Vec<T>` completes the builtin generic set: the mutable, growable,
handle-shared sequence — `push`/`pop`, indexing, `.len()` (RFC 0016 §4).
Construction is a type-call: `Vec<f32>(1024)` (n zeroed elements),
`Vec<i32>()` (empty), and `Vec.from(xs)` (copies out of a fixed
`Array<T, N>`; folded at compile time, RFC 0033 §3). Vec's **named**
API (`len`/`push`/`pop`) is internal-native — `callnat`s into a
boot-registered module (RFC 0022 §2, RFC 0032 §1.1 R2) — while element
access on a concrete `Vec<T>` stays the fused `arrget`/`arrset` ops
(RFC 0016 §4).

Two more builtins are type syntax rather than constructors:

- **`Array<T, N>` — the fixed array**: a builtin **const-generic** value
  type (the only one in v1 — RFC 0013 §2). `N` is a constant expression
  and part of the type's identity (`Array<i32, 3> ≠ Array<i32, 4>`,
  RFC 0015 §3). Inline, headerless, **copied on assignment** like a
  dataclass; the literal is pure data: `[a, b, c] : Array<T, 3>`
  (RFC 0007 §1). Indexing, `for..of`, `.len()` (len folds — the const `N`,
RFC 0032 §1.1 R1); OOB traps. Widens to
  `dyn Slice<T>` by implicit boxing (RFC 0011 §3).
- **`Slice<T>` — the slice interface**: builtin and undeclarable (like
  `Any`, RFC 0014 — `implements Slice<T>` in user source is a compile
  error). It is implemented by exactly two builtins — `Vec<T>` and
  `Array<T, N>` — via the builtin-impl registry (RFC 0022 §2), so either
  widens to `dyn Slice<T>` by implicit boxing at the widening site
  (RFC 0011 §3). Its object type `dyn Slice<T>` follows every `dyn I`
  rule: `dyn` spelling, unsized (the slot stores the slice-cell handle,
  RFC 0031 §4), legal in any type position (`Rc<dyn Slice<i32>>`,
  `MyCow<dyn Slice<T>>`). The surface is VM-handled like `dyn Any`'s
  accessors — `x[i]` get/set, `.len()`, `for..of` — and nothing else:
  slices never grow (each surface member lowers to a `calli` on the
  builtin `Slice<T>` slots, RFC 0032 §1.1 R2). Two internal cell kinds hide behind one type
  (RFC 0016 §4): the **owned** cell (a boxed copy of the `Array<T, N>`,
  minted when an Array widens) and the **backing** cell (holds the
  owner Vec's cell handle + `off`/`len`, minted when a Vec widens;
  reads/writes dispatch through the owner, so later growth keeps
  existing `dyn Slice<T>` values valid and an out-of-range index traps,
  never reads garbage). Neither kind is nameable in script — there is
  no view type and no borrowing.

`Vec<T>` and `Array<T, N>` implement `std:reflect`'s `Reflectable` and
`Deserializable` for **every instantiation** via the builtin-impl
registry (RFC 0037) — sequences are data: reflectable like records.

`Map<K, V>` / `Set<T>` are
deliberately absent from it: containers are **library types**, provided by
`std:collection` as a declaration file + Rust bodies (RFC 0025, RFC 0026, RFC
0028) — the example spans three files: `examples/host/plugin/my_map.d.rut`
(decl), `examples/host/my-map.rut` (consumer) +
`examples/host/my_map.rs` (implementation).

Traps (`Option.value` on `None` included) unwind to the host boundary only
(RFC 0034 §2) — errors as values, bugs as traps (RFC 0001 P4).

## Note

- ~~`map<K,V>` / `set<T>` builtin vs library~~ — **resolved: library**; see
  RFC 0028. The VM stays container-free.
