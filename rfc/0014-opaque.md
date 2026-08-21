# RFC 0014: `Opaque` — Explicit Erasure with Checked Recovery

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0005 (builtin generics — `Option`), RFC 0015
  (reification — read §1 after)
- **Supersedes:** RFC 0002 §3.1 (pre-restructure)
- **Part:** B — Language surface

## Summary

`Opaque` is a builtin heap-cell type that **forgets the static type and
keeps the runtime type** — the controlled replacement for `any`, built on
reification (RFC 0015). Erasure is a *value-level operation you spell out*;
the static type system never loosens.

- `Opaque(v): Opaque` (a type-call, RFC 0002 §3) — box any value. **Value
  semantics apply at boxing time**: a dataclass or bare class value is
  snapshotted (copied into the cell); ref values (`Array`, `string`, `Rc<T>`)
  store the handle. To share a class by identity, box the handle:
  `Opaque(Rc(c))`.
- `downcast<T>(o: Opaque): Option<T>` — checked recovery. `T` concrete:
  exact runtime-type match. `T` an interface: the same check `is<T>` uses.
  `None` on mismatch — never a trap.
- `is<T>(o)` sees through the box (the boxed value's type).
- **That is the whole API** — apart from the three layout accessors of
  RFC 0015 §3 (`type_id()`, `size()`, `as_bytes()`): no members, no fields, no
  equality (OQ-1), no `f""` formatting. An `Opaque` can't do anything
  until it is recovered — which is exactly how it differs from gradual
  typing.
- Crossing isolates: allowed iff the boxed value's type is crossable
  (RFC 0021 §3), checked at runtime via the type descriptor.
- Also the host-FFI escape hatch for host data with no static shape
  (RFC 0023).

## IR contract (why `Opaque` does not break optimization)

1. **Boxes are immutable.** `Opaque(v)` snapshots its value; no operation
   mutates a box in place (store-style writers swap whole cells).
2. **`downcast` is therefore pure** in `(cell, T)`: the compiler may CSE
   repeated downcasts of the same cell, hoist invariant checks out of
   loops, and cache results — validity is permanent.
3. **`downcast` is a refinement point**: within the `is_some()` branch the
   value's lattice position is *exact concrete type* — everything
   downstream optimizes as if the value had never been erased.
4. **Downcast chains over one cell fold to a `TypeId` switch** (jump
   table), the erasure analogue of exhaustive `when`.
5. **`Opaque` is a storage type, not a flow type**: lints flag
   downcasts inside loop bodies and `Opaque` parameters/returns on
   non-storage functions. The hot path keeps concrete types.

The lattice machinery itself is RFC 0030 §4. See
**`examples/basic/opaque.rut`**. The motivating user-land pattern — a
tur-style reactive store (`state` / `source` / `derive` / `mutation` /
`watch` / `Store`, with an `Atom` interface unifying the three atom kinds
for `watch`) — is **`examples/gui/dashboard/reactive.rut`**; the app-shaped
consumer is the `examples/gui/dashboard/` project (`state.rut` declares the
graph, `main.rut` runs it end to end).

## Open questions

- OQ-1: `Opaque` extras: the `type_id()` accessor **shipped** (RFC 0015 §3,
  with `size()` and `as_bytes()`); content equality/hashing for identity use in
  collections remains open — `std:collection`'s `Equal<T>` / `Hashable`
  interfaces (RFC 0026, RFC 0028) are the shipped answer for user types.
- OQ-2: derived-atom caching (deps/version tracking for `derive`-style
  user libraries) — pure recompute-on-read is the v1-simple contract;
  decide whether the stdlib ships an invalidation-based cache.
