# The json-perf batch — the report

The record of the json-perf batch: five phases, one shared tree, base
`139ee7d`, all on `master`. Companion to
[`docs/json-perf-survey.md`](json-perf-survey.md) (phase 0 — the
per-stage breakdown whose §-numbers every later phase argues from; the
fuel ledger reconciles to the pin EXACTLY). The batch asked one
question — how far can the `json-roundtrip` lane close toward qjs
without breaking the law — and answered it in four commits after the
survey:

**23.1× → ~2.7× (probe exec 796.4 → 110.3 ms, −86%), every checksum
immovable, json still rut.**

THE LAW (binding, from the plan, restated by the survey and honored by
every phase): json STAYS rut — the host never learns what json is; the
host/stdlib gains GENERAL machinery only (anything any tokenizer
wants); still rejected on record: host-part json impls, engine-woven
builtin traits, runtime reflection, a JsonValue DOM for decode;
checksums immutable across every phase; fuel/heap movers disclosed
re-pins with the old values verbatim; every other bench row
bit-identical; house bench discipline (interleaved A/B, fresh-VM probe
iters, ±1% noise gate, inside-noise → revert and keep the analysis).

## 1. The story, in one section

1. **What does the row actually spend on?** (phase 0, `39c3326` — the
   survey.) Scratch harnesses over the row's own generator decompose
   the pin exactly: fuel 60,933,262 reconciles ZERO-GAP — classify
   44.9%, writer 21.5%, the codepoint split 15.1%, carve 8.3%, mint
   residual 5.7%, gen+fold 4.5%. But TIME disagrees with fuel: encode
   owns ~84% of the wall (222.7 of 265 ms/rep) on ~22% of the ops. The
   finding that ordered the batch: the writer's class-field
   accumulator (`self.out = f"{self.out}{t}"`) costs ~19 µs/event vs
   ~25 ns for the same append through a local — **~750×** — because
   the rc==1 in-place append path (RFC 0042 §4) does not fire through
   a field, so every event copied the WHOLE accumulator: ~6 GB of
   memcpy per rep, invisible to fuel. Also measured: trait dispatch is
   FREE (+0.06% of encode fuel — the mapset-perf dispatch work already
   holds), which closed phase 4 before it opened. Order called: 1 → 2
   → 3, phase 4 menu'd with a named re-open trigger.
2. **The writer's chunk buffer.** (phase 1, `6d9c21b` — rut-side only,
   zero engine change.) One change lands in `rut/json`'s writer: a
   bounded ~1 KB chunk buffer composes per-event output and drains
   into `out` in ONE field append per chunk (`fail`/`finish` drain
   first — error offsets and the finished document byte-identical).
   **Row exec 796.4 → 215.5 ms (−72.9%)**, heap peak 10,674,377 →
   4,550,508 B (−57.4%), **rut/qjs 23.1× → 8.6×** — already past the
   survey's honest projection for phases 1-2 combined. Fuel +1.49%
   (the cap checks and buffer traffic pricing the killed memcpy),
   disclosed. Two attempts REVERTED with their numbers (§3): the LUT
   and the octet charset scan — both re-baseline phase 2.
3. **The general surface.** (phase 2, `e880345` — the engine phase.)
   The survey's §5 gap list becomes std surface, and json rewires onto
   it in pkg sources only: `str.code_at(i)`, `str.scan(from, set)`,
   `str.starts_with(from, head)`, the `StrBuf` builder (§4), and the
   codepoint split re-rides — the reader's `[?u32]` column is GONE,
   the classify loops ride host-side `scan` end-to-end. **Row exec
   216.9 → 110.3 ms (−49.1%)**, fuel 61,840,477 → 31,543,783 (−49.0%),
   heap → 3,423,642 B (−24.8%); the checksum and all 29 rows × 3
   runtimes bit-identical; **VERSION 9 → 10 disclosed**. Nothing from
   this phase was reverted.
