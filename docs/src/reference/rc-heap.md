# The Rc heap and destructors

rut's heap is **reference-counted**. When an object's strong count reaches
zero it is destroyed **immediately**, at a deterministic point in the
program. There is no collector: strong cycles leak by design —
[weak references](weak-refs.md) are the answer. Every VM owns one heap on
one thread and nothing is shared between isolates
([workers and channels](workers-and-channels.md)), so the counters are
plain cells with no atomics and no locks.

## The cell model

Everything except primitives and `fn` values is a heap cell:

| Inline (moved by plain copy) | Heap cells (refcounted handles) |
|---|---|
| `u8..u64`, `i8..i64`, `u`/`isize`, `f32`, `f64`, `bool`, `nil` | `str`, `bytes`, `Vec<T>`, `[T]`, enums, structs, classes, trait objects, `opaque` boxes, host boxes, `?T` boxes, coroutine frames |

Assignment, argument passing, and returning copy the **handle** (retain),
never the bytes. Mutation through one alias is visible through every
alias — reference semantics is the one default regime; there is no eager
copy anywhere. `bytes.clone()` is the **only** copy escape hatch.

There is no borrow syntax and no lifetimes: a handle simply keeps its
referent alive, so nothing dangles. Uniqueness matters only when
transferring buffers across isolates, where it is detected at runtime
(`rc == 1`) — never proven statically.

## Refcounting rules

The compiler knows the static type of every register, so ref-aware ops
are emitted only where a reference can flow:

- Primitives and `fn` values move with a plain move — no counting.
- References move with a ref move — retain the new, release the old;
  both steps are exact.
- Function boundaries pass references in registers; call/return
  sequences emit the paired inc/dec. A loop over a `Vec<f64>` does no
  per-element counting.
- **Overflow**: an increment past `u32::MAX` *immortalizes* the object —
  the count is pinned to the `0` sentinel (the same value interned
  literals use) — a deliberate, logged leak instead of unsoundness.
- Debug builds assert the full counter discipline: retain/release
  pairing, no underflow, no double destruction.

## Destruction — `on_drop`

A cleanup attaches to a **nullable binding** — the cell reference
itself — and runs when that cell's count reaches zero:

```rut
use core::{ on_drop };

fn work(log: ?AuditLog) {
    let buf: ?bytes = load();
    on_drop(buf, fn (b: ?bytes) { audit(b.len()); });  // runs at rc 0
    use_buf(buf);
}
```

Laws:

- Signature: `on_drop<T>(p: ?T, cleanup: fn(?T))` — exactly two
  arguments; `p` must be a nullable; the cleanup must be a function
  value. All three are compile errors otherwise.
- **One callback per cell**: a second `on_drop` on the same reference is
  an error, never a silent overwrite.
- `on_drop(nil, f)` traps ("on_drop on nil"); a nil cleanup traps at the
  attach.
- The callback runs **at a call boundary**, not re-entrantly inside the
  release: death pins the cell, queues the cleanup, and the interpreter
  drains the queue between calls, passing the dying referent as the
  `?T` argument.
- Cancellation drops locals at the suspension point through the same
  machinery ([tasks](tasks.md)) — no special case.

Note the difference from finalizer-based runtimes: a cleanup always
runs, exactly once, at a knowable point. There is no "later or never".

## Destruction order

When a cell's strong count reaches zero:

1. Every `Weak` box watching the cell is nulled **before any user code
   runs** — a cleanup that calls `upgrade()` sees `nil`, deterministically
   ([weak references](weak-refs.md)).
2. A queued `on_drop` cleanup runs (pinned cell, then released).
3. Ref-typed children are released **recursively**: record fields in
   declaration order, sum payloads, array elements, `opaque` box inners,
   closure captures. The walk is driven by a per-type release plan built
   once from the type table.
4. Host box payloads run their Rust `Drop` at the same point — sockets,
   files, and textures die with the last handle, not "sometime later". An
   opt-in `finalize` hook (no-op default) runs *before* the payload's
   `Drop`; a rut value the payload held releases through the same walk.
5. The block store frees the cell's variable-size payload; the slot
   returns to the arena free list and is reused by the next mint
   ([the VM heap](vm-heap.md)).

## Buffers, strings, and views

- **Primitive-element `[T]` buffers stay flat**: a `[T]` over numeric or
  `bool` element types stores raw values inline behind the header.
  Element copies in and out are plain moves — no counting. This is the
  one place the everything-is-a-cell law does not reach.
- **`Vec<T>` is not flat**: its backing is `buf: [?T]`, so element
  traffic crosses nullable handles. `push`/`pop`/`set` emit the paired
  retain/release.
- **Composite-element buffers store handles**: `[Point]` and `Vec<Point>`
  hold one cell pointer per element; the buffer itself is the counted
  unit.
- `[T]` is a fixed-length cell; `N` is a compile-time constant and part
  of the type's identity. Fixed-array literals that fold at compile time
  become immortal constant-pool cells.
- **`Slice<T>` view cells** hold a handle to their owner plus `off`/`len`
  — never a pointer into the data block. Views dispatch through the
  owner, so growth keeps existing views valid; indexing bounds-checks
  against `len ∩ owner len` and **traps** out of range instead of reading
  garbage. Slices are born only from implicit widening at an argument
  site; they are not nameable or constructible in script.
- **`str` is immutable**: interned literals live in the module's constant
  pool (immortal); runtime-built strings are ordinary cells whose
  buffers are internally copy-on-write — an implementation detail no
  program can observe, because `str` has no mutation API.
- **No interior pointers exist anywhere**, which is what keeps every
  heap walk a simple typed walk.

## Internals

```text
Header:  rc: u32        // strong count; 0 = immortal sentinel
         ty: u32        // index into the type table
         flags: u32     // weak-list bit, drop-callback bit
```

Cell kinds: string, vec (growable), array (fixed), slice view, record
(every user struct/class — vtable + payload slots), enum (tag + payload
slots; dataless variants are immortal singletons), opaque box, weak box,
and the coroutine frames the async weave mints. Reified layouts are
covered in [reified types and layout](reified-types.md).

The engine's two counting ops are exact and total:

```rust
fn retain(&self, p: Slot) { /* rc += 1 */ }

fn release(&self, p: Slot) {
    let n = rc(p) - 1;                 // underflow is a bug: assert
    set_rc(p, n);
    if n == 0 { destroy(p); }          // order above; no suspect list
}
```

Destruction is fully deterministic under the virtual clock, so a drop
order reproduces exactly in tests. Identity and equality semantics for
cell values are covered in [Rc, dispose, and identity](rc-dispose-identity.md).
