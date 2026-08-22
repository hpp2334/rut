# RFC 0004: Primitive Types & Integer Semantics — the By-Value Regime

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0003 (modules)
- **Supersedes:** RFC 0002 §2, §8 (pre-restructure)
- **Part:** B — Language surface

## Summary

The primitive and heap-handle types, the **one default copy regime: by
value**, and why `box<T>` is rejected outright rather than deferred.

## 1. The type table

| Group | Types | Notes |
|-------|-------|-------|
| unsigned int | `u8 u16 u32 u64` | fixed width |
| signed int | `i8 i16 i32 i64` | two's complement |
| float | `f32 f64` | IEEE 754 |
| misc | `bool`, `char` (Unicode scalar, 4 bytes) | |
| heap: text | `string` | immutable, UTF-8, length-prefixed; format literals `f"a={x}"` (RFC 0007 §2), raw literals `r"..."` |
| heap: binary | `bytes` | mutable, growable byte buffer |
| heap: seq | `Vec<T>` | mutable, growable, **unboxed** homogeneous storage |
| value: seq | `Array<T, N>` | fixed array — inline value, **copied** on assignment; `N` const, part of identity (RFC 0005) |
| heap: slice | `Slice<T>` | builtin interface — object type `dyn Slice<T>` only (RFC 0005, RFC 0012 §2) |
| user: value | `dataclass D { .. }` | open record — inline value, copied on assignment, methods & interface impls allowed (RFC 0009); **repr C** (RFC 0015 §4) |
| user: value | `class C { .. }` | sealed record — also a value type (RFC 0010), **repr C** (RFC 0015 §4); `Rc<C>` for refs (RFC 0011) |

- `Vec<f32>` is a flat `f32` buffer behind a header — no per-element boxing,
  no per-element refcount traffic (RFC 0016 §4). `Vec<Point>` (dataclass or
  class element) is likewise flat: values stored inline, no headers, no
  refcounts — and `Array<Point, N>` has no header at all: it *is* the
  N-slot inline block, copied whole.
- No `null`, no `undefined`. Absence is `Option<T>` (RFC 0005).
- **One size accessor everywhere**: `.len()` — `string`, `bytes`, `Vec<T>`,
  `Array<T, N>`, and `dyn Slice<T>` all spell it the same way; there is no
  `.length` property or `.count()` variant anywhere in the language.

## 2. One default copy regime: by value

Primitives and both user data types (`dataclass`, `class`) copy on
assignment/passing/return — shallow copies; ref-typed *fields* copy the
handle (a retain). The reference regime is **opt-in and explicit**: `Rc<C>`
boxes a class value in a refcounted heap cell; Rc handles are ref-copied
(RFC 0011). Builtin heap values (vecs, strings, bytes) are handles — they
were born shared. Fixed arrays `Array<T, N>` are values like everything
else: inline, copied on assignment (share one via `dyn Slice<T>` boxing,
RFC 0005). There is no `&`/`*` syntax anywhere.

**No `box<T>`, no loans.** A loan needs an exclusivity proof; rut has no
compile-time borrow checker and no runtime aliasing control over inline
values (they are plain copies) — so loans could alias silently. The
construct is **rejected, not deferred**. `Rc<T>` is the only boxing; the
only borrows anywhere are host-side, call-scoped, flag-guarded ones at the
FFI (RFC 0023).

## 3. Integer semantics

- Overflow in `+ - * <<` **traps** in debug and release by default.
  Wrapping escapes: `&+ &- &* &<<` (compound: `&+= &-= &*= &<<=`);
  saturating: `x.saturating_add(y)`.
- Division by zero traps; `int / int` is integer division.
- Mixed-width arithmetic: both operands must have equal width (convert
  first — RFC 0007 §1).

## Open questions

- OQ-1: default integer type for uncontextualized literals (`i32` proposed,
  RFC 0007 §1) and implicit widening ladder.
- OQ-2: `f16`/`bf16` for GPU-facing vecs.
