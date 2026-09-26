# RFC 0017: Weak References — Cycles Are the Program's Responsibility

- **Status:** Draft — **IMPLEMENTED (2026-09-26, the weak batch;
  see §1a and `docs/weak-report.md`)**
- **Date:** 2026-08-23
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

## 1a. Landed (the weak batch, 2026-09-26)

`Weak<T>` ships as the engine's first generic `builtin class`
(`docs/weak-report.md` is the realization record; the deltas from this
file's draft wording):

- **The nullable-era spelling.** §1's `upgrade() -> Option<T>` is
  `upgrade() -> ?T` (RFC 0044): nil is the null slot, and on a dead
  referent `upgrade` answers the null slot itself — never a box
  containing nil. On `Weak<?U>` (legal, D2) the answer is `??U` — the
  sticky-`?` law.
- **Construction is a type-call** — `Weak(v)`, the `opaque(v)` law; `T`
  infers from `v`. Admission at the instantiation: `T` must be a
  reference type (`Weak<i32>` diagnoses; fn values are RFC 0016 §1's
  one non-cell non-prim and refuse with the same message).
  `weak(nil)` traps ("weak on nil" — the `on_drop` text).
- **The referent's weak list is realized as the arena's `weak_lists`
  side map** (the `drop_fns` shape — lazy, uncharged, keyed by the slot
  word for plain cells AND opaque store entries; both
  weak-referenceable). The draft's "header keeps the weak list" is this
  map; RFC 0017 OQ-2's "header weak-list bit" hook is the map's
  existence.
- **The consuming construction.** `Weak(v)`'s op consumes the incoming
  reference (releases the temporary's +1 and nulls the register): the
  weak observes the BINDING's lifetime, never the temporary's. It is
  the engine's one consuming op.
- **Deterministic ordering, now test-pinned:** the referent's death
  nulls every box BEFORE the on_drop pin check and before any user code
  runs — a `dispose` body or a queued cleanup that calls `upgrade()`
  sees nil. The cycle pin runs the upgrade from INSIDE the child's
  cleanup and requires nil.
- **The wire:** `TyKind::Weak { elem }` (16), opcodes `WeakNew` (92) /
  `WeakUpgrade` (93) — the `MakeOpt { .., ty }` operand law, because the
  ops carry the instantiated TypeId the `CallNat` form cannot —
  `NativeTy::Weak` in core's surface, VERSION 13.
- **§3's status:** the rc-discipline debug asserts and the deterministic
  virtual-clock behavior hold as drafted; the shutdown leak report,
  host-queryable live counts and the fuzz targets are recorded as the
  menu (`docs/weak-report.md` §7) — the batch landed Weak alone.

## 1. Weak references

See **`demo/src/examples/weak-cache.rut`** and
**`demo/src/examples/node-cycle.rut`** — both still show the STRONG-ref
shapes (the weak batch's engine landed under them; the demo rewrites
are the demo lane's). The landed surface, per §1a:

- `Weak(v)` mints a `Weak<T>` box — a WeakBox side cell holding the
  referent's slot word, **unretained**: a weak never keeps anything
  alive. It works over **any cell** (RFC 0016 §1) — a class, a
  dataclass, a `Vec` (via `pouch`), an enum, `str`, `bytes`, a user
  `opaque` box, a HOST box; primitives and fn values refuse at the
  instantiation. The referent's weak list (the arena's map) is walked
  at referent death; `upgrade()` is a dead-check + retain (`?T`, nil
  when gone).
- Weak refs do not keep objects alive and are **not** destructors: they
  are for caches/observers/back-pointers. A path through a `Weak` does
  not close a strong cycle.
- Host opaques can expose their own `Weak` views (e.g. to detach a
  bridge when the script side is gone) — the script-side half (weak
  over a host box) is landed; the host-side view remains future.

## 2. Cycle policy

- **A strong cycle is a leak, deterministically.** Its members live until
  `Vm::drop`. No collector runs, no pass interrupts the VM, no destructor
  ordering surprises exist — what leaks leaks wholly and predictably.
- **Guidance (review/lints, not runtime):** resource-holding classes
  (`Disposal` impls, host opaques) must not participate in
  strong cycles; parent/child and observer shapes take `Weak`
  back-pointers; pure-data cycles are harmless (memory only). Cycles are
  possible through **ordinary dataclass fields** (`next: Option<Node>`
  back-pointers — RFC 0016 §1) as well as collections.
- The compiler may warn on obvious self-reference patterns (a value
  stored into its own field through a handle path); general cycle
  detection stays out of scope.

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
