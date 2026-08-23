# RFC 0040: Resource Limits — Heap Budget, Fuel & Hang Detection

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0016 (heap), RFC 0034 (VM core, budgets),
  RFC 0035 (embedding), RFC 0039 (the self-managed heap)
- **Part:** F — Toolchain & artifacts

## Summary

Two knobs make rut safe to run third-party code: a **heap budget** and
**fuel**. Both are enforced as **resumable traps**, both are set at
construction (`Vm::new_in(host, limits)`) and both are mutable live:

```rust
pub struct Limits {
    pub heap_limit_bytes: Option<u64>, // None = host-enforced only
    pub fuel:            Option<u64>,  // None = unbounded (host still
    pub interrupt_every: u32,          //   owns interrupt())
}
```

## 1. Heap budget

- **What counts**: everything the self-managed heap tracks (RFC 0039
  §1) — cell headers + payloads, buffer blocks (Vec/Array data),
  string blocks, frames and register blocks. What does not: module
  binaries, the shared type table, and host-side structures.
- **Check points**: `Heap::alloc` (RFC 0039 §2) — cell mint
  (`newcell` — RFC 0032 §1), `Vec` growth (`push` reallocation),
  string concat, frame-pool growth all route through it.
- **`Trap::OutOfMemory`**: the check runs *before* any write — a failed
  allocation leaves the heap byte-identical to the state before the
  op. The frame is parked (RFC 0034 §4 semantics): the host may raise
  the limit and resume, or drop refs and retry. Live usage is readable
  (`vm.heap_usage()`) and feeds `heap_stats()` (RFC 0017 §3).

## 2. Fuel

- One unit per executed op; `fuel` counts **down**. The counter is
  checked every `interrupt_every` ops (default 1024) and at loop
  back-edges (the RFC 0034 §1 loop's natural checkpoints).
- **`Trap::OutOfFuel`** parks the frame exactly like `Interrupted`:
  nothing is unwound, resumption is `vm.add_fuel(n)` then `vm.resume()`
  — the frame *is* the loop state (RFC 0034 §4).
- Native fns run **outside** fuel (RFC 0034 §4): a hanging native is a
  host bug; the docs say so, and `interrupt()` (RFC 0035 §1) is the
  host's lever at native return points.
- Wall-clock deadlines stay in the core budget too (`deadline:
  Option<Instant>`, RFC 0034 §4) — same parked-frame, resumable
  semantics; fuel is the deterministic one (tests, reproducible
  reports), the deadline the ergonomic one.

## 3. Hang-detection recipes (host-owned — RFC 0001 G8)

| Host | Recipe |
|---|---|
| UI / game | `run_until_idle()` inside the frame loop; `interrupt()` returns true past a 16 ms slice (RFC 0035 §4) |
| wasm page | `run` granted finite fuel; a `setTimeout` watchdog stops refueling and flips `interrupt()` — the pending frame dies at its next check (RFC 0041 §3) |
| desktop service | watchdog thread holds an `Arc<AtomicBool>` the `interrupt` hook reads; no joins, no signals |
| tests | finite fuel + virtual clock hook → fully deterministic hang proofs |

## 4. Workers

`spawn_worker` (RFC 0021 §3) constructs the child `Vm` with **its own
`Limits`**; argument transfers are counted against the *child* budget
before the worker starts, so a worker cannot OOM its parent.

## Open questions

- OQ-1: op weighting — should `call`/allocation ops cost more than
  register moves? Proposed: no, keep 1:1; the default `interrupt_every`
  already coarse-grains.
- OQ-2: `OutOfMemory` during unwind (Disposal releasing fields) —
  document exact guarantees; proposed: unwind-path allocations are
  arena-charged but never re-trap.
- OQ-3: per-task fuel fairness (a greedy task starving others within
  one `run_until_idle`) — round-robin the ready ring proposed.
