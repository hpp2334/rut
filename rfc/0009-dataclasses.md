# RFC 0009: Dataclasses — Open Value Records

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0004 (by-value regime), RFC 0005 (builtin generics),
  RFC 0012 (interfaces — read §2 after)
- **Supersedes:** RFC 0002 §5.1 (pre-restructure)
- **Part:** B — Language surface

## Summary

See **`examples/basic/dataclasses.rut`** — literal construction everywhere,
copy-on-assignment, field initializers, free functions over data — and
**`examples/basic/interfaces.rut`** for dataclass `implements` + `requires`
in action.

- **Value semantics**: assignment, argument passing, and returning copy the
  whole value (shallow: ref-typed fields copy the handle + retain; nested
  dataclass fields copy inline). No `new`, no factory — `Name { field: expr, .. }`
  is the only construction, available **everywhere** (module-const
  initializers included, RFC 0003 §1).
- **All fields public, always.** A dataclass is an open data record —
  `private` in a dataclass is a compile error. Privacy needs construction
  control, which is the class's job (RFC 0010).
- Dataclass literals must initialize **every** field (any order, by name);
  fields may declare initializers (`x: f32 = 0`), which the literal may then
  omit.
- **Methods and `implements` are allowed** — the dataclass is no longer
  "pure data". Its body may contain fns: inherent methods and interface
  impls, declared and dispatched exactly like class methods (explicit
  `self` receiver included — RFC 0010 §2):

  ```rut
  dataclass Point implements Hashable, Equal<Point> {
      x: f32;
      y: f32;
      fn hash(self): u64 { .. }
      fn eq(self, other: Point): bool {
          return other.x == self.x && other.y == self.y;
      }
  }
  ```

  Calls on a concrete `Point` are direct (RFC 0012 §1); the `dyn I` ref
  form boxes — below. The limits on a dataclass, exhaustively: **no `private`
  fields** (above), **no `static` members**, **no `factory`** (the
  literal is the only construction — that split *is* the
  dataclass/class distinction), and **no `dispose()`** (a value that is
  copied around has no single death to hook). Everything else
  class-shaped is allowed. Free functions over data remain the default
  idiom; methods are for interface impls and tight helpers.
- **Boxing for interface refs.** A dataclass is still a bare inline
  value; an interface value *is* an Rc cell reference (RFC 0011). A bare
  dataclass widens to `dyn I` by **implicit boxing** at the
  widening site — exactly the bare-class rule of RFC 0011 — and cells
  minted this way (or by `Rc(p)`) carry the dataclass's impl vtable
  (RFC 0015 §6). Layout never changes: methods and impl tables add
  **nothing** to `size_of(D)`; the repr-C field block is copied into
  the cell as-is.
- **Representation: no header, no refcount** (RFC 0016 §1). A dataclass is
  its fields back-to-back, inline wherever it lives: registers/stack for
  locals, inline in class fields and Rc cells, inline in `Vec<Point>`
  elements (unboxed and contiguous — a flat buffer of pairs). RC and the
  cycle collector only see a dataclass's ref-typed fields, via the
  compile-time field table.
- `Option<Point>` / `Result<Point, E>` hold the value inline in the payload.
- Copy cost is `size_of(D)` bytes — dataclasses are for small data (points,
  rects, colors, configs). Any size is allowed; the compiler warns past a
  threshold (OQ-1).

## Open questions

- OQ-1: dataclass size warning threshold (compiler warns when a dataclass
  grows past N bytes, since copies are `size_of` memcpys).
