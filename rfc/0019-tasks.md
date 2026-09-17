# RFC 0019: Tasks — `spawn`, `cancel`, `select`

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0018 (async & await)
- **Supersedes:** RFC 0003 §3 (pre-restructure)
- **Part:** D — Concurrency

## Summary

`spawn(fut) -> Task<T>` schedules a future on the current VM;
`cancel()` **drops the coroutine frame at its next suspension point**
(real cancellation — pending sleeps die with it, locals with destructors
run deterministically); `await select { .. }` races futures, the winner's
arm produces the value, the losers are dropped.

## 1. `spawn`

- `spawn(fut) -> Task<T>` schedules the future on the current VM. `Task<T>` is
  itself a `Future<Result<T, Cancelled>>`: `await task` joins it.

## 2. `cancel`

- `cancel()` marks the task cancelled and **drops the coroutine frame at its
  next suspension point** — pending `sleep`s die with it, and every local with
  a destructor (RFC 0016 §3) runs deterministically. Cancellation never
  interrupts mid-expression.

## 3. `select`

- `await select { .. }` races futures; the winner's value is produced by its
  arm expression, the losers are dropped (i.e. cancelled). Arms use `->`
  like `when` (RFC 0008); `fut -> expr` discards the resolved value,
  `fut x -> expr` binds the resolved value to `x` (arm-local binding —\n  `as` stays reserved, RFC 0002 §4).
- `select_all(futs)` (stdlib, built on `select`) resolves with the first
  ready value — used in the worker examples (RFC 0021).
- v1 tasks are unstructured (no automatic child cancellation). Structured
  scopes (`scope { .. }` cancelling children on exit) are OQ-3.

## Open questions

- OQ-1: `await task` on a cancelled task yields
  `Result<T, Cancelled>` — §1; never a trap.
- OQ-2: priorities/fairness — round-robin ready queue in v1; do we need
  task priorities for UI responsiveness before M4?
- OQ-3: structured concurrency scopes with child cancellation.
