# err-channel — the batch report

The record of the err-channel batch: four phases, one shared tree, base
`4ceb17e`, all on `master`. Companion to `docs/err-channel-survey.md`
(phase 0 — the census, the feasibility calls, the pre-registered
bench; its amendment list is the one phase 4 executed). The verdict
section at the end records the user's rulings as law.

## 1. The story, in one section

The v1.1 convention (RFC 0005 §10) answers a call with a pair —
`checked_add` answers `(value, ok)`, a parser answers `(T, err)` — and
that pair is a heap record. The batch asked three questions about the
language's one error channel and the machinery under it, and landed the
answers in four commits:

1. **How much does the mint cost, and where does it live?** (phase 0,
   `fa8faf8` — the survey.) A census tool walked the
   post-optimization IR of 39 units (all 27 bench workloads, the
   `rut/` pkgs, the examples): **668 surviving `MakeRecord` mints**,
   of which the class SROA exists for — a record whose register is
   only read back by field loads — is **empty in real code**. What
   survives is the return channel: ~69% of survivors are 2-field pair
   mints handed to a ret slot and destructured by the caller; two
   `(T, str)` parser programs (digest.rut 338 mints, json-decode 262)
   own 90% of the population. The feasibility call: the pairs live
   POST-INLINE (one function body, one op stream), so a lowering pass
   in the SROA family deletes them — **not** a pair-return IR shape,
   which would spend the 24-byte op pin, binary format v2, and three
   interpreters for a benefit the pass already delivers.
2. **Does deleting it move anything observable?** (phase 1,
   `685d7e5` + `02ced3e` — the checkedadd row, then the fusion.) The
   row was pinned BEFORE the engine change; after `ret_round` landed in
   `rut-lir`'s SROA family, the row fell **−60.3%** with its checksum
   bit-identical and fuel down EXACTLY the IR diff. Every other row in
   the suite: bit-identical. VERSION stayed 7 — pure lowering.
3. **Where is the stack when you want it?** (phase 2, `ed14b9b` —
   capture_stacktrace + the StackTrace builtin class.) RFC 0036's
   wrapper design (a host fn + a rut-side class over an `Opaque`) was
   superseded by ruling: the engine implements it. Capture walks the
   frame chain storing RAW `(func, pc)` pairs — 8 B/frame, no
   symbolication; every member symbolicates lazily against the loaded
   program. wasm32 runs the same walk, byte-exact. VERSION 7 → 8.
4. **Can the pair cross the host boundary?** (phase 3, `b367af1` —
   the entry-err contract.) Tuples already crossed field-by-field; the
   one measured gap was `TyKind::Opt` falling through
   `crosses_boundary`. One arm closed it — `?T` crosses iff `T`
   crosses, decoded nil-flattened — and `(?T, err)` entries type under
   the ORIGINAL rule, no err carve-out. The envelopes: the driver
   result, `"err"` beside `"trap"` in the rut-wasm envelope (trap
   never fills err), and the web pump turned soft-fail (decode, report,
   keep draining). VERSION rides 8.

The two-channel law, now complete end to end: **a returned err is
data** — it crosses as the pair's second component and the host reads
and acts on it; **a panic is drift** — it stays loud, kills the turn,
and never fills an err field.

## 2. The numbers

### 2.1 The proving row — checkedadd, before/after

The row isolates the `(T, ok)` pair economy: 10M `a.checked_add(b)`
calls through a helper fn (the callee/caller shape every `(T, err)`
return rides post-inline), destructured `let (v, ok) = ..`; both lanes
load-bearing in the checksum; the overflow arm deterministic at ~1.1%
of calls. Pinned pre-change at `685d7e5` (the survey §5.1
pre-registration; the only sanctioned `expected.json` edit: a new-row
addition).

| meter | before (`685d7e5`) | after (`02ced3e`) | delta |
|---|---|---|---|
| checksum | `-10015207202` | `-10015207202` | **identical** (rut = node twin = python model, three-way exact) |
| exec median | 542.0 – 574.1 ms | 214.0 – 217.9 ms | **−60.3%** |
| per checked call | ~54.2 – 57.4 ns | ~21.4 – 21.8 ns | −32.7 – −35.6 ns |
| fuel | 310,336,881 | 270,336,881 | **−40,000,000 = −4 ops/call × 10M, exact** |
| VM heap peak | 272 B | 232 B | **−40 B** (the 2-slot record cell out of the high-water) |

