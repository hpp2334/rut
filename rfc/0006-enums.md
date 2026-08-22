# RFC 0006: Enums — Simple Named-Int Sets

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0005 (builtin generics — `Option` returns)
- **Supersedes:** RFC 0002 §7 (pre-restructure)
- **Part:** B — Language surface

## Summary

`enum Color { Red, Green, Blue }` — 0, 1, 2; `enum Direction { Up = 1, Down,
Left, Right }` — 1, 2, 3, 4 (explicit initializers allowed; members are the
enum's values). This is the *entire* feature: rut has **no data-carrying
enums** — that omission is why `Option`/`Result` are builtins (RFC 0005)
and why heterogeneous data goes through interfaces (RFC 0012).

- An enum is a distinct named type over `i32` constants. Members are the
  enum's values; no data payloads, no methods, no computed members.
- `when` over an enum must be exhaustive: every member covered, or an
  explicit `else` arm — anything else is a compile error (RFC 0008 §2).
- Enums cross the host boundary as their runtime identity + `i32`
  (RFC 0022 §2); the host can register its own enum types.
- Where TS would use a union of literals (`"left" | "right"`), rut uses an
  enum; where TS would use a union of *shapes*, rut uses a `dyn` interface
  ref (RFC 0012 §2).
- Enum ↔ `i32` goes through per-enum builtins — `Color.to_int(c): i32` and
  `Color.from_int(i: i32): Option<Color>` (`None` on unknown values) — not a
  cast operator (RFC 0012 §3).
