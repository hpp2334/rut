# nmapset-hostops survey — phase 0

Batch: **nmapset-hostops** (the host takeover): `nmapset.rut` becomes a
thin wrapper — every impl control flow (the grow-retry loops, the
sidecar relocation drain, the val-column slot addressing) moves
host-side behind fused one-crossing ops over STABLE VAL HANDLES; the
`PrimMap*` classes are REMOVED; the legacy crossings are KEPT as
documented escape hatches. Phase 0 = the census, the probe compiles,
the micro-calls. Docs only — zero code changes.

THE USER DIRECTIVE (verbatim): *"for nmapset, I recommend we move the
impl to hostm rut part is just a wrapper at all"* — plus the three
locked answers: **(A) stable val handles** (the full takeover; the
drain dies), **(B) legacy crossings kept**, **(C) `remove primmap\***.

THE LANDED STATE this survey censuses (base `f9016f5`, i.e. post
type-name-law `7f9cb67` + todolist-nonnull): VERSION 11; every map
rides the `[?V]` sidecar (the alias rows are repealed — one name, one
decl); the PrimMap lane classes are nmapset-private and UNREACHED by
any tree program; `nmap-primmap` is an i64-val SIDECAR twin of
`nmapset-int` (the column lane left the surface with the repeal and
the row's pins moved to the sidecar's price — the type-name-law
movers: fuel 17,950,301 → 20,903,285, heap 324 B → 3,539,324 B,
checksum immovable).

---

## 1. The census

### 1.1 The crossing surface (`rut/nmap_host/nmap.d.rut`)

31 crossings declared; 27 wrapped by `nmapset.rut`'s `use`; 4 dead:

| group | count | wrapped by nmapset.rut | fate under this batch |
|---|---|---|---|
| round-1 opaque lanes (`map_entry`/`map_find`/`map_remove` + `map_needs_grow`) | 4 | NO (declared, never wrapped) | KEPT, marked legacy |
| table core (`map_new`/`map_grow`/`map_take_reloc`/`map_cap`/`map_len`) | 5 | yes (5) | KEPT — `map_new`/`map_cap`/`map_len` stay wrapped (construct/len); `map_grow`/`map_take_reloc` legacy after P2 |
| typed lanes (`map_{entry,find,remove}_{i,u,b,s,y}`) | 15 | yes (15) | KEPT, legacy after P2 (superseded by the fused family) |
| sv lanes (`map_{entry,find,remove}_sv`) | 3 | yes (3) | KEPT, legacy after P2 |
| val column (`map_val_set_u`/`map_val_get_u`/`map_val_set_f`/`map_val_get_f`) | 4 | yes (4, PrimMap only) | KEPT — the classes die but the crossings stay (the user's keep-legacy call; `nmap_valcolumn.rs` still exercises them raw) |

### 1.2 The wrapper impl logic that moves (the "impl" of the directive)

`rut/nmapset/nmapset.rut` (596 lines, `inline = true`):

- **7 copies of the fused-sentinel grow-retry loop**
  (`while (at <= -2147483647)`) — `HashMap.put`/`put_range`,
  `HashSet.put`, the three PrimMap puts.
- **2 copies of the sidecar relocation drain** — `HashMap.put` /
  `put_range` interleaving `map_take_reloc` with `[?V]` writes into a
  fresh `[nil; new_cap]`.
- **PrimMap val-column slot addressing** — `map_val_set_u`/`_f` at the
  answered slot + the reinterpret casts (dies with the classes).
- `KeyLane` (11 impls × 3 one-liners) — STAYS: it is the union bound's
  admission + lane-pick machinery, not impl logic.

### 1.3 The PrimMap* reality (the removal is cheap)

Live-code grep for `PrimMap` outside docs: **zero consumers**. The
survivors are (a) the class bodies themselves (`nmapset.rut:390-542`),
and (b) the two NEGATIVE pins that assert their absence —
`crates/rut-lsp/src/std_surface.rs:128` and
`integrations/vscode-extension/test/e2e-wasm.js:215`. Deletion keeps
them absent: both pins hold automatically. The driver suites spell
only family spellings: `nmap_primmap.rs` is `HashMap<K, i64/u64/f64>`
(sidecar — its header already discloses the re-seat) and
`nmap_valcolumn.rs` mounts `rut/nmap_host` alone and drives the RAW
crossings (kept). **No fixture dies.**

### 1.4 Consumer census (surface does NOT move — no consumer changes)

`use nmapset::` sites (17): benches ×7 (`nmapset-int`, `nmapset-str`,
`nmap-hashset`, `nmap-knucleotide`, `kmer-view`, `strview`, `refvals`,
`nmap-primmap` — the last dies), examples/05 ×5 (`t1_harness`,
`store/atom/atom`, `store/derived/derived`, `app/todo_list/todo_list`,
`t1/core/t1`), `rut/json/group-nmapset.rut` (HashMap + HashSet only),
driver fixtures (`jsonpkg/main.rut`, `peers/json/serde_nmapset.rut` —
the latter's `Map` is the peers fixture's own pkg-local name),
`demo/src/examples/maps.rut`. Rust-side: `install_std_nmap` callers
(cli, wasm, probe, examples' mounts/tests) are additive-registration
only. `opt_prim_store.rs`, `nmapset.rs`, `nmap_typelanes.rs`,
`nmap_viewkeys.rs` ride the class surface — unchanged.

### 1.5 Pin census

- **Checksums (the gate, never edited for survivors)**:
  `expected.json` map-family rows — `nmapset-int` 734932704,
  `nmapset-str` 1264308351, `nmap-hashset` 21500055,
  `nmap-knucleotide` 2198604, `nmap-primmap` 734932704 (the row dies;
  its entry is removed WITH the workload, disclosed). Parity rows:
  `kmer-view` (2198604) and `strview` (1264308351) — same values by
  design, no separate lines.
- **Live fuel/heap receipts** (the type-name-law record `81bc9f6`):
  nmapset-int 20,703,284/1,966,551; nmapset-str 9,551,761/983,620;
  nmap-knucleotide 38,814,389/4,195,084; nmap-hashset 13,267,176/551;
  nmap-primmap 20,903,285/3,539,324 (dies). These MOVE at phase 2
  (fewer crossings per op, no drain arrays, the doubling sidecar) and
  are re-measured there — the movers table is phase 2's deliverable.
- **No pin hashes `nmapset.rut` bytes**: `std_surface.rs:27`
  `include_str!`s the file to build the INDEX (name-based pins:
  HashMap/HashSet present, PrimMap absent, no `pub type` rows) — all
  three hold under the rewrite + deletion.

## 2. The design (phases 1-2, decided here)

**The handle column (host-side, `nmap.rs`)**: `NativeTable` gains one
stable handle per key — a MONOTONIC birth index assigned at first
insert, relocated internally during grow (the same re-slot walk that
already moves keys/hashes/vals), never recycled in v1 (§3a). Growth
becomes host-internal; the wrapper never learns it happened.

**The fused family** (additive decls in `nmap.d.rut`, bodies in
`install_std_nmap`; one shape per lane op, over the closed key set +
the sv range lanes):

- `map_hput_{i,u,b,s,y,sv}(m, k[, off, len]) -> i64` — entry +
  internal grow + answer, packed `(handle << 1) | newly`;
- `map_hfind_{...}(m, k) -> i32` — the key's handle, or `-1`;
- `map_hremove_{...}(m, k) -> i32` — the freed key's handle, or `-1`
  (the wrapper nils the sidecar slot to release the cell).

`HashSet` SHARES `hput` and reads bit 0 (`(ans & 1) == 1`) — one
family, no bool twin. The old crossings stay declared and bound
(§1.1), their headers marked legacy.

**The wrapper (`nmapset.rut` phase 2)**: `HashMap`'s sidecar becomes
the append-only triple `vals: [?V]` + `vlen`/`vcap` with the
pouch-push doubling pattern inlined (NO pouch dep — the spell is
probe-verified §4): fresh handle `== vlen` → double-when-full → store
+ bump; interior handle → plain store. `with_capacity` pre-sizes from
`map_cap(t)` as today. Every method is one crossing + ≤3 lines; ZERO
loops in the file; the `get -> ?V` aliasing law holds by construction
(`sidecar[handle]` IS the cell; handles never move).

## 3. The micro-calls (plan §0.8 — resolved with probe evidence)

- **(a) Monotonic handles, no free-list (v1)** — stands. The tradeoff:
  a removed key's sidecar slot nils and is never reused; under churn
  the cost is ONE nil slot (a pointer) per distinct key ever inserted.
  Recycling is a measured follow-up, not landed blind.
- **(b) HashSet shares `hput`** — bit 0 is the newly flag; probe-proven
  (`(ans & 1) == 1`).
- **(c) Packing widths** — packed answer `i64` (`h << 1 | flag`),
  handles `i32`: probe-verified at handle 10^9 (answer 2·10^9+1 rides
  i64; the unpacked handle rides i32 with headroom). Handles are
  bounded by BIRTHS, not cap — the i64 carrier is the headroom.
- **(d) Fixture fate** — NO fixture dies (§1.3): `nmap_primmap.rs` and
  `nmap_valcolumn.rs` stand unchanged; the val-column UNIT tests in
  `nmap.rs` stand (crossings kept).
- **(e) Pin audit** — no byte-hash pin exists (§1.5); name-based pins
  hold under deletion.

## 4. The probes (receipts under `/tmp/opencode/batch-nmapset-hostops/p0/`)

`probe-hostops.rut` (scratch, never committed) spells the phase-2
wrapper needs with the host SIMULATED (`pack()` mints the fused
answers; handles are birth indices):

| row | value | proves |
|---|---|---|
| `PACK 0 1 1000000000 true` | exact | the packed spell — `(h << 1) \| flag`, `ans >> 1`, `ans & 1`; handle 0 under both flags; 10^9 handle fits |
| `PRIM 14950 15650 100` | exact | prim `V=i32`: 100 fresh births force the doublings 4→8→…→128; the full replace pass exercises the interior store; `len` = high-water mark |
| `REF 140 50` | exact | reference `V=str`: str cells stored/read through `?V` nil-compare + auto-unbox (the demo maps idiom), same machinery, one program with the prim leg |
| `OK 30740` | exact | the fold |

Mechanism: `rut-bench-probe` (compile+run, fuel 15,261 / heap 5,076 B,
no trap — the JSON lane) and `rut-cli run` (the printed rows above —
the probe binary discards program output by design, probe main.rs:221).

## 5. The phase-2 migration mapping

- `rut/nmapset/nmapset.rut`: header law rewritten (the host owns keys
  AND their identity; vals stay rut-side indexed by stable handle; no
  drain, no sentinel); `KeyLane` as-is; `HashMap` (8 methods + 4 range
  methods) and `HashSet` (4 methods) become one-crossing delegations;
  PrimMap* deleted. Est. ~596 → ~250 lines (the `inline = true` source
  splices smaller into every consumer's unit).
- Retirements (disclosed in the phase-2 commit): 
  `benches/workloads/nmap-primmap/{main.rut,rut.toml}` +
  `benches/workloads/nmap-primmap.js` + the `expected.json` line +
  the README live-table row; the README performance log stays
  append-only history (the strbuild not-rewritten precedent).
- Gates: the four surviving checksum pins (+ the kmer-view/strview
  parity pins) BIT-IDENTICAL on rut/qjs/node; driver suites green
  unchanged; workspace green; wasm32 check green; the fuel/heap movers
  table measured and recorded (the sidekick-pricing precedent —
  checksums are the gate, fuel/heap are receipts).
- VERSION stays 11 (additive crossings + std pkg source; no wire/repr
  change). RFC 0023 amendment + the batch report land at phase 3.

## 6. The calls

1. Phase 1 (host): the handle column + the fused family + unit tests
   (handle stability across multi-grow sweeps; packing
   collision-freedom; sv lanes ≡ s lanes; hput's internal grow firing
   exactly at the old sentinel boundary — the load-factor law's
   constants unchanged, so probe-slot assignment never moves and the
   checksums cannot move).
2. Phase 2 (wrapper): the rewrite + the retirements + the movers table.
3. Phase 3 (close-out): the report, the RFC 0023 amendment (the handle
   law + the fused family + the legacy list), the README sections.
