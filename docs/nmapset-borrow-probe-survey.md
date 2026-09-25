# nmapset-borrow-probe survey — phase 0

Batch: **nmapset-borrow-probe** (the per-crossing key copy dies): the
`nmap` table's `s`/`sv`/`y` lanes probe BORROWED — a borrowed `&str`/
`&[u8]` window goes in, `hashbrown::HashTable::find/entry(hash, eq)`
does one probe over the pinned hash word, and the owned `KeyVal` is
materialized ONLY at a fresh insert (where the store needs it). Phase 0
= the census, the scratch A/B/C, the design, the decision. Docs only —
zero code changes (the scratch test below is UNTRACKED, deleted after
capture; its receipts live in this doc).

THE GOAL (user directive): *"See how can we optimize nmapset more?"*
after the native-fastpath/hostvals batches — with one row still losing
to its qjs twin (`nmapset-str` 49.1 vs 39.6 ms net, same-session pre
run) and the module header itself naming the remaining price:
`crates/rut-std/src/nmap.rs:54-60` — *"std's `HashMap` has no borrowed
probe, so the s/sv lanes materialize one owned `KeyVal::Str` per
crossing — the copy moved from insert-only to per-crossing, the one
measurable price of the real-HashMap directive."*

---

## 1. The census — every per-probe materialization site

| site | lane | cost per op |
|---|---|---|
| `fused_put_s` (nmap.rs:648) / `fused_find_s` (:653) / `fused_remove_s` (:658) | `hput/hfind/hremove` s | `KeyVal::Str(k.to_owned())` |
| `fused_put_sv`/`find_sv`/`remove_sv` (nmap.rs:667-682) | the sv twins | `sv_range` → `fused_*_s` → the same copy |
| `decode_key` (nmap.rs:437, arms :449-450) | every any lane — `map_hvput/hvget/hvremove` (:929-960) and the `_sv` twins (:961-992) | `Str`: `to_owned()`; `Bytes`: `bytes_copy()` |
| `map_hput_y`/`hfind_y`/`hremove_y` (nmap.rs:795/:843/:891) | `y` lanes | `k.to_vec()` |
| `impl Hash for KeyVal` + `IdHasher` + `MapBuild` (nmap.rs:101-125) | the storage design | the machinery the probe replaces |

**Zero materialization** (control lanes): `i`/`u`/`b` — `KeyVal::Bits`
is `Copy`, and that is exactly why `nmapset-int` (i32) and
`nmap-hashset` (i32) already run copy-free and already beat their qjs
twins; `refvals` is `HashMap<i64, Pt>` — Bits keys, unaffected too.

**Affected rows** (str/sv-keyed): `nmapset-str`, `strview`,
`kmer-view`, `nmap-knucleotide` — and downstream, `rut/json`'s
`HashMap<str, …>` deserialization (`json-roundtrip`/`json-decode`).

**The op count** (measured, n = 50 000, the row's own 7-phase churn —
build, replace, hit-get, miss-get, remove-⅓, rescan, re-add-⅓):
**283,332 map ops ⇒ 283,332 materializations per run.** Every one of
them is a malloc + memcpy + free whose only purpose is to probe and
then be dropped (hit) or moved (fresh insert).

## 2. The design (landed in phase 1)

- `NativeTable.map: HashMap<KeyVal, Entry, MapBuild>` becomes
  `hashbrown::HashTable<(KeyVal, Entry)>` — same stored `KeyVal`, same
  `Entry { handle, val }` laws, no hasher trait anywhere: the LANE
  computes the hash word itself.
- **`KeyRef<'a> { Bits(u64), Str(&'a str), Bytes(&'a [u8]) }`** — the
  probe type. `KeyRef::hash()` and `hash_payload` funnel through ONE
  pair of helpers (`hash_bits`/`hash_bytes`), so the pinned FNV/mix64
  words are bit-identical to today (the pinned-literal unit tests stand
  unchanged).
- Op mapping, ALL single-probe: `find(hash, eq)` for `hfind`/`hvget`;
  `entry(hash, eq, hasher)` for `hput`/`hvput` (Occupied → same
  handle + in-place value replace, zero materialization; Vacant →
  `key.to_owned_key()` — the ONE remaining copy); `find_entry(hash,
  eq)` for `hremove`. The `hasher` closure (`|item| hash_payload(&item.0)`)
  runs only on grow — the load-factor/rehash behavior is the same
  SwissTable machinery std uses, so growth semantics do not move.
- The sv lanes become fully zero-copy on probes: `sv_range` (the
  house `Invalid` checks) → `KeyRef::Str(range)` → probe; the window
  copy survives only at a fresh range insert.
- `decode_key` returns `KeyRef` (tied to the `HostVal` borrow; the
  cell outlives the call under the register-retention contract the
  `&str` crossing already relies on). **The admission-before-retain
  law keeps its exact ordering**: `decode_key` (no retain) →
  `check_kind` → `decode_val` (the retain) → `hvput_ref`.
- Laws preserved by construction: no iteration surface exists
  (`put/get/has/remove/len` + range twins only), so bucket internals
  are unobservable; sv ≡ s identity (same hash, same `eq` → same
  entry/handle); handles are births, growth-invisible; fuel and VM
  heap cannot move (host-only change — the strongest cheap gate).

## 3. The decision — Option A

