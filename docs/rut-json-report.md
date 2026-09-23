# rut-json — the batch report

The record of the rut-json batch: four phases, one shared tree, base
`23f433f`, all on `master`. Companion to `docs/rut-json-survey.md`
(phase 0 — the census, the locked rulings embedded verbatim, the
design; its §-numbers are the ones the landed code, tests, and
diagnostics cite). The law lives in the RFC 0028 amendment (the ninth
pkg + the dependency-direction decision record); the user-facing
surface summary lives in `examples/README.md`; the bench record lives
in `benches/README.md`'s performance log. This report is the record.

## 1. The story, in one section

rut had no JSON story in its std tree — only `examples/02-digest`'s
private hand-rolled DOM codec (~375 lines, depth 64, per-char str
dispatch) and a pinned bench row measuring its churn. The batch asked
what it costs to land json as the ninth std pkg under this repo's
laws — no reflection (RFC 0037 was on the table and priced), no wire
change, the error/nullable/union laws held — and landed it in four
commits:

1. **What does the tree actually do today, and what should json be?**
   (phase 0, `0063b61` — the survey.) The census: the 8-pkg std
   surface, the three mount chains, the two migration targets, and
   what the checker already answers — plus the two gaps it confirmed
   (no `TyKind::Opt` impl-target arm; no type-parameter static
   receiver). The design: DIRECT schema-driven decode (no DOM — the
   11.8× census gap was ~24% per-char dispatch + ~64% mint machinery,
   and a DOM doubles both walks); the int-codes reader (the
   pre-measured −36% menu item); the accumulator writer (`string_join`
   rejected on the +3.6% measurement); the number/depth policies; the
   final impl matrix; the migration census; the pre-registered bench
   method. The locked rulings went in verbatim (§0) as the phases'
   contract.
2. **The pkg lands.** (phase 1, `bd03877`.) `rut/json/` — the three
   `(?T, ?E)` entries, the two traits (`Self`, no `<T>` param), the
   int-codes `JsonReader` + accumulator `JsonWriter`, the error
   kinds/structs, the number and depth policies, the base impl matrix
   (prims, `?T`, `[T]`), the manifest verbatim from RFC 0045 §2 with
   the two peer groups, the two sanctioned checker arms, the mount
   slots after pouch/nmapset everywhere (`assemble_peers` included),
   the LSP ninth, and the 29-test gate. VERSION stayed 8.

   **The retry story, on the record:** a provider outage killed
   attempt 1 mid-phase. The half-work was adopted and finished rather
   than redone; finishing it surfaced and fixed 4 defects — the
   load-bearing one is the `inline = true` link law: json's entries
   are GENERIC FUNCTIONS, a linked json answers `unknown function
   decodeJson` in every consumer (the light world failed on it live;
   the full world had worked only by accident — the pouch group drags
   `Vec<T>` into json's unit, flipping the splice heuristic).
   Source-inlining composes json into each consumer, where the
   entries specialize. The other disclosed fix: the `jsonlight`
   fixture listing the peers as dev-deps was a silent self-defeat
   (dev-deps mount into the closure, so the gate appended the groups
   and the light claim was vacuous) — it deps json alone now.
3. **The row + the migration.** (phase 2, `33f597e`.) The
   `json-roundtrip` bench row — the survey's pre-registered method
   executed: json-decode's generator ported VERBATIM (same LCG, seed
   42), `decodeJson<Vec<DocRow>>` DIRECT into typed rows, `encodeJson`,
   the value fold; the JS twin does the same job with the engine's own
   `JSON.parse`/`JSON.stringify`. Pin `1960875332163557684`, all three
   runtimes agree: rut net 930.9 ms vs qjs 40.3 / node 43.0 — 23.1× /
   21.7×, the honest weakest lane. Fuel 60,933,262, heap 10,674,377 B,
   bit-identical across all 5 paired sessions. And digest's encode
   half migrated onto the lib (`impl JsonSerialize for Json` replaces
   the private `jencode`/`jquote`; the example's own golden tests —
   serde_json cross-checks — prove the output byte-identical, 9/9;
   two disclosed relaxations: raw C0 controls re-encode as `\u00xx`,
   and a deeper-than-128 tree answers the writer's recoverable `Depth`
   err where the old encoder recursed unboundedly). The decode half
   stays private — DIRECT has no DOM to decode INTO; it waits for the
   `JsonValue` menu item. The old `json-decode` row is untouched: its
   pins re-measured bit-identical (checksum `4502015958359127277`,
   fuel 111,322,915, heap 34,377,027 B) — the canary did not move.
