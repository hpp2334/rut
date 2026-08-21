# RFC 0011: `Rc<T>`, Dispose & Identity — Explicit References

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0009 (dataclasses), RFC 0010 (classes), RFC 0012
  (interfaces — read §3 after)
- **Supersedes:** RFC 0002 §5.3 (pre-restructure)
- **Part:** B — Language surface

## Summary

Want shared, mutable, or long-lived identity? Say so — wrap the value.
See **`examples/basic/rc-and-dispose.rut`** (aliasing vs value copies) and
**`examples/memory/temp-file.rut`** (dispose at rc 0).

## 1. `Rc<T>` — explicit references

- `Rc<T>` is a builtin generic wrapping a class **or dataclass** `T` in a
  heap cell: `Header + vtable + fields inline` (RFC 0016 §1). Creation:
  `Rc(v)` (type inferred) or `Rc<Circle>(v)`. `Rc(p)` over a dataclass is
  how a value record gets shared identity — and what interface impls on
  dataclasses box into (RFC 0009).
- **Copy = ref-copy.** Assignment/passing/returning an `Rc<T>` copies the
  handle (`rc++`) — JS-like aliasing, but always visible in the type.
- **Access goes through**: `b.x` reads the box's field; `b.x = 1` and
  `b.move_by(..)` mutate the shared box. There is no explicit deref syntax.

## 2. Dispose classes must live behind `Rc`

- If a class declares `dispose()`, using it as a bare value is a compile
  error (a copyable value has no single death). Construct then box —
  `Rc(TempFile("tmp.dat"))` — or hand out `Rc` from a factory (RFC 0010 §1).
  When an Rc cell's count hits 0, `dispose()` runs, then fields are
  released in order — deterministic destruction (RFC 0016 §3).

## 3. Interfaces need the box

- An interface value *is* an Rc cell reference (with its vtable — RFC 0015
  §6). `Rc<Circle>` widens implicitly to `Drawable`; a **bare** class value
  — or, identically, a **bare dataclass** (RFC 0009) — converts to an
  interface type by **implicit boxing** (allocates the Rc cell, vtable from
  the type's impl table) at the widening site — the one place rut
  heap-allocates without `rc` spelled out. `Array<Circle>` stays inline;
  `Array<Rc<Circle>>` and `Array<Drawable>` store cell pointers.

## 4. Weak references

- `Weak(b)` creates a `Weak<C>` that does not keep the cell alive
  (RFC 0017 §1). Rc cells are also what the cycle collector walks
  (RFC 0017 §2).

## Note

- ~~`rc<dataclass>`~~ — **resolved: allowed** (§1). Interface impls on
  dataclasses (RFC 0009) made dataclass cells mandatory anyway.
