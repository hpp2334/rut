# json-perf survey — the breakdown (phase 0)

Batch: **json-perf** (closing the json gap WITHOUT host-part impls).
Phase 0 delivers the measurement base every later phase argues from:
the per-stage decomposition of the `json-roundtrip` row, the call-site
census in `rut/json`, the std surface gap list, and the phase-order
decision with predicted yields. Docs only — the measurement harnesses
are scratch (under `/tmp/opencode/batch-json-perf/p0/`, not committed,
not pinned as rows; §8 records the durability argument for landing
none).

THE LAW (binding, from the plan): json stays rut; the host gains
GENERAL machinery only (scan/classify primitives, builder growth, SIMD
str ops — anything any tokenizer wants). Rejected on record and still
rejected: host-part json impls, engine-woven builtin traits, runtime
reflection, JsonValue DOM for decode. Checksums are immutable across
every phase; fuel/heap movers are disclosed re-pins with the old values
verbatim; every OTHER bench row stays bit-identical.

## 1. The method, restated for every future phase

- House bench discipline throughout: **5×7 paired** (5 interleaved
  matched sessions × 7 reps for cross-runtime net medians), probe
  **fresh-VM iters** for in-process exec, **±1% parity** as the noise
  gate. A change that lands inside noise is REVERTED, the analysis
  kept.
- **Checksums immutable**: `json-roundtrip` must print
  `1960875332163557684` in every phase, on every runtime.
- **Fuel re-pins disclosed verbatim**: fuel is a pure op count
  (RFC 0040) — bit-identical unless the op stream changed; any mover
  is disclosed with its old value in the commit body.
- Every other bench row bit-identical: the general-surface phases
  (2-3) prove no-mover on the WHOLE suite, plus the wasm32/LSP gate
  when the std surface moves.
