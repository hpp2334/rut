# RFC 0019: Join, `select`, and Structured Scopes

- **Status:** Draft — rewritten in the Future-only vocabulary; the
  `Task` noun is gone (superseded by RFC 0018's landing: the receipt
  type `LaunchedFutureHandle<T>` is its own type, never a `Task`)
- **Date:** 2026-08-23 · **Amended:** 2026-09-27
- **Author:** hpp2334
- **Depends on:** RFC 0018 (async & await — landed)
- **Supersedes:** RFC 0003 §3 (pre-restructure)
- **Part:** D — Concurrency

## Summary

Nothing in this RFC landed with RFC 0018's phase 2 — the landing kept
the vocabulary to one noun (`Future<T>`), one consume law, one trigger.
This RFC is the deferred remainder, spelled in the landed vocabulary:

- **join** — `await h` on a `LaunchedFutureHandle<T>` (today: a compile
  error naming RFC 0019).
- **`await select { .. }`** — races futures; parses today, compile-gated
  "not in this build — RFC 0019".
- **structured scopes** — `scope { .. }` cancelling children on exit
  (OQ-3).

## 1. Join (was `spawn`/`await task`)

There is no `spawn` and no `Task<T>`. A launched future's receipt —
`LaunchedFutureHandle<T>` — is the join surface: `await h` joins the
launched future, producing its (currently discarded) completion value.
Until this RFC lands, `await h` diagnoses with the join law, and the
receipt stays non-launchable and non-awaitable (RFC 0018's ruling 6).

## 2. Cancel

Landed with RFC 0018: the receipt's `abort()` flags the task's
cancellation and re-enqueues it; **the probe at its checkpoint runs the
drop path** — pending `sleep`s die with it (the drop path clears the
frame's pending edge), and every local with a destructor (RFC 0016 §3)
runs deterministically, in reverse declaration order. Cancellation never
interrupts mid-expression: the probe is the only place a cancellation
becomes observable.

## 3. `select`

- `await select { .. }` races futures; the winner's value is produced by
  its arm expression, the losers are dropped (i.e. cancelled). Arms use
  `->` like `when` (RFC 0008); `fut -> expr` discards the resolved
  value, `fut x -> expr` binds the resolved value to `x` (arm-local
  binding — `as` stays reserved, RFC 0002 §4).
- The grammar LANDED with the async parser (the arms parse today); the
  semantics are this RFC's — the compiler gates the expression with
  "`await select` is not in this build — structured competition lands
  with RFC 0019".
- `select_all(futs)` (stdlib, built on `select`) resolves with the first
  ready value — used in the worker examples (RFC 0021).
- v1 tasks are unstructured (no automatic child cancellation). Structured
  scopes (`scope { .. }` cancelling children on exit) are OQ-3.

## Open questions

- OQ-1: `await h` on an already-cancelled receipt yields
  `Result<T, Cancelled>` — never a trap.
- OQ-2: priorities/fairness — round-robin ready queue in the landing's
  driving loop; do we need task priorities for UI responsiveness
  before M4?
- OQ-3: structured concurrency scopes with child cancellation.
- OQ-4 (new): a user-spelled checkpoint algebra — `cx.next_checkpoint(v)`
  from user bodies (the weave writes the states it owns today).