4. **The width question, answered NO.** (phase 3, `806b259` —
   record-only, the disclosed deviation.) The survey's §6 brief said
   measure the SIMD width factor, don't assume. Measured: **the row's
   scans have no width to widen** — the census (exact, deterministic
   across sessions) shows `StrScan` at 446,409 calls/iter with 67.8%
   scanning 0 bytes, 22.0% scanning 4-7, and **ZERO calls ≥ 8 bytes**;
   the general shape-specialized SWAR form needs per-call set
   compilation priced at 62-78 ns against the ENTIRE current call at
   1.2-1.4 ns (~50× worse), and still loses at forced 64-byte scans.
   The one defensible trim (fused cell unpacking, single borrow,
   length-specialized loops) measured **−0.91% — inside the ±1% gate**
   — and was REVERTED per the method, probe re-verifying the standing
   record after. The phase's deliverable is the record
   (`benches/README.md`'s phase-3 section): wasm-simd128 moot, the
   SWAR re-open crossover measured and named.
5. **The close-out.** (phase 4, this commit — docs only.) This
   report; the perf log's consolidated close-out section (one coherent
   before/after table + the no-mover attestations); the RFC
   touchpoints for the surface phase 2 added (§5); the verdict and the
   menu (§6, §7). Gates re-run as paranoia (§8).

## 2. The scoreboard

The probe chain (in-process exec, fresh-VM iters, interleaved A/B,
medians of round medians; fuel/heap are bit-exact pure counts) is the
stable record; the JS nets drift ±10%+ between sessions, so the
rut/qjs ratio is quoted against the baseline session's qjs net
(40.3 ms) and annotated with each phase's same-day cross-runtime run.

| landed state | row exec (probe) | fuel (bit-exact)   | VM heap peak (bit-exact) | rut/qjs                  |
|---|---|---|---|---|
| baseline (the rut-json pin)  | 796.4 ms¹ | 60,933,262 | 10,674,377 B | **23.1×** (nets 930.9 vs 40.3) |
| phase 1 (`6d9c21b`) the chunk buffer | 215.5 ms (−72.9%) | 61,840,477 (+1.49%, disclosed) | 4,550,508 B (−57.4%) | 8.6× (nets 326.5 vs 37.9) |
| phase 2 (`e880345`) the general surface | **110.3 ms (−49.1%)** | **31,543,783 (−49.0%)** | **3,423,642 B (−24.8%)** | ~5.5× same-day (nets 205.0 vs 37.2) |
| phase 3 (`806b259`) the honest NO | 110.3 ms (attempt −0.91% inside the gate → reverted) | 31,543,783 (bit-flat) | 3,423,642 B (bit-flat) | unchanged |
| **close-out total**          | **−86.1%**       | **−48.2%**         | **−67.9%**               | **~2.7×**²               |

¹ The pin's own session measured the same binary at 797.2 ms probe exec
/ 930.9 ms net; the probe exec and the net differ by process-vs-probe
accounting (the CLI's compile/mount) plus day drift — the survey §1
control note.
² 110.3 ms probe exec against the baseline session's qjs net 40.3 ms.
The honest same-day comparison at phase 2's HEAD was 5.5× (205.0 vs
37.2 net, where rut's net pays the CLI's compile/mount the probe
excludes); the JS nets drift ±10%+ between sessions and the host
itself drifts ±8-13% between days. The probe chain above is the
record; both framings are disclosed so nobody overclaims either
direction.