- This phase's own discipline: fuel claims are EXACT (deterministic,
  bit-reproduced across runs); exec claims are medians of 5 interleaved
  rounds × fresh-VM probe iters, taken in one session (this host drifts
  ±8-13% between days, ±2-4% across minutes — the crossing-nop and
  mapset-perf logs' standing notes).

Control of record, re-measured this session against the pin:
checksum `1960875332163557684` ✓, fuel `60,933,262` ✓ (bit-identical),
heap peak `10,674,377` B ✓, exec median 797.2 ms (the recorded net
930.9 ms and this 797 ms probe exec differ by process-vs-probe
accounting plus day drift; the pin's own session measured the same
binary at 882-930 ms across runs).

## 2. The per-stage breakdown

Harnesses (scratch, `p0/`): each shares the row's generator VERBATIM
(the knucleotide LCG, seed 42, ported byte-for-byte from
`benches/workloads/json-roundtrip/main.rut`) and the row's dep graph
(json + pouch + ink), so fuel differencing attributes stages exactly.
The decode-interior harnesses (`split`, `nav`, `navcarve`) run a
shape-faithful clone of `JsonReader` (`Nav`): json.rut's classify loops
copied verbatim (peek/skip_ws/lit/digit_run/str_scan/more/next_key),
with the carve (digit folds, `build_f64`, `src.slice` views) and the
mint (records, Vecs, `?T` boxes) progressively added back.

### 2.1 The fuel ledger — reconciles to the pin EXACTLY

Fuel is deterministic; every number below bit-reproduced across runs.
Differencing chain: `gen` → `dec` → `decenc` → `full` (control), and
inside decode: `split` → `nav` → `navcarve` → `dec`.

| stage                              | fuel, 3 reps + gen | per rep (÷3)  | % of row |
|------------------------------------|-------------------:|--------------:|---------:|
| gen (LCG + 1,200 f-string rows + join) |        526,785 |             — |    0.86% |
| **decode** (`decodeJson<Vec<DocRow>>`) |     45,102,972 |    15,034,324 |  **74.02%** |
| — codepoint split (`JsonReader.of`)    |      9,215,124 |     3,071,708 |  15.13% |
| — classify (structural scans)          |     27,370,761 |     9,123,587 |  44.92% |
| — carve (digit folds, `build_f64`, views) |    5,040,459 |     1,680,153 |   8.27% |
| — mint + dispatch + plumbing (residual)|      3,476,628 |     1,158,876 |   5.71% |
| **encode** (`encodeJson` + writer)     |     13,109,727 |     4,369,909 |  21.51% |
| **fold + rep control** (`rows_sum` + asserts) | 2,193,778 |   731,259 |   3.60% |
| **TOTAL**                              | **60,933,262** |               | 100.00% |

The total is the pin, bit-exact — the decomposition closes with no
unexplained gap. The naive stage list (classify / carve / mint / write)
was indeed missing stages; they are found and named:

1. **The codepoint split** — `JsonReader.of` materializes the whole doc
   into a `[?u32]` column before the first token: **15.1% of the row**
   (3.07 M fuel/rep, and the column's alloc + drop ride every rep).
2. **The fold** — `rows_sum` re-walks the decoded rows and formats
   3,600 f64s through `f"{v}"`: 3.6%.
3. **The generator** — paid once: 0.86%.
4. Inside decode, the residual that is neither classify nor carve:
   record/Vec/`?T` mints, the `when (k)` key dispatch, and the ~20
   `r.failed()` sticky-error checks per row — 5.7% of the row.

The decode-interior clone fidelity: `split + classify + carve` rebuilds
92.3% of the real decode's fuel; the 7.7% residual is the mint/dispatch
plumbing the clone deliberately lacks (plus clone-shaping drift — the
clone is shape-faithful, not op-identical; disclosed).

### 2.2 The exec ledger — the writer owns the wall clock

Medians of 5 interleaved rounds × fresh-VM probe iters, one session
(spread across rounds ≤ ±1.5% on every harness). Exec is where fuel
and time DISAGREE, and the disagreement is the headline:

| stage                     | exec               | per rep        | % of row exec |
|---------------------------|--------------------|---------------:|--------------:|
| gen                       | 2.3 ms once        |              — |           ~0% |
| decode                    | (137.2−2.3)/3      | **45.0 ms**    |         17.0% |
| — split                   | (27.9−2.3)/3       |   8.5 ms       |          3.2% |
| — classify                | (90.1−27.9)/3      |  20.8 ms       |          7.9% |
| — carve                   | (101.3−90.1)/3     |   3.7 ms       |          1.4% |
| — mint + dispatch (resid.)| 45.0−29.3−3.7      |  12.0 ms       |          4.5% |
| **encode**                | (805.4−137.2)/3    | **222.7 ms**   |   **~84%**    |
| fold + control            | below differencing resolution | ≤3 ms |         ~1%  |
| **whole row (control)**   | **797.2 ms**       |  265 ms/rep    |          100% |

Fuel says decode 74% / encode 22%; time says encode **~84%** / decode
~17% (the stage execs sum to the row within the ±1% cross-harness
wobble). Per-op cost: decode ≈ 3.0 ns/op (pure interpreter dispatch),
encode ≈ 51 ns/op (host string work per op). The row's TIME is a
string-machinery problem that fuel cannot see; §4 nails the mechanism.

(Per-op sanity: the residual mint/dispatch term runs ~10 ns/op —
allocation-shaped ops (record mints, Vec pushes, `?T` boxes) cost host
heap work per op, vs the scanner's 2-3 ns/op dispatch floor.)

### 2.3 VM heap peaks (probe, bytes)

| harness | heap peak | note |
|---|---:|---|
| full (control) | 10,674,377 | = the pin, bit-identical |
| gen | 773,900 | doc + parts vec |
| dec | 3,780,786 | doc + cs column + rows |
| decenc | 9,716,861 | + the 204 KB out accumulator + fragments |
| split / nav / navcarve | 2,822,853 / 2,821,933 / 2,822,025 | doc + cs column |

(The control's heap matched the pin only once the harness's panic/
assert literals were spelled exactly as the row's — literals
materialize per frame; a 34-byte heap delta traced to the differing
message strings. Fuel was identical throughout.)

## 3. The call-site census (rut/json sources)

Doc of record: 204,778 codepoints, 1,200 rows × 3 reps. Codepoint
census (one classify pass over the doc, the `stats` harness): digits
49,411 (24.1%), letters 72,600 (35.5%), quotes 36,000 (17.6%),
structural `[ ] { } , :` 40,801 (19.9%), `-` 4,766 (2.3%), `.` 1,200
(0.6%), whitespace **0** (the generated doc has none — every skip_ws
call is a one-peek miss), other 0. Tokens: 18,000 strings (10 keys + 5
string values per row), 12,000 digit runs (id, score int + frac, 6
vals, rev).

**The classify loop's shape.** Decode-side classify indexes the `[?u32]`
column directly (`peek()` = bounds check + load + `as i32`) and
branches on int ranges:

- `skip_ws`: 4 equality compares per call — but ws = 0 in this doc, so
  it is one failed compare per token boundary (~60 k calls/rep).
- `digit_run`: 2 compares/byte (`< 0x30 || > 0x39`) — 12 k runs, ~43 k
  digits/rep.
- `read_str`'s scan: 3 compares/byte (`"`, `< 0x20`, `\`) — 18 k
  strings, ~40 k content bytes/rep.
- `more()`: 1-4 compares per element boundary — ~26.4 k calls/rep.
- `lit()` ("true"/"false"/"null"): per-index compares where EACH index
  pays `lit.slice(i, i+1).code()` — a str-cell mint per character
  compared (2,400 calls, ~9,600 per-index reads/rep).

Content classify is ~2.2 compares/byte — cheap. The stage's 44.6
ops/codepoint (9.12 M / 204,778) is dominated by reader-METHOD call
frames (peek/skip_ws/more/next_key: ~250 k calls/rep at ~10-15 ops of
frame+body each) plus the per-byte loads and `pos` traffic. The
classify stage is as much call-frame-bound as compare-bound.

**The escape/number walk shapes.** Number walks are two-pass per
lexeme: a classify pass (`signed_head` + `digit_run` + lead-zero +
`.`/`e` peek) then a re-read fold pass over the same digits from the
column (`read_i64`'s checked fold; `build_f64`'s 18-digit fold +
leading-zero scan + `pow10` loop). String walks are single-pass over
the column, then `src.slice` carves the payload as an O(1) view (RFC
0042) — no escapes occur in this doc, so `unescape` (per-char
`raw.slice` + f-string accumulator) never runs.

**The ENCODE-side scans are a different shape — the asymmetry that
matters.** Where the reader indexes its column, the writer's
`quote()` re-scans the string with `s.slice(i, i + 1).code()` — **one
str-cell mint per character** on the clean path of every string and
every key (`lit()` shares the disease). Measured from the writer
micros: ~87 ops per string leaf (scan + wrap + append) vs ~15 for an
i64 leaf.

**Writer append granularity.** Per field/token the writer issues
separate `out = f"{out}{t}"` accumulator events: `key()` = quote mint +
one concat (`{out}{quoted}:`) + a comma concat; `write_i64`/`write_f64`
= format mint + one concat; `write_str` = quote mint + one concat;
brackets and `value_comma` commas = one concat each. Per row ≈ 60-64
concat events → **~62 k events/rep** (plus ~12 k quote mints). The
writer micro floor (all leaves constant-raw) measures this machinery
alone: 3.68 M fuel/pass (84% of encode fuel) and ~133.6 ms/pass (the
majority of encode time).

**Allocation/retain churn points.** Decode: the `[?u32]` column mint +
drop per rep (204,778 slots); one `Vec` alloc per `vals`/`tags` +
7,200/3,600 element pushes (primitive elements box per the RFC 0044
`?T`-slot law — MakeOpt + retain per push); 1,200 `DocRow` + 1,200
`DocMeta` mints; the `?T` boxes for every field local. Encode: ~12 k
quote mints + ~62 k accumulator events (each a fresh str block under
the field shape — see §4).

**Trait dispatch — measured free.** `wmic-trait` vs `wmic-local`
(identical traversal, real `encodeJson` trait path vs plain fns):
+2,412 ops/pass = **+0.06% of encode fuel**; exec delta within the ±1%
gate. The boxless static trait dispatch + impl inlining already landed
(mapset-perf) holds here. Dispatch is NOT a json cost.

## 4. The writer's mechanism — the finding that orders the phases

An isolated A/B (the `append3a`/`append3b` scratch harnesses) on the
two accumulator shapes, 100 k identical f-string appends each:

| accumulator shape                    | exec          | per event |
|--------------------------------------|--------------:|----------:|
| `out = f"{out}{t}"` — plain LOCAL    | 2.2 ms        | **~25 ns** |
| `self.out = f"{self.out}{t}"` — class FIELD through `mut self` (the `JsonWriter` shape) | 1,893 ms | **~19 µs** |

The field shape is **~750× slower per append**. The local's rc==1
in-place append path fires; through the field it does not — every
event allocates a fresh block and copies the accumulator. At doc scale
the writer micro floor confirms the cost model directly: ~133.6 ms per
pass over ~62 k events ≈ **2.2 µs/event ≈ one ~100 KB average memcpy**
— i.e. encode pays O(events × accumulator size) copying: ~6 GB of
memcpy per rep, invisible to fuel (each event costs 1-8 ops).

Control: a bare local accumulator growing to 1.2 MB over 200 k appends
runs 9.5 ms total — appends are cheap when the in-place path fires;
size alone is not the cost. The shape is.

Encode's 222.7 ms/rep decomposes (writer micros, per pass):
~133.6 ms event machinery (the field-append copies), ~12.1 ms string
leaves (`quote`'s slice-per-char scan), ~1.0 ms number/bool leaves
(i64/f64 formatting is cheap — including the shortest-round-trip f64
print at 1,200 scores/pass), trait dispatch ~0.

## 5. The std surface gap list

Today's `str` surface (core.d.rut) is exactly: `len`, `code` (FIRST
codepoint), `encode` (→ bytes), `slice` (O(1) view). `bytes`: `len`,
`decode`, `clone`. Indexed reads, scans, finds, builders: none. The
gaps, each with the call sites it serves and the expected yield:

1. **Indexed codepoint read** — `code_at(i)`-style member (or byte
   access on a UTF-8 view). Serves: `JsonWriter.quote`/`quote_slow`
   (~12 k mints + ~90 k slice-per-char reads/rep), `lit()` (2,400
   calls), `unescape`/`hex4_at` (escaped docs), `DecodeJsonError`
   formatting. Yield: string-leaf cost 87 → ~25-35 ops (−60-70% of the
   0.52 M string-leaf fuel; −~10 ms/rep of quote time). General: every
   per-char encoder/normalizer wants this.
2. **Class-scan primitive (index + class for the next byte in a set)**
   — host-side loop (SIMD-width in phase 3). Serves: `read_str`'s
   quote/control/backslash scan, `digit_run`, `skip_ws`, and — via the
   same primitive — the codepoint split itself. Yield: classify
   9.12 M fuel → ~1.5-2.5 M (the VM per-byte loop and its call frames
   leave the interpreter); classify time 20.8 → ~4-6 ms/rep. THE
   general tokenizer primitive — any scanner (json, the LSP's
   tokenizer, the digest parser) collapses onto it.
3. **Multi-char find / starts-with** — `find(needle) -> ?i32`,
   `starts_with`. Serves: `lit()` ("true"/"false"/"null"), broken-literal
   diagnostics; minor here (~70 k ops/rep), standard tokenizer fare.
4. **Growable builder** — a str builder with amortized capacity
   (push slice / push code, `finish` → str). Serves: `JsonWriter.out`
   (the §4 mechanism — 62 k O(n) copies → amortized O(n) total),
   `unescape`'s accumulator, `quote_slow`. Yield: encode time
   222.7 → single-digit ms/rep; row exec roughly HALVED by this alone.
   The plan's own "builder growth" item; the measured justification.
5. **The codepoint split's surface** (phase 2's "split re-rides"):
   today decode materializes a `[?u32]` column per rep (3.07 M fuel =
   15.1% of the row, 8.5 ms). A byte-offset cursor over the str (ASCII
   fast path, validation only where content is carved) removes the
   materialization. General: any parser over str pays this split
   today.

## 6. The decision — phase order 2 → 3 → 4 (1 fixed first)

**Phase 1 (rut-side only) stays first as fixed**: LUT classification, 
charset-scan call sites where TODAY'S surface allows, writer batching.

Predicted yields (fuel is the firm claim; exec carries the day-drift
caveat):

- **Phase 1**: fuel 60.93 M → ~55-58 M (−5-10%): the LUT trims the
  per-byte compare chains (~1.5-3 M) and batching collapses writer
  events 62 k → ~16-20 k/rep (−0.3-0.5 M fuel) — but the classify call
  frames and the field-append copies REMAIN (batching shrinks the
  event count, the O(n) copy per event stays). Exec ~797 → ~450-550 ms
  (encode 222.7 → ~60-90 ms as the copies scale with event count).
  Honest note: with NO find/scan primitive on today's surface, the
  "charset-scan call sites" lever is nearly empty — phase 1's real
  content is LUT + batching, and its job is also to re-profile for 2.
- **Phase 2 — general scan/classify surface + the codepoint split +
  the builder — FIRST of 2-4, biggest predicted yield**, because it is
  the only phase that touches BOTH dominant terms: classify (44.9% of
  fuel — the scan primitive moves it host-side) and the writer's
  append machinery (the measured time sink — the builder removes the
  O(n) copy per event). Predicted: fuel → ~38-45 M (−25-35% vs
  baseline; classify −5-7 M, split −2.5 M, encode concat events →
  builder pushes −1 M+); exec → ~250-350 ms (writer 222.7 → <10 ms,
  classify 20.8 → 4-6 ms, split 8.5 → ~1 ms). Costs honored: VERSION
  call per surface precedent; LSP wasm + vsix re-issued; the no-mover
  proof runs on the WHOLE suite.
- **Phase 3 — SIMD str ops — SECOND**, immediately after 2: SIMD
  accelerates host scan kernels, so it needs phase 2's host-side scans
  to exist; before 2 it has nothing to speed up. Predicted: fuel FLAT
  (fuel cannot see host SIMD — the gate must be time-based, disclosed
  as such); exec −10-20% of the remaining scan time (tens of ms);
  the honest host-drift note applies.
- **Phase 4 — compiler-general — MENU'D, closed by this data, with the
  re-open trigger named.** The numbers that close it: trait dispatch
  on encode = +0.06% fuel (§3 — already free); the decode residual
  (dispatch + mint + plumbing) = 5.7% of row fuel; the only
  meaningful frame-bound target is the classify stage's method-call
  share — and that dies in phase 2 anyway (per-byte VM loops leave the
  interpreter with the scan primitive). Loop ILP is VM-internal
  dispatch work no json phase needs. RE-OPEN TRIGGER: if the
  post-phase-2 profile still shows >10% of row fuel in rut-side call
  frames, trait-impl/method inlining comes off the menu with that
  number.

Combined honest projection: 23.1× qjs → roughly 8-10× after 1-2 (qjs
net 40.3 ms vs rut ~250-350 ms net), with decode's residual the
interpreter floor. The scoreboard restated at close-out.

## 7. What phase 0 deliberately does NOT do

No optimizations (phase 1+), no engine/std changes, no bench rows
added or re-pinned, `expected.json` untouched, gates untouched. The
decomposition harnesses stay scratch: they are probes of TODAY's op
stream — each phase's re-pin invalidates their absolutes, and a
durable stage row (e.g. a writer-only or decode-only row) would need
the full row treatment (JS twin, `expected.json` pin, README entry)
proposed on its own merits. The durability argument is recorded here;
nothing is landed.

## 8. Scratch inventory (`/tmp/opencode/batch-json-perf/p0/`)

| harness | isolates |
|---|---|
| `full` | the row, control (checksum + fuel + heap = pin) |
| `gen` | the generator |
| `dec` / `decenc` | decode / decode+encode (differencing) |
| `split` | `JsonReader.of`'s codepoint split |
| `nav` | the classify loops (carve + mint cut) |
| `navcarve` | + the carve (folds, `build_f64`, views) |
| `stats` | the per-codepoint/token census |
| `wmic-trait/local/nostr/nonum/floor` | the writer: trait vs fn traversal, string leaves, number leaves, event-machinery floor |
| `append3a` / `append3b` | the accumulator A/B: local vs class-field |
| `append`, `append2` | append size-dependence controls (none found) |
| `gen_harnesses.py`, `time.mjs` | the generator + the interleaved timing driver |

All run against the repo's own `rut/json`, `rut/pouch`, `rut/ink` via
absolute dep paths; the tree was not modified to run them.
