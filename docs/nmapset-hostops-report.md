# nmapset-hostops — the batch report

The record of the nmapset-hostops batch: three phases plus this
record, one shared tree, base `f9016f5` (post todolist-nonnull), all
on `master`. Companion to `docs/nmapset-hostops-survey.md` (phase 0 —
the census, the probes, the micro-calls; its §-numbers are the ones
the phases cite). The law lives in the RFC 0023 hostops amendment
(the stable-handle law); the movers live in `benches/README.md`'s
hostops section; the user-facing summary lives in
`examples/README.md` (the keyed-collections section).

---

## 1. The story, in one section

The directive: *"for nmapset, I recommend we move the impl to hostm
rut part is just a wrapper at all"* — locked at planning into three
calls: **(A) stable val handles** (the full takeover; the drain dies),
**(B) legacy crossings kept** as escape hatches, **(C) remove
`PrimMap*`**.

The survey (`a445096`, Sep 25) resolved the ground truth the plan
assumed and priced the removal at zero: live-code `PrimMap` consumers
numbered ZERO (the only matches were the class bodies and the two
NEGATIVE pins asserting their absence — both of which deletion keeps
true), the `nmap-primmap` twin had already lost the column lane to the
type-name-law repeal (it ran the sidecar's i64-val variant — pins
20,903,285 / 3,539,324 B at its last receipt), and the driver suites
`nmap_primmap.rs` / `nmap_valcolumn.rs` never spelled the classes —
no fixture dies. The probes (scratch, `/tmp/opencode/batch-nmapset-
hostops/p0/`) verified the two new rut-side spells with the host
simulated — the packed `(handle << 1) | newly` answer and the
append-only sidecar triple with the pouch-push doubling inlined — all
rows exact (`PACK 0 1 1000000000 true` / `PRIM 14950 15650 100` /
`REF 140 50` / `OK 30740`).