The fuel ledger reconciles to the op: the fused window deletes
`makerecord` + `movref` + 2 × `getf` = −4 ops per call; the two
replacement movs were forwarded away by the peephole. The survey's
predicted mint-tax band was 40–80 ns; the measured time delta is
32.7–35.6 ns — disclosed just under the band — of which ~3.1 ns is
dispatch at the crossing-nop calibration (0.77 ns/op), the rest the
allocation + rc work. Post-fusion, rut ~matches V8's node column on
the row (217.8 ms net).

**Every other row: bit-identical.** All checksums, fuel and heap pins
in `expected.json` equal before/after (runner exit 0). The survey
predicted no existing row would move (no bench hot loop destructures
tuples — census §1.4) and none moved. One honesty note carried from the
commit: json-decode read +9.1% in one paired run; re-measured 5× at
663.8–703.4 ms straddling the before-reading with fuel bit-identical
run-to-run — scheduler noise on a shared box, not the fusion; the
identical op stream is the proof.

### 2.2 The capture cost — opt-in, pay-per-capture

Phase 2's micro-estimate (scratch, `/tmp/opencode/batch-err-channel/p2/`):

| depth | capture cost | note |
|---|---|---|
| 1 | ~30 ns | the base walk + one raw frame push |
| 10 | ~45 ns | ~+0.2 ns/frame marginal |
| 100 | ~50 ns | depth-proportional, sub-linear in practice |