4. **The record.** (phase 3, this commit — docs only.) The RFC 0028
   amendment (json joins the swappable set; the dependency-direction
   decision record), the `examples/README.md` section (the surface,
   the rulings, the fusion interplay, the scoreboard, the run
   recipe), and this report. The `benches/README.md` perf-log entry
   for the row landed with the row itself in phase 2 (landing
   numbers, the 5×7 method, the canary-not-moved note) and is
   cross-referenced here, not duplicated.

## 2. The impl matrix, as landed

The survey's §2.6 matrix vs what ships. The base and the two
impl-only groups (`group-pouch.rut`, `group-nmapset.rut`) are one
module — the groups see json's private fields exactly like the base.

| type | lives in | encode | decode |
|---|---|---|---|
| `i64` | base | decimal (i64::MIN included) | exact per §2.4 — overflow/float-lexeme = `WrongType`, never a silent wrap |
| `f64` | base | shortest round-trip; non-finite → `NotFinite` | two-tier: tier 1 IEEE-exact, tier 2 ±1 ulp disclosed |
| `bool` | base | `true`/`false` | literal only (`1`/`0` = `WrongType`) |
| `str` | base | RFC 8259 escape set | any JSON string; lone-surrogate escapes rejected |
| `?T` | base (`impl JsonSerialize for ?T`) | `nil` → `null`, else T | `null` → `nil`, else T |
| `[T]` | base | array | array, elements via T |
| `Vec<T>` | group-pouch (peer: pouch, optional) | array | array → `push` |
| `HashMap<K,V>` / `HashSet<T>` | group-nmapset (peer: nmapset, optional) | **DEFERRED — see below** | object / array |
| `PrimMapI64/U64/F64<K>` | group-nmapset | **DEFERRED — see below** | object |

**Map ENCODE is DEFERRED, on the record — decode is complete.** A map
can be encoded only by WALKING it, and nmapset ships no iteration
surface (`rut/nmapset/nmapset.rut`: its table is an opaque host
object; the class comment says so itself). Growing a pinned sibling
std pkg's declared surface (new `nmap_host` host fns) is outside the
batch and would ripple RFC 0025's exactness pins. Consequently
`KeyUnsupported` is DECLARED but DORMANT: its only sanctioned fire
site is inside the deferred map-encode rows. The decode-side bytes-key
law answers `WrongType` (with the spelling disclosure in `expected`).
Both facts are the menu's first item, not silent gaps — every landed
group impl is decode-only and says so in-source.

The sanctioned checker arms (the survey's §1.4 gaps, closed in phase
1, fixture-gated so they cannot rot): the `TyKind::Opt` impl-target
arm with its dispatch half (the exact-nullable receiver binding
before the auto-deref — a `?T` receiver calls the `?T` impl), and the
type-parameter static receiver (`T.decode(r)` inside `decodeJson`'s
own body) with `Self` resolving at any structural depth.

## 3. The design decisions, with their measurements

- **int-codes reader: ADOPTED (−36%).** The reader splits the doc's
  codepoints once into `[?u32]` and classifies with two int compares
  where the digest parser paid up to ten one-char str compares per
  digit. The measurement predates the pkg — the strings-round1 census
  measured the exact transformation on the identical op stream:
  499.1 ms vs the baseline row's 779.8 ms, **−36.0%**, fuel −33.3%
  (the per-char str dispatch it deletes was ~189 ms ≈ 24% of the row,
  twice the build term). Tokens carve as one O(1) `StrView` slice
  each (RFC 0042); the reader carries the source `str` beside the
  codepoint column (the 8-byte word per codepoint buys the dispatch,
  the views buy the payloads).
