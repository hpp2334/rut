# RFC 0011: Reference Semantics, `own`, `Weak` & Disposal

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0009 (dataclasses), RFC 0010 (classes), RFC 0012
  (traits — read §3 after)
- **Supersedes:** RFC 0002 §5.3 (pre-restructure)
- **Part:** B — Language surface

## Summary

One regime, no opt-ins: everything except primitives is a shared,
refcounted cell (RFC 0016 §1). Want a private copy? Say `own(x)`. See
**`examples/basic/rc-and-dispose.rut`** (aliasing, `own`, dispose at
rc 0) and **`examples/memory/temp-file.rut`**.

## 1. Reference semantics — and the `own` escape hatch

- Every non-primitive value (dataclass, class, `str`, `Vec`, `Array`,
  enums, `Opaque`, `dyn I`) is a heap cell handle: assignment, passing,
  and returning copy the handle (`rc++`), and **mutation is visible
  through every alias**. Two handles are equal (`==`, RFC 0012 §4)
  exactly when they point at the same cell — identity is the default
  comparison for composites; field-wise comparison goes
  through `Hashable.eq` (RFC 0028) when a type opts in. Copying is always
  explicit — a program never depends on when a copy happens.
- **`own(x) -> T`** — prelude builtin, the eager **shallow** copy: a fresh
  cell with `x`'s payload cloned (primitive fields copied, handle-typed
  fields still shared — divergence is one level deep; `own` it again for
  deeper cuts). `own` is the only copy in the language. Over a buffer it
  clones the buffer but shares composite element cells. Over a primitive
  it is a no-op (lint). New identity by definition: `own(x) == x` is
  `false`.

## 2. `Disposal` — destructors for classes

- A class implementing `Disposal` is just a class: construct it, store
  it, pass it — the handle *is* the ownership. When a cell's count hits
  0, `Disposal.dispose(mut self)` runs, then fields are released in
  order — deterministic destruction (RFC 0016 §3).

## 3. Trait objects share, never copy

- A trait object is a **fat ref** — the cell handle plus the impl
  vtable (RFC 0015 §6). Widening a composite to `dyn I` **attaches the
  vtable and keeps the handle** — no allocation, no copy: the
  trait-object ref aliases the same object, and mutations through it are visible to
  every other handle. `Vec<Circle>`, `Vec<dyn Drawable>`, and
  `dyn Slice<T>` all store cell pointers (RFC 0016 §4).

## 4. Weak references

- `Weak(x)` creates a `Weak<T>` that does not keep the cell alive — over
  **any** cell (RFC 0017 §1); `upgrade()`:
  `Option<T>`. Cells are also what the shutdown leak report walks
  (RFC 0017 §2), and cycles are possible through any handle-typed field
  or composite element — plain dataclass fields included (recursive
  types are legal — RFC 0016 §1).