Nothing is captured unless rut code calls `capture_stacktrace()`. The
`?`/err propagation path stays zero-cost (RFC 0036 §1's law).
Representation: one `CellData::Trace` cell, 8 B per raw `(func, pc)`
frame, innermost first. Members are pay-per-access and lazy — `len()`
O(1); `name(i)` reads the interner, `line(i)`/`col(i)` binary-search
the `FuncCode.pos` table (empty when stripped → 0); `render()` is the
only whole-trace pass. Recorded, not a bench row: no bench workload
calls capture (grep-verified at phase 2), and the pins stayed
bit-identical through phases 2 and 3.

### 2.3 The census — 668 mints, and what survives after the fusion

| class (phase 0) | count | share | phase 1 outcome |
|---|---|---|---|
| pair (2-field), return-family | 459 | 68.7% | the fusion's target class — elides where the pure shape holds (see below) |
| 3+-field record, return-family | ~100 | ~15% | same shapes, aggregates |
| 3+-field record, stored | 107 | 16.0% | genuine escape (container nodes, `Vec` cells) — correctly still mints |
| boxed `?T` payload | 9 | 1.3% | `MakeOpt` constructions — untouched (8 pins: exactly one makeopt per some-coercion, none for nil) |

Two units own 90% of everything: digest.rut (338) and json-decode
(262) — both `(T, str)`-returning parser programs.

The phase-1 reconciliation, disclosed: the census tool rebuilt against
the post-fusion tree, same 39 units — **668 → 668**. The corpus's
surviving pair mints are (a) terminal `ret` of NON-INLINED callees
(pouch `Vec` internals, digest's error constructors) — genuine
frame-boundary escapes the fusion must not cross; (b) reused join
temps whose use sets include non-consumable reads or multi-writer ret
slots — declined by the single-def envelope, correctly. The pure
ret-slot→destructure chain the survey traced (§1.5 probe) DOES fuse —
that probe's 5 survivors → 2 (only the two-return join left) — and the
checkedadd row is that class at scale. The reading: the corpus's
surviving population was never the pure shape; the pure shape is what
every future inlined `(T, err)` call site (and phase 3's entry-err
economy) rides, and the row now guards it.

### 2.4 The pins ledger

| gate point | result |
|---|---|
| phase 1 (`02ced3e`) | all rows' checksums + fuel + heap bit-identical (runner exit 0); only checkedadd was ADDED (pre-pinned at `685d7e5`) |
| phase 2 (`ed14b9b`) | all 26 `expected.json` pins bit-identical; no row calls capture |
| phase 3 (`b367af1`) | all 26 pins bit-identical AND a base-vs-phase-3 rut-only comparison (the `ed14b9b` worktree) shows every row's checksum AND fuel AND heap peak identical — no adopting row exists in the suite, so nothing re-pinned, nothing disclosed |

`expected.json` was never touched after the phase-1 row addition.

## 3. The VERSION ledger

| step | when | why |
|---|---|---|
| **7** | entering the batch (the downcast → `?T` amendment's bump) | baseline |
| **stays 7** | phase 1 — the fusion | pure lowering: no op, no encoding, no surface change; old binaries load, new binaries just mint fewer cells. Fuel re-pins? None — disclosed old values verbatim in the commit body as required, all unchanged |
| **7 → 8** | phase 2 — StackTrace | a declared-surface change: `builtin fn capture_stacktrace` + `builtin class StackTrace` in `core.d.rut` (TY_STACK_TRACE = 16, `TyKind::Trace` encode/decode tag 14, a new Nat family appended); the func table also serializes `pos` beside spans — stale v7 artifacts are rejected with the standard version error (the 6 → 7 precedent, pre-registered in the survey §3.1) |
| **rides 8** | phase 3 — the entry-err contract | a CHECK relaxation, not a format change: no new op, encoding, or surface; old artifacts are behaviorally bit-identical (their root rets can never mention `Opt` — the old checker rejected exactly that), and the reverse direction is toolchain-guarded. Recorded in `binary.rs`'s history note |

## 4. The adoption record

Where the shape was adopted, and where it was deliberately not —
the scope calls phase 3 recorded:

**Adopted.**

- **digest.rut's `json_dec`** — the teaching adoption: `(?opaque, str)`;
  the `noopaque()` sentinel is gone from the crossing surface
  (internals keep it). The runner asserts `(None, why)` on failure and
  `(Some, "")` on success.
- **The todolist store's answer lines** — `answer -> (line, err)`: a
  commit is `(line, "")`; a rejection or a lost id is `("", why)`.
  `last` keeps the user-facing line either way; the status line text is
  unchanged; `on_event` surfaces the why through the err channel.
- **`on_event -> (?opaque, str)`** on all three page programs (app,
  harness, t1 harness) — every turn now answers through the pair.
- **The demo runner surfaces the field** where cheap: `RunResult.err`
  in `rut-api.d.ts` + `runner.ts` + an `Err::` line in the Output pane
  beside `Trap::`; `tsc --noEmit` clean.

**Recorded, not adopted.**

- **`store_request_toggle`/`store_request_remove` keep the `""` tag** —
  that absence feeds the app's LOUD drift path (a missing answer is a
  wiring bug, not a soft rejection), and the Opt decode is already
  taught twice elsewhere in the same app.
- **The demo's cheap surfaces only** — the demo runner reads the err
  field where a line already exists; no new UI.

**The containment proofs** (twin tests through the REAL pump,
`examples/05-todolist-web/tests/softfail.rut`): a turn returning
`(nil, "boom")` comes back `Ok`, is reported in `turned_errs`, and the
next events work in the same drain (container usable, the hits probe
reads 2) — a returned err does not poison the container. A panicked
turn kills the pump loud, never crosses as data, and the `Vm` survives
for the next good turn. On the real app: the duplicate-add rejection
crosses the err channel (`turned_errs` + status line) and the NEXT add
commits and paints; the lost-id skip turn proves the page alive after
the err. The engine's negative pin: a `?Bag` entry whose element does
not cross still rejects — there is no carve-out.

## 5. Honest limits

- **The invariant is the CALLER's convention, never boundary-enforced.**
  Exactly-one-non-nil in the `(?T, err)` pair: the engine types the
  components and decodes them; it does not police the combination. The
  convention (empty err + a value = success; empty err + nil = "not
  found"; non-empty err = "failed") is documented and the adopters
  follow it — nothing in the VM enforces it.
- **The census survivors are real escapes.** 668 → 668 post-fusion, on
  the disclosed causes: non-inlined callees' terminal rets and
  multi-writer/mixed-use join slots. The fusion correctly declines
  them — crossing a frame boundary or guessing at a join is exactly
  the bug class the single-def envelope exists to prevent. The row
  guards the pure shape; the corpus guards the declines.
- **Capture is opt-in.** No auto-capture on `?` propagation, no trace
  in the default err path. A rich err carries a trace only where the
  producer decides the cost is worth it (`trace: ?StackTrace`, nil the
  default — the RFC 0028 §`debug` sketch). `Trap`'s own unwind capture
  is unchanged.
- **The fusion is bounded by inlining.** The mint tax dies where
  callee and caller are one function body post-inline (the checker's
  ≤24-statement window); a non-inlined `(T, err)` callee still mints.
  The inline law is pinned (a capture/checksum inside an inlined
  callee reports the frame it LANDED in).
- **Symbolication degrades honestly.** Stripped builds answer `0`
  line/col and pc-only render text (`at c_big (core #3 @ pc 25)` vs
  the symbolicated `at c_big (core:27:13)`); restoration stays RFC
  0036's three paths (in-VM, `.rutc.map` sidecar,
  recompile-by-determinism).

## 6. The verdict — the rulings, recorded as law

The user's rulings (Sep 2026, the settled design), now landed and
binding:

1. **`(?T, err)` is THE answer channel — KEPT.** `Option`/`Result`
   stay dead by design (the v1.1 removal law); reasons belong in the
   type system, spelled as the pair. The engine now makes the pair
   cheap where it is used as a pair (the fusion) and lets it cross the
   host boundary as a first-class answer (the contract). RFC 0044's
   amendment records the law.
2. **`capture_stacktrace() -> StackTrace` is a `builtin class`** — the
   engine itself implements it, compiler-lowered, declared in the
   toolchain's decl file only (RFC 0025's builtin-class row; NOT a
   `builtin primitive`, which is for value types; NOT the RFC 0036
   host-fn wrapper, which this supersedes). RFC 0036's amendment
   records the surface and reconciles the spelling to
   `capture_stacktrace`.
3. **Entry fn returns follow the ORIGINAL rule** — builtin primitives
   (and `opaque`, the container pattern), tuples crossing
   field-by-field; the one `Opt` arm closes the measured gap. No
   err-specific carve-out exists or may be added: the negative pin
   (`?Bag` with a non-crossing element) guards this forever. RFC
   0023's amendment records the arm; RFC 0025's records the decl-side
   consequence.
4. **Soft-fail containment at boundaries.** A turn that fails returns
   its err; the host decodes it, reports it, and keeps going — the
   container survives. `panic` stays loud, for bugs and wiring drift
   only: returned err = data, panic = drift. The rut-wasm envelope
   carries `"err"` beside `"trap"`, and trap never fills err. RFC
   0035's amendment records the host-loop semantics.

## 7. The menu going forward

What the batch leaves on the table, honestly priced:

- **The remaining mint population** (668 survivors): the dominant
  residual is non-inlined callees' terminal rets. The known lever is
  inlining reach (the 24-statement window), not IR — a pair-return IR
  shape stays rejected (the survey §2 call; the op pin and format v2
  are pinned assets).
- **A direct-ref lane for nmapset's val column** — refcolumn round 2's
  verdict already names it: the stored-unit-is-a-box floor (~+88
  ns/get) survives every spelling change; an engine representation
  change (the record cell behind a rut-owned handle) is where a third
  attempt would start. Not this batch's scope.
- **Trace ergonomics** — RFC 0036's open questions stand (skip/filter
  args for capture; `TaskHandle.trace()` for parked coroutines; the
  `?at` opt-in sugar). One menu note from this batch: an err-carrying
  convention (`trace: ?StackTrace` in err data) now has its enabling
  surface and its cost table; adopting it in `debug`'s sketches is a
  one-line follow-up when wanted.