Phase 1 (`2f5ea84`) landed the host half additively: the handle column
(one stable birth index per key, assigned in `insert_at`, relocated
inside `grow`'s existing re-slot walk, monotonic v1) + the fused
family — 18 crossings `map_h{put,find,remove}_{i,u,b,s,y,sv}`, `hput`
growing internally at the load boundary and answering the packed i64,
`hfind`/`hremove` answering the handle or `-1` — plus 8 unit tests
pinning the packed-answer law, handle stability across multi-grow
sweeps, the internal-grow boundary (the legacy sentinel's exact load
law), the dead-handle answer, sv≡s identity, legacy-lane agreement,
and the kind-mixing trap. Two test bugs were corrected on receipt
(the sv/s law is the HANDLE equality, not the full answer — a fresh
birth and its replace differ in bit 0 by design; bool and bytes need
separate tables — kinds are homogeneous per table). Additive-state
proof: the workspace green at exactly base+8, zero existing tests
edited.

Phase 2 (`54fceca`) re-seated the wrapper: `nmapset.rut` 596 → 310
lines, ZERO loops — `KeyLane`'s three ops re-mounted on the fused
lanes (the same 11 one-line impls), `HashMap`'s sidecar the
handle-indexed `vals/vlen/vcap` triple with `vstore` (fresh birth
appends with the doubling; a replace stores over the cell), every
method ONE crossing plus at most the sidecar store, the four range
methods on the sv twins, `HashSet` sharing `hput` at bit 0.
(ERRATA, Sep 2026 — the nmap-hostvals batch: "ZERO loops" was true of
the METHOD BODIES only; the same paragraph's `vstore` "fresh birth
appends with the doubling" WAS the surviving loop, and the claim
became true of the FILE only when this sidecar deleted with the
nmap-hostvals value migration — `docs/nmap-hostvals-report.md` §7.
History as written above.)
`PrimMap*` deleted; the twin retired (workload dir + `.js` +
ONE `expected.json` line + the examples/README recipe; the perf log
above stays as written — the not-rewritten precedent). Every driver
suite green UNCHANGED through the new wrapper — including the
aliasing-law pins, the range-method suite (`nmap_viewkeys`, 2572351),
and the LSP std-surface pins. The bench gate: all seven surviving
family rows × {rut, node, qjs} GREEN against `expected.json` — the
checksums are immovable by construction (the handle column is
additive state the legacy paths never read), and the four recorded
rows' base measurements cross-checked the type-name-law receipts
exactly.

Phase 3 (this commit) is the close-out: this report, the RFC 0023
amendment (the stable-handle law — handle column, fused family,
wrapper law, legacy list, checksum law), and the examples/README
keyed-collections amendments (the drain sentence, the
columns-stay-internal parenthetical, the Prim-prefixed paragraph —
the verify-and-amend pattern, true sentences untouched).

## 2. The movers (the honest table)

Base `f9016f5` vs landed, both trees measured same-session through
`rut-bench-probe` (5 fresh-VM iters, exact values; full table + the
anatomy in `benches/README.md`'s hostops section):

| workload | fuel | heap | exec |
|---|---|---|---|
| nmapset-int | 20,703,284 → 21,571,682 (+4.2%) | 1,966,551 → 1,966,663 B | 69.1 → 68.3 ms |
| nmapset-str | 9,551,761 → 9,910,968 (+3.8%) | 983,620 → 984,086 B | 53.6 → 51.7 ms |
| nmap-hashset | 13,267,176 → 12,716,782 (**−4.2%**) | 551 → 615 B | 43.6 → 44.0 ms |
| nmap-knucleotide | 38,814,389 → 37,419,557 (**−3.6%**) | 4,195,084 → 2,229,052 B (**−46.9%**) | 170.6 → 155.5 ms (**−8.9%**) |
| kmer-view | 37,614,177 → 35,019,405 (**−6.9%**) | 4,194,916 → 2,228,884 B (**−46.9%**) | 147.4 → 141.0 ms |
| strview | 10,801,744 → 11,094,312 (+2.7%) | 2,032,173 → 2,032,285 B | 38.8 → 39.0 ms |
| refvals | 56,899,541 → 55,633,341 (**−2.2%**) | 28,801,340 → 26,843,812 B (**−6.8%**) | 383.8 → 260.6 ms (**−32.1%**) |

The split is the takeover's anatomy, disclosed as measured: FUEL goes
BOTH ways — the put-heavy sidecar rows pay `vstore`'s append check +
`vlen` bump (+2.7–4.2%), the probe-and-grow rows save the sentinel
loop condition and the drain bookkeeping (−2.2–6.9%). HEAP is the
clean win: the view rows halve (birth-dense sidecar vs the cap-indexed
`[nil; cap]`), `refvals` −6.8%, the flat rows +112 B, `nmap-hashset`
+64 B (the handle column's Vec header in the box — the phase-1
accounting note, measured through the gate). EXEC's headline is
`refvals` −32.1% — the row built to stress the relocation drain no
longer pays it.

## 3. Survey claims corrected on receipt

- The survey's §5 estimate "~596 → ~250 lines" — landed at **310**
  (the header law rewrite kept more of the history than estimated).
- The survey's §5 retirement list included "the README live-table
  row" — there WAS no live-table row for `nmap-primmap` (its README
  presence was entirely historical perf-log sections, which stay as
  written); the retirement was one file smaller than mapped.
- The survey's §0.4 expectation "fuel/heap pins WILL move (fewer
  crossings…)" — the honest receipt splits the fuel story both ways
  (§2 above); the crossing COUNT per put did drop (one fused call vs
  entry + loop-check + val store), but the wrapper's one new sidecar
  mechanism prices above zero on put-heavy streams.
- The batch's own execution route: the user redirected mid-batch from
  the batch-plan-impl skill (invoked on inference, never explicitly
  asked) to direct in-session execution, and the skill was gated to
  explicit-by-name invocation first (commit `a91c677`). No repository
  source was touched by the aborted orchestration; the plan document
  stands unchanged.

## 4. The gates (on the exact trees, in order)

- Phase 0: workspace 782/0 (the base count exact), docs-only.
- Phase 1: workspace **790/0** (base + the 8 new unit tests, zero
  existing tests edited — the additive-state proof), wasm32 check
  exit 0, both `.d.rut` copies content-identical (49 decls = 31
  legacy + 18 fused).
- Phase 2: workspace 790/0 again, wasm32 exit 0, the bench gate —
  seven rows × {rut, node, qjs} green against `expected.json`
  (734932704 / 1264308351 / 21500055 / 2198604 + the kmer-view/
  strview parity pins + refvals 140052990000), `expected.json`
  edited by exactly the one disclosed removal.

## 5. The menu (recorded, not landed)

- **Handle recycling under churn** — the monotonic v1 law's cost is
  one nil sidecar slot per distinct key ever inserted; a host-side
  free-list (handing a removed handle to a later birth) is the
  measured follow-up if a churn-heavy workload ever prices it.
- **The bool val lane** — unchanged, deferred on the `?bool` nil-law
  checker gap (the round-3 repro stands; a bool val rides the sidecar
  today, which the takeover makes the only spelling).
- **Float keys** — absent by design (no stable equality contract).
- **A differently-named public column class** (`I64Map<K>` shapes) —
  the type-name-law menu item; the columns' crossings stay bound as
  legacy, so the shape remains one declaration away.
- **An iteration surface** — still absent; json's map ENCODE still
  waits on it.