- **string_join writer: REJECTED (+3.6%).** The alternative
  accumulator — `Vec<str>` parts joined at `finish` — was measured on
  token-shaped build streams in the same census: **+3.6% SLOWER**
  (808.0 vs 779.8 ms). The rc==1 in-place append path is already the
  optimal build spelling, so the writer appends through `mut self`
  methods and the hot path does zero per-char work until a string
  needs escaping.
- **DIRECT decode, JsonValue deferred.** The DOM-first alternative
  doubles the walk and re-mints the tree; the census priced the
  existing DOM row's cost as ~64% mint machinery. DIRECT mints
  exactly the program's own values, once. The cost is borne honestly:
  schema-less consumers (digest) keep private parsers until
  `JsonValue` lands (menu).
- **The number policy.** i64 exact or `WrongType` naming the lexeme;
  f64 two-tier (≤18-digit mantissa × 10^k, |e| ≤ 22 → split-multiply
  single-rounding, IEEE-exact; beyond → best-effort ±1 ulp,
  disclosed); encode rides `f"{v}"`'s shortest-round-trip — rut's own
  f-string IS the minimal-valid-JSON rendering. Round-trip law
  pinned as a test.
- **Depth 128, both directions, recoverable.** Encode-side `Depth` is
  the RFC 0017 story: the rc heap leaks strong cycles by law, so
  encoding a cyclic structure is EXPECTED failure — data, not trap.
- **The strict-UTF-8 bytes entry.** A ~15-state DFA over `b[i]`
  octets first; never the lossy `bytes.decode()` on unvalidated
  input. Valid bytes re-enter the SAME str cursor — one reader, one
  codepath, two entries.
- **The error plumbing.** Internal steps return the small stuff;
  the entry assembles the struct once at the top — `got`/`expected`
  f-strings are failure-path-only, the happy path never formats.