- **The envelope's growth discipline held** — `rut_web_*` kept the
  `0 | -1` + `rut_web_last_error` slot (take-once, no ABI growth). If
  a second soft channel is ever needed, the err-beside-trap envelope is
  the pattern to extend, not a reason to grow the ABI.
- **Bench suite guard**: the checkedadd row is permanent — the pair
  economy's canary. Any future change that touches minting, SROA, or
  the peephole reads its fuel pin first: −4 ops/call was the fusion;
  any drift from 270,336,881 without a disclosed cause is a bug.

## 8. The gates, as landed per phase

| phase | commit | workspace tests | wasm32 | bench suite |
|---|---|---|---|---|
| 0 | `fa8faf8` | docs-only, untouched | n/a | n/a |
| 1 | `685d7e5` + `02ced3e` | 655 passed / 0 failed | exit 0 | green, checksums bit-identical |
| 2 | `ed14b9b` | 666 passed / 0 failed | exit 0 (render byte-exact) | green, 26 pins bit-identical |
| 3 | `b367af1` | 675 passed / 0 failed (687 incl. suites) | exit 0 | green, 26 pins + base-vs-phase-3 bit-identical |
| 4 | this commit | docs-only | re-verified | re-verified (paranoia — docs cannot move pins) |

Phase 4 changed no code. The RFC amendments record landed behavior;
the perf-log entry records measured numbers; nothing here moves a pin.
