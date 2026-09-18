# RFC 0042: String Slicing — O(1) Views over the Block Store

- **Status:** Draft
- **Date:** 2026-09-17
- **Author:** hpp2334
- **Depends on:** RFC 0004 (primitives), RFC 0015 (layout), RFC 0016 (RC heap), RFC 0039 (VM heap — the block store)
- **Part:** B — Language surface

## Summary

`s.slice(from, to)` — the string-slice member. A slice is an **O(1)
view**: no octets move or copy; the new `str` cell retains the original
and records a byte window. Slicing is safe by the same argument as the
rest of v1.1's sharing: `str` is immutable, so a view can never observe
mutation — there is no aliasing hazard to manage, and no pointer
spelling is needed (this is why arrays are NOT covered here; see §6).

## 1. Surface

```rut
let s = "hello world";
let w = s.slice(6, 11);        // "world" — no copy
```

- `from`/`to` are **codepoint indices**, `from <= to <= s.len()`;
  bounds violations trap `IndexOutOfBounds` (same as `s[i]`).
- Internally the window is recorded in **byte offsets** — codepoint
  bounds are resolved once at slice time (a UTF-8 walk only when the
  string is non-ASCII; ASCII strings are byte-indexed by the cached
  flag, RFC 0008).
- The result is an ordinary `str`: it prints, compares by content
  (`==`), iterates (`for (c of w)`), renders in f-strings, and `len()`
  counts its own codepoints. There is no separate view type on the
  surface — a slice of a `str` *is* a `str` (invisible backing).
- `own(slice)` (and any deep copy) materializes an owned copy — value
  semantics win wherever a copy is requested, exactly like every other
  ref-repr value.

## 2. The view cell

A slice mints a **`StrView` cell**: `{ parent, off, len, ascii }` where
`parent` is a *retained* handle to an **owned** `str` cell and
`off`/`len` are byte offsets into its block. The view charges only its
own header (~32 bytes) against the heap budget (RFC 0040); the parent's
octets stay alive as long as any view does — deterministic destruction
per RFC 0016 (view rc-0 → parent release).

- **View-of-view flattens**: slicing a view combines offsets onto the
  root, so `parent` is always an owned `Str` and reads never chain.
- The ascii flag is inherited from the root at slice time (a window of
  an ASCII string is ASCII).

## 3. Reads go through

Every str operation reads through views via the cell accessors
(`as_bytes`/`as_str`/`char_len`/`str_ascii`): content equality
(`StrCmp`), codepoint count, `s[i]` (`StrCharAt`), iteration, f-string
rendering, `encode`, and the host boundary (`Value::Str` copies out at
the crossing). Nothing in the engine needs to know a view exists beyond
the cell variant.

## 4. Writes never go through

The in-place append fast path (the `out = f"{out}.."` accumulator,
RFC 0007 §2) is gated to **owned, uniquely-referenced** cells
(`rc == 1` and `CellData::Str`). Concatenating a view copies its bytes
out — `f"{view}!"` yields a fresh owned string, and the view (and its
parent) are unchanged. Assignment deep-copies nothing here: binding a
view shares the window (it is a `str`), but no operation can ever
mutate through one.

## 5. Cost model

| operation | cost |
|---|---|
| `s.slice(a, b)` | O(1) — one small cell + one retain (UTF-8 walk only for non-ASCII bounds) |
| read/compare/render a view | O(window) — same as any str |
| `own(view)` / concat out | O(window) — the materializing copy |
| parsing a 1 KiB line out of a 1 MiB buffer | one 32-byte cell, zero copies |

Digest-style workloads (splitting, tokenizing, windowing) stop copying
entirely; `heap_peak` now reports the buffer once instead of per-piece.

## 6. `[T]` windows — shipped in this revision

`v.slice(from, to)` on a `Vec<T>` (and on `[T]`) mints an
`ArrView` cell — a fixed-length window over the backing array — and
boxes it as **`*Vec<T>`**: the view IS a pointer, so sharing is the
spelled semantics and **writes through the window hit the parent**
(the `*T` aliasing law; RFC 0012 §6).

- reads: `w[i]`, `w.len()`, `for (x of w)`, f-string holes — all
  auto-deref the pointer at the use site and go through the window
  (element `i` is `parent[off + i]`, bounds are the window's).
- writes: `w[i] = x` (and compound assignment) hit the parent.
- **fixed-length**: `push`/`pop`/re-backing through a view trap —
  copy the elements out to grow (a `for`-push loop does it today).
- reslicing flattens onto the root backing (`w.slice(a, b)`).
- parent growth **detaches**: `push` re-backs the `Vec` with a fresh
  array; the window keeps pinning the old backing via retain — the
  same aliasing rule Go slices have.
- element-ref iteration (`for` yields `*T`) boxes parent elements, so
  writes through the loop variable hit the parent, per RFC 0012 §6.

## 7. Why strings came first

`str`/`bytes` are immutable, so their views carry no mutability law —
they are purely an optimization plus a nicer parsing surface. [T]
windows change what writes mean, which is why the pointer spelling is
mandatory for them and optional (invisible) for strings.

## 8. Shipped state

- `Nat::StrSlice` (code 5) — `callnat StrSlice r_s(r_from, r_to) -> r_dst`.
- `CellData::StrView { parent, off, len, ascii }` in `rut-vm/src/heap/cell.rs`;
  reads through `CellVal::{as_str, as_bytes, char_len, str_ascii}`.
- The block store (RFC 0039) backs the parent octets; the release walk
  releases the parent as the view's one ref-typed child.
- Declared in `core.d.rut` (str members) — lockstep-tested against the
  compiler's surface.

## Open questions

- OQ-1: `s.slice(from)` / `s.slice(..to)` half-open spellings — defer
  until range syntax exists (RFC 0008 has none today).
- OQ-2: `Array<T, N>`-typed windows currently surface as `*Array<T>`;
  a `&[T; N]`-style length-typed spelling is unnecessary until const
  generics meet real code.
