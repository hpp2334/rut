# RFC 0004: Primitive Types & Integer Semantics — the By-Value Regime

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0003 (modules)
- **Supersedes:** RFC 0002 §2, §8 (pre-restructure)
- **Part:** B — Language surface

## Summary

The primitive and heap-handle types, the **one sharing regime:
primitives by value, everything else a refcounted cell handle**
(RFC 0011, RFC 0016), and why `box<T>` is rejected outright rather than
deferred.

## 1. The type table

| Group | Types | Notes |
|-------|-------|-------|
| unsigned int | `u8 u16 u32 u64` | fixed width |
| signed int | `i8 i16 i32 i64` | two's complement |
| float | `f32 f64` | IEEE 754 |
| misc | `bool`, `char` (Unicode scalar, 4 bytes) | |
| heap: text | `str` | immutable, UTF-8, length-prefixed; format literals `f"a={x}"` (RFC 0007 §2), raw literals `r"..."`; compared by content; internally COW-shared (RFC 0016 §4); `s.slice(a, b)` is an O(1) view over the same octets (RFC 0042) |
| heap: binary | `bytes` | immutable, content-compared octet buffer; at the engine level a `u8` array; built with `bytes(n)` (zeroed), `bytes_from(Array<u8>)`, or `Vec<u8>.freeze()` (RFC 0005) |
| heap: seq | `Vec<T>` | mutable, growable buffer — cell handle, **shared**; flat storage for primitive `T` (RFC 0016 §4) |
| heap: seq | `Array<T, N>` | fixed array — cell handle, **shared**; `N` const, part of identity (RFC 0005) |
| heap: slice | `Slice<T>` | builtin trait — object type `dyn Slice<T>` only (RFC 0005, RFC 0012 §2) |
| user: value | `dataclass D { .. }` | open record — **cell handle, shared** (reference semantics; `own` for copies), methods & impl blocks allowed (RFC 0009); payload a slot array (RFC 0015 §4) |
| user: value | `class C { .. }` | sealed record — also a cell handle, shared (RFC 0010), payload a slot array (RFC 0015 §4) |

- `Vec<f32>` is a flat `f32` buffer behind a header — no per-element boxing,
  no per-element refcount traffic (RFC 0016 §4). `Vec<Point>` (dataclass or
  class element) is likewise flat: values stored inline, no headers, no
  refcounts — and `Array<Point, N>` has no header at all: it *is* the
  N-slot inline block, copied whole.
- `Vec<f32>` is a flat `f32` buffer behind a header — no per-element
  handles, no per-element refcount traffic (RFC 0016 §4). `Vec<Point>`
  (composite elements) stores one cell pointer per element; `Array<T, N>`
  is the same shape with the length frozen at `N`.
- **Binary data is `bytes`**: an immutable, content-compared octet buffer
  (`Vec<u8>` remains the mutable builder; `freeze()` turns one into a
  `bytes`). Compare it with `==`, index it as `b[i]: u8`, iterate it with
  `for (let b of b)`, and cross the host boundary directly (RFC 0023 §2).
  At the engine level `bytes` is a `u8` array (RFC 0005).
- No `null`, no `undefined`. Absence is `Option<T>` (RFC 0005).
- **Size accessor**: `.len()` on `Vec<T>`/`Array<T>` (and any `Index<T>`
  object); `str`/`bytes` have no method syntax, so their size is the
  free functions `string_len(s)` / `bytes_len(b)` (RFC 0012). There is no
  `.length` property or `.count()` variant anywhere in the language.

## 2. One regime: primitives by value, everything else shared

Primitives (`u8..u64`, `i8..i64`, `u/isize`, `f32`/`f64`, `bool`,
`char`) copy on assignment/passing/return — plain slot moves. **Every
other type is a refcounted heap cell handle** (RFC 0016 §1):
assignment shares, and mutation through any alias is visible through
all of them — dataclass and class instances, `str`, `Vec`, `Array`,
enums, `Opaque`, `dyn I` alike. Writing is gated by the `mut`-binding
law (RFC 0003 §1), never by the sharing. The **eager copy is the
`own(x)` builtin** (RFC 0011 §1): shallow — primitive fields copied,
handle fields still shared. `Weak(x)` demotes any handle to a
non-keeping ref (RFC 0017). There is no `&`/`*` syntax anywhere.

**No `box<T>`, no loans.** A loan needs an exclusivity proof; rut has no
compile-time borrow checker and no runtime aliasing control over
handles (they are all shared) — so loans could alias silently. The
construct is **rejected, not deferred**. The only borrows anywhere are
host-side, call-scoped, flag-guarded ones at the FFI (RFC 0023).

## 3. Integer semantics

- Overflow in `+ - * <<` **traps** in debug and release by default.
  Wrapping escapes: `Math.wrapping_add`/`wrapping_sub`/`wrapping_mul`/
  `wrapping_shl` (`std:math`, RFC 0028); saturating:
  `Math.saturating_add`/`saturating_sub`/`saturating_mul`; checked:
  `Math.checked_add`/`checked_sub`/`checked_mul` → `Option<T>`.
- Division by zero traps; `int / int` is integer division.
- Mixed-width arithmetic: both operands must have equal width (convert
  first — RFC 0007 §1).

## 4. v1.1 — `char` is gone; codepoints are `u32`; records are `(T, ..)`

`char` is removed — there is no character type, no `'x'` literal (the
lexer diagnoses the removal), and no `char` in `str` iteration: a
`str`'s elements are one-codepoint `str`s. Codepoint access is spelled
with integers:

- `s.code() -> u32` — the FIRST codepoint of `s` (traps on empty).
- `str.from_code(n: u32) -> str` — the 1-codepoint `str` for `n`
  (compiler-lowered; UTF-8 encoded at materialization).

Records — the `(a, b, ..)` literal and its `(T0, T1, ..)` type, with
numeric fields `.0`, `.1`, .. — are the v1.1 error convention:
**`fn f(..) -> (T, err)` returns a result; an empty/`false`/`nil`
second element is success** (§5 of RFC 0005 for the `Option`/`Result`
removal this replaces). `()` is the unit value; a function without a
result arrow returns it.

## Open questions

- OQ-1: default type for uncontextualized literals — `i32` for integers and
  `f32` for floats (RFC 0007 §1) — and implicit widening ladder.
- OQ-2: `f16`/`bf16` for GPU-facing vecs.
