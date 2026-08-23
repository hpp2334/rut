# RFC 0009: Dataclasses — Open Data Records

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0004 (sharing regime), RFC 0005 (builtin generics),
  RFC 0012 (interfaces — read §2 after)
- **Supersedes:** RFC 0002 §5.1 (pre-restructure)
- **Part:** B — Language surface

## Summary

See **`examples/basic/dataclasses.rut`** — literal construction everywhere,
shared cells + `own` divergence, field initializers, free functions over
data — and **`examples/basic/interfaces.rut`** for dataclass `implements`
in action.

- **Reference semantics, like everything non-primitive** (RFC 0004 §2,
  RFC 0016 §1): a dataclass value is a heap cell handle; assignment,
  argument passing, and returning share it, and mutation through any
  alias is visible through all of them. The private eager copy is
  `own(p)` — shallow: primitive fields cloned, handle-typed fields still
  shared (RFC 0011 §1). Writing is gated by the `mut`-binding law
  (RFC 0003 §1). No `new`, no constructor — `Name { field: expr, .. }`
  is the only construction, available **everywhere** (module-let
  initializers included, RFC 0003 §1); the literal allocates the cell.
- **All fields public, always.** A dataclass is an open data record —
  `private` in a dataclass is a compile error. Privacy needs construction
  control, which is the class's job (RFC 0010).
- Dataclass literals must initialize **every** field (any order, by name);
  fields may declare initializers (`x: f32 = 0`), which the literal may then
  omit.
- **Methods and `implements` are allowed** — a dataclass is data
  *plus* behavior. Its body may contain fns: inherent methods and interface
  impls, declared and dispatched exactly like class methods (explicit
  `self` receiver included — RFC 0010 §2):

  ```rut
  dataclass Point implements Hashable {
      x: f32;
      y: f32;
      fn hash(self): u64 { .. }
      fn eq(self, other: Point): bool { .. }
  }
  ```

  `hash`/`eq` are interface-declared members — calls dispatch through
  the vtable per RFC 0012 §1. The limits on a dataclass, exhaustively:
  **no `private` fields** (above), **no `static` members**, **no
  `constructor`** (the literal is the only construction — that split
  *is* the dataclass/class distinction), and **no `dispose()`** (a value
  shared everywhere has no single death to hook; if you need a
  destructor, write a class — RFC 0010 §3, RFC 0011 §2). Everything else
  class-shaped is allowed. Free functions over data remain the default
  idiom; methods are for interface impls and tight helpers.
- **Equality is identity, comparison is `Hashable`.** `==` on two
  dataclass values is a cell-identity test (RFC 0012 §4) — `own(p) == p`
  is false, two literals are never equal. Field-wise comparison is the
  `eq` method of an opted-in `Hashable` impl (RFC 0028) — the mechanism
  `Map`/`Set` keys use.
- **Interface refs share, never box.** A dataclass value is already a
  cell; widening to `dyn I` **attaches the impl vtable to the same
  handle** (RFC 0011 §3, RFC 0015 §6) — no allocation, no copy: the
  interface ref aliases the value. Layout never changes: methods and
  impl tables add **nothing** to `size_of(D)`.
- **Representation: payload repr C inside the cell** (RFC 0016 §5). A
  dataclass payload is its fields back-to-back — primitive fields inline,
  composite fields as cell-handle slots; `Vec<Point>` stores one handle
  per element (only primitive-element buffers are flat — RFC 0016 §4).
  RC and the cycle collector see every handle field via the
  compile-time field table; recursive shapes (`next: Option<Node>`) are
  legal because composite fields are pointer-sized.
- `Option<Point>` / `Result<Point, E>` hold the value's **handle** in
  the payload slot (RFC 0005).
- Sizing guidance: dataclasses are for small data (points, rects,
  colors, configs) — they share like everything else, but `own(x)`/field
  clones are `size_of(D)` memcpys when you do
  diverge. Any size is allowed; the compiler warns past a threshold
  (OQ-1).

## Open questions

- OQ-1: dataclass size warning threshold (compiler warns when a
  dataclass payload grows past N bytes, since `own` clones are
  `size_of` memcpys).