**The honest residual: decode at the interpreter floor.** What remains
of the row is decode + fold + the CLI's compile/mount. Decode's
classify and the split ride the host now; its residual fuel is the
carve (digit folds, `build_f64`, view carving), the mints (records,
Vecs, `?T` boxes — the DIRECT-decode law's own values), the sticky-
error checks, and ~60 k host calls at one per token boundary. That is
the interpreter's dispatch + RC-heap floor for minting a typed tree —
the same floor the map rows live at. The JS twins ride their engines'
native `JSON.parse`/`JSON.stringify`; closing further would take the
host learning what json is, which the law forbids.

## 3. The method's record — what landed, what was reverted, and the lesson

The batch's second deliverable, every phase: the REVERTED attempts are
recorded with their numbers because they are the evidence the landed
design argues from.

### Landed

- **The chunk buffer** (phase 1) — 796.4 → 215.5 ms, −72.9%. Lesson:
  a rut-side shape fix, invisible to fuel, can own 73% of a row's
  wall — the survey's fuel-only ledger would have missed it entirely;
  exec and fuel must both be interrogated (§2.2's 51 ns/op encode vs
  3 ns/op decode was the tell).
- **The general surface + the split** (phase 2) — 216.9 → 110.3 ms,
  −49.1%, fuel halved. Lesson: when a stage is call-frame-bound, the
  WHOLE loop must leave the interpreter — the primitive that lets it
  (`scan`) had to be shaped by the census (one call per token
  boundary, the class table caller-owned), not by API taste.

### Measured and REVERTED (each with its number and its lesson)

- **The 256-entry LUT** (phase 1) — predicted −1.5-3 M fuel; measured
  +1.65 M (+11% of decode fuel) AND +5.9% decode exec. Lesson: the
  classify stage was call-frame-bound, not compare-bound — rut's
  compares are nearly free while array loads are not. The phase-2
  primitive had to move the whole loop host-side; a LUT was polishing
  the wrong term. (The classification semantics the LUT preserved are
  now unit-pinned over all 256 codepoints — the pin any future
  classification change keeps.)
- **The octet-view charset scan in `quote`** (phase 1) — exact
  semantics, measured +84 ops/call and +1.96 M fuel with no exec win:
  rut's `bytes` indexing is fuel-expensive per read. Lesson: on
  TODAY's surface the charset-scan lever was measured EMPTY — the fix
  is a surface primitive, not a rut-side spelling. Recorded as
  phase 2's binding handoff.
- **Comma-folding** (phase 1 exploration) — ditched during design:
  folding the separator into the previous event saves one concat of
  ~1 octet while complicating the drain paths; the chunk buffer
  deletes the event cost it was chasing. Lesson recorded, nothing
  measured to the row.
- **SWAR / explicit SIMD** (phase 3) — the census: max scan 7 bytes,
  zero ≥ 8 B; shape-specialized SWAR needs per-call set compilation,
  62-78 ns vs the whole call's 1.2-1.4 ns (~50×); forced 64-byte scans
  still lose (67-77 vs 14 ns); the only win is 4 KB runs (−45%) no
  tokenizer ever performs — and there it is bandwidth-capped, not
  width-capped. Lesson: **width is a property of the data** — measure
  the data before widening the kernel. The loop was already ~2
  cycles/byte scalar; the "don't trust auto-vectorization" suspicion
  was checked and the loop needs no rescue.
- **The preamble/loop trim** (phase 3) — the one defensible candidate,
  semantics byte-identical, fuel/heap bit-exact: −0.91% exec, INSIDE
  the ±1% gate, round ranges overlapping, the attempt owning the worst
  round — the noise signature. REVERTED per the method; probe
  re-verified the standing record after. Lesson: inside-noise → revert
  is what keeps every other number in this report credible.

## 4. The surface as shipped

The engine never learns json exists. What landed is machinery any
tokenizer, scanner, or encoder wants (declared in `rut/core/core.d.rut`;
documented in RFC 0028's prelude section and RFC 0004 §4):

| surface | signature | who may use it |
|---|---|---|
| indexed codepoint read | `str.code_at(i: i32) -> u32` — the codepoint at codepoint index `i` (traps out of bounds — the index is a bug, not data; lowers to the existing `StrCharAt`, rides existing encodings) | any per-character walker: encoders, normalizers, lexers, error formatters |
| fused scan/classify | `str.scan(from: i32, set: [u8]) -> i64` — walks codepoints from `from`, classifies each through the CALLER's `[u8]` table (`set[min(cp, set.len()-1)]`, `0` keeps scanning), stops at the first nonzero class, returns the packed pair `(stop << 8) | class`; end of input is `(s.len() << 8) | 0`. The whole per-byte loop runs host-side — one rut call per token boundary, O(calls) not O(bytes) of interpreted ops. New `StrScan` native | any tokenizer: json's reader, the LSP's tokenizer, the digest parser, uri scanners — bring your own class table, keep your own semantics |
| prefix test at an offset | `str.starts_with(from: i32, head: str) -> bool` — host-side compare (no per-character str cells), false past the end. New `StrStartsWith` native | keyword/literal matching (`lit()`'s "true"/"false"/"null"), any multi-char find built on it |
| growable builder | `builtin class StrBuf` — `StrBuf(cap)` (octet capacity hint, pre-sizing), `push(s)` / `push_code(c)` (invalid scalars mint U+FFFD — the `from_code` rule) mutate the builder's own block in place (geometric growth, amortized O(1)), O(1) tracked `len()`, `finish()` the ONE materialization (fresh immutable str; the builder keeps its buffer). Five new natives (`StrBufNew`/`Push`/`PushCode`/`Len`/`Finish`), a `CellData::StrBuf` payload — own() deep-copies, assignment shares, the class law | any accumulator: encoder writers, unescape paths, template renderers, string-heavy folds — the shape the field-append accumulator could never be |

The VERSION call: **9 → 10, disclosed** (the surface ADDS encodings —
the seven new `Nat` tags, the `StrBuf` boot type, the `NativeTy::StrBuf`
surface row; a v9 engine must refuse v10 artifacts rather than misread
them — the 7→8 declared-surface precedent). `code_at` itself rides
existing encodings; the call is per-surface. The LSP lane per the
lsp-align law: rut-lsp-wasm rebuilt at HEAD, vsix re-issued SAME
VERSION 0.2.2.

Tests: the engine's `scan_builder_surface` suite (11 pins — code_at
over ASCII/astral text, the packed scan result, the `set[min(cp,
len-1)]` table rule, a scan-vs-table parity sweep over all 256
codepoints, starts_with boundaries, byte-exact builder accumulation
across 40k appends, pre-sizing, push_code's U+FFFD rule, the class
aliasing law, a scan+builder round trip); the pkg's 34-test gate green
(semantics identical — the checksum proves it); workspace 761/0.

## 5. The RFC touchpoints

Phase 2 moved the core surface, and the docs that carry it now carry
it: RFC 0028's core-prelude section names the tokenizer members and
`StrBuf` beside the older `str`/`bytes` contracts; RFC 0028's json
section describes the reader/writer as they now ARE (direct cursor
over `src`, classify on `scan`, the writer on `StrBuf` — the old
`[?u32]`-column and rc==1-field-accumulator sentences were stale);
RFC 0004 §4's codepoint-access list carries `code_at` beside `code()`
and `from_code`. RFC 0007 needed nothing — it documents literals and
formatting, not the member surface, and the append fast path it
describes is untouched. Each amendment cross-links this report; no
landed decision is re-litigated anywhere.

## 6. The verdict

**The law held.** json stayed rut — every phase's diff is readable
without knowing what json is: a writer buffer (phase 1), general str
primitives + a builder (phase 2), host scan internals measured and
left alone (phase 3). The host gained general machinery only, and the
engine never learned json exists at any point in the batch. Every
checksum was immovable throughout — `json-roundtrip`
`1960875332163557684` on rut/qjs/node in every phase, `expected.json`
untouched from the first commit to the last, the json-decode canary
`4502015958359127277` bit-identical throughout, and every OTHER row of
the suite bit-identical at every phase that touched the engine (the
no-mover proofs).

Four phases, four kinds of deliverable: two phases shipped code (the
surface and the split; the writer's buffer inside them), one shipped
the analysis that ordered the work and priced the levers (phase 0 —
including the zero-gap fuel reconciliation and the trait-dispatch
free-pass that closed phase 4 before it opened), one shipped the
analysis that stopped a mistake (phase 3 — the honest NO that kept
SWAR/SIMD out of the tree), and one shipped a −72.9% buffer fix that
no engine change could have bought. **23.1× → ~2.7×**, fuel −48%, VM
heap −68%, output byte-identical, and the row's residual is honestly
named: decode at the interpreter floor, which is where the law says
it should be.

## 7. The menu going forward

- **Phase 4 (compiler-general): data-closed.** The phase-0 numbers
  closed it — trait dispatch measured free (+0.06% of encode fuel),
  the frame-bound term that mattered (classify's method-call share)
  left the interpreter with `scan`, and the loop-ILP target is VM-
  internal dispatch no json phase needs. The re-open trigger stands
  restated: a post-phase-2 profile showing >10% of the row's fuel in
  rut-side call frames re-opens trait-impl/method inlining with that
  number. Nothing in phases 1-3 measured such a share — the remaining
  fuel is carve/mint/fold work (§2), not frames.
- **wasm-simd128: moot.** Phase 3's census killed the premise (no scan
  reaches even one 8-byte block). The re-open condition is measured
  and named: a future workload showing ≥ 64-byte scans brings the
  shape-specialized SWAR back, with its per-call compilation
  threshold and the 4 KB crossover from the phase-3 micro table.
- **Encode-side int-codes: if ever.** The encode side is builder
  pushes + one clean-path scan per string now; phase 0's writer micros
  price number/bool leaves at ~1.0 ms/pass — there is no measured term
  asking for integer-coded tokens, and the surface that would carry
  them (a bytes-typed writer) is a general-surface question for a
  batch with a row that needs it.
- **JsonValue DOM: still rejected** (the law, unchanged — DIRECT
  decode's mint-exactly-your-values shape is now also the measured
  winner).
- **Banked for whoever re-opens any of this:** the 256-codepoint
  classification pins (`classify_report` + the reader tests); the
  scan/code_at/builder call census (the phase-3 table — the op stream
  is pinned, so the native stream is deterministic); the pre-sizing
  lever now expressible (`StrBuf(cap)`, previously inexpressible — the
  phase-1 residual); the survey's harness inventory (scratch under
  `/tmp/opencode/batch-json-perf/p0..p3/`, reproducible against the
  mounted pkgs); and the standing host-drift notes (±8-13% between
  days — same-day interleaved A/B is the only clean delta).

## 8. The gates, as landed per phase

| phase | commit | workspace tests | wasm32 | bench suite |
|---|---|---|---|---|
| 0 | `39c3326` | docs-only, untouched (green at base) | n/a | n/a (the survey changes no row; the pin re-verified in-session) |
| 1 | `6d9c21b` | green (pkg tests + the new classification/chunk pins) | exit 0 | full suite green — checksum immovable `1960875332163557684` on rut/qjs/node, `expected.json` untouched; json-decode canary bit-identical; every other row's probe fuel/heap bit-identical; LSP wasm + vsix re-issued |
| 2 | `e880345` | 761 passed / 0 failed (+11 engine pins) | exit 0 | full suite green — 29 workloads × {rut, qjs, node} all equal to `expected.json`, every row's probe fuel/heap equal to the phase-1 record; VERSION 10; LSP wasm + vsix re-issued (md5s in the perf log) |
| 3 | `806b259` | 761 passed / 0 failed (nothing landed, re-proven) | exit 0 | full suite green — every row bit-identical to its standing record; probe re-verified after the revert; LSP lane untouched (state md5s re-verified unchanged) |
| 4 | this commit | docs-only; re-verified — 761 passed / 0 failed | re-verified, exit 0 | re-verified (paranoia — docs cannot move pins): full suite exit 0, json-roundtrip `1960875332163557684` and the json-decode canary exactly as pinned |

Phase 4 changed no code. The report documents landed behavior; the RFC
amendments record the surface as shipped; nothing here moves a pin.
Scratch under `/tmp/opencode/batch-json-perf/p4/`.

---

*Companions: [`docs/json-perf-survey.md`](json-perf-survey.md) (the
phase-0 contract — the zero-gap ledger, the census, the gap list), the
performance log in [`benches/README.md`](../benches/README.md) (the
per-phase entries + the consolidated close-out table),
`rut/core/core.d.rut` (the surface as declared),
`crates/rut-driver/tests/scan_builder_surface.rs` (the surface's
executable record).*
