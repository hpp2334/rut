# RFC 0004: Primitive Types & Integer Semantics — the By-Value Regime

- **Status:** Draft
- **Date:** 2026-08-23
- **Revised:** 2026-09 — the byref-nullable amendment (RFC 0044): the
  regime is pass-by-reference sharing (bindings share cells; only
  primitives and `fn` values copy), the eager copy is `bytes.clone()`
  (`own` is removed), `==` on cells is identity, and absence is the
  nullable `?T` (the old `*T` spelling).
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
| heap: binary | `bytes` | immutable, content-compared octet buffer; at the engine level a `u8` array; built with `bytes(n)` (zeroed), `bytes_from([u8])`, or `Vec<u8>.freeze()` (RFC 0005) |
| heap: seq | `Vec<T>` | mutable, growable buffer — cell handle, **shared**; flat storage for primitive `T` (RFC 0016 §4) |
| heap: seq | `[T]` | fixed array — cell handle, **shared**; `N` const, part of identity (RFC 0005) |
| heap: slice | `Slice<T>` | builtin trait — trait-typed values spell the bare name (RFC 0005, RFC 0012 §2) |
| user: value | `dataclass D { .. }` | open record — **cell handle, shared** (reference semantics; `own` for copies), methods & impl blocks allowed (RFC 0009); payload a slot array (RFC 0015 §4) |
| user: value | `class C { .. }` | sealed record — also a cell handle, shared (RFC 0010), payload a slot array (RFC 0015 §4) |

- `Vec<f32>` is a flat `f32` buffer behind a header — no per-element
  handles, no per-element refcount traffic (RFC 0016 §4). `Vec<Point>`
  (composite elements) stores one cell pointer per element; `[T]`
  is the same shape with the length frozen at `N` — and binding a
  `[T]` (any `[T]`) shares the cell; nothing is ever copied whole
  (RFC 0044).
- **Binary data is `bytes`**: an immutable, content-compared octet buffer
  (`Vec<u8>` remains the mutable builder; `freeze()` turns one into a
  `bytes`). Compare it with `==`, index it as `b[i]: u8`, iterate it with
  `for (let b of b)`, and cross the host boundary directly (RFC 0023 §2).
  At the engine level `bytes` is a `u8` array (RFC 0005).
- No `null`, no `undefined`. Absence is `Option<T>` (RFC 0005).
- **Size accessor**: `.len()` on `Vec<T>`/`[T]` (and any `Index<T>`
  object); `str`/`bytes` have no method syntax, so their size is the
  free functions `string_len(s)` / `bytes_len(b)` (RFC 0012). There is no
  `.length` property or `.count()` variant anywhere in the language.

## 2. One regime: primitives by value, everything else shared

Primitives (`u8..u64`, `i8..i64`, `u/isize`, `f32`/`f64`, `bool`) and
`fn` values copy on assignment/passing/return — plain slot moves.
**Every other type is a refcounted heap cell handle** (RFC 0016 §1):
assignment shares, and mutation through any alias is visible through
all of them — struct and class instances, `str`, `bytes`, `Vec`, `[T]`,
enums, `opaque` boxes, trait-typed values, `?T` boxes alike (RFC 0044).
Writing is gated by the `mut`-binding law (RFC 0003 §1), never by the
sharing. There is no eager copy: `own` is removed — **`bytes.clone()`
is the one copy escape hatch** (RFC 0044 §4). `==` on cells is
**identity** (the raw slot compare); `str`/`bytes` compare by content
(RFC 0044 §3). `Weak(x)` demotes any handle to a
non-keeping ref (RFC 0017). There is no `&`/`*` syntax anywhere;
absence is the nullable `?T` (RFC 0005 §8).

**No `box<T>`, no loans.** A loan needs an exclusivity proof; rut has no
compile-time borrow checker and no runtime aliasing control over
handles (they are all shared) — so loans could alias silently. The
construct is **rejected, not deferred**. The only borrows anywhere are
host-side, call-scoped, flag-guarded ones at the FFI (RFC 0023).

## 3. Integer semantics

- Overflow in `+ - * <<` **traps** in debug and release by default.
  Wrapping escapes: `x.wrapping_add(y)`/`wrapping_sub`/`wrapping_mul`/
  `wrapping_shl` — `builtin impl` methods on every integer primitive
  (`core`, RFC 0032 §1.1 R2): ambient on the primitive (no `use`,
  the primitives have none), lowered inline off the receiver's width.
  Saturating: `x.saturating_add(y)`/`saturating_sub`/`saturating_mul`;
  checked: `x.checked_add(y)`/`checked_sub`/`checked_mul` → the
  `(T, bool)` tuple (v1.1 convention). The float `abs`/`min`/`max`/
  `signum` helpers are `calc`'s (f64 host fns); the integer helper
  forms are gone — the comparisons are one line of rut.
