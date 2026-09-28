# String slicing and views

`s.slice(from, to)` — the O(1) string window. A slice is a **view**: no
octets move or copy; the new `str` cell retains the original and
records a byte window. Slicing is safe because `str` is immutable — a
view can never observe mutation, so no pointer spelling is needed.

## Surface

```rut
let s = "hello world";
let w = s.slice(6, 11);        // "world" — no copy
```

- `from`/`to` are **codepoint indices**, `from <= to <= s.len()`;
  bounds violations trap (same as an out-of-range index).
- The window is recorded internally in **byte offsets** — codepoint
  bounds are resolved once at slice time (a UTF-8 walk only when the
  string is non-ASCII).
- The result is an ordinary `str`: it prints, compares by content
  (`==`), iterates (`for (c of w)`), renders in f-strings, and
  `len()` counts its own codepoints. **There is no separate view type
  on the surface** — a slice of a `str` *is* a `str`.
- Binding the slice shares it like every cell (see
  [By-reference and nullable](by-reference-and-nullable.md)); `str` has
  no clone — a materializing copy happens only where an operation needs
  one (the append fast path, the host crossing).

```rut
let accented = "héllo!";
accented.len();                  // 6 codepoints
accented.encode().len();         // 7 octets
accented.slice(1, 2);            // "é"
```

## The view cell

A slice mints a small view cell: `{ parent, off, len, ascii }` — a
**retained** handle to the owned parent string plus byte offsets into
its block.

- The view charges only its own header (~32 bytes) against the heap
  budget; the parent's octets stay alive as long as any view does
  (deterministic destruction: view rc-0 → parent release).
- **View-of-view flattens**: slicing a view combines offsets onto the
  root, so the parent is always an owned string and reads never chain.
- The ascii flag is inherited from the root at slice time (a window of
  an ASCII string is ASCII).

## Reads go through, writes never do

Every string operation reads through views transparently: content
equality, codepoint count, indexing, iteration, f-string rendering,
`encode`, and the host boundary (which copies the window's bytes out).

The in-place append fast path (the `out = f"{out}.."` accumulator) is
gated to **owned, uniquely-referenced** cells. Concatenating a view
copies its bytes out — `f"{view}!"` yields a fresh owned string, and
the view (and its parent) are unchanged. No operation can ever mutate
through a string view.

## Cost model

| Operation | Cost |
|---|---|
| `s.slice(a, b)` | O(1) — one small cell + one retain (UTF-8 walk only for non-ASCII bounds) |
| read/compare/render a view | O(window) — same as any str |
| concat out / the host crossing | O(window) — the materializing copy |
| parsing a 1 KiB line out of a 1 MiB buffer | one 32-byte cell, zero copies |

Digest-style workloads (splitting, tokenizing, windowing) stop copying
entirely; peak heap reports the buffer once instead of per-piece.

## `[T]` windows

`v.slice(from, to)` on a `Vec<T>` (and on `[T]`) mints a fixed-length
window over the backing array, boxed as **`?Vec<T>`** (the `T → ?T`
coercion — see [Builtin generic types](builtin-generic-types.md)). The
window **is** the shared cell:

- reads: `w[i]`, `w.len()`, `for (x of w)` — auto-deref the nullable
  and go through the window (element `i` is `parent[off + i]`, bounds
  are the window's);
- writes: `w[i] = x` (and compound assignment) **hit the parent**;
- **fixed-length**: `push`/`pop` through a view trap — copy the
  elements out to grow (a `for`-push loop does it);
- reslicing flattens onto the root backing (`w.slice(a, b)`);
- parent growth **detaches**: `push` re-backs the `Vec` with a fresh
  array; the window keeps pinning the old backing — the same aliasing
  rule Go slices have;
- iteration yields the stored elements as-is under the sharing law: a
  cell element shares its cell, so a write through the loop variable
  hits the parent; primitive elements copy out as slots always do.

The asymmetry is deliberate: `str`/`bytes` are immutable, so their
views carry no mutability law — purely an optimization plus a nicer
parsing surface. Array windows change what writes mean, which is why
they surface as an explicit `?Vec<T>` value while the string view is
invisible.
