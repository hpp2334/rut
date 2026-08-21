# RFC 0005: Builtin Generic Types — `Option`, `Result`, `Array`

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

`Array<T>` completes the builtin generic set. `Map<K, V>` / `Set<T>` are
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
