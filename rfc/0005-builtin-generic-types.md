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
`Array<T, N>`; const-foldable, RFC 0033 §3). `Vec<T>` also defines
**`as_slice(): dyn Slice<T>`** — a lending view of its buffer (§ below).

Two more builtins are type syntax rather than constructors:

- **`Array<T, N>` — the fixed array**: a builtin **const-generic** value
  type (the only one in v1 — RFC 0013 §2). `N` is a constant expression
  and part of the type's identity (`Array<i32, 3> ≠ Array<i32, 4>`,
  RFC 0015 §3). Inline, headerless, **copied on assignment** like a
  dataclass; the literal is pure data: `[a, b, c] : Array<T, 3>`
  (RFC 0007 §1). Indexing, `for..of`, `.len()`; OOB traps. Widens to
  `dyn Slice<T>` by implicit boxing (RFC 0011 §3).
- **`Slice<T>` — the slice interface**: builtin and undeclarable (like
  `Any`, RFC 0014 — `implements Slice<T>` is a compile error). Its object
  type `dyn Slice<T>` follows every `dyn I` rule: `dyn` spelling,
  unsized (the slot stores the slice-cell handle, RFC 0031 §4), legal in
  any type position (`Rc<dyn Slice<i32>>`, `MyCow<dyn Slice<T>>`). The
  surface is VM-handled like `dyn Any`'s accessors — `x[i]` get/set,
  `.len()`, `for..of` — and nothing else: slices never grow. Two cell
  kinds hide behind one type: the **owned** cell (boxed `Array<T, N>`,
  copied at the boxing) and the **lending view** (`Vec.as_slice()` —
  `Header + owner handle + off + len`, no copy, strong ref keeps the Vec
  alive; reads/writes dispatch through the owner cell, so growth keeps
  views valid and indexing bounds-checks view len ∩ owner len — a view
  that outlives a shrink traps, never reads garbage; RFC 0016 §4).

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
