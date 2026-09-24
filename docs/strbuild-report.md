# strbuild — the batch report

The record of the strbuild batch: four phases, one shared tree, base
`5985617`, all on `master`. Companion to `docs/strbuild-survey.md`
(phase 0 — the census, the member contract, the PRE-REGISTERED bench
design; its §-numbers are the ones the landed pkg and the bench row
cite) and `docs/strbuild-phase1.md` (the phase-1 landing record,
including the one place reality answered differently than the survey
predicted). The law lives in the RFC 0028 strbuild amendment (§
"`strbuild` — the builder package"); the user-facing summary lives in
[`examples/README.md`](../examples/README.md) (the std-pkg section
after json's); the bench record lives in `benches/README.md`'s two
strbuild performance-log sections. This report is the record.

## 1. The story, in one section

The directive named a class — `StringBuilder` with `append` and
`build() -> str` — over the engine's `StrBuf` cell, and asked for the
standard arc: census the cell, land the pkg face, migrate json's
writer onto it, give the pkg its own bench row, and run the
optimization pass against the survey's ranked candidates. The batch
landed in four commits:

1. **The census** (phase 0, `05271ce` — the survey). The engine
   `StrBuf` at VERSION 11 exactly as json-perf phase 2 landed it: the
   five natives (`StrBufNew`/`Push`/`PushCode`/`Len`/`Finish`), boot
   type id 17 / kind tag 15 / nat ids 16–20, the AMBIENT surface
   (`mount_std_core`, `core.d.rut:73` — no `use`, probed live), and
   the block store's factor-2 geometric growth class-rounded and
   MEASURED at 68 grows across ~30M builder ops (growth is O(log n)
   — not a hot path). json's writer call-site census: 3 mints / 31
   pushes / 2 push_codes / 1 len / 3 finishes. The class-law
   share/copy shape probed and pinned as what must not regress
   (alias appends visible through the original; the built `str`
   immune; `build` twice answers the same text). The member contract
   named from real need ONLY — six members — with
   `clear`/`reserve`/`capacity`/`append_char`/`append_i64` ABSENT ON
   RECORD (no nat, no call site; adding one would be a VERSION
   conversation for zero need). The directive's class-sketch syntax
   corrected by probe (methods live in `impl` blocks, RFC 0012;
   `mut self` receiver). The bench design PRE-REGISTERED (§5): the
   two-phase workload, the `Array#join` twin, the candidates RANKED
   with predicted yields, the house 5×7 paired ±1% method. The calls:
   VERSION NO BUMP (11 stays — zero new encoded vocabulary); the LSP
   surface 9 → 10 with wasm + vsix re-issue owed by phase 1.

2. **The pkg** (phase 1, `ed5aa29`). The 10th std pkg
   (`rut/strbuild/`): `pub class StringBuilder { out: StrBuf }` + the
   six members EXACTLY as censused; deps NONE (the cell is ambient);
   `inline = true` LOAD-BEARING (a class-method module cannot be
   linked); the absent-on-record list held. json's writer migrated
   onto it per the survey §4 mapping (the field, 3 mints →
   `with_cap`, 31 `push` → `append`, 2 `push_code` → `append_code`,
   3 `finish` → `build`; `rut/json/rut.toml` gains the REQUIRED
   `[deps] strbuild`). The wiring: CLI presence list 10th after json,
   the probe's matching mount row, the RFC 0028 amendment, the LSP
   `std_surface` 9 → 10 with the wasm rebuilt (md5 `b61f653e…`) and
   the vsix re-issued 0.2.3 (md5 `ecf522b7…`), 8 unit pins. AND the
   survey's ZERO-ops-data prediction answered **NO**, honestly — §2
   below.

3. **The row** (phase 2a, `9bf45a8`). The pre-registered two-phase
   workload lands (`benches/workloads/strbuild.rut` + the `.js` twin
   + the `expected.json` pin, `-1259380140398818172` on rut/qjs/node
   first-run agreement) — and the scoreboard headline: **rut AHEAD of
   qjs on the row, 0.42× net** — the first string-churn row where the
   interpreter beats the JS idiom. §3 below.

4. **The optimization pass** (phase 2b, `9fc5ccc`). Every survey
   candidate measured one by one against the row's 5×7 baseline, per
   the house method. **NONE LANDED** — the full reverted-with-numbers
   ledger is §4, and the honest argument for why that is the method
   WORKING is §5.

5. **The record** (phase 3, this commit). The batch report (this
   document), the RFC 0028 verification + the two missing links
   amended (§6), the user-facing README section (§7), and the perf-log
   verification (§8). Docs only; the engine byte-identical to the
   landing.

## 2. A prediction falsified — with root causes, not excuses

The survey predicted the json migration would be fuel **bit-identical**
("same CallNat stream, same nat ids — fuel bit-identical or the
migration is wrong"), with `inline = true` specializing the `with_cap`
mints away at the consumer. The survey wired its own disclosure
clause ("inside the ±1% gate or honestly disclosed") before any run —
which is exactly why the miss is a result and not an embarrassment.

**Measured: fuel 31,543,783 → 31,975,807 = +432,024 (+1.370%), exec
+3-4% paired** — outside the gate, disclosed with the numbers, and
root-caused to THREE mechanisms, each IR-verified via `rut dump` plus
a 300k-append scratch A/B (4,500,017 → 5,100,023 fuel = **+2.000
ops/append, exactly**):

1. **The instance-cell hop** — `out: StringBuilder` inserts ONE
   record cell between the writer and the engine cell (writer →
   instance → `StrBuf`, three cells, not the survey's two): +1 `getf`
   per member call.
2. **The inlined frame's arg re-bind** — P1.3 inlines `append`'s
   body, but the native `CallNat` ABI re-binds the forwarded
   argument: +1 `movref` per argument-carrying call.
3. **Static mints do not ride P1.3** — `StringBuilder.with_cap(c)` is
   a static call: a real `call` + callee frame (`callnat StrBufNew`,
   `makerecord`, `ret`), +3 ops per mint. The survey's
   "inline=true specializes it away" is FALSE for static calls.

The mechanism's proof is the exact zero elsewhere: **json-decode fuel
111,322,915 → 111,322,915, BIT-IDENTICAL** — its doc has no escaped
strings, so the builder member-call path never runs; the delta lives
exactly where the mechanisms say it lives.

What makes the falsification honest rather than merely survived:

- the immovables held through it — checksum `1960875332163557684`
  exact (the fold rides VALUES), json-decode `4502015958359127277`
  exact, `expected.json` untouched, heap pin 3,423,642 → 3,423,706 B
  (+64 B — the instance cells at peak);
- the historical pins (`docs/json-perf-report.md`,
  `benches/README.md`'s 31,543,783) were NOT rewritten — they are
  that batch's record; the new era's pin is 31,975,807 / 3,423,706 B;
- the cost was then carried FORWARD as the pass's first target
  (candidate 0) instead of being filed and forgotten.

## 3. The row — the scoreboard headline

The strbuild row is the pkg's reason to exist, in both costs the
directive names (`benches/README.md`'s landing section has the full
entry; the survey §5 pre-registered the design before any run):

- **Phase A — churn**: ONE `with_cap(64)` builder, n = 2²⁰ LCG-drawn
  fragments (the knucleotide constants, seed 42) of lengths
  {1, 3, 5, 7, 9} straddling the 8-byte class rounding; ~16
  class-rounded geometric grows per round ride INSIDE the
  measurement.
- **Phase B — build**: ONE `build()` on the ~5.2 MB doc (the
  amortized pattern, json's writer shape) plus m = 2¹⁶ per-fragment
  `with_cap(hint)` → append → `build()` mints (the `quote_slow`
  shape), so the materialization cost is visible per call.

The checksum folds round-tripped COUNTS + 32 grid-sampled
content-exact fragments re-sliced from the BUILT doc (the
json-roundtrip values-not-lexemes precedent); the `.js` twin's idiom
choice (`Array#push` + one `Array#join("")` per round) is DISCLOSED
on its header. All three runtimes agreed on
`-1259380140398818172` first-run.

**The 5×7 baseline** (the pass argues against this, not against
vibes): probe fuel 162,285,165 and VM-heap peak 22,028,108 B
bit-identical in all 35 iters; exec session medians 344.1–348.8 ms
(med-of-med 345.3, spread +1.4%). Cross nets (same-day full suite):

| runtime | net | ratio |
|---|---|---|
| **rut** | **352.6 ms** | **0.42× qjs — AHEAD** |
| qjs | 842.5 ms | 2.39× rut |
| node | 233.1 ms | the JIT holds the lead |

**Rut ahead of QuickJS on a string-churn row, for the first time on
this suite.** The why is structural, not luck: the JS idiom for
accumulating 1M tiny strings is `Array#push` + `Array#join("")`, and
`join` re-materializes the whole rope per round — while rut's builder
appends in place through the engine cell and materializes ONCE per
round. The pkg face's per-append wrapper tax (§2) is paid inside a
lane rut still wins; rut's residual gap to node is the interpreter
floor on the 4.3M-iteration append loops, not the builder.

`expected.json` gained ONLY the appended strbuild row; every
pre-existing row untouched; json-roundtrip `1960875332163557684`
immovable with fuel 31,975,807 / heap 3,423,706 B bit-identical — the
row is a NEW mount and moves nothing.

## 4. The optimization pass — the reverted-with-numbers ledger

Every survey §5.4 candidate, measured one by one against the 5×7
baseline per the house method (interleaved A/B, fresh-VM probe iters,
order alternating, medians of round medians; fuel/heap verified
bit-exact at every step). The full ledger lives in
`benches/README.md`'s optimization-pass section; the one-table shape:

| # | candidate | predicted | measured | verdict |
|---|---|---|---|---|
| 0 | the +432,024 wrapper cost (NEW — phase 1's disclosure) | reclaim it | reproduced bit-exact; rut-side restructure priced **+10.000 ops/escaped string WORSE** (33,600,581 → 34,600,581 per 100k; exec −7.9% — a worse op stream traded for time on paths the rows never execute) | **NOT reclaimable rut-side — the mechanisms are engine-lowering, MENU** (§6) |
| 1 | per-append double type-check | 5–15% | fold built clean, fuel/heap bit-identical; exec −0.95% (5×7), −1.18% (7×9) — then the REBUILD TEST: same source, fresh build, **+4.35%** | **REVERTED** — the delta tracks the BUILD, not the source; true yield ≲1% |
| 2 | build()'s double copy → single-copy mint | 1–3% | mechanically real (a whole `Vec` cycle dies); build #1 +1.30% vs build #2 −1.88% — a 3.2-point same-source spread; json −0.97% | **REVERTED** — at/below the placement floor; re-opens with a build-ensemble method |
| 3 | pkg-level mint churn (reuse) | several % | needs `clear` between strings | **BLOCKED BY LAW** — `clear` is ABSENT ON RECORD (no nat, no call site; new std surface out of scope) |
| 4 | growth factor 2× → 1.5× | INSIDE NOISE → revert | exec inside the placement band (row +2.75%, json −2.20%) — but the DETERMINISTIC heap column: row −6.58% while json-roundtrip **+2.16% UP** (more, smaller grows → more old+new block coexistencies) | **REVERTED** — a heap mover in BOTH directions; the power-of-2 factor's alignment with the block-store class list and malloc size classes recorded as load-bearing |
| 5 | share/copy is the law | zero by design | verified per candidate: no fold optimized a copy in; the RFC 0044 shape stayed pinned green in the 8 `strbuild_pkg` unit tests | **HELD** |

Reconciliation (the batch's law): json-roundtrip
`1960875332163557684` immovable at every probe run of every candidate
and at the gates; fuel movers NONE — the pin stays **31,975,807 /
3,423,706 B** at every step (host-side candidates are fuel-blind by
construction; the pkg-side restructure never landed). **Phase 1's
+432,024 is NOT reclaimed, and the goal is answered honestly: it is
not reclaimable in this batch's scope** — the ops live in the
engine's lowering, and the mechanisms are the menu (§6).

## 5. Why NONE-survived is the method WORKING

A pass that lands nothing looks like a pass that did nothing. This
one is the house method doing exactly what it is built for, four
ways:

- **The pre-registered death died on schedule.** Candidate 4 was
  predicted INSIDE-NOISE → revert before any run existed; it died
  exactly there — and the measurement still bought something new: the
  deterministic heap column overturned the "free slack win" reading
  and proved the growth factor is a whole-suite re-pin bill, not a
  knob.
- **Two candidates were killed by a test, not a hunch.** Candidate 1
  read as a win on two powered runs (−0.95%, −1.18%) and died on the
  rebuild test (+4.35% same source, fresh build). Without the
  reverted-with-numbers ledger, a future batch would have inherited
  "the append fold wins 5-15%" as folklore. Instead it inherits: the
  true yield is ≲1%, with the receipts.
- **The non-reclaimability of the +432,024 is a scoping ANSWER, not a
  shrug.** Candidate 0 priced the only admissible rut-side shape
  (+10 ops/escaped string WORSE, under its own where-honest charge)
  and named the engine mechanisms that would flip the answer — so the
  next engine batch starts at the menu, not at a fresh census.
- **The batch changed nothing it cannot defend.** The engine left
  byte-identical: every row's fuel/VM-heap pin bit-exact to the
  landing record, checksums immovable on all three runtimes at every
  step. Zero regressions were shipped in pursuit of wins that would
  not resolve.

And the pass produced a METHOD deliverable bigger than any candidate:
the row is **build-placement sensitive** (±2-5% between same-source
builds — 22 MB of heap churn, 64+ grow events, 262k transient mints
per run). Single-binary A/Bs cannot resolve ±1-3% effects on it. Any
future sub-3% work here owes a build ensemble (N independent builds
per side, med-of-builds) or a same-binary toggle; candidates 1 and 2
are the standing test cases.

## 6. The menu — what a future batch picks up

1. **The three engine-lowering mechanisms** (each IR-verified, none
   engine-hopped):
   - a receiver-cell forwarding lowering (deletes the instance-cell
     `getf` hop);
   - a CallNat-ABI re-bind elision for P1.3-inlined frames (deletes
     the `movref`);
   - static class-fn mint eligibility in the P1.3 splicer (deletes
     the +3 per `with_cap`).
   Landing any takes the +432,024 back — the append pair alone is
   ~432,006 ops/iter on json-roundtrip — and would retrospectively
   make the rejected quote restructure a pure win (its op-stream
   regression inverts if the append tax dies). The strbuild row now
   prices the same tax on its own turf: **+2 ops/append over ~4.3M
   appends + 3/mint over 262k mints** — the menu item is measurable
   the day an engine phase picks it up.
2. **Build-placement sensitivity** — the method finding of §5: sub-3%
   work on this row needs a build ensemble or a same-binary toggle.
   This is a benches-method deliverable, not a code change.
3. **`clear` stays blocked by law** — absent a call site. The reuse
   pattern (one scratch builder per writer, reused across strings) is
   the shape that would want it, and it is priced (candidate 3 +
   candidate 0's restructure): blocked until a real call site exists,
   because adding std surface is a VERSION conversation and the need
   is currently zero. The row's phase B stays the pre-registered mint
   workload — it is the row's purpose, not a defect.

## 7. The RFC verification + the README section (phase 3)

**RFC 0028** was amended in phase 1 and re-verified complete this
phase: the header bullet (strbuild joins the swappable set as the
tenth package), the swappable-list row (`pouch`, `calc`, `ink`,
`nmapset`, `json`, `strbuild`), and the §-level section (§
"`strbuild` — the builder package": the class over the ambient cell,
the six-member surface, the `inline = true` law, the RFC 0044
share/copy law, the closed member contract, mount order 10th, the
VERSION-11 statement, json as first consumer). Two cross-links were
MISSING and are amended by this commit — nothing else touched:

- the json section still described the writer as accumulating through
  the raw `StrBuf` builder with a `finish()` materialization — the
  pre-migration spelling; amended to name the std `strbuild` pkg's
  `StringBuilder` (with `append`/`build()`), the ambient cell under
  it, and the phase-1 disclosure pointer;
- the core-prelude paragraph's `StrBuf` entry did not point at its
  std face; amended with the one-clause pointer to the builder-pkg
  section.

**The user-facing README** (`examples/README.md`, following the std
json package's section as the precedent): a new section documents the
class contract (the six members + the absent-on-record list), the mut
law (RFC 0044 sharing underneath — alias and `mut` param appends land
in the caller's document; copies at exactly two engineered points),
json as the reference consumer (with the honest +432,024 disclosure),
the row's scoreboard (rut 0.42× qjs), and the run recipe.

**The perf log** (`benches/README.md`): verified consolidated — the
row's landing and the optimization pass are TWO sections (not one
blurred entry), the candidate ledger carries every number of §4
exactly once, and both cross-reference the survey's pre-registration
and the phase-1 pin. Phase 3 adds nothing to it; this report
cross-references instead of duplicating.

## 8. Gates

Docs-only commit (the staged set is `.md` files only; the parallel
deploy lane's `demo/package.json` mod + untracked `scripts/` and
`.wrangler/` untouched, not staged). Re-run on this exact tree:

- `cargo test --workspace`: **779 passed / 0 failed**.
- `cargo check --workspace --target wasm32-unknown-unknown`: exit 0.
- Bench pins, paranoia pass: `strbuild` checksum
  `-1259380140398818172` and `json-roundtrip` `1960875332163557684`
  equal to `expected.json` on rut/qjs/node (the full suite ran green
  at the landing and at the pass's final tree; docs cannot move pins,
  and the two-row re-run is the receipt).
- Tree clean after the commit; single commit, pushed to
  `origin/master`.

Scratch for phase 3: `/tmp/opencode/batch-strbuild/p3/` (the earlier
phases' A/B receipts, IR dumps, and interleaved JSONL records remain
under `p0/`–`p2/`).
