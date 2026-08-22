# RFC 0017: Weak References — Cycles Are the Program's Responsibility

- **Status:** Draft
- **Date:** 2026-08-22
- **Revised:** 2026-08-22 — the cycle collector was **dropped**. rut ships
  RC + deterministic destructors + `Weak<T>`, and nothing else: reference
  cycles leak by design, and avoiding them is the program author's job
  (back-pointers go through `Weak`). This file keeps the weak-reference
  contract and the leak-reporting tooling; the trial-deletion collector
  is gone (header colors, suspect lists, stop-the-VM passes removed).
- **Author:** hpp2334
- **Depends on:** RFC 0016 (the RC heap)
- **Supersedes:** the previous "Weak References & the Cycle Collector"
  revision of this file
- **Part:** C — Memory

## Summary

Reference counting frees an object when its strong count reaches zero
(RFC 0016). Strong cycles never reach zero — so **cycles leak until the
`Vm` is dropped**. rut makes this a stated rule instead of hiding it
behind a collector: the heap stays trivially simple (no mark bits, no
colors, no pauses beyond destructor chains), destructors stay exactly
deterministic for everything that *is* released, and `Weak<T>` exists so
the two shapes that cause accidental cycles — back-pointers and caches —
have a first-class answer.

## 1. Weak references

See **`examples/memory/weak-cache.rut`** — `Weak(v): Weak<C>` and
`upgrade(): Option<Rc<C>>`; **`examples/memory/node-cycle.rut`** shows a
strong cycle and its `Weak` fix.

- `Weak(v)` allocates a `WeakBox` side object holding a back-pointer that
  is nulled when the referent dies (strong count hits 0). The referent's
  header keeps the weak list; `upgrade()` is a strong-count check +
  retain (`None` when gone).
- Weak refs do not keep objects alive and are **not** destructors: they
  are for caches/observers/back-pointers. A path through a `Weak` does
  not close a strong cycle.
- Host opaques can expose their own `Weak` views (e.g. to detach a bridge
  when the script side is gone).

## 2. Cycle policy

- **A strong cycle is a leak, deterministically.** Its members live until
  `Vm::drop`. No collector runs, no pass interrupts the VM, no destructor
  ordering surprises exist — what leaks leaks wholly and predictably.
- **Guidance (review/lints, not runtime):** resource-holding classes
  (`Disposal` impls, host opaques behind `Rc`) must not participate in
  strong cycles; parent/child and observer shapes take `Weak`
  back-pointers; pure-data cycles are harmless (memory only).
- The compiler may warn on obvious self-boxing patterns (`Rc(c)` stored
  into `c`'s own field); general cycle detection stays out of scope.

## 3. Safety & leak reporting

- rc discipline is asserted in debug builds (retain/release pairing, no
  underflow, no double-free of the same header);
- **shutdown leak report**: on `Vm::drop`, remaining non-immortal objects
  are logged grouped by type with allocation sites (debug builds) — the
  primary tool for finding leaked cycles; the host can also query live
  counts per type at any time for dev UIs;
- fuzz targets: random programs asserting (a) no crash, (b) destructor
  counts match allocations minus reported survivors;
- behavior is fully deterministic under the virtual clock — a leak
  reproduces exactly.

## Open questions

- OQ-1: opt-in richer diagnostics (retained-by graph dumps) — defer until
  the leak report proves insufficient.
- OQ-2: if real code demands it, a collector returns as a **separate,
  opt-in** module RFC — the header's weak-list bit is the only hook it
  would need; v1 heap assumes nothing.