- Division by zero traps; `int / int` is integer division.
- Mixed-width arithmetic: both operands must have equal width (convert
  first — RFC 0007 §1).

## 4. v1.1 — `char` is gone; codepoints are `u32`; records are `(T, ..)`

`char` is removed — there is no character type, no `'x'` literal (the
lexer diagnoses the removal), and no `char` in `str` iteration: a
`str`'s elements are one-codepoint `str`s. Codepoint access is spelled
with integers:

- `s.code() -> u32` — the FIRST codepoint of `s` (traps on empty).
- `s.code_at(i: i32) -> u32` — the codepoint at codepoint index `i`
  (traps out of bounds — the index is a bug, not data). Added with the
  json-perf batch's tokenizer surface (`docs/json-perf-report.md`).
- `str.from_code(n: u32) -> str` — the 1-codepoint `str` for `n`
  (compiler-lowered; UTF-8 encoded at materialization).

Records — the `(a, b, ..)` literal and its `(T0, T1, ..)` type, with
numeric fields `.0`, `.1`, .. — are the v1.1 error convention:
**`fn f(..) -> (T, err)` returns a result; an empty/`false`/`nil`
second element is success** (§5 of RFC 0005 for the `Option`/`Result`
removal this replaces). `nil` is the empty type and its one value; a
function without a result arrow returns it, and a context-free `nil`
has type `nil` — nullable positions (`let p: ?T = nil`, `p == nil`,
`left: nil` in a literal) type it as `?T` through expected-type
propagation (RFC 0005 §8, RFC 0044).

## Amendment (Sep 2026, nmap-hostvals): the char removal COMPLETED — the exorcism, wire tags 3/11 dead, opcode 48 retired, VERSION 12

Landed (docs/nmap-hostvals-report.md; phase `8352b50`). §4 killed the
SURFACE; the enumerated machinery survived in the IR, the VM, and the
boundary. The exorcism deleted it — the removal is now total:

- **Codepoints ride u32 end-to-end** — the decisive receipt was that
  they always had: slots stored `v as u32 as i64`, and the VM's
  conversion table has NO char arm (`Conv{Char↔U32}` rode the untagged
  int branch as a raw bit trunc). `s.code()`/`s.code_at(i)` emit ONE
  new op each (`StrCodeAt`, answering the u32 directly — the old
  `StrCharAt` + `Conv` pair dies), `str.from_code(n)` and
  `bytes.decode`'s mint go through the new `StrFromCode` native, and
  the utf8 walkers drop their char intermediates (a small fuel win on
  utf8-heavy rows, disclosed as receipts — fasta's fuel came back
  bit-identical: its loop never rode the re-spelled walkers).
- **The enumerated deletion**: `PrimTy::Char`, `ConstVal::Char`,
  `ArrKind::Char`/`[char]`/`?char`, `Value::Char`, `Slot::ch/as_char`,
  `alloc_char`'s char (retuned to take the u32 codepoint —
  `char::from_u32`, U+FFFD lossy like `bytes.decode`; the repr
  unchanged, a 1-codepoint STR cell — char never had a heap repr of
  its own), the boundary's char impls and type-match arm.
- **The wire** (VERSION 11 → 12): prim tag 11 withdrawn, const tag 3
  withdrawn (decode-only dead since the literal removal), **opcode 48
  RETIRED, never re-meaninged** (`StrCodeAt` takes NEW number 91; a
  forced-past-version artifact fails by name), nat tag 21 added. The
  boot type-table row at id 12 STAYS as a RESERVED `Nil` shell — the
  fixed `TY_*` ids are wire-stable and never reorder; nothing reaches
  it.
- **§1's type-table `char` row is dead as of this amendment** (the
  table above is kept as history). The surface laws are unchanged:
  the lexer's char-literal diagnostic verbatim; type positions
  diagnose through the removed-core check with the dedicated v1.1
  message — now pin-tested with the enumerated kind gone from the
  compiler entirely.

## Open questions

- OQ-1: default type for uncontextualized literals — `i32` for integers and
  `f32` for floats (RFC 0007 §1) — and implicit widening ladder.
- OQ-2: `f16`/`bf16` for GPU-facing vecs.