- **Peers + dev (RFC 0045's first real consumer).** The container
  impls ride optional peer groups; the dev-deps pairing guarantees
  them in json's own test runs; consumers mount json light and the
  D2 diagnostic names the peer if a group impl is referenced without
  it. The end-to-end gate fixture: the SAME Vec-consuming source
  diagnoses in the light world and compiles clean in the full one.
- **`inline = true`.** Not about the groups — about the fn shape:
  generic entries cannot cross a module link boundary (the outage's
  defect 1, above). The base alone splices identically.
- **VERSION 8 stays.** Additive pkg: zero builtin decls, zero
  engine-woven names (the traits are ordinary nominal traits — the
  serde-model ruling), zero new cell kinds, zero host fns. Pkg
  additions never bumped VERSION.

## 4. Honest limits

- **Map encode is deferred** (§2) — the matrix's only open half, and
  `KeyUnsupported` is dormant until its fire sites land.
- **f64 decode tier 2 is ±1 ulp best-effort** — the only place json's
  output can differ from an engine-grade parser, disclosed in-source
  and in the survey.
- **f64 object KEYS normalize spelling** (`1.0` encodes `1`, decodes
  back value-equal) — injective on values, not on lexemes.
- **digest's decode half stays private** and the example mounts json
  LIGHT (base only — also the prudent side of an engine gap, §5):
  DIRECT has no DOM to decode into until `JsonValue`.
- **The checker arms are arms, not solutions.** `impl JsonSerialize
  for ?T` and `T.decode(r)` work; the neighboring gaps stand — a
  trait call ON a `?T`-typed receiver compiles but nil-derefs at
  runtime (json's own `?T` impls nil-check internally; workload-side
  spellings do the same), and `?bool == nil` still diagnoses
  (nmapset's disclosed limit).
- **The LSP sees json's base only** — the groups declare no public
  names and the LSP never parses manifests; it is advisory, the
  compiler is the law.
- **The old json-decode row stays** — a pinned engine-churn canary,
  deliberately not migrated; retiring it is a menu decision.
- **json is the suite's weakest lane vs qjs (23.1×)** — recorded as
  the baseline, with the why (every classify/carve is interpreter
  dispatch vs the engine's own JSON paths); no engine phase was in
  this batch.

## 5. The menu going forward

- **The nmapset iteration surface** — unlocks map encode; wakes the
  dormant `KeyUnsupported`; retires the matrix's only DEFERRED half.
  Requires new `nmap_host` host fns (a declared-surface event for
  that pkg, RFC 0025's exactness pins re-checked).
- **The two engine gaps phase 2 found:**
  - a unit that mounts the pouch peer group AND instantiates
    `Vec<opaque>` fails load-time verify — the group's
    `impl JsonSerialize for Vec<T>` at T = opaque devirtualizes its
    element `x.encode(w)` into the `?T` row's UNCOMPILED concrete
    twin (`JsonSerialize#$8$encode`, a 0-op `is_method=false` stub)
    → "CallM to a non-method". Reproduced minimal; digest mounts json
    light partly because of it;
  - a trait method call ON a `?T`-typed receiver compiles but
    nil-derefs at runtime — the sanctioned workaround (json's own
    spelling) is the explicit nil-check + deref.
  Both are engine/lir-side; neither is a json defect.
- **`?bool == nil`** — the checker limit nmapset disclosed
  (nmapset.rut:373): `?bool` unboxes at value reads, so nil compares
  diagnose "operands must have equal width"; a bool val map and the
  `?bool` slots that could ride a plain presence compare both wait on
  it (json's `?bool` fields ride a user-side flag today).
- **`JsonValue` + digest's decode half** — the DOM, menu-gated since
  the survey: lands as an ordinary json type (+ a `parseValue` entry)
  when a schema-less consumer needs it; digest's private parser
  retires the same day.
- **SIMD str ops** — the reader's classify/carve loops are the
  natural first customer (whitespace skips, digit runs, the escape
  scan); an engine-side lever, measured against the `json-roundtrip`
  baseline.
- **int-codes for the encode side, if ever needed** — the writer's
  hot path is already append-only with zero per-char work on the
  happy path; a codepoint column would only pay if an escape-heavy
  workload ever shows one in the profile. Not needed today.
- Carried from the survey's menu, unchanged: a core `f64.parse`
  builtin (the strict tier-2 fix — a declared-surface VERSION event,
  not a pkg patch); the remaining prim widths (i8..u32, f32) and
  `[?T]`/enum impls (pure matrix additions); bytes views for the
  reader's byte-entry fast path; pretty-print options; a base64 key
  policy; the json-decode row's retirement once the opaque-fastpath
  comparisons it anchors are history.

## 6. The gates, as landed per phase

| phase | commit | workspace tests | wasm32 | bench suite |
|---|---|---|---|---|
| 0 | `0063b61` | docs-only, untouched (green at base) | n/a | n/a |
| 1 | `bd03877` | 733 passed / 0 failed (incl. the 29-test json gate) | exit 0 | green — all 26 checksum rows equal, all 11 fuel/heap pins bit-identical, expected.json untouched |
| 2 | `33f597e` | green (91 suites ok, incl. digest's 9-test gate) | exit 0 | green — 29 workloads × {rut, qjs, node} all equal; the json-decode canary bit-identical; no fuel/heap movers |
| 3 | this commit | docs-only; re-verified — 733 passed / 0 failed | re-verified, exit 0 | re-verified (paranoia — docs cannot move pins): 29 workloads × {rut, qjs, node}, 0 mismatches; json-roundtrip `1960875332163557684` and the json-decode canary `4502015958359127277` exactly as pinned |

Phase 3 changed no code. The RFC amendment records landed behavior
(the ninth pkg, the dependency direction); the README section
documents it; nothing here moves a pin. Scratch under
`/tmp/opencode/batch-rut-json/p3/`.