**A: `hashbrown = "0.14.5"` direct in `rut-std`** — `HashTable`'s
`find/entry/find_entry` take the hash and an `eq` closure, so the
probe borrows for free, one pass, no `Borrow`-trait unification. The
version is ALREADY in `Cargo.lock` (dashmap → the LSP chain), so no
new vendored code; wasm32-clean (`cargo check --workspace --target
wasm32-unknown-unknown` is a gate). **Disclosed: this is the runtime's
first external dependency** (rut-std is currently path-deps only).

Rejected — **B (std-only enum core)**: `Borrow<str>` on a kind-split
enum needs `IdHasher` reshaped to unify `write_str`/`write_u8`
(std's `0xff` terminator byte) with `write_u64` (the mix64 word) into
the pinned bits — a subtle hasher-funnel where a mistake silently
changes bucket assignment. Same single-probe goal, more risk, no
stronger guarantee.

Also rejected (by record): hash-algorithm change (fasthash phase 0 —
NO), any-lane decode polish (≤2%, below the placement floor), key-hash
memoization (FNV over ≤12 bytes ≈ 4 ns).

## 4. The receipts (scratch, never committed)

Command: `cargo test -p rut-std --release scratch_borrow_probe -- --nocapture --test-threads=1`
(full raw output: `/tmp/opencode/nmapset-borrow/phase0-scratch-receipts.txt`;
baseline 3-runtime bench: `/tmp/opencode/nmapset-borrow/pre.{json,md}`).
Paths: **A** = the landed `NativeTable` (valued lanes, one owned key
per op — the call-site `buf.clone()` stands in for `decode_key`'s
`to_owned`, same bytes, same count), **B** = borrowed probes over std's
default hasher (SipHash — the borrowed SHAPE, overpaying the probe),
**C** = the LANDING PROXY (borrowed probes over an FNV fold — what the
hashbrown core with hand-rolled `hash` runs). Alternating order, same
op stream, sink equality asserted every round (both paths compute the
same observable work).

**The census law (n = 50 000, exact to the operation):**

| phase | ops | A allocs | B allocs | C allocs |
|---|---:|---:|---:|---:|
| build | 50 000 | 50 012 | 50 012 | 50 012 |
| replace | 50 000 | 50 000 | **0** | **0** |
| hit-get | 50 000 | 50 000 | **0** | **0** |
| miss-get | 50 000 | 50 000 | **0** | **0** |
| remove-⅓ | 16 666 | 16 667 | **0** | **0** |
| rescan | 50 000 | 50 000 | **0** | **0** |
| re-add-⅓ | 16 666 | 16 667 | 16 667 | 16 667 |
| **total** | **283 332** | **283 346** | **66 679** | **66 679** |

Exactly one materialization per probe in A; the borrowed paths pay only
fresh inserts (build + re-add + growth) — **−216,667 allocs = −76.5%
per run**.

**Wall (median of 5 alternating rounds, row scale n = 50 000):**
A 11.6 ms / B 11.7 ms / **C 9.9 ms** (second run: A 10.2 / C 8.3) —
**A → C ≈ −1.7…−1.9 ms, ~−18% of the isolated table body**, and C's
round-to-round spread is far tighter (8.3–10.4 vs A's 9.6–15.6): the
per-op allocator churn also destabilizes A.

**Sensitivity (n = 400 000, 3 rounds):** A 205.2 ms vs B 163.2 ms —
**−42 ms (−20.5%)** with the census scaling to 2,266,683 vs 533,349
allocs. Caveat recorded: B's entries are narrower than the landing's
(`String→u64` vs `KeyVal+Entry`), so this bounds the candidate class;
the landing's own number is the A→C column plus phase 2's movers.

**HONEST RECALIBRATION:** the batch's planning prediction (7–11 ms per
str row from ~283k malloc/free pairs) was OVERSTATED — glibc's tcache
absorbs the small-size pairs at this scale. The measured row-scale
effect is **≈ −1.7…−1.9 ms (≈ −2…4% of the str rows)** — above the
strbuild ledger's ±1–3% unresolvable band only marginally, so the WALL
verdict is phase 2's (same-session movers + interleaved A/B, the house
method). What is NOT marginal and NOT subject to drift: the structural
receipt — **−76.5% of the host allocator traffic per str-row run**,
portable to hosts whose malloc is not tcache-optimized, and scaling
with map size (−20% at 8× the row).

## 5. Gates (phase 1) and expected movers

Gates: `rut-std` unit suite; `rut-driver --test nmapset` (7/7);
`rut-cli` `nmap`/`nmap_primmap`/`nmap_viewkeys`; full workspace test;
wasm32 check; **fuel + VM-heap BIT-IDENTICAL on every row** (host-only
change); family checksums + kmer-view/strview parity pins
bit-identical on {rut, qjs, node}. Test moves disclosed:
`native_table_shallow_size_is_pinned` (40 — re-pinned to the new
`size_of::<NativeTable>()`), and `rut-cli/tests/nmap.rs:357`'s
comment (the `> 4000×32` assertion itself keeps holding).

Expected movers (revised down to the evidence): the four str/sv rows
≈ −1.5…−2.5 ms each with less run-to-run variance; the three Bits
controls (`nmapset-int`, `nmap-hashset`, `refvals`) bit-flat; no row
regresses (checksums immovable, fuel/heap identical).

## 6. The call

**Phase 1 runs.** The census is exact and structural, the wall
direction is positive and consistent across every run at the row's own
scale, the sensitivity run shows the effect grows with size, and no
gate can move (no op, no checksum, no heap accounting is visible from
host code). The wall adjudication — the one open question — belongs to
phase 2's movers, measured under the house method.
