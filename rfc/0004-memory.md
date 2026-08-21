# RFC 0004: Memory — Refcounting, Destructors, Cycle Collection

- **Status:** Draft
- **Date:** 2026-08-21
- **Author:** hpp2334
- **Depends on:** RFC 0001 (pillar P3), RFC 0002 (types), RFC 0003 (workers)
- **Implementation sketches:** RFC 5004 (heap layout, RC ops, collector core)

## Summary

The rut heap is **reference-counted**. Strong counts reach zero ⇒ the object
is destroyed immediately — deterministic destructors for host resources, no
pause-the-world collector in the common case. A budgeted **cycle collector**
(trial deletion) runs occasionally to reclaim reference cycles; weak refs are
supported. Every VM owns one heap on one thread; nothing is shared between
isolates (RFC 0003), so all counters are plain `Cell`s — no atomics.

## 1. Heap object model

Only these are heap objects: **Rc cells** (RFC 0002 §5.3 — `Header +
vtable + fields`), **arrays**, **strings**, **bytes**, builtin
`Option`/`Result` *of references* (payload of a ref type is a pointer;
payload of a value type is inline), coroutine frames, and host opaques.
Dataclasses and bare class values are **never heap objects** — they are
inline byte sequences with no header and no refcount (RFC 0002 §10.1).

Each heap object starts with a header (rc count, type id, flags); the exact
layout and the untagged `Slot` union are specified in RFC 5004 §1. The
tagged `Value` enum exists **only at the FFI boundary** (RFC 0005 §3).

## 2. Refcounting rules

The compiler knows the static type of every register, so it emits ref-aware
ops only where a reference can flow (`Mov` for scalars, `MovRef` for refs —
RFC 5004 §2). Consequences:

- **Function boundaries** pass references in registers; call/ret sequences
  do the paired inc/dec. Nothing per-element happens inside `Array<f32>`
  loops.
- **rc overflow**: on increment past `u32::MAX` the object is *immortalized*
  (rc pinned to 0) — a deliberate, logged leak instead of memory unsafety
  (Swift's rule). Debug builds assert the counter discipline.
- **Reference semantics** (RFC 0002 §2): Rc cells, arrays, strings and bytes
  are handles — passing them retains. There is no borrow syntax and no
  lifetimes; a handle simply keeps the referent alive, so nothing dangles.
  Uniqueness matters only for buffer *transfer* across isolates (RFC 0003
  §5.2), detected via rc==1 at runtime — never via static proofs.

## 3. Deterministic destructors

See **`examples/memory/temp-file.rut`**. A class may declare a reserved
`dispose(): void`; such classes must live behind `Rc` (bare use is a
compile error — RFC 0002 §5.3). Ordering guarantees:

1. the user `dispose()` method runs first, with all fields still valid;
2. fields are then released in declaration order (recursively);
3. host opaques (RFC 0005 §5) run their Rust `Drop` at the same point —
   releasing textures/sockets when the last handle goes away, not "sometime
   later at GC";
4. coroutine frames are objects too: **task cancellation drops locals at the
   suspension point** (RFC 0003 §3) — same machinery, no special case.

Note the difference from boa: no `FinalizationRegistry`, no flush jobs, no
"finalizer may run later or never".

## 4. Weak references

See **`examples/memory/weak-cache.rut`** — `Weak(v): Weak<C>` and
`upgrade(): Option<Rc<C>>`.

- `Weak(v)` allocates a `WeakBox` side object holding a back-pointer that is
  nulled when the referent dies (strong count hits 0). The referent's header
  keeps the weak list; `upgrade()` is a strong-count check + retain.
- Weak refs do not keep objects alive and are **not** destructors: they are
  for caches/observers. Cycles through weak refs are not cycles.
- Host opaques can expose their own `Weak` views (e.g. to detach a bridge
  when the script side is gone).

## 5. Cycle collector (trial deletion)

Reference cycles are the one thing RC cannot free. See
**`examples/memory/node-cycle.rut`** for a minimal cycle.

Cycles can only form through **mutable heap slots**: Rc-cell class fields,
builtin `Option`/`Result` payloads holding refs, and `Array<T: ref>`
elements. Strings, bytes, numeric arrays, dataclasses-without-refs, and bare
class values can never participate (§6) — the scanner skips them entirely.

Algorithm (Bacon–Rajan style trial deletion, stop-the-VM, budgeted):

1. every `release` that leaves rc==1 pushes the object to a *suspect* list;
2. when the suspect list crosses a threshold (or a byte-budget delta), run a
   collection pass;
3. **trial deletion**: DFS from suspects, decrementing internal counts;
4. **scan**: re-walk from the suspects; objects whose trial count is still 0
   are garbage (nothing outside pointed at them); reachable ones get their
   counts restored;
5. collected cycles are freed — destructors run (§3), then fields.

Core code (`trial_delete`, `scan_and_free`) is sketched in RFC 5004 §3.

- The whole pass walks only composite objects that *can* point at refs —
  never string/bytes/numeric-array payloads, never registers.
- Budget: passes are capped by a work counter; if a pass exceeds it, the
  remainder is deferred to the next trigger (embedding-safe, like everything
  else — the host can also force `vm.collect_cycles()` between frames).
- Destructors of collected cycle members run in deterministic order
  (discovery order), and run **exactly once**.

## 6. Arrays & strings: why they stay cheap

- `Array<T>` for numeric/bool/char `T` stores raw elements inline
  (`Array<f32>` is literally `Vec<f32>` behind a header). Only the header is
  refcounted; element copies in/out are plain `Slot` moves, no inc/dec.
  `Array<Point>` (dataclass elements) is likewise inline and uncounted.
- `Array<T: ref>` stores pointers; the scanner sees element pointers, RC
  sees one count for the array. `push`/`pop`/`set` emit the right inc/dec
  ops.
- `string` is immutable → interned literals live in the module's constant
  pool (immortal, rc==0 sentinel); runtime-built strings are ordinary
  objects with no interior pointers. `bytes` likewise.
- No interior pointers exist anywhere (no `&mut` into the middle of an
  array/bytes in v1 — see RFC 0002 OQ-5), which is exactly what keeps the
  cycle scanner a simple typed walk.

## 7. Safety & testing

- rc discipline is asserted in debug builds (retain/release pairing, no
  underflow, no double-free of the same header);
- shutdown leak report: on `Vm::drop`, remaining non-immortal heap objects
  are logged with allocation sites (debug builds) — the same facility that
  makes cycle-collector tests trustworthy;
- fuzz targets: random programs asserting (a) no crash, (b) destructor
  counts match allocations, (c) cycle collection reclaims forced cycles;
- the collector is fully deterministic under the virtual clock, so tests
  reproduce exactly.

## Open questions

- OQ-1: moving/compacting collector for long-lived UI heaps — defer to v2;
  non-moving keeps host `&mut` borrows into `bytes`/`Array` trivially sound
  (RFC 0005 §3 borrow guards).
- OQ-2: should `suspect` threshold be adaptive (RC-drop-ratio heuristic)
  rather than a fixed count?
- OQ-3: finalizer-style `dispose()` observer vs destructor-only —
  destructor-only proposed (weak refs cover the observer use case).
- OQ-4: per-worker heap quotas (`vm.set_heap_budget`) so a runaway worker
  traps instead of OOM-ing the host — likely yes, cheap to add.
