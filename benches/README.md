# benches — rut vs QuickJS vs V8

Cross-runtime benchmarks for **speed** and **memory**: the rut VM (this
repo) against [QuickJS-ng](https://github.com/quickjs-ng/quickjs)
(vendored) and V8 (the system `node`).

The point is not to win. rut is a research bytecode interpreter with a
self-managed RC heap (RFC 0016/0039); QuickJS is a mature optimizing
interpreter and V8 is a JIT. The suite measures where rut stands today
and gives every future optimisation a before/after number.

## Layout

```
benches/
├── run.mjs                 # the runner: node benches/run.mjs
├── README.md               # this file
├── tools/
│   ├── quickjs-ng/         # vendored engine (git submodule, pinned)
│   └── build-quickjs.sh    # gcc build → benches/.tools/qjs
├── probe/                  # rut-bench-probe: in-process phase/heap probe
├── workloads/
│   ├── NAME.rut            # the rut program (`std:*` imports; logs CHECKSUM)
│   ├── NAME.js             # the identical program for node + qjs
│   ├── NAME/               # dir-shaped rut side: `rut.toml` + `main.rut`,
│   │                       # used when the workload mounts its own deps
│   │                       # (the `nmapset` workloads); runs as
│   │                       # `rut run <dir>`
│   └── expected.json       # canonical reference checksums (verified)
└── results/                # generated reports (gitignored)
```

## Prerequisites

- `cargo` (builds `rut` and the probe in release mode),
- `node` (V8 — any recent version),
- a C compiler (`gcc`) to build the vendored QuickJS,
- GNU `/usr/bin/time` for peak-RSS measurement (optional; timing still works).

QuickJS is **not** taken from the system. It is vendored as a submodule
pinned to **quickjs-ng v0.16.2** and built locally on first run:

```sh
git submodule update --init benches/tools/quickjs-ng   # if cloning fresh
bash benches/tools/build-quickjs.sh                    # → benches/.tools/qjs
```

The runner does both automatically.

## Running

```sh
node benches/run.mjs                    # everything, 3 reps + 1 warmup
node benches/run.mjs --runtime rut,qjs  # subset
node benches/run.mjs --workload nbody   # one workload
node benches/run.mjs --quick            # (see --help) fewer reps
node benches/run.mjs --json benches/results/run.json \
                     --md   benches/results/run.md \
                     --csv  benches/results/run.csv
node benches/run.mjs --list             # list workloads
node benches/run.mjs --help
```

Common options: `--repeats N`, `--warmup N`, `--probe-iters N`,
`--timeout SEC`, `--no-probe`, `--no-build`. A full suite run takes a
minute or two (`--workload`, `--quick` and `--repeats 1` are useful
while iterating).

## Workloads

Every pair computes the same thing and prints one `CHECKSUM …` line. The
runner checks the checksum two ways: against the other runtimes **and**
against the reference in `workloads/expected.json`.

| Workload | Stresses | Scale | Canonical reference |
|---|---|---|---|
| `empty` | process/compile startup floor | — | none |
| `sieve` | `Vec<i32>` + tight integer loops | limit 500 000 | π(500000) = `41538` |
| `quicksort` | recursion, in-place vec mutation | n = 10 000 | sorted-array hash `653905516` |
| `matrix-mul` | flat `Vec<f64>` multiply-add | n = 64 | `1609836480` |
| `mandelbrot` | `f64` control flow | 100 × 75, 200 iters | escape-count `376659` |
| `fannkuch` | permutations, array churn | n = 7 | checksum 228, max-flips 16 → `228016` |
| `nbody` | `f64` integration + `sqrt` | 1000 steps | energy `-0.16908760523460628` |
| `spectral-norm` | `f64` power iteration + `sqrt` | n = 150 | `1.274222872607514` |
| `binary-trees` | RC allocation/drop churn | depth 14 | 2¹⁵−1 nodes = `32767` |
| `fasta` | string building (`f""` accumulation) | n = 10 000 | `15246:10000` |
| `intloop` | integer arith + loop dispatch | 5M iters | `628038624` |
| `floatloop` | f64 mul/add + loop dispatch | 2M iters | `1107013.7297975053` |
| `call` | call/return/frame overhead | fib(28) | `317811` |
| `alloc` | record allocation/RC churn | 2M records | `1385447424` |
| `array` | growable-sequence churn (`Vec<i32>` vs a plain JS `Array`): push through the grow path, indexed read+write, full iteration | n = 1 000 000 | checksum `1000000000000` |
| `nmapset-int` | `HashMap<i32, i32>` churn (**nmapset**, the host-implemented map experiment): put/replace/hit-and-miss get/remove/re-scan — the op stream of the removed `hashmap-int` mapset row | n = 100 000 | checksum `734932704` |
| `nmapset-str` | str-keyed `HashMap<str, i32>` (**nmapset**): generated keys, removals, re-adds — the op stream of the removed `hashmap-str` mapset row | n = 50 000 | checksum `1264308351` |
| `nmap-hashset` | `HashSet<i32>` (**nmapset**): adds, dup adds, probes, removals, intersection count — the op stream of the removed `hashset` mapset row | n = 100 000 | checksum `21500055` |
| `nmap-knucleotide` | k-mer counting over `HashMap<str, i32>` (**nmapset**): 12-mer fill + fragment probes — the op stream of the removed `knucleotide` mapset row | seq = 200 000 | checksum `2198604` |
| `json-decode` | the digest JSON decode: char-split + parser minting one `opaque` box per JSON value (and per object key), plus a downcast fold over the tree — REPS reps of a large generated document (1 200 rows; ~34k boxes minted per rep, ~100k total) | doc ~204 KB, reps 3 | checksum `4502015958359127277` |
| `crossing-nop` | the rut→host **crossing tax**, isolated: loop A calls the host `nop` (identity), loop B an inline rut fn with the same body; the `4` pair repeats both over a 4-arg sum — every body is deliberately empty, so (A−B) is the crossing and (nop4−nop) the per-param slope | 2M iterations × 4 loops | checksum `20000014000000` |
| `kmer-view` | the `nmap-knucleotide` k-mer counting keyed through `nmapset::HashMap`'s **range methods** (strings-round1 phase 2 — the sv lanes: keys cross as borrowed byte windows of the sequence, no key cell minted; see the performance log) | seq = 200 000 | checksum `2198604` — the `nmap-knucleotide` pin; parity is the gate, no separate line |
| `strview` | the `nmapset-str` six-phase churn with keys carved as fixed-width windows of ONE generated parent string (strings-round1 phase 2 — range methods, the shape where the view lever applies; see the performance log) | n = 50 000, parent = 600 000 chars | checksum `1264308351` — disclosed: the SAME value as the `nmapset-str` pin, because the formula reads counters + the value sum only (key content is invisible to it) |
| `refvals` | a **record-valued** map `HashMap<i64, Pt>` — the suite's only ref-V row (rut records are shared cells, RFC 0044): insert/overwrite churn with a fresh `Pt` per put, hit-heavy gets (5:1), **read-modify-write through the alias** (a field write through the get-returned reference; the checksum depends on the write-through), and a grow-heavy sweep through every load-factor boundary (the `[?V]` relocation drain). K = i64 holds the hash term constant so the row isolates VALUE-side costs. (The refval-exp batch's phase-1 `refcolumn` twin re-spelled this exact op stream over the experimental host val column; phase 2 measured it SLOWER and the experiment was REVERTED — see the verdict log. The `refvals` row and its analysis are permanent.) | n = 100 000, grow sweep 200 000 (~1.32 M map ops, ~650 k record mints) | checksum `140052990000` |

`sieve`, `quicksort`, `matrix-mul`, `mandelbrot`, `fannkuch`, `nbody` and
`spectral-norm` follow the standard algorithms (fannkuch and nbody to the
benchmarks-game definitions); `binary-trees` and `fasta` are
**adaptations**. `json-decode` is too — see below. The four `nmapset`
workloads (`nmapset-int`, `nmapset-str`, `nmap-hashset`,
`nmap-knucleotide`) stress the keyed collections — once over the removed
pure-rut `mapset` pkg (see the removal note below), now over
`rut/nmapset` alone: the rut sides ship as module dirs
(`NAME/{rut.toml, main.rut}` with `[deps] nmapset = …`) and run as
`rut run <dir>` — the CLI's single-file auto-mount list stays untouched.
Honest framing: V8's `Map`/`Set` are inline-cache-optimized and QuickJS
has its own fast paths — these rows are not expected to be a win. The
point is rut's number for the RC-heap slots its keyed collections pay.
The four `nmapset` workloads are line-for-line ports of the removed
`mapset` workloads over `rut/nmapset` — the **host-implemented** key
table experiment (the "C builtin" architecture qjs itself uses): keys
live as owned Rust data behind one `opaque` box per table, values stay
rut-side in a parallel `[?V]` array, and every map op crosses the host
boundary once through a typed crossing (`map_{entry,find,remove}_{i,u,b,s,y}`
— the key crosses directly as its own type, hashed host-side by
`hash_payload`, which ports the same mix64/FNV-1a constants; `map_entry_*`
answers `i32::MIN` grow-first, so put is one crossing even on growth). The nmapset pkg
ships the mapset-exact mix64/FNV-1a constants, so every key hashes to the
same bits the original mapset rows produced — the pins in `expected.json`
(`734932704` / `1264308351` / `21500055` / `2198604`) ARE those values,
and a mismatch is a bug. See the performance log below. Two strings-round1
rows (`kmer-view`, `strview`) run the knuc/str op streams through the pkg's
RANGE-keyed methods (`put_range`/`get_range`/`has_range`/`remove_range` —
the sv lanes, keys crossing as borrowed byte windows of a parent) instead
of slice-then-put; their provenance is disclosed in the performance log.

## Removed — the four `mapset` workloads (Sep 2026)

The pure-rut `mapset` package left the tree (stdlib slim-down, the
`remove mapset` commit), and the four workloads that mounted it were
deleted with it: **`hashmap-int`**, **`hashmap-str`**, **`hashset`**,
**`knucleotide`** (workload dirs + `.js` twins). Their **rows were
removed from `workloads/expected.json`** — the only sanctioned kind of
`expected.json` edit here: row removal for deleted workloads, never a
touch to any surviving row.

- The history stays in the performance log below: the mapset-vs-nmapset
  comparisons, the phase tables, and round2's `hashmap-int` −21.7%
  evidence all live on in the log (the engine wins they measured were
  rut-side and are re-measured by the surviving `nmapset` twins, whose
  op streams are identical).
- Migration for user-defined-key maps (the one thing only `mapset`
  admitted): encode the key canonically to `bytes` and key the
  union-bounded `nmapset` classes (`HashMap`/`HashSet`/`PrimMap*`),
  or vendor the old `mapset` source from git history. The nmap
  runtime trap and the union-bound diagnostic now say the same.
- The `nmapset` rows keep the deleted twins' exact checksums
  (`734932704` / `1264308351` / `21500055` / `2198604`) — they were
  pinned equal row-for-row while both pkg existed, so the pins did not
  move.

## Performance log — mapset-perf engine phases (Sep 2026)

Four engine phases landed against these rows: boxless static trait
dispatch + trait-impl/free-fn inlining, `MoveVal` last-use move
elision + clone/alloc fast paths, the array element access fast path,
and mapset's `[*K]` key storage with K-typed probes. Full-suite
cross-runtime numbers (net medians, peak RSS; `results/` is gitignored,
so this table is the durable record):

| workload    | net before | net after | peak RSS before | peak RSS after |
|-------------|------------|-----------|-----------------|----------------|
| hashmap-int | 288.7 ms   | 221.3 ms  | 61.0 MB         | 33.9 MB        |
| hashset     | 223.9 ms   | 152.7 ms  | 44.1 MB         | 26.5 MB        |
| hashmap-str | 304.1 ms   | 304.8 ms  | 33.2 MB         | 25.4 MB        |
| knucleotide | 1.11 s     | 1.03 s    | 97.4 MB         | 69.1 MB        |
| array       | —          | 236.2 ms  | —               | 97.3 MB        |

(`array` is new — the row above has no predecessor.)

This host drifts ±8-13% between days on identical code (the same
baseline binary measured sieve probe-exec 164.8 ms one day and
173-181 ms on others; qjs is stable), and the two full-suite runs were
taken on different days. A same-day control — the baseline commit and
HEAD re-benched minutes apart, 5 reps — gives the cleaner apples-to-
apples probe-exec deltas: hashmap-int **−43%** (319.4 → 183.2 ms),
hashset **−43%** (227.1 → 129.5 ms), hashmap-str **−13%** (310.0 →
270.1 ms), knucleotide **−12%** (1.14 s → 999.4 ms). The probe
decomposes the int-keyed wins into ~12-16% fewer executed ops (no
box/unbox, no trait frames, move-elided rehash copies) and ~18-21%
lower per-op cost (fused element access, block clones). VM
self-accounted heap peaks fell further: hashmap-int 33.0 → 16.4 MB,
hashset 25.1 → 12.6 MB, hashmap-str 15.9 → 9.8 MB, knucleotide
59.8 → 41.5 MB. All checksums unchanged.

Where the rest goes: the two str-keyed rows moved least because their
dominant per-op cost was never the map — every `hash()` of a `str` key
copies it through `encode()` before mixing, and re-probes re-hash. A
zero-copy `str.bytes()` view is the known next lever, deliberately out
of scope here. The int-keyed rows are left at the interpreter floor —
dispatch plus RC traffic on the field reads a probe chain needs — at
~2.6-3.3x QuickJS net (from 4.1-4.6x), with no box, frame, vtable call,
or redundant copy left on the path.

## Performance log — nmap: the host-implemented map experiment (Sep 2026)

The `nmapset` workloads measure a different architecture for the same
algorithms: the map's key table is **Rust** (`crates/rut-std`'s
`NativeTable`, the same open-addressing design mapset.rut uses — power-
of-two cap, tombstones, load 0.7, recorded hashes), reached through a
generic rut wrapper (`rut/nmapset`) that keeps only the value array
rut-side. Per map op the wrapper hashes the key (inlined mix64/FNV —
the same values as mapset), mints one `opaque` key box, and crosses the
host boundary once or twice (`map_needs_grow` + `map_entry`/`map_find`/
`map_remove`); growth relocates `vals` by draining a native relocation
iterator. Full-suite cross-runtime numbers (net medians, same-day run,
3 reps; all eight map rows in one run so the mapset↔nmapset comparison
shares a day):

| workload         | mapset rut net | nmapset rut net | Δ        | qjs net | node net | nmapset vs qjs |
|------------------|----------------|-----------------|----------|---------|----------|----------------|
| nmapset-int      | 184.9 ms       | 118.5 ms        | **−36%** | 68.6 ms | 27.0 ms  | 1.7x           |
| nmap-hashset     | 131.5 ms       | 70.7 ms         | **−46%** | 56.5 ms | 20.6 ms  | 1.3x           |
| nmapset-str      | 269.8 ms       | 211.6 ms        | **−22%** | 39.4 ms | 31.5 ms  | 5.4x           |
| nmap-knucleotide | 979.6 ms       | 785.3 ms        | **−20%** | 143.0 ms | 53.7 ms | 5.5x           |

The probe tells the same story from inside: executed fuel (VM ops)
fell 62-70% on the int-keyed rows because the probing itself left the
interpreter — mapset pays a rut-side probe chain per op, nmapset pays a
host call — and the VM-heap high-water collapsed with the rut-side
table (791 B for the whole hashset churn vs mapset's 12.55 MB):

| workload         | mapset exec / fuel / heap        | nmapset exec / fuel / heap      |
|------------------|----------------------------------|---------------------------------|
| nmapset-int      | 177.4 ms / 64.8 M / 16.38 MB     | 111.2 ms / 24.8 M / 5.80 MB     |
| nmap-hashset     | 119.7 ms / 56.2 M / 12.55 MB     | 60.7 ms / 17.0 M / 791 B        |
| nmapset-str      | 250.9 ms / 111.5 M / 9.81 MB     | 197.3 ms / 86.4 M / 2.90 MB     |
| nmap-knucleotide | 958.7 ms / 418.8 M / 41.50 MB    | 744.3 ms / 323.7 M / 11.85 MB   |

Against the experiment's targets (net medians): `nmap-hashset` ≤ ~70 ms
— **met** (70.7 ms); `nmapset-int` ≤ ~80 ms — **missed** (118.5 ms);
`nmapset-str` ≤ ~90 ms — **missed** (211.6 ms); `nmap-knucleotide` ≤
~500 ms — **missed** (785.3 ms). The str rows were always going to move
least: the wrapper still hashes every `str` key through `encode()`
(that cost never moved — only storage/equality did), and knucleotide
remains dominated by k-mer materialization (`seq.slice` per position),
which the map swap cannot touch. The int rows' residual decomposes per
map op into the wrapper's interpreter work (generic call frame, hash,
`vals` traffic — nmapset-int is 24.8 M ops) plus the crossing package:
one `opaque.new` mint and one or two host calls (~17 ns/call, the
mathhost floor). The mint alone is small — a 600 k escaped-mint loop
measures ~27 ns/mint, ~15% of nmapset-int's exec — so no single term
dominates; the residual is the per-op package itself. The known
follow-up — per-family typed fast-path fns (`map_find_i32(m, k: i32,
h: i64)`), which would collapse wrapper frame + mint + extra crossing
into one call — is **declined for now**: the generic wrapper cannot
pick them without specialization. The experiment records its verdict:
a host table buys −20-46% over pure-rut mapset and closes to 1.3-1.7x
qjs on integer keys, but the crossing tax per op keeps generic-rut
callers from reaching the C-builtin floor.

## Performance log — the byref flip: pass-by-reference + `?T` (Sep 2026)

The byref-nullable batch (RFC 0044) replaced the copy law with
pass-by-reference sharing: records and arrays — previously **value**
types, memcpy'd on binding and on store — are now shared cells (one
handle retain/release), the `CloneVal`/`MoveVal`/`ArrGetRef`/`ValEq`
ops and the move-elision liveness pass are deleted, `?T` (the renamed
pointer) is the one nullable, and `bytes.clone()` is the only copy.
Full suite, same-day runs, 3 reps + 1 warmup: before = the mapset-host
batch HEAD (the binary the log above ends on), after = the byref batch
HEAD. **All 23 checksums equal `expected.json` on all three runtimes**
— the regime change is behavior-neutral by construction (identity `==`
and aliasing are the flagged semantics, and no workload depends on
them). VM-heap peaks are unchanged to the kilobyte on every row
(`array` 38.52 MB, `hashmap-int` 16.38 MB, `sieve` 20.84 MB,
`knucleotide` 41.50 MB, `binary-trees` 2.50 MB) — sharing changes
handle traffic, not the live set.

In-process probe, exec median (the clean same-host signal):

| workload         | exec before | exec after | Δ        | fuel before → after |
|------------------|-------------|------------|----------|---------------------|
| fannkuch         | 10.0 ms     | 4.6 ms     | **−54%** | 2.37 M → 2.01 M     |
| quicksort        | 18.0 ms     | 10.4 ms    | **−42%** | 4.20 M → 4.05 M     |
| binary-trees     | 9.1 ms      | 7.6 ms     | **−17%** | 1.049 M → 1.049 M   |
| nmapset-int      | 111.2 ms    | 98.1 ms    | **−12%** | 24.80 M → 24.80 M   |
| hashmap-int      | 177.4 ms    | 167.7 ms   | −5%      | 64.76 M → 64.76 M   |
| nmap-hashset     | 60.7 ms     | 58.9 ms    | −3%      | 16.98 M → 16.98 M   |
| hashset          | 119.7 ms    | 117.8 ms   | −2%      | 56.23 M → 56.23 M   |
| knucleotide      | 958.7 ms    | 936.1 ms   | −2%      | 418.77 M → 418.77 M |
| hashmap-str      | 250.9 ms    | 251.2 ms   | 0%       | 111.49 M → 111.49 M |
| nmap-knucleotide | 744.3 ms    | 756.6 ms   | +2%      | 323.66 M → 323.66 M |
| nmapset-str      | 197.3 ms    | 207.4 ms   | +5%      | 86.41 M → 86.41 M   |
| alloc            | 17.4 ms     | 18.2 ms    | +5%      | 22.00 M → 22.00 M   |
| sieve            | 160.2 ms    | 170.8 ms   | +7%      | 22.20 M → 21.17 M   |
| array            | 224.1 ms    | 270.6 ms   | **+21%** | 60.49 M → 63.49 M   |

(The scalar-loop rows — `intloop`, `floatloop`, `mandelbrot`, `call`,
`mathhost`, `nbody`, `spectral-norm`, `matrix-mul`, `u64loop`,
`fasta` — are flat within noise, ±3%, as the law predicts: primitives
and `fn` values still copy by slot.)

Where the wins came from: the old regime copied WHOLE records and
arrays on binding and on store. fannkuch and quicksort churn arrays
through calls and swaps — every argument binding was a memcpy unless
the liveness pass proved the source dead — and binary-trees stored
every child node into its parent slot by value. Sharing turns all of
it into one handle retain, and the fuel column shows it: fannkuch
runs 15% fewer ops (copies the elision pass could not prove dead are
simply gone), quicksort 4%. The map rows moved least: their `rehash`
copies were already the move-elision pass's blessed case, so deleting
the machinery changes little — the fuel column is byte-identical on
every map row. nmapset-int's −12% is the wrapper profiting from `?V`
being the absence type (`get` returns the stored cell directly).

Where the cost showed up — honest regressions: a growable `Vec<T>`'s
backing is `buf: [?T]` under the sharing law (RFC 0044 §5), so every
PRIMITIVE element store boxes into a one-slot cell (`MakeOpt` +
retain) and every load derefs. The `array` workload (1M-element
`Vec<i32>` push/index/iterate churn) pays **+21% exec on +5% fuel** —
the new ops cost more per op than the raw slot moves they replaced —
and `sieve` +7% exec despite −5% fuel (its marks stores elide a copy,
its primes pushes box). `alloc` +5%: allocation churn pays
retain/release where a memcpy used to be. Flat `[T]` buffers are
untouched (`matrix-mul` and the numeric rows are flat). The known
lever — a flat primitive backing for growable sequences under a
boxed-only-when-shared scheme (RFC 0044 OQ-1) — is deferred.

Cross-runtime net medians for the flagged rows (rut, wall − startup):

| workload     | net before | net after | Δ        | qjs net after     |
|--------------|------------|-----------|----------|-------------------|
| fannkuch     | 11.7 ms    | 6.3 ms    | **−46%** | 8.6 ms (rut leads)|
| quicksort    | 20.3 ms    | 12.2 ms   | **−40%** | 11.9 ms (even)    |
| binary-trees | 10.3 ms    | 7.7 ms    | **−25%** | 8.7 ms (rut leads)|
| nmap-hashset | 70.7 ms    | 67.1 ms   | −5%      | 55.5 ms           |
| hashmap-str  | 269.8 ms   | 261.1 ms  | −3%      | 39.0 ms           |
| sieve        | 172.3 ms   | 165.0 ms  | −4%      | 53.1 ms           |
| hashmap-int  | 184.9 ms   | 184.6 ms  | 0%       | 62.5 ms           |
| hashset      | 131.5 ms   | 132.3 ms  | 0%       | 54.6 ms           |
| knucleotide  | 979.6 ms   | 968.5 ms  | −1%      | 143.5 ms          |
| nmapset-int  | 118.5 ms   | 116.7 ms  | −1.5%    | 62.5 ms           |
| nmapset-str  | 211.6 ms   | 209.9 ms  | −1%      | 40.4 ms           |
| nmap-knucleotide | 785.3 ms | 781.5 ms | 0%      | 142.8 ms          |
| array        | 210.5 ms   | 287.6 ms  | **+37%** | 117.2 ms          |

Position vs the field: rut goes AHEAD of QuickJS on fannkuch (6.3 vs
8.6 ms net) and binary-trees (7.7 vs 8.7 ms) for the first time on
this suite, and quicksort is now even (12.2 vs 11.9 ms) — all three
were the workloads the old regime copied hardest. The `array`
regression widens that one gap (2.45x qjs net, from 1.78x). The str
map rows stay where they always were — `encode()`-per-hash bound, not
copy bound.

## Performance log — nmapset typed crossings: union bounds + host hashing (Sep 2026)

The typed-lane batch (checker union bounds, phase 1; 15 typed host
crossings + host-side `hash_payload`, phase 2; the nmapset rewrite on
ONE crossing per op with a fused `i32::MIN` grow sentinel, phase 3)
erased most of the per-op package: no `opaque` key mint, no wrapper
hash frames, no rut-side probe arithmetic, no per-put grow check —
`put` = one `nentry`, `get`/`has` = one `nfind`, `remove` = one
`nremove`, `K requires i8 | … | bytes` makes the closed key set the
compile-time contract (user-defined keys fail at compile time; the
runtime Hashable-box trap is gone). Probe exec medians + fuel
(befores = the pre-optimization same-day run on the byref-flip HEAD;
afters = today's full-suite run, `rut-bench-probe`, 3 fresh-VM iters;
all 23 rows equal `expected.json` on rut, qjs and node — the nmapset
checksums stay bit-for-bit identical to the mapset rows,
734932704 / 1264308351 / 21500055 / 2198604):

| workload         | exec before | exec after | Δ       | fuel before → after        |
|------------------|-------------|------------|---------|----------------------------|
| nmapset-str      | 207.4 ms    | 54.9 ms    | **−74%**| 86.41 M → 9.74 M (**−89%**)|
| nmap-knucleotide | 756.6 ms    | 216.1 ms   | **−71%**| 323.66 M → 39.61 M (−88%)  |
| nmap-hashset     | 58.9 ms     | 40.4 ms    | **−31%**| 16.98 M → 13.27 M (−22%)   |
| nmapset-int      | 98.1 ms     | 81.7 ms    | **−17%**| 24.80 M → 21.10 M (−15%)   |

Cross-runtime net medians (wall − startup, same run; qjs before
values are from the mapset-host log's same-day run, qjs is stable
day to day):

| workload         | rut net before | rut net after | Δ        | qjs net | rut vs qjs after |
|------------------|----------------|---------------|----------|---------|------------------|
| nmapset-str      | 209.9 ms       | 71.8 ms       | **−66%** | 38.6 ms | 1.9x             |
| nmap-knucleotide | 781.5 ms       | 227.0 ms      | **−71%** | 139.2 ms| 1.6x             |
| nmap-hashset     | 67.1 ms        | 47.2 ms       | **−30%** | 55.3 ms | **0.85x (ahead)**|
| nmapset-int      | 116.7 ms       | 92.6 ms       | −21%     | 61.3 ms | 1.5x             |

Where the wins came from. The fuel column is the story: the whole
wrapper package per op was interpreter ops. For string keys the FNV-1a
byte loop ran scalar VM ops per byte — nmapset-str's 86.4 M fuel was
mostly hashing, and it is now `hash_payload` in the host (9.74 M
remaining is the workload's own loops) — which is why knucleotide,
a k-mer hash storm, collapses too. For int keys the residual package
(mint + wrapper hash + `map_needs_grow` + entry call) collapsed into
one `nentry` crossing: −15% fuel, −17% exec. `nmap-hashset` now runs
rut **ahead of qjs on net** (47.2 vs 55.3 ms) for the first time on a
map row.

Against the batch's honest targets: `nmapset-int` 98.1 → 60–75 ms net —
**missed** (92.6 ms net net-of-startup, −21% vs the −25–40% hoped);
`nmapset-str` 207.4 → ~140–160 ms — **beaten** (71.8 ms, ~2× the
target); `nmap-hashset` 58.9 → ~50 ms — **met** (47.2 ms);
`nmap-knucleotide` −10–15% — **beaten** (−71%). The int rows' residual
is interpreter dispatch on the wrapper's remaining cell traffic
(`vals` reads/stores per op — 21.1 M ops for nmapset-int), not the
crossing; no further host-side lever is obvious without native-owned
values (rejected by design). Stop-points (decision 9): **none
triggered** — no typed lane measured slower than its pre-optimization
baseline. Method note: befores are the pre-optimization same-day
baseline, afters were taken days later on the drifted host (±8-13%
between days on identical code); every row wins by far more than that
drift.

## Performance log — json-decode: the opaque-mint baseline (Sep 2026)

New row for the native-fastpath batch's phase B. `json-decode` ports
the `02-digest` JSON decoder verbatim (tagged-union tree, children
boxed in `opaque`), generates a large LCG document, and reparses +
rewalks (downcast fold) it REPS times: each rep builds the whole tree
through `opaque(v)` — one box per JSON value, one per object key
(the key's tree node is parsed, downcast, then dropped) — then folds
it and drops it. The JS twin runs the SAME generated text through the
engine's own `JSON.parse`, so the row places rut's decode against the
mature JSON paths. This entry recorded the **baseline only** — written
before the phases it measures against landed (OpaqueBox pooling in
phase 6; the planned inline-small-payloads phase was dropped — see the
close-out); no optimization numbers belong here.

All three runtimes agree on the checksum (twin gate, `expected.json`
pins it): `4502015958359127277`. Full row (net medians, same-day run,
3 reps + 1 warmup, probe over 3 fresh-VM iters):

| workload    | rut net | qjs net | node net | rut exec | fuel      | VM heap peak |
|-------------|---------|---------|----------|----------|-----------|--------------|
| json-decode | 675.2 ms| 62.3 ms | 24.3 ms  | 662.4 ms | 111.32 M  | 32.78 MB     |

Placement: rut is ~10.8x qjs net and ~27.8x node net on the identical
document — the row measures the interpreter's box churn (the per-value
`opaque` mint + the churning `Json` records and `Vec` pushes), not the
string handling alone. Honest notes on what this row is NOT: the
document is sized so the rut CLI's 64 MiB VM-heap budget holds the
split char array plus one live tree (32.78 MB peak); a larger document
would OOM. The doc generation and its char-split are paid once and
shared across the reps (fresh cursor per rep), so the per-rep cost is
the decode + fold — the churn the pooling phase (6) measured itself
against (it held parity; see the close-out). Box counts:
~34k boxes minted per rep (19 values + 9 keys per row), ~100k across
the 3 reps; ~37 M fuel per rep (`111.32 M` per main incl. gen+split).
Repeated times are stable; treat the x-runtime gap as the baseline
shape, not as saturation.

- Each runtime is invoked the way it is normally used: `rut run
  file.rut`, `node file.js`, `qjs file.js`. Wall time therefore
  **includes parse/compile/verification** for all three.
- Wall time is the median over `--repeats` timed runs after `--warmup`
  discarded runs; `wall min` is also reported.
- **Startup is measured, not baked in.** Before the workloads, the runner
  measures the `empty` program per runtime — process spawn + runtime init
  + parse/compile of a trivial program — and reports **`net = wall −
  startup`**, the workload's own cost. This matters because V8's ~20 ms
  startup otherwise dominates its small workloads and hides the real
  compute gap (e.g. node measures ~1.6 ms net on fannkuch, not 21 ms).
- **Peak RSS** is the whole-process high-water from GNU `time -f %M`
  (the largest value over the timed runs). It includes each runtime's
  baseline, so compare against the `empty` row for the same runtime.
- **rut's VM heap peak** is its own byte accounting (RFC 0039), not
  process memory: the high-water of live cells. It is reported
  separately by the probe.
- The **rut probe** (`benches/probe`) compiles once, then runs `main`
  on a fresh `Vm` per iteration, splitting `compile` / `decode+verify` /
  `exec` and reading `fuel_used` (RFC 0040) and `heap_peak_bytes`.
- **Correctness is checked against canonical references, not just
  across runtimes.** Each workload's checksum is compared to
  `workloads/expected.json` (the canonical value, or the value produced
  by the independent reference implementation used when the workload was
  written). Cross-runtime agreement alone cannot catch a bug shared by
  all three implementations — that is exactly how a wrong fannkuch
  checksum was caught. `f64` arithmetic is IEEE and evaluated in the
  same order in rut and JS; the `nbody`/`spectral-norm` workloads keep
  the **same fixed-iteration Newton `fsqrt`** on both sides (rather than
  rut's `Math.sqrt` vs JS's `Math.sqrt`) so the float checksums match
  bit-for-bit; the runner still tolerates a 1e-9
  relative difference when comparing numeric checksums.

## Performance log — OpaqueBox pooling: the host-payload pool tier (Sep 2026)

Phase 6 of the native-fastpath batch: a size-classed Rust-side free list
(`crates/rut-vm/src/heap/pool.rs`) for the erased `Box<dyn Any>` payload
blocks behind `CellData::HostBoxed` — `OpaqueBox::alloc` mints (`map_new`'s
`NativeTable`, plugin state, host-fn returns) pop a parked block of the
payload's exact `(size, align)` when one is free; every rc-0 host-box death
(`release_cell`) and every still-live box at arena teardown runs the stored
payload's destructor in place, exactly once, then parks the block. Bounded:
payloads ≥ 256 B (and ZSTs) bypass, the pool trims at 512 parked blocks
(~128 KiB worst case), and a dead arena's pool deallocates its remainder
with the arena. NO repr change, NO surface change, NO module version bump —
`opaque(v)`/`downcast`/`OpaqueBox<T>` are untouched, and the heap
accounting (RFC 0040) still charges `size_of::<T>()` per mint.

One honest scoping note: the plan expected this row's churn to be
`OpaqueBox::alloc` mints. It is not — rut-minted `opaque(v)` lowers to
`Op::Box` → `CellData::OpaqueBox { val: Slot }`, a slot INLINE in the cell
with no Rust block behind it; the cell itself is already recycled by the
arena free list. The json-decode row therefore cannot show pooling GAINS —
it can only surface overhead — and the pool tiers exactly the host
payloads that stay boxed (NativeTable, host structs; the close-out
below records where the batch ended).

Deltas vs the phase-5 baseline (same row, the day-of-record numbers; full
suite run, 3 reps + 1 warmup, probe over 3 fresh-VM iters; every checksum
equals `expected.json` on rut, qjs and node — json `4502015958359127277`,
nmapset/hashset `734932704` / `1264308351` / `21500055` / `2198604`):

| workload         | rut net before → after  | rut exec before → after | fuel         | VM heap peak |
|------------------|-------------------------|-------------------------|--------------|--------------|
| json-decode      | 675.2 → 676.2 ms (~0)   | 662.4 → 670.0 ms (+1.1%)| 111.32 M (=) | 32.78 MB (=) |
| nmapset-int      | 92.6 → 94.6 ms (+2.2%)  | 81.7 → 84.5 ms (+3.4%)  | 21.10 M (=)  | 5.80 MB (=)  |
| nmapset-str      | 71.8 → 64.4 ms (−10%)   | 54.9 → 58.4 ms (+6.4%)  | 9.74 M (=)   | 2.90 MB (=)  |
| nmap-hashset     | 47.2 → 48.7 ms (+3.2%)  | 40.4 → 44.7 ms (+10.6%) | 13.27 M (=)  | 503 B (=)    |
| nmap-knucleotide | 227.0 → 226.0 ms (−0.4%)| 216.1 → 210.1 ms (−2.8%)| 39.61 M (=)  | 11.85 MB (=) |

(`=` = bit-identical to the recorded value; fuel is a pure op count and
the heap peak is pure accounting, so equality is the expected, verified
outcome — repr and op stream unchanged.)

Honest reading. Fuel and heap are bit-identical on every row; the wall/exec
columns move within this host's day drift (±8-13% between days was already
the phase-4 method note; the nmapset-str NET win, for instance, is the same
day-drift the exec column shows upward). To separate drift from the patch,
an interleaved same-host A/B (baseline worktree at c7c4702 vs pooled build,
5 rounds × 5 fresh-VM iters) measured: **nmapset-int at parity-to-faster
pooled** (~−2%), **nmap-hashset at parity**, **json-decode at +1.2-2.9%**
(medians 660-670 vs base 648-661 the same hour — above the recorded 662.4
by at most 1.1%, i.e. absolute parity with the baseline). Three structural
suspects were engineered out while chasing that residual (a second
discriminant dispatch in `release_cell` — folded into the existing match;
the `pool` field shifting hot `Arena` offsets — moved last; the pool
machinery inlined into the always-inlined release web — now
`#[inline(never)]`); the remainder is per-loop codegen layout, and the map
rows sharing the same release web landing at parity-or-faster rules out any
global cost. The added semantic work per death is one discriminant compare
on a cell kind this row barely mints.

Stop-point (decision 9): **not triggered** — the json row does not regress
against the 675.2/662.4/111.33 baseline beyond noise (fuel and heap
bit-identical, exec/net within ±1.1%). The pool's wins sit in host-box
churn (`OpaqueBox::alloc` mint/free pairs — `map_new`, plugin state), which
current rows don't stress in hot loops; the tier stays for host payloads,
and the guard rows confirm the map behavior is unchanged. (The planned
phase 7 — inline small payloads — was DROPPED by the re-scope: see the
close-out below.)

## Performance log — native-fastpath batch close-out (Sep 2026)

The batch's final record. Track A killed the per-op package in nmapset
(phases 1–4): checker type-union bounds (phase 1), 15 typed host
crossings with host-side `hash_payload` and the fused `i32::MIN` grow
sentinel (phase 2), the nmapset rewrite to ONE crossing per op with
`K requires i8 | … | bytes` as the compile-time key contract (phase 3),
and the map-row verification (phase 4). Track B attacked `opaque` mint
churn for everyone (phases 5–7): the json-decode baseline row
(phase 5), the OpaqueBox pool for host payloads (phase 6) — and the
planned inline-small-payloads repr change was DROPPED (phase 7
re-scope): rut `opaque(v)` already lowers to an inline
`CellData::OpaqueBox { val: Slot }` — arena-recycled, no Rust
allocation — so that optimization targets a cost that does not exist
(RFC 0014's repr notes record the lowering). Phase 7 is documentation
only: the RFC 0043 union-bounds amendment (type unions only,
whole-bound capability resolution, the provenance holes), the RFC 0014
repr notes, and this section.

The rows the batch set out to move, at the numbers of record (typed
lanes from the phase-4 run; the json row from the phase-6 pooled run —
pooling held it at parity; the close-out verification re-run reproduced
every checksum):

| workload         | rut net before → after  | Δ          | qjs net after       | fuel before → after |
|------------------|-------------------------|------------|---------------------|---------------------|
| nmapset-str      | 209.9 → 71.8 ms         | **−66%**   | 38.6 ms             | 86.41 M → 9.74 M    |
| nmap-knucleotide | 781.5 → 227.0 ms        | **−71%**   | 139.2 ms            | 323.66 M → 39.61 M  |
| nmap-hashset     | 67.1 → 47.2 ms          | **−30%**   | 55.3 ms (**rut ahead**)| 16.98 M → 13.27 M |
| nmapset-int      | 116.7 → 92.6 ms         | −21%       | 61.3 ms             | 24.80 M → 21.10 M   |
| json-decode      | 675.2 → 676.2 ms        | ~0 (parity)| 62.3 ms            | 111.32 M (=)        |

(`=` bit-identical. Exec medians for the typed lanes: nmapset-str
54.9 ms −74%, nmap-knucleotide 216.1 ms −71%, nmap-hashset 40.4 ms
−31%, nmapset-int 81.7 ms −17% — the phase-4 log has the full
decomposition. Close-out verification re-run: full suite, all three
runtimes, exit 0 — every row equals `expected.json`; the probe's fuel
and VM-heap peaks are bit-identical on every nmap/json row (json
665.0 ms / 111.32 M / 32.78 MB; nmapset-str 57.1 ms; nmap-knucleotide
207.9 ms; nmap-hashset 42.8 ms / 503 B; nmapset-int 82.8 ms), and the
str rows' drift (nmapset-str net 63.9 ms this run) is the host's
known day drift, not a code change — nothing has moved since phase 6.)

Behavior deltas for the whole batch: none beyond performance, plus ONE
admission change — user-defined nmapset keys now fail at COMPILE time
(the union-bound diagnostic names the offending type and the allowed
set) where nmapset's deleted `Hashable` used to trap at runtime;
mapset is untouched. Checksum law held throughout: every map row stays
bit-for-bit equal to its mapset twin (`734932704` / `1264308351` /
`21500055` / `2198604`), json-decode stays `4502015958359127277`, and
`expected.json` was never touched. Stop-points (decision 9): never
triggered — no typed lane measured slower than its baseline, pooling
held the json row at parity (fuel and heap bit-identical, exec within
±1.1%). Deferred, not forgotten: borrowed crossings (their own
RFC-level proposal), the str `encode()`-per-hash cost in mapset's own
rows, and closing the union-provenance holes (RFC 0043 §3).

## Performance log — crossing-nop: the crossing-tax baseline (Sep 2026)

New row for the crossing-fastpath batch, phase 0. `crossing-nop` mounts
`rut/bench-cross` (host pkg: `nop(x: i64) -> i64`, the identity, and
`nop4(a..d: i64) -> i64`, the sum — bodies deliberately EMPTY, bound
infallibly in rut-std's `install_std_bench_cross`) and runs four loops
of 2M iterations each: **A** = N host `nop` calls, **B** = N calls of an
inline rut fn with the SAME body, **A4**/**B4** = the same pair over the
4-arg sum. Every call does no work beyond its own signature, so the
host-minus-inline delta is the rut→host **crossing** — the per-call
package (arg snapshot, adapter, dispatch, write-back) the batch's
phases 1-2 mean to shrink.

**This entry is the pre-optimization baseline** — recorded before
phases 1-2 landed, exactly like the json-decode entry before it; the
phase 1/2 comparisons read their before columns HERE. No optimization
numbers belong in this section.

All three runtimes agree on the checksum (twin gate, `expected.json`
pins it): `20000014000000`. Full row (net medians, same-day run, 3 reps
+ 1 warmup, probe over 3 fresh-VM iters):

| workload     | rut net  | qjs net  | node net | rut exec  | fuel     | VM heap peak |
|--------------|----------|----------|----------|-----------|----------|--------------|
| crossing-nop | 121.3 ms | 521.1 ms | 4.5 ms   | 118.0 ms  | 104.00 M | 236 B        |

Placement: the JS twins are plain fn-call loops — JS has no host
boundary, so both loops of each pair measure the same plain-call work.
qjs, an interpreter with no call inlining, is the slowest runtime on
the row by far (521.1 ms for the identical 8M calls, ~115x node);
node's JIT inlines the nops to nothing (4.5 ms net). rut's 121.3 ms net
is the crossing package on top of its interpreter loop — and it beats
qjs on a call-shaped row for the first time, which is the honest shape
of "an interpreter that dispatches one host op per call" vs "an
interpreter that interprets every call". The 236 B heap peak is the
point: the row holds no allocation at all — it is call machinery only.

The decomposition (the numbers the phases compare against): the
committed row runs all four loops, so the segments were timed as
single-loop clones of it (same body, same deps, fresh process, 5
interleaved rounds × 7-9 fresh-VM probe iters, median of the round
medians; fuel counts are exact):

| segment        | exec median | per call | fuel/iter | what it is                       |
|----------------|-------------|----------|-----------|----------------------------------|
| A  — host nop  | 23.98 ms    | 12.0 ns  | 8 ops     | one crossing per call            |
| B  — inline nop| 18.24 ms    | 9.1 ns   | 12 ops    | same body, no crossing           |
| A4 — host nop4 | 48.88 ms    | 24.4 ns  | 14 ops    | one crossing + 3 extra params    |
| B4 — inline n4 | 27.42 ms    | 13.7 ns  | 18 ops    | same body, no crossing           |

Baseline readings, per the plan's model:

- **(A−B) = total crossing overhead**: +2.9 ns/call at one param
  (5.74 ms per 2M) and **+10.7 ns/call at four params** (21.47 ms per
  2M). The 4-param shape is where the tax lives — that is the shape
  nmapset's one-crossing-per-op ops pay.
- **(nop4−nop) = per-param slope**: host-side +12.4 ns/call for the 3
  extra params (~4.1 ns/param); crossing-only (A4−B4 minus A−B)
  ~+2.6 ns per extra param. The per-param marshalling
  (`expect_kind` + `narrow_i64` per param, boundary.rs) is exactly
  what phase 1 removes.
- Fuel facts: the host loops run FEWER VM ops than the spliced twins
  (8 vs 12, 14 vs 18 per iter) — the inline twin bodies splice into
  the caller (P1.3 free-fn inlining), so B/B4 are the no-crossing
  floors and A−B is net of that op-stream difference. The four
  segments' fuel sums exactly to the row's 104,000,032.
- Method note: segment times are single-loop clones probed in fresh
  processes and interleaved across rounds; the committed row runs the
  four loops back to back (its probe exec 118.0 ms ≈ the segments'
  118.5 ms sum, cross-checked exactly in fuel: 16+24+28+36 M =
  104,000,032). Day drift on this host is ±8-13% (see the mapset-perf
  log) — phases 1-2 should re-run the same clones same-day when they
  claim their deltas.

## Performance log — the host-param fast lane: checked-once reads (Sep 2026)

Phase 1 of the crossing-fastpath batch. The host-fn entry adapters (the
`HostSlot` code pointers — the rut→host hot lane, shared by both interp
engines) now read params through an **unchecked fast lane**
(`HostParam::read`, `crates/rut-vm/src/interp/boundary.rs`): prim
params are raw slot-bit reads — `slot.i` reinterpret + cast, u64 keeps
its raw-bit read, floats/bool/char the raw union reads — and the
per-param `expect_kind` (a `types.kind()` lookup + string-compare match
chain re-verifying that boot type TY_U8 has kind Prim(U8), a tautology
the join already proved), the `narrow_i64` range check (the checker
already proved the fit at the call site), and the string compares are
GONE. What stayed guards genuinely dynamic facts only: the nil check on
`OpaqueRef`/`OpaqueBox<T>` params (the transitive `??T` coercion funnel
leaves "nil cannot reach here" unproven), the cell-kind match on
`&str`/`&[u8]` (that IS the read), and the owned `String`/`Vec<u8>`
copies. The historical checked read stays in-source as
`HostParam::read_checked` (semantics and trap messages preserved) for
debug verification; embedder marshalling (`Vm::call`, `value_in`,
`Ret::from_slot`) keeps every check — that input is Rust-shaped and
`value_in` checks it before slots exist. The SAFETY case is in-source:
the join verified the binding against the mounted `.d.rut` row
(`verify_against`, RFC 0025, pre-boot panic) and the checker typed the
call site against the same row, so the shape was **checked once**,
upstream of every call. No `.d.rut` surface change, no repr change, no
version bump.

Same-day interleaved A/B on the segment clones (baseline commit
ae6ceb4's binary vs the fast-lane build, 5 rounds × 9 fresh-VM probe
iters per binary per segment, median of round medians — the method the
baseline section prescribes; the recorded phase-0 befores reproduce
within 0.6 ms: A 23.98→23.83, B 18.24→18.36, A4 48.88→48.60, B4
27.42→27.39):

| segment           | exec before | exec after | Δ         | per call        |
|-------------------|-------------|------------|-----------|-----------------|
| A  — host nop     | 23.83 ms    | 19.19 ms   | **−19.7%**| 12.0 → 9.6 ns   |
| B  — inline nop   | 18.36 ms    | 18.27 ms   | −0.5%     | 9.1 ns (floor)  |
| A4 — host nop4    | 48.60 ms    | 31.80 ms   | **−34.7%**| 24.4 → 15.9 ns  |
| B4 — inline nop4  | 27.39 ms    | 27.38 ms   | −0.04%    | 13.7 ns (floor) |

The crossing reads (A−B, and the per-param slope), before → after:

- **(A−B) @1 param**: +2.73 → **+0.46 ns/crossing** (5.47 → 0.92 ms
  per 2M calls).
- **(A4−B4) @4 params**: +10.61 → **+2.21 ns/crossing** (21.21 → 4.42
  ms per 2M).
- **per-param slope**: +2.62 → **+0.58 ns/param** — the
  `expect_kind`+`narrow_i64` package the plan targeted is gone; what
  remains is the `call_host` frame package (the `Rc::clone` borrow
  split and the `is_ref(slot.ret)` write-back lookup), which is phase
  2's.

The committed row (all four loops; B/B4 are half of it, so NET is
nearly flat and exec is the honest comparator): rut exec 118.0 →
96.8 ms (−18%), net 121.3 → 121.1 ms, fuel 104,000,032 and VM heap
236 B **bit-identical** (pure host-side work; the op stream cannot
move). Checksum `20000014000000` equal on rut/qjs/node.

Downstream rows, same-day interleaved probe A/B (base binary vs
fast-lane build, 3 rounds × 5 iters; the crossing pays once per host
op, so these should all be same-or-faster):

| workload    | exec before | exec after | Δ      |
|-------------|-------------|------------|--------|
| nmapset-int | 95.2 ms     | 89.6 ms    | −5.8%  |
| nmapset-str | 60.7 ms     | 58.4 ms    | −3.7%  |
| json-decode | 674.6 ms    | 657.2 ms   | −2.6%  |
| nmap-hashset| 42.9 ms     | 39.7 ms    | **−7.5%**|

Honest note on nmap-hashset: a first 3-round pass measured it +3.7%
(43.6 vs 42.0) — a dedicated 5-round × 7-iter pass flipped it to −7.5%
(39.7 vs 43.0, round medians 38.8-39.9 vs 42.3-45.3, fuel
13,267,176 bit-identical both directions); the +3.7% was jitter on a
±2 ms row, and the powered run is the record. No row regressed on its
powered measurement — **stop-point (§0.5) not triggered**, and the
inline floors B/B4 moving −0.5%/−0.04% rules out a global codegen
layout effect.

Full suite after: all 26 rows checksum-equal to `expected.json` on
rut/qjs/node (exit 0, every row `yes yes`). Tests: the boundary suite
gained `fast_lane_tests` (prim bit-fidelity incl. the u64 high half
through `entry`, the nil trap through BOTH adapter families, the
box payload-token trap, the borrow kind match, the owned copies, and
the fast/checked split demonstrated on one slot — 300 crosses as
`44i8` unchecked and traps "does not fit" checked), plus the
`rut-cli` `fastlane` driver tests over a real `.d.rut` pkg
(`tests/data/fastlane`): end-to-end round-trips, borrow/owned shapes,
and the embedder wrong-shape traps unchanged.

## Performance log — call-frame trims: the fixed per-call work (Sep 2026)

Phase 2 of the crossing-fastpath batch, on top of the fast lane. Three
trims to the crossing's fixed frame, both engines at once (the threaded
engine's `Op::Call` delegates to the SAME `Vm::call_host` — one
implementation, so one edit covers both; lock-step by construction):

- **the per-call `Rc::clone(&self.prog)` is gone**
  (`crates/rut-vm/src/interp/mod.rs` `call_host`): it existed only to
  split borrows — the argv slice had to outlive the `&mut self` the arg
  snapshot takes. The slice is now carried by a raw pointer derived
  from the `Rc` (`Rc::as_ptr` + deref, SAFETY case in-source: `prog`
  is assigned once at construction, never reassigned; only immutable
  argv-table words are read — the same trust the threaded loop's raw
  pointers run on, RFC 0034). No refcount traffic, no new field.
- **`ret_is_ref: bool` precomputed into `HostSlot` at the join**
  (`crates/rut-vm/src/interp/host.rs`): the write-back refcount law
  used to re-derive `is_ref(slot.ret)` per call (a `type_repr` lookup);
  the join now freezes the exact `Vm::is_ref` predicate as a bit. The
  bool packs into the struct's existing padding hole — `HostSlot` stays
  24 bytes and `Copy` (measured, not assumed).
- **`#[inline]` on the hot `HostParam::read` / `Ret::into_slot` impls**
  (`crates/rut-vm/src/interp/boundary.rs`) — the fast lane and the
  slot-builders are single-instruction bodies; the checked twins stay
  unannotated (deliberately cold).

No adapter semantics, `HostParam`, or phase-1 surface changed; fuel
(104,000,032), VM heap (236 B), and checksum `20000014000000` are
bit-identical (pure host-side work; the op stream cannot move).

Same-day interleaved A/B on the segment clones (phase-1 commit 2f362f9's
binary vs the trim build — **both built from the SAME directory**, one
checkout, before-source then after-source, so the embedded build path —
and therefore codegen layout — is identical between the pair; 5 rounds
× 7 fresh-VM probe iters per binary per segment, order alternated per
round, median of the round medians):

| segment           | exec before | exec after | Δ         | per call        |
|-------------------|-------------|------------|-----------|-----------------|
| A  — host nop     | 19.60 ms    | 18.13 ms   | **−7.5%** | 9.8 → 9.1 ns    |
| B  — inline nop   | 18.27 ms    | 18.18 ms   | −0.5%     | 9.1 ns (floor)  |
| A4 — host nop4    | 32.33 ms    | 31.24 ms   | **−3.4%** | 16.2 → 15.6 ns  |
| B4 — inline nop4  | 27.56 ms    | 27.48 ms   | −0.3%     | 13.7 ns (floor) |

The crossing reads, before → after: **(A−B) @1 param**: +0.66 →
**+0.00 ns/crossing** — the 1-param crossing is now AT the inline
floor, within measurement noise. **(A4−B4) @4 params**: +2.39 →
+1.88 ns/crossing. **per-param slope**: ~0.57 → ~0.64 ns/param —
unchanged within noise (the slope is arg-snapshot + adapter-shape work,
not the frame; that is the honest residual and it is not this phase's).
Phase 0 put the fixed intercept at ~0.3 ns; the trims recovered about
that plus layout goodwill: the headline is **A −7.5% / A4 −3.4% with
the no-crossing floors flat (−0.5%/−0.3%)** — the win is real but
small, exactly the size the phase-0 decomposition predicted.

Method note (a trap for the next batch): cross-directory A/B builds are
NOT layout-safe on this host. The first measurement pass compared a
worktree-built baseline against a shared-tree-built after-binary — the
two builds embed different path strings, shifting constant-pool layout,
and the floors read B +18% / B4 +33% (while the host segments read
faster). Rebuilding BOTH sides from one directory reproduced the floors
at parity, exposing the first reading as a build-placement artifact.
Same-path pairs are the house method from here on.

Downstream rows, same-path interleaved probe A/B (3 rounds × 3 iters;
json-decode powered to 5 × 3 after a first-pass +4.2% flag):

| workload     | exec before | exec after | Δ                        |
|--------------|-------------|------------|--------------------------|
| crossing-nop | 97.51 ms    | 95.25 ms   | **−2.3%**                |
| nmapset-int  | 99.65 ms    | 97.76 ms   | −1.9% (noisy hour)       |
| nmapset-str  | 61.17 ms    | 60.47 ms   | −1.1%                    |
| nmap-hashset | 43.22 ms    | 40.61 ms   | **−6.0%**                |
| json-decode  | 676.5 ms    | 679.2 ms   | +0.4% — parity (neutral) |

Honest note on json-decode: the 3×3 pass flagged +4.2%; a dedicated
5-round pass read +0.4% (round medians before 660.9-680.2 vs after
676.3-688.2, overlapping; fuel 111,322,915 and heap bit-identical both
directions) — recorded as NEUTRAL, not a win and not a regression. No
committed row regressed on its powered measurement — **stop-point
(§0.5) not triggered**; the flat floors rule out a global codegen cost
of the trim itself.

Full suite after: all 26 rows checksum-equal to `expected.json` on
rut/qjs/node (exit 0). Workspace tests green with the phase-1
fast-lane suites untouched and passing (`fast_lane_tests` 8/8,
`fastlane` driver 4/4).

## Performance log — crossing-fastpath batch close-out: the verdict (Sep 2026)

The batch's final record. Two phases landed against the crossing-nop
row: the host-param fast lane (phase 1 — the per-param `expect_kind`,
`narrow_i64` and string compares dropped from the rut→host hot lane in
favor of raw slot-bit reads; genuinely dynamic checks stayed) and the
call-frame trims (phase 2 — the per-call `Rc::clone(&self.prog)`, the
per-call `is_ref(slot.ret)` write-back lookup, `#[inline]` on the hot
slot builders). Phase 3 measured and reported only — no engine file
changed after phase 2; this section is the verdict.

**Checksum gate (§0.4):** full suite from one checkout, all 26 rows ×
rut/qjs/node, exit 0 — every checksum equal to `expected.json`
(crossing-nop `20000014000000`, json-decode `4502015958359127277`, the
map rows `734932704` / `1264308351` / `21500055` / `2198604`).
`expected.json` was never touched all batch.

**crossing-nop before/after vs the phase-0 baseline.** Segment A/B
(the phase-0 commit ae6ceb4's binary vs batch-HEAD c40edf8 — **both
built from ONE checkout directory**, 5 interleaved rounds × 7 fresh-VM
probe iters per binary per segment, order alternated per round, median
of the round medians; the before-side reproduces the recorded phase-0
numbers within 2.3%: 23.95/18.24/47.75/27.42 vs 23.98/18.24/48.88/
27.42):

| segment           | exec before | exec after | Δ          | per call        |
|-------------------|-------------|------------|------------|-----------------|
| A  — host nop     | 23.95 ms    | 18.22 ms   | **−23.9%** | 12.0 → 9.1 ns   |
| B  — inline nop   | 18.24 ms    | 18.36 ms   | +0.6%      | 9.1 ns (floor)  |
| A4 — host nop4    | 47.75 ms    | 31.33 ms   | **−34.4%** | 23.9 → 15.7 ns  |
| B4 — inline nop4  | 27.42 ms    | 27.15 ms   | −1.0%      | 13.7 ns (floor) |

The crossing reads, before → after:

- **(A−B) @1 param**: +2.86 → **−0.07 ns/crossing** — the 1-param
  crossing is AT the inline floor (the sign flips inside noise; phase
  2's same-day pair read +0.00).
- **(A4−B4) @4 params**: +10.17 → **+2.09 ns/crossing**.
- **per-param slope**: +2.44 → **+0.72 ns/param** (phase 2 read ~0.64
  the same way — call it ~0.6-0.7).

Against the phase-0 record (A 23.98 / A4 48.88 ms per 2M): cumulative
**A −24% / A4 −36%**. The plan's ~10-15 ns/crossing re-verification
estimate was, in the end, mostly recovered at 4 params (+10.7 →
+2.1 ns) and entirely at 1 param (at the floor).

The committed row (all four loops): rut exec 118.0 → 96.3 ms (−18%),
net 121.3 → 96.2 ms (−21%), and — as the law demands for host-side-only
work — **fuel 104,000,032 and VM heap peak 236 B are bit-identical** to
the phase-0 record (the op stream cannot move; the verdict run read
both exact).

**Downstream rows, cumulative vs the phase-0 baseline** (same-day
interleaved probe A/B, 5 rounds × 3 fresh-VM iters, order alternated;
the crossing pays once per host op, so every nmap/json row inherits
both phases):

| workload         | exec before | exec after | Δ                        |
|------------------|-------------|------------|--------------------------|
| nmapset-int      | 90.35 ms    | 85.74 ms   | **−5.1%**                |
| nmap-hashset     | 42.29 ms    | 40.32 ms   | **−4.7%**                |
| json-decode      | 672.4 ms    | 665.9 ms   | −1.0%                    |
| nmap-knucleotide | 209.7 ms    | 208.5 ms   | −0.5% — NEUTRAL          |
| nmapset-str      | 57.31 ms    | 57.75 ms   | +0.8% — NEUTRAL (noise)  |

Honest notes. This hour read the str row NEUTRAL where phase 1's
same-day pass had −3.7% and phase 2's −1.1%: the round medians overlap
(after 55.6-60.8 vs before 57.1-59.4 ms) and the row's own history in
this log (jitter on ±2 ms segments) says noise — recorded as neutral,
not a win. nmap-knucleotide was never tier-measured in phases 1-2; its
~1.7 M crossings × a few ns recovered predicts −1-2% and it reads
−0.5% — consistent with a row dominated by k-mer materialization, not
the map op. json-decode's −1.0% is within the parity band its phase-2
entry already recorded (+0.4%); call it parity-to-slightly-better. Fuel
and VM-heap peaks were **bit-identical before/after on every row**
(13,267,176 / 21,103,284 / 9,735,095 / 39,612,955 / 111,322,915 ops;
503 B / 5.80 MB / 2.90 MB / 11.85 MB / 32.78 MB). **Stop-point (§0.5):
never triggered** — no row regressed beyond noise on its interleaved
measurement, and the flat floors (B +0.6% / B4 −1.0%) rule out a
global codegen effect of the batch.

Method note (the house rule this batch leaves behind): both binaries
of every A/B pair above were built from ONE checkout directory
(before-source, then after-source, in place). Cross-directory builds
embed different path strings, shift constant-pool layout, and are NOT
layout-safe — phase 2's trap note records a worktree-vs-tree pair
reading the no-crossing floors +18%/+33% before the same-path rebuild
exposed it as a build-placement artifact.

The residual, stated honestly: the 1-param crossing sits AT the inline
floor — A and B are the same speed within noise, i.e. the fixed
per-call package (frame, arg snapshot, indirect call, write-back) now
costs no more than the rut-side call it replaces. What remains is the
per-param slope, ~0.6-0.7 ns/param: each parameter's slot copy through
the snapshot plus the adapter's raw-bit read and the return write.
That is memory movement, not checking — irreducible without a repr
change (registers across the boundary, or the host reading caller
slots in place), which §0.1 takes off the table for this batch.

## Performance log — nmapset-round2 phase 0: dispatch shape + sidecar ceiling (Sep 2026)

Evidence phase for the wrapper-residual batch: three findings, no engine
change, nothing committed but this section. The nmapset-int row sits
~1.3× its qjs twin after crossing-fastpath; the two candidate levers are
wrapper-side — the `KeyLane` trait dispatch at the instantiated body,
and the boxed `[?V]` sidecar store/load. This phase measures both and
states the per-op budget. Row baselines re-measured for this batch
(probe, fresh-VM iters, same day, one binary; fuel and VM-heap peaks
**bit-identical** to the close-out records — nothing has moved):

| workload         | exec median      | fuel          | VM heap peak     |
|------------------|------------------|---------------|------------------|
| nmapset-int      | 77.8 ms (A/B below) | 21,103,284 | 6,082,111 B (5.80 MiB) |
| nmap-hashset     | 38.9 ms          | 13,267,176    | 503 B            |
| json-decode      | 727.9 ms         | 111,322,915   | 34,377,027 B (32.78 MiB) |
| crossing-nop     | 97.9 ms          | 104,000,032   | 236 B            |

### Finding 0a — KeyLane dispatch is already DIRECT (spliced to the lane crossing)

Method: the crossing-fastpath phase-0 dump facility (`ir_dump_of` over
the linked graph program), used from a throwaway driver test (scratch
file, deleted after capture — dumps preserved in the batch run dir):
instantiate `HashMap<i64, i64>` (and `<i32, i32>`, the row's shape), and
dump the actual bench dir's program. What the binary contains:

- The wrapper does not survive as functions. `HashMap.put/get/has/remove`
  are **fully spliced into the instantiated consumer body** (the pkg is
  `inline = true` — source-inlined into the consumer's one compilation
  unit — then monomorphized per instantiation, then the tiny-body
  splicer inlines the `KeyLane` impl bodies). The bench row's `churn`
  is one 389-op function containing all seven loops.
- `k.nentry(self.t)` lowers to `call f9` — a **direct** call to f9,
  which is the host lane slot `map_entry_i` (an empty-code
  host-declared func; the engines route `Op::Call` on `host_id` through
  `Vm::call_host` — the exact crossing crossing-nop calibrates). Same
  shape everywhere: `get`/`has` → `call f14 map_find_i`, `remove` →
  `call f19 map_remove_i`.
- **Zero `CallI` (trait-vtable) ops in the entire program** — both the
  scratch instantiations and the bench row. The 33 compiled
  `nentry/nfind/nremove` impl bodies (11 key types × 3) are dead code
  in the instantiation: nothing references them.

Excerpt (the bench row's `churn`, build loop — key arithmetic, the
crossing, the grow-sentinel check, the `vals` store; no dispatch, no
wrapper frame):

```
 8 wmuli.i32 r9, r3, r8          ; key = i * -1640531535
10 getf r12, r1, f0 :12          ; self.t (the opaque table)
12 conv r15, r9, i32 -> i64
13 call f11(r14, r15) -> r13     ; map_entry_i(t, k) — DIRECT, the crossing
15 loophead                      ; grow-sentinel retry head
18 lei.i32 r20, r13, r19         ; at <= i32::MIN ?
56 getf r54, r1, f1 :12          ; self.vals
57 makeopt r57, r10              ; box the i32 into a one-slot cell
58 arrset r54, r13, r57 :12      ; vals[at] = v
```

**Verdict (§0.5 gate for phase 1): SKIP devirt.** The trait call never
reaches the binary — the sealed-private-trait + inline-pkg + splice
pipeline already resolves it per-instantiation, one step past what a
devirtualization pass would produce (it removes not just the vtable hop
but the whole wrapper frame). A devirt phase would have nothing to do;
this finding is its record.

### Finding 0b — the sidecar ceiling: 45.3% of the row (≥ 10% → phase 2 runs)

Method: a throwaway stub of `rut/nmapset/nmapset.rut` (NEVER committed —
restored with `git checkout --`, tree verified clean, row checksum
re-verified `734932704` after restore): `put` drops the `vals` stores
and the whole grow-relocation drain (the `HashSet` shape — `map_grow` +
retry, no vals machinery); `get` returns `nil` after the `nfind`
(hit/miss control flow kept); `remove` drops the nil store. Checksum
integrity under the stub: every counter (added/replaced/hits/misses/
removed/present/added2/len) is unchanged except `sum = 0, hits = 0` —
deterministic `25150000` on every run, and it reconciles bit-exact:
25,150,000 + Σ(i+7) 705,682,704 + hits·41 4,100,000 = **734,932,704**.
Control: `nmap-hashset` still checksums `21500055` under the stub
(HashSet never touches vals — the stub provably moves only the sidecar).

A/B mechanics, and a method simplification the measurement exposed:
pkg sources (`rut/nmapset`) are **runtime-mounted, not compiled into
the binaries** — the probe binary never rebuilds when the wrapper
changes. The A/B therefore ran as ONE fixed binary with the wrapper
source flipped between interleaved rounds (`git checkout --` restores
A, a saved copy applies B): 5 rounds × 7 fresh-VM iters per side, order
alternated per round. Same-day, one tree, zero build-placement risk —
the house rule's intent with even less layout confound than the
two-binary method.

| side             | round medians (ms)                  | median of medians | fuel       | heap peak  |
|------------------|-------------------------------------|-------------------|------------|------------|
| A — unstubbed    | 82.1, 77.8, 75.2, 78.0, 74.4        | **77.84 ms**      | 21,103,284 | 6,082,111 B|
| B — vals-stubbed | 42.6, 42.4, 43.3, 42.4, 43.7        | **42.57 ms**      | 15,650,305 | 423 B      |

Round ranges do not overlap (A 74.4–82.1 vs B 42.4–43.7): the sidecar
package — the `MakeOpt` cell mint + retain per put store, the release on
overwrite, the random-access `arrget` + cell deref per get, the grow-time
relocation drains — costs **35.3 ms of the 77.8 ms row = 45.3%**. Fuel
drops 5,452,979 ops (−25.8%) and the VM heap high-water collapses
6.08 MB → 423 B: the cells behind the `[?i32]` sidecar are the entire
live heap of the row.

Honest scoping: 45.3% is the CEILING — it removes ALL vals work,
including the relocation drains and the deref on read. Phase 2's
primitive-store keeps raw element traffic (raw slot stores/loads plus
relocation copies), so its realizable win is strictly less than the
ceiling; the gate asks only whether the lever is ≥ 10%, and it clears
that with 4.5× margin.

**Verdict (§0.5 gate for phase 2): RUN primitive-store.**

### Finding 0c — the per-op budget: where a 130 ns map op goes

Calibration, re-read today with this binary (the segment clones from
crossing-nop phase 0): segment A (2M host `nop` calls) 20.98 ms →
**10.5 ns/call**, 8 fuel-ops/iter; segment B (inline twin) 18.46 ms →
9.2 ns/call, 12 ops/iter — **0.77 ns per VM op**. The row crosses the
boundary exactly once per map op (600k crossings for n = 100k: 2.5
puts, 2 gets, 1 has, 0.5 remove per key), each a direct `Op::Call` on a
host slot — fuel-wise that is exactly 1 op of the stream; time-wise the
crossing frame's marginal cost over an inline call is the crossing-nop
A−B delta (recorded −0.07…+2.9 ns; today's read +1.3), so ≤ ~1.7 ms of
the row, under 2.5%.

Fuel per op shape, from the spliced `churn` dump (steady-state dynamic
paths): **put ≈ 35 ops** (loop control 6, key arithmetic 4, arg prep 4,
crossing 1, sentinel check 4, insert path + counter 15, grows
amortized ~2); **get ≈ 30 hit / 24 miss** (hit adds the `vals` load:
`getf` + `arrget` + the cell `getf` deref); **has ≈ 24**; **remove ≈ 28**
(minus the nil store under the stub's accounting). Measured total
21,103,284 / 600k = 35.2 avg ✓; the stubbed side's 15,650,305
(−5.45 M) reconciles with those sidecar op counts (put stores ~4-5 ops
each, get loads ~5-6, relocation drains ~2.3 M across 14 grows).

Where the fuel goes: of 21.1 M ops, **25.8% is sidecar traffic**
(measured by the stub), **0% is trait dispatch** (finding 0a — spliced
away), and the remaining ~74% is loop control, key arithmetic
(`wmuli`), branch traffic and arg prep — the irreducible shape of a
generic rut loop calling a host table.

Where the TIME goes (77.8 ms row, ~130 ns per map op; the residual
terms are estimates against the calibration — honesty over precision):

| component                                    | time        | share |
|----------------------------------------------|-------------|-------|
| the `[?V]` sidecar package (measured, 0b)    | 35.3 ms     | **45%** |
| VM interpreter stream ex-sidecar (15.65 M ops × 0.77 ns) | ~12.1 ms | ~16% |
| the host table body itself (mix64 + open-addressing probe over multi-MB keys/hashes/states — cache-miss bound, invisible to fuel) | ~29 ms | ~37% |
| the crossing frame (600k × ≤ 2.9 ns)         | ≤ 1.7 ms    | ≤ 2%  |

The budget's headline: the wrapper is already optimal-shaped (one
direct crossing per op, no dispatch, no frame), but the values still
live in boxed cells — the single largest cost in the row is the
sidecar's boxing and cell traffic, and the second largest is host-side
probing that no wrapper change can touch.

Deviations, recorded: (1) the stub removed the relocation drain
entirely instead of draining-without-copy — the drain exists only
because `vals` exists, and the plan's own `HashSet` precedent calls
that shape "no vals machinery at all"; (2) the A/B used one fixed
binary with runtime source flips instead of two binaries from one
checkout — possible only because pkg sources are runtime-mounted, and
strictly stronger on layout isolation; (3) the IR dumps came from a
scratch driver test, deleted after capture (untracked; the committed
tree is this README section only).

## Performance log — nmapset-round2 phase 1: devirt skipped (Sep 2026)

Docs-only record of a skip, per the batch's one-commit-per-phase rule.
Phase 1 (devirtualize sealed trait impls) was conditional on finding 0a
showing indirect dispatch at instantiated body-compile; it shows the
opposite, so there is nothing to build — this note plus an observation
line in RFC 0031 is the entire phase.

Why the lever is dead: the wrapper never reaches the binary as a
dispatch point. `HashMap.put/get/has/remove` are source-inlined (the
pkg is `inline = true`) and monomorphized per instantiation, the
tiny-body splicer then inlines the `KeyLane` impl bodies, and the
bench row's churn is one 389-op function whose `k.nentry(self.t)` is a
direct `call` onto the host lane slot (`map_entry_i`; `get`/`has` →
`map_find_i`, `remove` → `map_remove_i`). Zero `CallI` trait-vtable
ops in the program; trait dispatch is **0% of the row's fuel**
(finding 0c). Per-instantiation compilation (RFC 0031 §2) already
devirtualizes private-trait calls — one step past what a
devirtualization pass would produce, since the wrapper frame is gone
along with the vtable hop.

Observed artifact, recorded and deliberately left alone (out of
scope): the 33 compiled `nentry/nfind/nremove` impl bodies
(11 key types × 3) are dead code under instantiation — nothing
references them; they serve the general, non-instantiated compile
path.

No engine, no stdlib, no bench change; `expected.json` untouched;
tree clean.

## Performance log — nmapset-round2 phase 2: the primitive-optional store (Sep 2026)

The `[?V]` sidecar's raw repr: an array whose element type is `?prim`
stores each element as the raw payload plus a one-byte nil tag instead
of a boxed one-slot cell, element ops become repr-keyed raw
reads/writes, and the module binary VERSION bumps 5 → 6. This is the
tier the phase-0 ceiling measurement gated on (45.3% ≥ 10%).

**Design, and why it changed shape from the plan's default.** The plan
offered "interpreter-level branch on store kind OR compile-time
kind-specialized ops — your judgment per the codebase's op lowering;
measure if in doubt." Measurement reasoning settled it before a line of
bench code existed: a read of a `?prim` element MUST build a proper opt
value in a register (nil is the null slot; raw bits cannot also encode
nil), so the pure runtime-branch design mints a fresh cell on every
read (~15-20 ns vs today's ~6 ns retain/release pair). The `array` row
alone reads `[?i32]` elements ~3 M times — the runtime-branch design
regresses that row by roughly the very regression this phase is the
recovery target for, tripping its own stop-point. The compile-time form
was landed instead: array element ops bake a new `Repr::OptPrim`
family, and one new fixed-point peephole pass performs two local folds
that remove the box from the hot path entirely —

- **store elision**: `makeopt d, v` feeding only the adjacent
  `arrset{repr: optprim}` is folded to `arrset{repr: optprimraw, val:
  v}` — the store writes the raw payload; the box never exists (sound
  because a primitive box's identity is unobservable, RFC 0044 §3's
  payload-compare law). The phase-0 dump's `makeopt r57, r10; arrset
  r54, r13, r57 :12` sequence is now a single `arrset .. :33`.
- **deref fold**: `arrget{repr: optprim}` feeding only the adjacent
  `getf d, r, f0` (the exact pair the RFC 0044 auto-deref emits)
  becomes one `arrget{repr: optprimload}` — the payload comes straight
  out of the raw store, a nil tag still traps `NilDeref`, and the
  fresh opt value is never minted.

**Repr + heap repr.** `ArrKind::Opt(PrimTy)`: stride `payload width +
1`, tag byte LAST, one fixed-stride region — every block-size
computation rides `width()` unchanged, payload and tag share a cache
line for ≤7-byte payloads, and a zeroed block is all-nil (the
`[nil; cap]` memset law, RFC 0015 §5). A nil store also zeroes the
payload, so raw-byte views of the block stay deterministic. The
release walk collects no children from these arrays (the plan's
"drop/release skips slot release" — this is where the heap high-water
collapses), `own` copies the raw block, and `array_eq` falls into the
raw-byte compare (value equality — the determinism rule makes it
meaningful). Reference payloads (`?str`, `?record`, `??T`) keep the
cell backing and the `Ref` repr — json/knuc/str-key paths execute
identical ops (their fuel/heap parity is checked below). Both engines
route the element ops through shared helpers, and the churn runs the
threaded dispatch, so the qjs-parity law holds by construction.

| workload         | exec before | exec after | Δ          | fuel before → after     | VM heap before → after    |
|------------------|-------------|------------|------------|-------------------------|---------------------------|
| nmapset-int      | 77.59 ms    | 63.19 ms   | **−18.6%** | 21,103,284 → 20,703,284 | 6,082,111 → 1,966,527 B   |
| nmapset-str      | 56.68 ms    | 53.08 ms   | **−6.4%**  | 9,735,095 → 9,551,761   | 3,041,356 → 983,596 B     |
| nmap-knucleotide | 202.53 ms   | 177.00 ms  | **−12.6%** | 39,612,955 → 38,814,389 | 12,430,420 → 4,195,060 B  |
| hashmap-int      | 172.35 ms   | 134.86 ms  | **−21.7%** | 64,758,210 → 63,758,210 | 17,174,663 → 8,258,103 B  |
| hashmap-str      | 251.18 ms   | 249.97 ms  | −0.5%      | 111,489,939 → 111,306,605 | 10,287,538 → 8,294,322 B |
| hashset          | 121.58 ms   | 113.22 ms  | **−6.9%**  | 56,230,024 → 55,696,689 | 13,158,071 → 7,832,407 B  |
| knucleotide      | 952.18 ms   | 925.88 ms  | **−2.8%**  | 418,769,549 → 417,970,983 | 43,516,672 → 35,285,408 B |
| array            | 267.56 ms   | 210.01 ms  | **−21.5%** | 63,486,108 → 60,486,108 | 40,389,042 → 7,864,540 B  |
| sieve            | 155.97 ms   | 43.10 ms   | **−72.4%** | 21,167,665 → 19,592,209 | 21,853,906 → 1,491,846 B  |
| alloc            | 17.46 ms    | 17.31 ms   | −0.9%      | 22,000,020 (identical)  | 228 B (identical)         |
| json-decode      | 671.5 ms    | 677.9 ms   | +0.9%      | 111,322,915 (identical) | 34,377,027 B (identical)  |

Method: interleaved A/B, both probe binaries built from THIS checkout
directory (before-source = b5ff868 rebuilt in place, then after-source
in place; the after binary re-verified byte-identical after a second
rebuild), nmapset-int at 5 rounds × 7 fresh-VM iters, the rest 5 × 3,
order alternated per round, medians of round medians. The headline:
**nmapset-int 77.84 → 63.19 ms against the batch baseline (−18.8%),
with the sidecar heap collapsing 6.08 MB → 1.97 MB (−68%)** — the
phase-0 floor (42.57 ms) stays unreachable because a hit-`get` must
still mint the fresh `?i32` it returns; the win is the box and the
stored-cell traffic, exactly where finding 0c placed the sidecar cost
(the fuel delta is only −400 k ops = the deleted `makeopt`s and folded
loads; the −18.6% is host-side mint/RC/release work fuel never
counted).

Honest neutrals and the stop-point sweep:

- **nmap-hashset** read +3.0% then +5.9% on consecutive 5×3 passes —
  the stop-point investigation, in order: the row's fuel AND heap are
  **bit-identical** (13,267,176 / 503 B — `HashSet` has no `vals` at
  all, the churn executes zero element ops); a powered 7×9 pass read
  +3.2%; and a **rebuild of the same after source read +0.3%** — the
  delta tracks the BUILD, not the source, the same build-placement
  artifact the crossing-fastpath close-out documented. Recorded as
  NEUTRAL with layout jitter (±2-3% on a cache-miss-bound host row);
  not a regression the change can own.
- **json-decode** +0.9% with bit-identical fuel/heap — inside the ±1%
  parity band this row's history already established (+0.4% phase 2,
  −1.0% close-out). **alloc** −0.9%, likewise bit-identical — parity.
- Every row whose fuel/heap MOVED moved in the direction the change
  predicts (raw stores delete `makeopt`s and cell traffic exactly where
  `[?prim]` backings exist — including the pure-rut `mapset` twins,
  which inherit the store for free through `pouch`/`mapset`'s own
  `vals: [?V]` / `Vec<T>.buf`). `hashmap-str`'s −0.5% time with a −2 MB
  heap reads honestly as: the row is str-key dominated, the sidecar win
  is real but small.

Stop-point (§0.5): **not triggered** — the only candidate (nmap-hashset)
was investigated and attributed to build placement by the rebuild test;
no row regressed beyond noise on its measurement with an op-stream
delta that could own it.

Deviations, recorded: (1) the design is the plan's compile-time option
(repr-keyed ops + two peephole folds), not the default runtime-branch —
rationale above, and the shape change is recorded in the commit body
per the batch rules; (2) `[T]` non-optional primitives did NOT ride the
new store — their existing packed kinds already store raw payloads with
no nil tag to represent, and nothing fell out naturally; (3) the
get-into-set copy fold (relocation drains, `Vec` grow loops) was NOT
attempted — it needs a composite element-copy op; recorded as the
surviving follow-up alongside the native val column. `expected.json`
untouched; workspace green (74 committed suites + the new 11-test
`opt_prim_store` driver suite); tree clean.

## Performance log — nmapset-round2 close-out: the verdict (Sep 2026)

The batch's final record. Two gated phases followed the phase-0
evidence: devirt was SKIPPED (phase 1 — finding 0a showed `KeyLane`
dispatch already direct) and the primitive-optional store landed
(phase 2 — `ArrKind::Opt(PrimTy)`: raw payload + one-byte nil tag,
compile-time repr-keyed element ops + two peephole folds, module
VERSION 5 → 6). Phase 3 measured and reported only — no engine file
changed after phase 2; this section is the verdict.

**Checksum gate (§0.1):** full suite from one checkout — both runtime
binaries (`rut`, `rut-bench-probe`) rebuilt in place from the batch
HEAD (8230973) — all **79 rows × rut/qjs/node, exit 0, every checksum
equal to `expected.json`** (the nmapset rows bit-identical to their
mapset twins `734932704` / `1264308351` / `21500055` / `2198604`; json
`4502015958359127277`; crossing-nop `20000014000000`). `expected.json`
was never touched all batch. The verdict run's probe columns confirm
the phase-2 table byte-for-byte where the op stream is concerned:
**fuel is bit-exact to the phase-2 after-side on every row that moved**
(int 20,703,284; str 9,551,761; nmap-knucleotide 38,814,389;
hashmap-int 63,758,210; hashset 55,696,689; array 60,486,108; sieve
19,592,209; hashmap-str 111,306,605; knucleotide 417,970,983) **and
bit-identical to the pre-batch records on every row that could not
move** (json-decode 111,322,915 / 32.78 MB; alloc 22,000,020 / 228 B;
nmap-hashset 13,267,176 / 503 B; crossing-nop 104,000,032 / 236 B).

**Cumulative vs the batch baseline** (probe exec medians; before =
b5ff868 — the phase-0 engine under docs-only commits; phase-0's own row
baselines reproduce it within noise: int 77.84 recorded vs 77.59 in the
phase-2 matched pair; after = the phase-2 interleaved A/B, the batch's
recorded deltas; last column = today's verdict run):

| workload         | before    | after (phase-2 A/B) | Δ             | verdict run | fuel before → after       | VM heap before → after    |
|------------------|-----------|---------------------|---------------|-------------|---------------------------|---------------------------|
| sieve            | 155.97 ms | 43.10 ms            | **−72.4%**    | 42.5 ms     | 21,167,665 → 19,592,209   | 21.85 MB → 1.49 MB (−93%) |
| hashmap-int      | 172.35 ms | 134.86 ms           | **−21.7%**    | 134.8 ms    | 64,758,210 → 63,758,210   | 17.17 MB → 8.26 MB (−52%) |
| array            | 267.56 ms | 210.01 ms           | **−21.5%**    | 213.5 ms    | 63,486,108 → 60,486,108   | 40.39 MB → 7.86 MB (−81%) |
| nmapset-int      | 77.84 ms  | 63.19 ms            | **−18.8%**    | 63.6 ms     | 21,103,284 → 20,703,284   | 6.08 MB → 1.97 MB (−68%)  |
| nmap-knucleotide | 202.53 ms | 177.00 ms           | **−12.6%**    | 172.9 ms    | 39,612,955 → 38,814,389   | 12.43 MB → 4.20 MB (−66%) |
| hashset          | 121.58 ms | 113.22 ms           | −6.9%         | 109.9 ms    | 56,230,024 → 55,696,689   | 13.16 MB → 7.83 MB (−40%) |
| nmapset-str      | 56.68 ms  | 53.08 ms            | −6.4%         | 53.6 ms     | 9,735,095 → 9,551,761     | 3.04 MB → 0.98 MB (−68%)  |
| knucleotide      | 952.18 ms | 925.88 ms           | −2.8%         | 910.9 ms    | 418,769,549 → 417,970,983 | 43.52 MB → 35.29 MB (−19%)|
| hashmap-str      | 251.18 ms | 249.97 ms           | −0.5%         | 249.2 ms    | 111,489,939 → 111,306,605 | 10.29 MB → 8.29 MB (−19%) |
| alloc            | 17.46 ms  | 17.31 ms            | −0.9% — parity| 17.4 ms     | 22,000,020 (identical)    | 228 B (identical)         |
| json-decode      | 671.5 ms  | 677.9 ms            | +0.9% — parity| 662.9 ms    | 111,322,915 (identical)   | 34.38 MB (identical)      |
| nmap-hashset     | 38.9 ms   | neutral — below     | —             | 38.0 ms     | 13,267,176 (identical)    | 503 B (identical)         |

(nmapset-int is −18.6% on the matched pair and −18.8% against the
recorded 77.84 baseline — both true, both stated; the batch's captured
number is −18.6%.) The verdict run reproduces the phase-2 after-side
within ±3% on every row (int +0.6%, sieve −1.4%, array +1.7%,
hashmap-int −0.0%, nmap-knucleotide −2.3%, str +1.0%, hashset −2.9%,
knucleotide −1.6%, hashmap-str −0.3%, json −2.2%, alloc +0.5%) —
same-day parity, no post-commit drift.

**The qjs scoreboard** (net medians, this run — the established
cross-runtime comparator; qjs is stable day to day):

| workload         | rut net  | qjs net | rut/qjs           | was (pre-batch record)          |
|------------------|----------|---------|-------------------|---------------------------------|
| sieve            | 43.1 ms  | 55.0 ms | **0.78× — ahead** | ~3.1× (rut behind)              |
| nmap-hashset     | 46.6 ms  | 54.8 ms | **0.85× — ahead** | 0.85× (holds)                   |
| nmapset-int      | 70.7 ms  | 63.3 ms | 1.12×             | ~1.4× (1.5× at the typed lanes) |
| nmap-knucleotide | 181.8 ms | 142.6 ms| 1.27×             | 1.6×                            |
| nmapset-str      | 56.8 ms  | 39.9 ms | 1.42×             | 1.9×                            |
| array            | 212.6 ms | 118.6 ms| 1.79×             | 2.45× byref / 1.78× pre-byref   |
| json-decode      | 693.5 ms | 59.6 ms | ~11× (11.6×)      | ~11× (unchanged all batch)      |

The plan's headline question, answered honestly: **nmapset-int is NOT
ahead of its qjs twin** — 70.7 vs 63.3 ms net (1.12×), down from ~1.4×.
The probe sharpens it: rut's pure execution on the row is 63.6 ms —
parity with qjs's whole NET (63.3 ms, parse included); the CLI net
carries ~7 ms of compile/mount the probe does not. The scoreboard's
other facts: **sieve goes ahead of qjs for the first time on this
suite** (43.1 vs 55.0 ms net — the −72.4% win's cross-runtime payoff),
nmap-hashset holds its ahead position (0.85×), and the `array` row's
byref-flip regression is fully recovered (1.79× vs its 1.78× pre-byref
placement).

**The neutrals, stated honestly:**

- **alloc** −0.9% (bit-identical fuel 22,000,020 / heap 228 B): parity.
  The row allocates RECORDS — reference payloads keep the cell backing
  by design (§0.3 scope), so there was nothing for the primitive store
  to remove; the byref-flip +5% is not recovered here, it is inherited
  unchanged.
- **json-decode** +0.9% on the matched pair (this run read 662.9 ms,
  below the matched 671.5 before; bit-identical fuel/heap): the tree is
  `?record`/`?str` values — structurally untouched. The phase-0 table's
  727.9 ms read was a drifted hour (its fuel was already bit-identical);
  time comparisons on this row use the matched pair.
- **nmap-hashset** — the phase-2 +3-6% reads closed out in the batch's
  favor: the jitter attribution (build placement, not source — the
  rebuild-of-same-source read was +0.3%) is CONFIRMED by this verdict
  run, itself a fresh rebuild, reading **38.0 ms, below the 38.9
  phase-0 baseline** (−2.3%). The row keeps no `vals` at all (`HashSet`
  has none), executes zero element ops, and its fuel (13,267,176) and
  heap (503 B) were bit-identical in every pass all batch. NEUTRAL.

**Ceiling accounting — the batch's central question.** Phase 0 proved
the whole `[?V]` sidecar costs 45.3% of the int row (35.3 of 77.84 ms;
the stub floor is 42.57 ms). The primitive store captured **−18.6% =
~41% of the ceiling**. The remaining 20.6 ms above the stub floor
(63.19 − 42.57) is exactly the sidecar work the repr keeps by law and
by design: (1) every hit-`get` must still MINT the fresh `?i32` it
returns (100k hits — value semantics; §0.3's unobservable-law scope is
what legitimizes the raw store, but a read still produces a real opt
value); (2) growth still relocates `vals` — now raw stride-w+1 copies,
cheaper per element but still traffic (the stub dropped the drains
entirely); (3) the remaining `arrset`/`arrget` stream — fuel fell only
−400k ops (−1.9%): the −18.6% was host-side mint/retain/release and
allocator work fuel never counts. Below that sits the phase-0 budget's
untouched floor: ~37% host-table body, ~16% VM stream, ≤2% crossing.

**The surviving follow-up menu, re-baselined** (each entry records the
number a future batch starts from):

- **(a) native val column for primitive V** — the int row's residual is
  the hit-`get` opt mint (term 1 above); a value-typed return for
  primitive `V` (or a val-typed lane) attacks it. Baseline: nmapset-int
  63.19/63.6 ms exec, 70.7 ms net (1.12× qjs), fuel 20,703,284, heap
  1.97 MB; the 42.57 ms stub floor sizes the whole residual at 20.6 ms.
- **(b) borrowed str slices** — the string churn the plan deferred.
  Baselines: nmapset-str 53.08/53.6 ms (1.42× qjs), nmap-knucleotide
  177.0/172.9 ms (1.27× qjs), json-decode 677.9/662.9 ms (11.6× qjs;
  its fuel never moved all batch — the row is box churn, and its boxes
  are cells by design).
- **(c) get-into-set copy fold** (the phase-2 deferral) — a composite
  element-copy op for the relocation drains and `Vec` grow loops: term
  (2) of the ceiling residual above.
- **(d) dead KeyLane impl bodies cleanup** (phase-0 artifact, cosmetic)
  — the 33 compiled `nentry`/`nfind`/`nremove` bodies (11 key types ×
  3) remain unreferenced under instantiation (0% of any row's fuel,
  findings 0a/0c); deleting them is hygiene only — no number will move.

Method note: phase 3 ran no A/B of its own — phase 2's interleaved
pairs (both binaries from this checkout, same day) are the batch's
deltas; the verdict run is the gate and the confirmation. Workspace
green (75 suites, 450 tests, zero failures). Tree clean; this section
is the entire commit.

## Performance log — nmapset-round3 phase 0: residual split, miss bound, consumer-shape spike (Sep 2026)

Evidence phase for the native-val-column batch: three findings, no engine
change, nothing committed but this section. The int row enters at 63.19 ms
exec / 70.7 net = 1.12× qjs (probe exec 63.6 is parity with qjs's whole
net), with 20.6 ms of measured residual above the round2 stub floor
(42.57). The column attacks that residual and the phase-0 floor beneath
it; this phase sizes the residual's terms, confirms the cache-miss bound
by scaling, and decides the phase-2 consumer shape. Row baselines
re-measured today for the batch (one probe binary, one session; fuel and
VM-heap **bit-identical** to the close-out records on every row — nothing
has moved):

| row          | exec today (probe)   | fuel       | VM heap peak |
|--------------|----------------------|------------|--------------|
| nmapset-int  | 77.9 ms (A/B below)  | 20,703,284 | 1,966,527 B  |
| nmapset-str  | 56.3 ms              | 9,551,761  | 983,596 B    |
| hashmap-int  | 142.7 ms             | 63,758,210 | 8,258,103 B  |
| crossing-nop | 96.8 ms              | 104,000,032| 236 B        |
| nmap-hashset | checksum 21500055 (CLI control run) | 13,267,176 | 503 B |

Machine note, recorded up front: today the box read the int row ~23% slower
than the 63.6 ms close-out record (a parallel session builds on this same
tree). Fuel and heap are bit-identical everywhere, and **every comparison
below is same-session interleaved**, so the deltas and the split are
machine-consistent; only the absolute medians carry the day's slowdown.

### Finding 0a — the residual split: the relocation drain dominates, not the mint

Method: throwaway stubs of `rut/nmapset/nmapset.rut` (NEVER committed —
`git checkout --` restored after every pass, md5 re-verified, row checksum
re-verified `734932704`), ONE fixed probe binary with the wrapper source
flipped between interleaved rounds (the round2 phase-0 method — pkg
sources are runtime-mounted), 5 rounds × 7 fresh-VM iters per side, order
alternated, medians of round medians. Three stub shapes:

- **MINT** — `get` returns a pre-built constant opt cell. Plan deviation,
  recorded: a generic `V` has no literal, so the constant is the FIRST
  hit's stored value, captured on hit #1 and returned for the remaining
  99,999 (one mint survives out of 100k). The escape path stays — the
  caller still receives a cell, derefs and releases it — so the measured
  delta is the mint + sidecar-load term with the escape cancelled.
  Checksum deterministic **29950000** and bit-exact reconciling:
  25,150,000 (all counters) + 700,000 (sum = the constant 7 × 100k hits)
  + 4,100,000 (hits·41).
- **NOGROW** (the plan's drain stub) — `with_capacity(200000)` → cap 2^18
  = 262144, the same final cap the row reaches anyway; the 0.7 load law
  `(count + tomb + 1) × 10 >= cap × 7` never fires for 100k entries, so
  the whole grow path is gone (host re-slot, table memsets, wrapper
  drain). Checksum unchanged, `734932704`.
- **DROPDRAIN** — grows kept, the 12-line relocation-drain loop dropped
  (`next` stays all-nil). The replace phase rewrites every value before
  the hit phase, so **the row checksum stays bit-identical
  `734932704`** — this stub provably moves only drain traffic, and the
  NOGROW-vs-DROPDRAIN pair splits the wrapper drain from the host grow
  body, which the plan's single stub conflates.

| side    | round medians (ms)            | median of medians | fuel       | heap peak  |
|---------|-------------------------------|-------------------|------------|------------|
| A — row | 74.95, 75.56, 79.46, 79.70, 78.44 | **78.44 ms**  | 20,703,284 | 1,966,527 B|
| M — mint| 74.59, 72.08, 74.83, 78.08, 76.36 | 74.83 ms      | 21,003,289 | 1,966,535 B|
| DG — no grow | 53.59, 56.04, 54.49, 59.10, 55.50 | **55.50 ms** | 17,400,095 | 1,311,143 B|
| DD — drain dropped | 59.44, 61.70, 63.59, 62.58, 59.20 | 61.70 ms | 17,400,365 | 1,966,463 B|
| MD — mint + no grow | 52.26, 51.77, 53.62, 54.56, 53.47 | **53.47 ms** | 17,700,100 | 1,311,119 B|

The fuel ledger reconciles exactly and independently corroborates every
stub: M +300,005 ops = the per-hit nil-test (~3 ops × 100k hits); DG/DD
−3,303,189 / −3,302,919 ops = the drain loop at **18 ops × 183,489 moved
slots** (15 grows; Σ count-at-grow = 183,489) plus the 15 grow crossings;
MD = DG + M's 300,005 bit-exact (perfect fuel additivity). Heap
corroborates the shapes: DG/MD sit at 1,311,143/1,311,119 B — the single
262144 × 5 B sidecar, no per-grow `next` arrays; M is +8 B — the one
cached cell.

The split (deltas vs A, same session; share of today's 78.44 ms row):

| term                                        | isolated by      | time      | share |
|---------------------------------------------|------------------|-----------|-------|
| wrapper relocation-drain loop               | DD               | **16.7 ms** | ~21% |
| host grow body (re-slot probing + table memsets) | DG − DD     | 6.2 ms    | ~8%  |
| hit-`get` mint + sidecar load               | M (powered pair) | 2.7 ms    | ~3.5%|
| remaining put/remove element stream         | by difference    | ~0.3 ms   | ~0.4%|

The mint term needed a powered pass to clear the day's noise: the 5-way
pass's A−M delta (3.6 ms) had overlapping round ranges (74.95–79.70 vs
72.08–78.08), so a dedicated A-vs-M 2-side pass ran — **A 77.94
(77.46–78.57) vs M 75.23 (74.15–75.63), ranges non-overlapping → 2.7 ms**.
Additivity check: DG + M terms sum to 26.55 ms vs the directly measured
MD delta 24.97 ms (94% — the gap is interaction noise).

**The correction this finding makes to the round2 close-out:** the
residual's dominant term is NOT the hit-`get` mint (which the close-out
listed first) — it is the **relocation drain, ~5× larger** (16.7 vs
2.7 ms). Per moved slot the drain costs ~91 ns: 18 VM ops (~14 ns at the
0.77 ns/op calibration) plus a `map_take_reloc` crossing plus the
scattered rehash-destination writes into the doubling sidecar. The
close-out's guess was wrong by rank, not by kind — and the rank swap
strengthens the column's case: the native val column eliminates the
wrapper drain **by construction** (the host relocates vals with the
keys), which no wrapper-side change could.

### Finding 0b — the cache-miss bound, confirmed by scaling

Method: the int shape at four sizes — scratch module dirs under the batch
run dir, deps pointing at the in-tree pkgs, pristine wrapper, one probe
binary, 5 rounds × 7 fresh-VM iters, order rotated per round. Working set
= keys + hashes + states (41 B/slot) + the `[?i32]` sidecar (5 B/slot):

| N       | cap    | working set     | exec median | per-map-op | fuel/op |
|---------|--------|-----------------|-------------|------------|---------|
| 300     | 512    | ~24 KB — L1     | 0.144 ms    | **79.9 ns**| 32.6    |
| 3,000   | 8192   | ~0.38 MB — L2   | 1.689 ms    | 93.8 ns    | 34.7    |
| 30,000  | 65536  | ~2.7 MB — L3    | 17.628 ms   | 97.9 ns    | 33.6    |
| 100,000 | 262144 | ~12 MB — DRAM   | 75.782 ms   | **126.3 ns**| 34.5   |

Per-op fuel is constant across the ladder — so per-op TIME cannot be the
stream. The climb is monotone across the hierarchy boundaries (+13.9,
+4.1, +28.4 ns): the cache-miss signature, and it prices the round2
phase-0 budget's estimated-but-unmeasured ~37% host-table term. Full size
pays **+46.4 ns/op (+58%) over the L1-resident shape = 37% of the
per-op** — the phase-0 budget's estimate, now measured.

What the interleaved column can recover, stated honestly: the +46 ns is
BOTH line streams — the key probe over keys/hashes/states AND the sidecar
access. Interleaving folds the val into the key's line, removing at most
the sidecar's share of the miss term: **projected ~half the miss term,
~20 ns/op (~15–20% of the row)**, with the key-probe misses remaining by
construction (same probe, same keys, §0.3's checksum gate). The drain
(16.7 ms) and mint/load (2.7–3.6 ms) terms are additional, directly
measured, and removed by the same design.

### Finding 0c — the consumer-shape spike: dual-mode is NOT expressible

The §0.5 gate question: can a generic wrapper body branch
per-instantiation on a type param's prim-ness (`HashMap<i32, i32>` routes
put/get/remove through a val column while `HashMap<str, str>` keeps the
sidecar)? Searched the RFCs and the compiler honestly:

- **`requires` bounds are admission-only** (RFC 0013 §2, RFC 0043): they
  gate which instantiations compile and prove widening to a bound-member
  slot; the union's whole-bound contract requires EVERY member to provide
  every called method — "the gate buys compile-time whole-bound checking,
  not dynamic dispatch" — the opposite of per-member divergence. Static
  dispatch on a bare `T` is RFC 0013 OQ-1, explicitly **deferred**.
- **Reflection is runtime** (RFC 0037): `reflect<T>()` / `type_id` are
  descriptor handles over host fns — a dual-mode body through them is a
  runtime branch paying a check per op, compiling both paths through the
  same ops, and the raw-repr element ops are not surface-reachable.
- **Templates** (RFC 0027) are `f"..."` carriers, not type-level
  computation. **No layout introspection exists at all** (RFC 0015:
  "no `size_of<T>()` / `align_of<T>()`"). **Const generics** exist only
  on builtin `Array<T, N>` and N is a value, not a type property
  (RFC 0013 §2). Generic-target impls (`impl I for Vec<T>`) monomorphize,
  but impl selection has no conditional-by-bound or specialization-order
  form (RFC 0012).
- What DOES exist is per-instantiation repr keying **inside the engine**:
  `Repr::OptPrim / OptPrimRaw / OptPrimLoad(PrimTy)` are chosen from the
  resolved element type at op-lowering (`rut-core/src/types.rs`), and
  generic-class methods monomorphize per instantiation
  (`rut-lir/src/lir/mod.rs`, "monomorphized per instantiation"). The
  machinery that would execute a dual-mode design is real, but no surface
  syntax routes a user generic's prim-ness into a body branch.

**Verdict (§0.5): dual-mode NOT expressible → the fallback runs.** Phase 2
lands a PUBLIC prim-val map class in nmapset over the phase-1 host val
column: **`PrimMap<K requires i8 | i16 | i32 | i64 | u8 | u16 | u32 |
u64 | bool | str | bytes, V requires i64 | u64 | f64 | bool>`** — the
name follows the plan's own example; K keeps the closed union (host
hashing unchanged), V is the four repr-distinct raw kinds that ride the
u64 val lane. `HashMap` stays untouched (sidecar path, ref-V law, all
laws); `get -> ?V` semantics preserved in the prim class (fresh opt per
hit, round2's primitive-store law); the vals sidecar is absent by
construction there. Constraints confirmed: a bench row may switch to
`PrimMap` only under §0.1 — its checksum MUST still equal its pinned
value AND the README discloses the switch; additive surface only, no
module VERSION bump (§0.4).

Deviations, recorded: (1) the mint stub's "pre-built constant" is
cache-on-first-hit — a generic `V` has no literal; one mint of 100k
survives and the escape path is kept so the delta isolates mint + load;
(2) the drain stub ran in TWO shapes (the plan's NOGROW plus the
checksum-preserving DROPDRAIN control) because the plan's single stub
conflates the wrapper drain with the host grow body — the pair separates
them; (3) today's absolute medians carry a ~+23% box slowdown vs the
close-out record (parallel session on this tree) — fuel/heap bit-identical
everywhere, all deltas same-session interleaved; (4) finding 0b ran a
four-point ladder rather than two points so the miss term reads off the
hierarchy boundaries instead of one ratio. Scratch (stubs, flip copies,
ladder dirs, raw JSON) lives under `/tmp/opencode/batch-nmapset-round3/p0/`;
`expected.json` untouched; workspace green (75 suites, 450 tests, zero
failures); tree clean — this section is the entire commit.

## Performance log — nmapset-round3 phase 1: the host val column + crossings (Sep 2026)

The native value column lands behind the existing surface: `NativeTable`
gains `vals: Vec<u64>` — one raw u64 per slot, **presence-by-key**
(plan §0.3: a val is valid iff its key slot is FULL; no nil tags) — and
two additive crossings, `map_val_set_u(t, slot, v: u64)` /
`map_val_get_u(t, slot) -> u64`, raw bits in/out, ONE pair for every
primitive val kind (the phase-2 `PrimMap` wrapper reinterprets
i64/u64/f64/bool client-side; no lane explosion). Slots cross as
caller-supplied `i32` and are bounds-checked host-side (negative or
`>= cap` traps, the house `Invalid` + "out of range" shape). `grow`
relocates vals WITH the keys inside the same re-slot walk — the term
that deletes the wrapper's 16.7 ms drain pass (finding 0a) for the
column-backed consumer. `remove` writes nothing (presence-by-key makes
the stale bits unreachable; the slot's next put stores a fresh val).

Design decisions, recorded:

- **Allocation: eager at creation** (zeroed `vec![0; cap]` in `new`,
  reallocated by `grow`). Cap is known at construction and §0.3 pins
  column cap == table cap, so eager keeps `vals.len() == cap` true at
  every observation point and the phase-2 hot path is a bounds check +
  an index — no per-op "is it allocated" branch. Cost for val-less
  tables (`HashSet`): one 8 B/slot zero-fill host-side, invisible to
  fuel and to cell accounting.
- **Layout: separate Vec** (the plan's phase-1 default). Interleaving
  stays on the menu: with no consumer wired, a separate-vs-interleaved
  A/B on any bench row measures nothing (both are bit-identical — the
  column is never called), and a host-only micro-bench would not
  reflect the op stream. Phase 2's `PrimMap` consumer makes the row-level
  A/B real; the phase-0b model is the predicted check (~20 ns/op, the
  sidecar's share of the +46.4 ns DRAM miss term).

Tests (all new, 10 host-side + 4 VM-side): raw-bit round-trips through
the u64 sign range (2^63..2^64-1) and f64 `to_bits` patterns; replace
on the found path; the remove/re-insert staleness law; sentinel-driven
multi-grow sweeps (cap 4 → 256) with tombstone churn and NO drain —
every val exact at its key's CURRENT slot; the exact old→new
relocation mapping pinned with the vals riding it; out-of-range slot
traps; the column sitting on the PINNED `hash_payload` home slots incl.
a linear-probe collision; and THE CHECKSUM LAW two ways — a
wrapper-shaped op sequence through column vs sidecar in one Rust test
(identical slots per key, identical checksum, pinned literal
`5344915641404318251`), and the nmapset-int `churn` shape at n = 2000
driven through the VM twice (`map_*` + column vs today's
`nmapset.HashMap` sidecar wrapper) answering the same `2598000`.

Sanity pass (house method: one probe binary per side, all 27 workloads,
rut only — no consumer uses the column yet): **checksums bit-identical
on every row** (incl. the four nmapset pins 734932704 / 1264308351 /
21500055 / 2198604) and **fuel bit-identical on every row** (nmapset-int
20,703,284; nmapset-str 9,551,761; hashmap-int 63,758,210;
crossing-nop 104,000,032; nmap-hashset 13,267,176 — the op stream did
not move). **Heap moved +24 B per table box** on the four rows that
build nmap tables (nmapset-int 1,966,527 → 1,966,551; nmapset-str
983,596 → 983,620; nmap-knucleotide 4,195,060 → 4,195,084;
nmap-hashset 503 → 551, two sets alive at peak); the other 23 rows are
bit-identical including heap. DEVIATION, recorded with the mechanism:
`OpaqueBox::alloc` charges the payload's shallow `size_of::<T>()` to
the RFC 0040 heap budget (`heap/mod.rs` `alloc_host_box`), and the
mandated `vals: Vec<u64>` column grows `size_of::<NativeTable>()`
120 → 144 — exactly one Vec header, charged once per `map_new`. No
rut-visible allocation changes (the column's backing memory is plain
Rust, invisible to cell accounting), and no layout within the plan's
letter avoids it: any column state in the struct is charged, and the
separate-Vec-first mandate rules out the one heap-neutral alternative
(interleaving the val into the existing `keys` allocation), which the
plan defers behind A/B numbers this phase cannot honestly produce.
Fuel/heap/checksum reconciliation for phase 2's gate should therefore
re-pin the four heap rows at these +24 B-per-live-table values; a
revert lever exists (the column is isolated to `nmap.rs` + the two
`.d.rut` decls). Exec medians wobbled with the box load (parallel
session; nmapset-int read 79.9 before / 70.0 after on single quick
passes) — informational only, fuel proves the stream unchanged.

Workspace green (76 suite runs, 460 tests, zero failures; was 75/450).
No consumer changes, no VERSION bump, `expected.json` untouched. Files:
`crates/rut-std/src/nmap.rs` (column + crossings + unit tests),
`rut/nmap_host/nmap.d.rut` + the CLI fixture mirror (the two decls),
`crates/rut-driver/tests/nmap_valcolumn.rs` (new), this section. Tree
clean apart from those; scratch under
`/tmp/opencode/batch-nmapset-round3/p1/`.

## Performance log — nmapset-round3 phase 2: the prim-val consumers (Sep 2026)

Phase 0c's verdict runs the §0.5 fallback: a PUBLIC prim-val map over
the phase-1 column, `HashMap` untouched. The consumer shape is three
classes in `nmapset` — `PrimMapI64<K>` / `PrimMapU64<K>` /
`PrimMapF64<K>` (each generic over the SAME closed key union, vals
fixed by the class) — not the plan-letter single `PrimMap<K, V>`, and
not the four-lane V union: two language facts, verified empirically
and recorded in the class header comment:

- **The get-side reinterpret has no spelling in a V-generic body.** The
  set direction is value-carried (a `v: V` would dispatch a lane method
  exactly like a KeyLane key); get has no `V` value to dispatch
  through. Rut's no-self trait methods (RFC 0012 §2, "engine
  contracts") have no call syntax on primitives — type names and type
  params are not expressions (`V.dec(raw)` / `i64.dec(raw)` both
  diagnose "unknown name"; there is no `::` path syntax), there is no
  `as V` cast to a type param, and the union bound is admission only,
  never per-member divergence (the phase-0c finding). Each class body
  therefore spells its two concrete reinterprets.
- **The bool lane is deferred, not dropped silently.** `?bool` cannot
  serve the `get -> ?V` nil law in today's checker: a `?bool` value
  unboxes to `bool` at value reads, so `p == nil` / `p != nil` diagnose
  "operands must have equal width (RFC 0004 §3): `bool` vs `nil`" —
  verified for direct-call, annotated-local, and inferred-local shapes,
  while the same compares compile and run for `?i64` / `?u64` / `?f64`
  / `?str`. A bool-val map needs that checker fix first; today a bool
  val encodes as `0u64` / `1u64` in `PrimMapU64`.

Per op the wrapper is: `put` = one lane call (`nentry`, the fused grow
sentinel) → grow + RETRY — **no relocation drain, the column moved
with the keys** — then the val store at the answered slot
(`map_val_set_u`, or the new `map_val_set_f` for floats); `get`/`has` =
one `nfind`, and a hit reads the column (`map_val_get_u` /
`map_val_get_f`) into a FRESH opt — the get -> ?V law's prim semantics,
round-2 style; `remove` = `nremove` only (presence-by-key: no val
write); `with_capacity` via `map_new`, `len` via `map_len`.

**The f64 lane needed the plan's "minimal honest surface": two
additive crossings.** Rut has no `f64 <-> u64` bitcast — the numeric
`as` is a VALUE conversion (RFC 0007 §1), so `raw as f64` would round,
not reinterpret, and no builtin exists. `map_val_set_f(m, slot, v:
f64)` / `map_val_get_f(m, slot) -> f64` read/write the SAME u64 column
through `f64::to_bits`/`from_bits` — raw bits byte-for-byte both
directions, the u lane's slot law (out-of-range traps), zero wrapper
reinterpretation. Declared in `rut/nmap_host/nmap.d.rut` + the CLI fixture
mirror (update-BOTH). The integer/bool-free client-side reinterprets
that DID spell (`as u64` / `as i64` raw wraps) stay client-side.

**Rider correction (recorded, nothing deleted):** the plan's phase-2
rider called 33 KeyLane impl bodies "dead" — they are NOT dead code in
the unreachable sense: the bench never calls them, but other K
instantiations (u8/u16/str/bytes keys, every non-bench program)
monomorphize exactly those impls. The cleanup rider is VOID; all impls
stay.

**New bench row `nmap-primmap`** (the recommended shape — `HashMap`'s
regression coverage stays AND the new path gets a row): a clone of
nmapset-int with `PrimMapI64<i32>` — identical keys, values (i64 lane),
op sequence, and scale; the churn accumulates in i64 and narrows once
at the return, bit-identical to the i32 wrapping arithmetic
(two's-complement addition is the same mod 2^32). Files:
`benches/workloads/nmap-primmap/{rut.toml,main.rut}`,
`benches/workloads/nmap-primmap.js` (the qjs/node twin, algorithm
unchanged), and ONE expected.json line — `"nmap-primmap":
"734932704"`, the sequence's pinned value. Nothing else in
expected.json moved; no VERSION bump anywhere.

### The numbers (house method, one checkout, release build)

- **Parity law through the wrapper**: the nmapset-int churn shape at
  n = 2000 through `PrimMapI64<i32>` vs `HashMap<i32, i64>` in one
  program answers the same `2598000` (the phase-1 column-law number);
  the prim get semantics (fresh opt per hit, held copies keep
  pre-replace bits, remove doesn't touch held copies) are identical on
  both classes, both directions.
- **Headline, same-session interleaved 10 rounds × 7 fresh-VM iters,
  order alternated**: nmap-primmap **61.40 ms** exec (60.7–65.1) vs
  nmapset-int **68.93 ms** (67.5–71.4) — round ranges non-overlapping,
  **−10.9 % matched-pair** (fuel 20,703,284 → 17,950,301, −13.4 %;
  ~−12.5 ns per map op). The phase-0b model predicted the column's
  one-line-per-op win at ≤ ~20 ns/op; the measured net lands at ~60 %
  of the naive drain+sidecar model, the honest residual being the
  added val crossing per op (~10 ns class) and the column's own cache
  line. Heap: **1,966,551 → 324 B** — the `[?V]` sidecar is GONE (the
  column is plain Rust, invisible to cell accounting; the residue is
  the logger + the +24 B table box). For absolute placement: the box
  runs ~+9 % over the round2 close-out (nmapset-int read 63.19 there,
  68.93 in this session — the matched pair is the number); the phase-0
  stub floor 42.57 remains the ceiling marker, and qjs: primmap net
  70.4 (rut CLI) vs 62.6 (qjs) = 1.13× — on probe-exec terms 61.4 vs
  qjs's whole 62.6 net, i.e. AT parity, the round2 placement held.
- **Guards**: full-suite sanity pass, all 28 workloads × rut/qjs/node,
  exit 0 — every checksum equal expected.json (the four nmapset pins
  734932704 / 1264308351 / 21500055 / 2198604 hold; the NEW row reads
  734932704 on all three runtimes), **fuel bit-identical on every
  pre-existing row**, and the four nmap heap re-pins hold exactly
  (nmapset-int 1,966,551; nmapset-str 983,620; nmap-knucleotide
  4,195,084; nmap-hashset 551).

Tests (7 new driver tests in `nmap_primmap.rs` + 1 Rust-side): the
parity law; prim get semantics mirrored against the sidecar; a
500-key multi-grow sweep from cap 4 under colliding keys (step 37)
with remove/re-add churn — every val exact, no drain; u64 raw-bit
round-trips through the wrapper (2^63, 2^64-1, replace, remove/
re-insert staleness); f64 round-trips incl. the −0.0 SIGN bit
(1/−0.0 < 0 — value equality can't see it, the column carries it);
K admission (the union diagnostic names `Pt` and the closed union,
unchanged); HashMap + HashSet + PrimMap coexistence in one program
with a str-keyed PrimMap riding the s lane. Workspace green (77 suite
runs, 468 tests, zero failures; was 76/460). Files:
`crates/rut-std/src/nmap.rs` (the f64 crossings + bit-pattern test),
`rut/nmap_host/nmap.d.rut` + the CLI fixture mirror (+2 decls),
`rut/nmapset/nmapset.rut` (the three classes + the header story),
`crates/rut-driver/tests/nmap_primmap.rs` (new), the three
`nmap-primmap` bench files + the expected.json line, this section.
Scratch under `/tmp/opencode/batch-nmapset-round3/p2/`.

## Performance log — nmapset-round3 close-out: the verdict (Sep 2026)

The batch's gate run, measurement only — no engine, stdlib, or workload
change. Both runtime binaries (`rut` CLI + `rut-bench-probe`) rebuilt
from THIS checkout at HEAD `5a26938` (tree clean; cargo fingerprints
Fresh — every artifact verified against the committed sources), then the
full suite in one pass: **28 workloads × rut/qjs/node from one checkout,
exit 0, every checksum equal `expected.json`** — 82 cross rows, 79 of
them checksum-carrying (`empty` carries none; `u64loop` is the one
rut-only workload with no JS twin), zero mismatches. `expected.json` was
touched exactly once all batch — phase 2's disclosed one-line
`nmap-primmap` addition. The five nmapset pins hold bit-identically on
every runtime: `734932704` / `1264308351` / `21500055` / `2198604`, and
the new row answers its twin's exact sequence as `nmap-primmap =
734932704` on rut/qjs/node. **Fuel and VM-heap are bit-identical to the
phase-2 records on all 28 probe rows** — nothing outside the shipped
design moved (nmapset-int 20,703,284; nmap-primmap 17,950,301; str
9,551,761; hashmap-int 63,758,210; crossing-nop 104,000,032; nmap-hashset
13,267,176) — and the **four phase-1 heap re-pins hold exactly**:
nmapset-int 1,966,551; nmapset-str 983,620; nmap-knucleotide 4,195,084;
nmap-hashset 551 B. Workspace green (77 suites, 468 tests, zero
failures).

Box state, recorded up front: this verdict ran calm (load ~0.2; phase 2
recorded ~+9 %, phase 0 ~+23 %). The int rows are still compared as
same-run pairs, because absolute medians carry day-to-day spread even
when calm: the sidecar row read 72.84 here, 68.93 in phase 2's session,
63.19 at the round2 calm close-out; the prim row reproduced 61.55 vs
phase 2's 61.40.

### The cumulative table (int shape: sidecar row → prim row)

The batch's delta instrument is the same-run matched pair; this table
reads it on verdict-day numbers, with the phase-0 baseline for
orientation:

| measure | phase-0 baseline (A) | verdict: nmapset-int (sidecar) | verdict: nmap-primmap (column) | pair delta |
|---------|----------------------|--------------------------------|--------------------------------|------------|
| probe exec | 78.44 ms¹ | 72.84 ms | **61.55 ms** | **−15.5 %** |
| rut net | — | 75.13 ms | **68.12 ms** | −9.3 % |
| fuel | 20,703,284 | 20,703,284 | 17,950,301 | −13.4 % |
| VM heap | 1,966,527 B | 1,966,551 B² | **324 B** | −99.98 % |
| per map op | ~130.7 ns¹ | 121.4 ns | **102.6 ns** | ~−18.8 ns/op |

¹ phase-0's A-side median carries that day's +23 % box load — it orients,
it does not measure. ² the +24 B Vec-header re-pin (phase 1), holding
exactly.

The batch's captured number stays phase 2's powered pair — interleaved
10 rounds × 7 fresh-VM iters, order alternated, **61.40 vs 68.93 ms,
−10.9 %, round ranges non-overlapping, ~−12.5 ns/map-op** — and this
verdict's same-run pair (61.55 vs 72.84 exec, ~−18.8 ns/op) reproduces
the direction with the honest spread stated: across the two sessions the
pair reads −12.5 to −18.8 ns/op over identical fuel, i.e. the
drain+mint deletion (the phase-0a stub model priced it at 16.7 + 2.7 =
19.4 ms ≈ 32 ns/op over the row's exactly 600 k map ops) landed at
roughly 40–60 % of the naive stub sum, the residual being the added val
crossing per op and the column's own cache line, both priced in the
phase-2 section. Floor marker unchanged: the round2 stub floor 42.57 —
the prim row at 61.55 sits 1.45× above it, the remaining gap being the
~37 % host-table body and the ~16 % VM stream the batch did not touch.

### The qjs verdict on nmap-primmap (same-run nets, both sides)

- **Whole-net terms: NOT ahead — 1.08×.** rut 68.12 vs qjs 62.97 ms.
  The CLI's compile/mount carry (~7 ms) is the difference.
- **Probe-exec terms: AT parity.** rut exec 61.55 vs qjs's WHOLE net
  62.97 — the round2 placement held, now on the prim path.
- Same run, the sidecar twin: rut 75.13 vs qjs 62.94 = 1.19×. The
  column moved rut from 1.19× to 1.08× whole-net while qjs did not move
  at all (62.94 → 62.97) — the entire gain is on the rut side, which is
  what makes the pair clean.

### The miss-model check: does the interleave keep its menu slot?

Measured against the phase-0b projection, precisely: what the batch
REALIZED (−12.5 to −18.8 ns/op) is the drain+mint deletion — VM-op
traffic the stub model priced at ~32 ns/op — not the cache-side term.
What shipped is a SEPARATE `vals` Vec (the phase-1 mandate), so the 0b
projection ("interleaving folds the val into the key's line, removing at
most the sidecar's share ~20 ns/op") remains UNTESTED by design. Verdict:
**the interleave keeps its menu slot**, on three measured grounds:
(1) the row is still miss-bound — the prim row's 102.6 ns/op sits
between the 0b ladder's L3 rung (97.9) and its DRAM rung (126.3), with
the ladder's +46.4 ns/op hierarchy climb still in the row; (2) every hit
still streams a second line (`vals[slot]` today, the sidecar before)
that interleaving folds into the key's line; (3) the A/B harness now
exists and is live — the primmap row IS the measurement, with this run's
numbers as its before-side. The batch DID validate the model's honest
half: the realized cache-neutral win landing under the stub sum confirms
the crossing+line costs phase 2 priced, and the ≤ ~20 ns/op interleave
ceiling stands as the untested upside on top of them.

### Honest neutrals

Every row outside the prim path is a NEUTRAL: fuel and VM-heap
bit-identical to the pre-batch records on all 27 non-primmap rows (the
op stream provably did not move), exec medians differing only by box
state (nmapset-str 53.50 vs the phase-0 loaded 56.3; hashmap-int 135.19
vs 142.7; crossing-nop 94.33 vs 96.8; nmap-hashset 39.85, zero element
ops). No performance claim is made for any row without a same-run pair
behind it.

Recorded here so the close-out is complete: the SHAPE DEVIATION stands
as ratified in the phase-2 section (three lane classes
`PrimMapI64/U64/F64<K>` — the V-generic get-side reinterpret is
unspellable: no no-self trait call on primitives, no `as V`, union
bounds admission-only). The **bool lane is DEFERRED, not dropped
silently**: `?bool` unboxes at value reads, so `p == nil` diagnoses a
width mismatch (RFC 0004 §3); a checker fix gates `PrimMapBool<K>`, and
bool vals ride `PrimMapU64` as 0/1 today. The dead-KeyLane rider stays
VOID. The `nmap-primmap` workload switch is disclosed per §0.1 (checksum
equal to the pin, this log records the switch). Four heap pins moved
once, in phase 1, by mechanism (+24 B Vec header per table box) and were
re-pinned; they hold exactly here.

### Re-baselined follow-up menu

- **(a) Borrowed str slices** — the next big lever, now the largest
  measured gap on the board: nmapset-str 1.49× its qjs twin (60.22 vs
  40.35 net, this run), nmap-knucleotide 1.33× (189.89 vs 142.41),
  json-decode 11.8× (722.39 vs 61.45) — all three copy str payloads
  that this batch's own result shows can stay host-side.
- **(b) The bool lane** — the `?bool` checker fix (value reads unbox →
  nil comparison diagnoses a width mismatch, RFC 0004 §3). Unlocks
  `PrimMapBool<K>` as the fourth lane; the design already prices it
  (vals as 0/1 in the u64 column).
- **(c) Interleaved column layout** — still priced (above), now with a
  live harness: fold `vals[slot]` into the key slot's line inside
  `nmap.rs`, A/B on the primmap row, predicted check ≤ ~20 ns/op against
  this run's 102.6 ns/op.
- **(d) Get-into-set copy fold** — mostly overtaken: the drain term it
  targeted on the prim path is deleted by construction; only the
  sidecar `HashMap` (ref-V) path retains the minor form. The dead-impl
  cleanup is closed VOID.
- **POST-BATCH (the NEXT session's job, not started here): the mapset
  pkg removal** — user-directed and pre-inventoried in the batch run
  log (no hard deps; 4 bench rows die with the pkg, workload deletion
  WITH disclosure, round2's hashmap wins stay in the history).

Files this phase: `benches/README.md` only. Scratch:
`/tmp/opencode/batch-nmapset-round3/p3/` (verdict.json/.csv/.md +
runner stdout/stderr). Tree clean apart from this section.

## Performance log — strings-round1 phase 0: the str-term evidence + view-shape spike (Sep 2026)

Evidence phase for the cheap-strings batch (`views over str`): five
findings, no engine change, nothing committed but this section. The
batch's premise — written into the plan's Why — was that k-mer rows
MINT a fresh str cell per slice (alloc + copy). **The premise is stale
and this section re-bases the batch on measured terms**: RFC 0042's
`b2` commit (Sep 17, in the base) already made `s.slice()` an O(1)
`CellData::StrView` — `{parent, off, len, ascii}` over the block
store, no copy — and the block store (`b1a`) made small str mints a
bump + flag. What remains on the string rows are smaller, different
terms, and they decide the shape gate (§0.5) differently than the plan
expected. Rows re-measured today for the batch (one probe binary, one
session, calm box load ~0.2; fuel and VM-heap **bit-identical** to the
pinned records on every row):

| row          | exec today (probe) | fuel        | VM heap peak |
|--------------|--------------------|-------------|--------------|
| nmap-knuc    | 174–208 ms (day range across passes) | 38,814,389 | 4,195,084 B |
| nmapset-str  | 53–63 ms           | 9,551,761   | 983,620 B    |
| json-decode  | 708–950 ms         | 111,322,915 | 34,377,027 B |
| nmapset-int  | 62–71 ms           | 20,703,284  | 1,966,551 B  |
| nmap-primmap | 62–65 ms           | 17,950,301  | 324 B        |
| fasta        | 0.49–0.53 ms       | 200,029     | 16,806 B     |

Check controls (CLI): nmap-knuc `2198604`, nmapset-str `1264308351`,
json `4502015958359127277` — all hold. Honest heap note: json's heap
peak reads 34.38 MB today vs round2's 32.78 MB note, with fuel
bit-identical — the delta is post-`b1a` block-store accounting, not a
workload change (no pin involves json heap).

### Finding 0a — the slice-mint term is already gone; what's left is the host copy

The k-mer row slices exactly **600,088** times per run (199,989
12-mer fill + 100 fragment probes + 200,000 1-mer + 199,999 2-mer).
Post-RFC-0042 each slice mints a ~32-byte view cell (one block-store
bump + one parent retain) — no copy. Sizing, two independent ways:

- **Stub bounds (both spellings SLOWER — the mint is cheaper than any
  surface alternative).** Pre-materializing every k-mer ONCE (checksum
  unchanged, `2198604`) and replacing the per-iter slice with an index
  read made the row dramatically worse: `Vec<str>` spelling
  **+96.2 ms** (+22.5M fuel, +55%); fixed `[?str]` array spelling
  **+65.2 ms** (+11.6M fuel, +32%). Reconciliation: a `callnat
  StrSlice` is ~4–5 VM ops; an index read of a str collection plus its
  nil-narrowing is ~10–37 ops. **Deleting the slice in favor of
  anything the surface can spell today is a pessimization.** The whole
  slice-call+mint share is bounded at ~10 ms ≈ **5–6% of the row**
  (~600k × ~15–20 ns).
- **The host `to_owned` term (the borrowed-probe stub).** The s-lane
  shims copy the key into an owned `String` on EVERY crossing —
  `map_find_s` included, where the copy is dropped unused after the
  probe (`KeyVal::Str(k.to_owned())` feeds a compare-only path). A
  throwaway `nmap.rs` stub (NEVER committed — `git checkout --`
  restored, md5 re-verified, both row checksums re-verified) rewired
  the three s-lane shims to a borrowed probe: hash + compare over the
  `&str` the crossing already borrows, copy only on fresh insert.
  Fuel bit-identical (host-side), behavior bit-identical, ONE fixed
  pair of probe binaries interleaved 5×7, three passes:

| pass | knuc deltas per round (ms)              | knuc median | str deltas per round (ms)         | str median |
|------|-----------------------------------------|-------------|-----------------------------------|------------|
| 1    | +17.3, −1.6, +4.0, −1.5, +4.5           | **+3.6 ms** | +1.7, +2.3, +8.9, +0.1, +2.8      | **+2.8 ms** |
| 2    | +12.6, +12.1, +5.6, +9.3, +3.9          | **+9.3 ms** | +5.7, −3.3, −0.4, −2.3, +0.9      | **−0.4 ms** |
| 3    | +17.2, +7.0, +9.3, +13.6, +2.2          | **+9.3 ms** | +9.7, +0.5, −3.5, −1.9, +6.5      | **+0.5 ms** |

  nmap-knuc: **13/15 rounds positive, ≈ 4–9 ms ≈ 2–5% of the row**
  (1.2M s-lane crossings × ~4–8 ns malloc+memcpy+free). nmapset-str:
  10/15 positive at **≈ 0–3 ms — at the noise floor** (300k
  crossings; ~4.6% best case). Real terms, small ones.
- **Unchanged by any of this**: the `?V` sidecar mint on hit-gets
  (round3's measured ~27 ns/hit; knuc's 1-/2-mer maps take ~400k
  mostly-hit gets ≈ ~11 ms ≈ 6%) and the FNV+probe table body.

**nmapset-str has no slices at all** — its keys are BUILT per op
(`f"k{i}"`: render + 2-part concat + fresh cell). The same two stub
spellings (pre-built keys, checksum unchanged `1264308351`) came out
SLOWER by 9.6 ms (`Vec<str>`) and 6.8 ms (fixed `[?str]`), so the
per-op build is also CHEAPER than the cheapest indexing alternative —
roughly ~10% of the row, and **not deletable by views on this row's
op stream** (the keys are minted, not carved from a parent). A str
twin whose keys ARE ranges of one generated parent is the shape where
the view lever applies (0e).

### Finding 0b — the json census: dispatch-bound; views alone buy ~12%

The decoder walks the doc char-by-char over a shared `Vec<str>` of
1-char cells (split once in setup), dispatches with str `==` chains
(`is_digit` alone is up to 10 one-char compares per digit), and builds
every token by the per-char accumulator (`out = f"{out}{cs[i]}"` — the
in-place append path). Per rep: ~204k chars walked, 10,800 numbers,
6,000 strings, 2,400 literals; the fold then RE-walks every number and
string (`lex_int`, `fold_str`) with the same str compares. Measured
census — three checksum-preserving scratch rewrites of the workload
(all hold `4502015958359127277`, never committed, one probe binary,
4-way interleaved 5×7):

| variant (what it changes)                 | median      | vs base          | fuel delta |
|-------------------------------------------|-------------|------------------|------------|
| base `json-decode`                        | 779.8 ms    | —                | —          |
| `string_join` instead of the accumulator  | 808.0 ms    | **+3.6% SLOWER** | +3.2M (+2.9%) |
| token = ONE `doc.slice` (scan unchanged)  | 687.8 ms    | **−11.8%**       | −14.8M (−13.3%) |
| int-code cursor + `slice` tokens          | 499.1 ms    | **−36.0%**       | −37.0M (−33.3%) |

Read: the **build-per-token term is ~92 ms ≈ 12%** (the slice-token
variant deletes exactly the per-char pushes + join); the one-pass
`string_join` is SLOWER than the accumulator it replaced — the rc==1
in-place append path is already the optimal build spelling, worth
knowing on its own. The **per-char str dispatch is ~189 ms ≈ 24%** —
twice the build term and invisible to views (it is str-cell compares,
not token construction; the int-code variant turns `is_digit` into two
int compares and the char cells into `u32` slots). The remaining ~64%
is the mint machinery (Json records, 3 empty `Vec`s + an opaque box
per value node) and the fold. **Verdict for phase 2: honest NO to
token-slicing-only** — it pays ~12% while the term views cannot touch
is 2× bigger. The measured -36% belongs to the **int-codes cursor**
(a parser rewrite — cursor over `Vec<u32>` codepoints, `slice` per
token), which is a different, non-additive lever; recorded for the
menu.

### Finding 0c — the bytes audit: immutability is provable, bytes views are SOUND

`bytes` carries exactly four members (`len`, `decode`, `clone` — the
one copy escape, RFC 0044 — and the `zeroed(n)`/`from([u8])`
type-methods; `Vec<u8>.freeze()` mints). Element assignment is a
**compile-time diagnostic** ("`str`/`bytes` are immutable — element
assignment is not allowed", `rut-lir/src/lir/slice.rs`), not a trap;
there is no push/set/truncate/resize anywhere on the type. **§0.3's
gate is CLEARED in the strong direction: bytes views would be sound**
by the same immutability argument str views lean on. The batch still
defers them (k-mer and json are str; no consumer needs a bytes view
today) — recorded as a menu item with the audit as its evidence.

### Finding 0d — the shape spike: (A) wins, and smaller than planned

First, the premise correction that reframes the gate: **shape B is
already shipped — internally.** `CellData::StrView` exists (RFC 0042);
`as_str`/`as_bytes`/`char_len`/`str_ascii` read through it; equality,
iteration, f-strings and the host boundary (`HostParam for &str`)
cross it zero-copy; `m.get(seq.slice(i, i+12))` works TODAY as a
range key. The cell repr is off-wire (module binaries never carry
heap cells). What WOULD be new in B is only the named surface type —
and a new builtin type id IS wire-visible (`ConstVal::TypeId` crosses
module surfaces, rebased at link), so the surface-type step is a
VERSION 7 event. The two shapes, honestly costed per use on the
k-mer hot path:

| | (A) sv-range lanes | (B) engine view as a surface type |
|---|---|---|
| key construction | **none** — the caller passes `(parent, off, len)` ints (slots) | one slice-cell mint per key (~15–25 ns) — `slice()` already exists |
| per map-op crossing | 1 crossing, 4 params (t, parent: str, off, len) ≈ +1.4 ns vs today's 2 (crossing-nop slope 0.72 ns/param) | unchanged 2-param `map_find_s` |
| host body | borrowed probe (fnv + compare over the range), copy ONLY on fresh insert — needs the same `nmap.rs` core either way | borrowed probe for the copy term (orthogonal), slice cell stays |
| deleted terms (knuc, measured) | slice-mint ~5–6% + to_owned ~2–5% ≈ **−7 to −12% projected** | to_owned only ≈ **−2 to −5%** |
| surface cost | 3 additive crossings (`nmap.d.rut` + CLI fixture mirror, update-BOTH), no engine repr change, **no VERSION bump** | type-system surface (boot-table type, checker admission into the key union, decls, RFC 0004 amendment), **VERSION 7 risk** |
| the `StrView` record part of A | `view()` ctor = one record mint ≈ today's slice cell (no win); eq/index/materialize need NEW crossings; a record is not a str cell — general `&str` acceptance FAILS under A without engine work | reads-as-str acceptance is free (the cell kind is the engine's) |

**Recommendation: (A), reduced to its minimal surface — the three sv
lanes and nothing else.** §0.7's condition for preferring B ("only if
A measures a real tax that a cell kind would remove") is not met: the
lanes add ~1.4 ns of crossing args against ~25–35 ns of deleted
per-key work, and the cell kind B would add is the one the engine
ALREADY has. The named-view surface (A's record or B's type) has **no
measured consumer need** — its construction costs what today's slice
costs — so both record and type are DEFERRED, and the plan's
"crossing acceptance" law lands as: the sv lanes accept any `str` as
the parent, including a slice view (flattens to the root — tested).

**§0.6 answered: no view-keyed twin class is needed.** The keys stay
`str`-kind; the sv lanes are an s-lane overload — the wrapper
`HashMap<str, V>` gains range-taking methods (`get_range`/
`put_range`/`has_range`/`remove_range` or the phase-2 naming) over
the same host table. The PrimMap precedent does not apply: PrimMap
split classes over the VAL kind (a get has no V value to dispatch
through); here nothing splits — the key type is unchanged and the
stored key remains an owned host copy, so "stored view keys pin their
parents" is satisfied trivially (nothing rut-side is stored) and the
documented law costs nothing.

**The minimal surface (phase 1's spec):**
1. `map_find_sv(t: opaque, parent: str, off: i64, len: i64) -> i32` —
   the `find_s` answer encoding; hashes (FNV-1a 64) and compares over
   the parent's borrowed byte range; NO copy.
2. `map_entry_sv(...same...) -> i32` — the `entry_s` encoding incl.
   the fused grow-first sentinel; the owned copy is taken only on the
   fresh-insert path.
3. `map_remove_sv(...same...) -> i32` — tombstone the range-match,
   `-1` absent.
4. Host: a borrowed-probe core (`probe_str_borrowed` + `fnv_bytes` —
   measured bit-identical in 0a's stub) inside `NativeTable`; offsets
   are BYTE offsets with a host-side UTF-8 boundary check (trap on a
   split codepoint) so ASCII callers get byte==codepoint for free.
5. Decls in `rut/nmap_host/nmap.d.rut` + the CLI fixture mirror
   (update-BOTH); wrapper methods on `HashMap<str, V>`; tests: the
   parity law (a range key and the equal-content str key answer the
   same slot and the same iteration position), parent-is-a-view
   acceptance, boundary/off/len traps, insert-copies-once behavior.
No engine repr change, no VERSION bump, no existing surface touched.

### Finding 0e — the workload plan

- **`kmer-view`** (dir twin of `nmap-knucleotide`): the identical
  k-mer counting with the 12-mer fill, fragment probes and 1-/2-mer
  maps keyed through the sv lanes (`m.get_range(seq, i, 12)` shape).
  **Checksum provenance: equals the pinned `2198604`** — same keys,
  same values, same formula; the .js twin is UNCHANGED (it already
  computes the same answer — the parity IS the gate). No new
  expected.json line.
- **`strview`** (twin of `nmapset-str`): the same six-phase churn with
  keys carved as ranges of ONE generated parent string — the shape
  where range keys apply (0a showed built keys gain nothing). Same
  counter formula → a NEW checksum, verified on rut/qjs/node, ADDED to
  `expected.json` as one disclosed line (the nmap-primmap precedent).
  The .js twin is the same materialized computation over
  `substring` — a few lines.
- **json: no view row.** The 0b census verdict is dispatch-bound;
  token-slicing alone measured −11.8% against a 2× bigger term views
  cannot touch. The int-codes cursor (−36.0% measured) stays on the
  menu as its own lever.
- Gates carried into phase 2: every existing row fuel+heap
  bit-identical (the lanes are additive crossings — nothing existing
  can move), the four nmap checksum pins hold, `expected.json` gains
  exactly one line (the `strview` pin).

House-keeping notes for the record: every stub this phase lived under
`/tmp/opencode/batch-strings-round1/p0/` (scratch workloads with
absolute dep paths) EXCEPT the one `nmap.rs` window — patched, both
checksums re-verified, binaries banked, restored in the same command
breath, md5-verified (`git checkout --`), pristine rebuild — matching
the round3 phase-0 restore discipline. One probe binary per
experiment; all deltas same-session interleaved; the nmapset-str
to_owned row was re-run twice more when the first pass's ranges
overlapped (three passes recorded above, honestly split).

Files this phase: `benches/README.md` only. Scratch:
`/tmp/opencode/batch-strings-round1/p0/` (stub workloads, pair JSONLs,
probe binaries, bytes-audit repro). Tree clean apart from this section.

## Performance log — strings-round1 phase 2: the view consumers + the view rows (Sep 2026)

Phase 0d's minimal surface lands as a CONSUMER: the phase-1 sv
crossings (`map_{entry,find,remove}_sv`) become reachable from
`nmapset` users, and the two disclosed view rows exercise them
end-to-end. No engine change, no VERSION bump, **no new crossings** —
`rut/nmap_host/nmap.d.rut` is untouched this phase (both copies: the CLI
fixture mirror already carries the sv decls), `expected.json` gains
exactly one line.

### The consumer surface: four additive range-keyed methods on `HashMap`

`put_range` / `get_range` / `has_range` / `remove_range` on
`HashMap<K, V>` (in `rut/nmapset/nmapset.rut`): the key crosses as a
borrowed `(parent: str, off: i32, len: i32)` BYTE window through the sv
lanes — hashed/compared over the range, no key cell ever minted on a
probe, one owned copy on a fresh insert (stored keys stay owned; the
stored-key/iteration law unchanged). `get_range -> ?V` is the plain
`get` law; `put_range`/`remove_range` are `put`/`remove`'s exact
shapes including the fused `i32::MIN` sentinel → grow + `vals`
relocation drain + retry. Answers follow the same-key law: a range key
and the equal-content `slice` key are THE SAME key through either
spell (same recorded FNV-1a, same slot, same iteration position).

Shape reasoning, recorded in the pkg header: **no view-keyed twin
class** — keys stay `str`-kind and the sv lanes are an s-lane overload
(§0.6; PrimMap's class split was over the VAL kind, where a get has no
`V` value to dispatch through — that pressure does not exist here).
The methods sit on the GENERIC class because rut has no
per-instantiation divergence (the union bound is admission only — the
round3 phase-0c verdict); the CONTRACT is `K = str` (the only key type
with a range spelling), with `off`/`len` in bytes — note `str.slice`
is codepoint-offset, so the two spellings agree numerically only on
ASCII parents (both bench rows are ASCII; the distinction is
documented, not hidden). `HashSet`/`PrimMap*` stay range-less: no
consumer row needs them, and additive discipline beats symmetry.

Tests (`crates/rut-driver/tests/nmap_viewkeys.rs`, 6 new): the parity
law through the churn — the nmapset-str six-phase shape at n = 2000
over one 6-char-slot parent, spelled both ways, one pinned checksum
(`2572351`); lane interchange both directions on ONE table (range put
→ slice get, slice put → range get, cross-lane removes, replace via
either spell never grows the table, a slice VIEW as the parent
flattens to the root); a 500-key multi-grow sweep through `put_range`
(cap 4, sentinel grows + remove/re-add churn, every val exact — the
drain works through the sv lane); the empty range key == the `""` key
at every boundary; the UTF-8 boundary and past-end traps surfacing the
house `Invalid` shape through the wrapper (the raw lanes'
store-nothing-on-trap law is phase 1's suite, still green).
Workspace: 77 suite runs, 474 tests, zero failures (was 76/468).

### The rows

- **`kmer-view`** (dir twin of `nmap-knucleotide`): the identical
  k-mer counting — same LCG sequence, same 12-mer fill + 100 fragment
  probes + 1-/2-mer maps, same readout and formula — with every hot
  k-mer keyed through `get_range(seq, i, k)` / `put_range(seq, i, k,
  ·)`. The twin's ~600,088 slice mints are gone; the readout tail
  (20 cold lookups) keeps the twin's slice/f-string spelling on
  purpose. Checksum = the pinned `2198604`; **no expected.json line**
  (the pin is knuc's own line — parity is the gate). The .js twin is
  nmap-knucleotide.js's materialized computation, unchanged (JS has no
  borrowed-range key).
- **`strview`** (twin of `nmapset-str`): the six-phase churn with keys
  carved as fixed-width 6-char injective decimal slots of ONE
  generated parent (`slot(i) = f"{100000+i}"`; churn keys read slots
  `0..n-1`, never-inserted miss keys read slots `n..2n-1`), every op
  through the range methods. This is the shape where the view lever
  applies — phase 0a showed built keys gain nothing, carved keys are
  the point. **Checksum provenance, disclosed**: the formula reads
  counters + the value sum only (key content is invisible to it), and
  the twin's counters and value stream were deliberately kept
  identical — so the row's pin lands on `1264308351`, the SAME value
  as the `nmapset-str` line, and `expected.json` gains that one
  disclosed line (the nmap-primmap precedent: its pin equals
  nmapset-int's `734932704`). Two provenance checks: the row answers
  identically on rut/node/qjs, and a perturbation run (replace value
  `i+3` → `i+7`) moves the checksum by exactly the predicted
  `+200000` — the row computes its own answer, it is not echoing the
  twin. The .js twin is the same materialized computation over
  `substring`.
- **json: no view row.** Phase 0b's census verdict stands
  (dispatch-bound; token-slicing alone −11.8% against a 2× bigger term
  views cannot touch).

### The matched pairs (house method)

One probe binary, both rows of each pair from the same tree,
interleaved 10 rounds × 7 fresh-VM iters, order alternated per round;
fuel/heap single-valued across every round of every row
(deterministic):

| pair | view row (ms) | str twin (ms) | median delta | rounds positive | fuel (view vs twin) | VM heap peak (view vs twin) |
|---|---|---|---|---|---|---|
| kmer-view vs nmap-knucleotide | **154.47** (147.9–158.5) | 172.53 (167.3–177.8) | **−19.59 ms = −11.4%** | 10/10, ranges non-overlapping | 37,614,177 vs 38,814,389 (−1,200,212, −3.1%) | 4,194,916 vs 4,195,084 (−168 B) |
| strview vs nmapset-str | **38.95** (38.5–42.1) | 52.84 (51.8–62.0) | **−13.66 ms = −25.9%** | 10/10, ranges non-overlapping | 10,801,744 vs 9,551,761 (**+1,249,983, +13.1%**) | 2,032,173 vs 983,620 (**+1,048,553 B**) |

- **kmer-view**: −19.59 ms over the row's exactly 600,088 k-mer sites
  ≈ **−32.6 ns per deleted slice+recross** — the slice-cell mint
  (phase 0a's ~10 ms bound) plus the view-key crossing indirection the
  borrowed probe deletes. Fuel −3.1% is the slice callnat's ops; heap
  moves −168 B because the peak only ever held a handful of the
  600k slice cells alive at once (they die immediately) — heap was
  never this row's story. Phase 0d's projection for the whole sv
  shape was −7 to −12%; the pair lands at the top of it (phase 1
  already banked the to_owned half; this is the slice-mint half plus
  the direct-range crossing).
- **strview**: −25.9% at **+13.1% fuel** — the honest inversion, and
  the row's story: the twin's per-op `f"k{i}"` build was
  TIME-expensive (render + concat + fresh cell + the probe reading a
  scattered allocation) but fuel-CHEAP (~3 ops); the view spelling
  trades it for a one-time parent build (100k f-string appends ≈ the
  whole fuel increase) plus per-op window arithmetic and two extra
  crossing args. Net: **−48.2 ns over the row's 283,334 range-keyed
  ops**, with the probe now reading packed bytes out of one
  contiguous parent block. Heap +1.05 MB is the 600,000-char parent
  (plus append transients) living through the whole run — a fixed
  O(parent) floor the per-op-mint twin never pays; both rows' live
  stored-key sets are the same size, so the delta is exactly the
  parent.

Guards: full sanity run all rows × rut/qjs/node — zero checksum
mismatches cross-runtime and against `expected.json` (76 cross rows;
the four nmap pins `2198604` / `1264308351` / `21500055` / `734932704`
and nmap-primmap's hold); fuel + heap **bit-identical** on every
pinned probe row (knuc 38,814,389/4,195,084, str 9,551,761/983,620,
int 20,703,284/1,966,551, primmap 17,950,301/324, hashset
13,267,176/551, json 111,322,915, crossing-nop 104,000,032/236,
hashmap-int 63,758,210, alloc 22,000,020/228, sieve 19,592,209). No
neutral row moved anywhere (fuel/heap deterministic across all
interleaved rounds — the phase-1 layout lesson's check; nothing needed
isolating). Files this phase: `rut/nmapset/nmapset.rut` (+94, all
additive), `benches/workloads/kmer-view/{main.rut,rut.toml}`,
`benches/workloads/kmer-view.js`, `benches/workloads/strview/
{main.rut,rut.toml}`, `benches/workloads/strview.js`,
`benches/workloads/expected.json` (+1 line), `benches/README.md` (this
section + the two Workloads-table lines), and
`crates/rut-driver/tests/nmap_viewkeys.rs`. Scratch:
`/tmp/opencode/batch-strings-round1/p2/` (smoke dirs, perturbation
run, guard JSON, pair script + JSONL).

## Performance log — strings-round1 close-out: the verdict (Sep 2026)

The batch's gate run, measurement only — no engine, stdlib, or
workload change. Both runtime binaries (`rut` CLI + `rut-bench-probe`)
rebuilt from THIS checkout at HEAD `a2f9d35` (tree clean; cargo
fingerprints Fresh — a second build pass is a no-op), then the full
suite in one pass: **26 workloads × rut/qjs/node from one checkout,
exit 0, every checksum equal `expected.json`** — 76 cross rows, 73 of
them checksum-carrying (`empty` carries none; `kmer-view` carries its
checksum on all three runtimes but deliberately has no `expected.json`
line — parity with knuc's pin IS its gate), zero mismatches. The pins
hold bit-identically on every runtime: **kmer-view = nmap-knucleotide
= `2198604`; strview = nmapset-str = `1264308351`; json-decode
`4502015958359127277`; nmapset-int = nmap-primmap = `734932704`;
nmap-hashset `21500055`**. **Fuel and VM-heap are bit-identical to the
phase-2 records on all 26 probe rows** (26/26 diff = 0, including the
two new view rows: knuc 38,814,389/4,195,084; kmer-view
37,614,177/4,194,916; str 9,551,761/983,620; strview
10,801,744/2,032,173; json 111,322,915/34,377,027 — the post-b1a
accounting; int 20,703,284/1,966,551; primmap 17,950,301/324;
hashset 13,267,176/551; crossing-nop 104,000,032/236; hashmap-int
63,758,210; alloc 22,000,020/228; sieve 19,592,209). Workspace green
(77 suites, 474 tests, zero failures).

Box state, recorded up front: the verdict run started at load ~1.5
(this close-out's own release build had just finished) decaying to
~0.3–0.5 — absolute medians run warm, which is why every verdict below
is a same-run matched pair, and the qjs ratios are cross-checked
against phase 2's own guard run.

### The cumulative story (matched pairs)

The batch's two instruments: phase 1 improved the EXISTING rows'
s-lane host path (fuel/heap provably identical — the win is
host-side); phase 2 added view twins that re-spell the key stream
(new rows, existing rows untouched). Both landed:

| pair | phase-1 banked (existing rows, 6/6 rounds) | phase-2 view twin (captured, 10/10) | verdict-day same-run (10/10) |
|---|---|---|---|
| kmer: nmap-knucleotide → kmer-view | −7.8 ms ≈ −4.3 % (borrowed probe deletes the per-crossing `to_owned`) | **−19.59 ms = −11.4 %** (154.47 vs 172.53; −32.6 ns per deleted slice+recross ×600,088) | **−19.58 ms = −10.6 %** (165.57 vs 185.15) |
| str: nmapset-str → strview | −2.15 ms ≈ −3.9 % (same deletion) | **−13.66 ms = −25.9 %** (38.95 vs 52.84; −48.2 ns over 283,334 range-keyed ops) | **−15.63 ms = −26.7 %** (42.89 vs 58.53) |

The verdict-day pairs reproduce both captured numbers in direction
and magnitude (10/10 rounds positive each, ranges non-overlapping);
the batch's captured numbers stay phase 2's powered runs. Fuel/heap
were single-valued in every round of every row and equal the pins —
the wins are time-only on knuc's side (fuel −3.1 % is the deleted
slice callnat) and bought-with-fuel on strview's (the honest +13.1 %
parent-build inversion, +1.05 MB parent-retention heap — both
disclosed in phase 2, unchanged).

### The qjs scoreboard (where twins exist)

Whole-net ratios (rut net / qjs net, wall − startup), verdict run,
with phase-2's guard run in parens — the placements reproduce across
sessions:

- **strview 0.88× (0.87×) — the batch's headline: the first
  str-shaped row AHEAD of its qjs twin whole-net** (50.18 vs 57.20 ms
  this run; 47.19 vs 54.14 in phase 2's). The carved-key churn shape
  is exactly where borrowed range keys win, and it is enough to close
  a 1.49× gap to negative.
- kmer-view 1.13× (1.16×) vs the twin row nmap-knucleotide 1.23×
  (1.28×) — the view spelling moved the row most of the way to qjs;
  the remainder is the readout tail + fill pattern, not keying.
- nmapset-str 1.40× (1.49×) — the phase-1 borrowed-probe half shows
  here without the view re-spelling.
- json-decode 11.99× (11.30×) — unchanged by this batch (below).
- nmapset-int 1.29× / nmap-primmap 1.20× — round3's rows, unmoved.

### Disclosures that ride the view rows (standing, from phase 2)

- **strview's same-pin disclosure**: its checksum `1264308351` EQUALS
  nmapset-str's line by structure — the formula reads counters + the
  value sum only, and the twin's counters/value stream were kept
  identical (the nmap-primmap precedent). Two provenance checks
  recorded: 3-runtime agreement, and a perturbation run (replace value
  `i+3` → `i+7`) moving the checksum by exactly the predicted
  `+200000`. The row computes its own answer; it is not an echo.
- **Byte-vs-codepoint offsets**: the range methods take BYTE
  windows; `str.slice` is codepoint-offset. The two spellings agree
  numerically only on ASCII parents — both view rows are ASCII, and
  the contract is documented in the `nmapset` pkg header (UTF-8
  boundary/past-end windows trap house `Invalid` host-side).

### json's honest NO (re-affirmed by the verdict)

No view row, per phase 0b's census: the row is DISPATCH-bound.
Per-char str dispatch ≈ 24 % of the row vs the build-per-token term
≈ 12 % — token-slicing via views measured −11.8 %, a real but
second-order lever against a 2× bigger term views cannot touch
(rewriting the char cursor is a decoder rewrite, not a key spelling).
The **int-codes cursor (−36.0 %, −37.0 M fuel) stays recorded as a
WORKLOAD-LEVEL lever** — it is a different decoder shape, not claimed
by this batch's view mechanism. `string_join` replacing the f-string
accumulator measured +3.6 % (the rc==1 in-place append path is
already the optimal build spelling). The row is untouched: fuel
111,322,915 / heap 34,377,027, checksum `4502015958359127277`,
bit-identical everywhere.

### Honest neutrals

Every row outside the two view twins is a NEUTRAL: this verdict's
26-row fuel/heap sweep diffs ZERO against phase-2's guard (and phase
2's guard diffed zero against the pre-batch pins on the 24 inherited
rows) — the op streams provably did not move. Exec medians differ
only by box state (warm verdict vs calm/loaded earlier sessions).
No performance claim is made for any row without a same-run pair
behind it. Phase 1's layout lesson stands recorded: a probe-dense
row can read a reproducible ±4 % from codegen layout alone — which
is why every batch here measured interleaved before believing a
number.

### Re-baselined follow-up menu

- **(a) Bytes views** — SOUND per phase 0c (bytes has NO in-place
  mutation op; element assignment is a compile-time diagnostic), and
  now cheap to add: the borrowed-probe core + one FNV-1a range
  hasher (`hash_bytes`) are type-agnostic over bytes. Needs a
  bytes-keyed consumer row before it earns the surface.
- **(b) json int-codes cursor** — the measured −36.0 % workload-level
  lever (phase 0b), the honest path at json-decode's 12× gap. A
  different decoder shape (Vec<u32> codepoints, int compares,
  slice tokens), i.e. next batch's row, not a view claim.
- **(c) PrimMapBool's `?bool` checker fix** — value reads unbox, so
  `p == nil` diagnoses a width mismatch (RFC 0004 §3); the fix gates
  the fourth prim lane, design already prices it (bool vals as 0/1
  in the u64 column).
- **(d) Interleaved column layout** — round3's menu item, still open
  with the nmap-primmap harness live (predicted check ≤ ~20 ns/op
  against 102.6 ns/op).
- **(e) SIMD str builtins** — only if a future census points there;
  phase 0b's census pointed at VM dispatch, not builtin bodies.

Files this phase: `benches/README.md` only. Scratch:
`/tmp/opencode/batch-strings-round1/p3/` (verdict.json/.md/.csv/.log,
pairs script + JSONL + load log).

## Performance log — refvals phase 0: the ref-V baseline, PRE-EXPERIMENT (Sep 2026)

The refval-exp batch's measuring stick. The batch (plan: an opaque val
column for reference V, read back via downcast) is gated on this row:
**no bench row today exercised reference values at all** — every map row
stores primitive/str vals — so phase 0 lands `refvals`, a record-valued
`HashMap<i64, Pt>` over the `[?V]` sidecar, and measures the baseline
the experimental `refcolumn` row must beat. The row is a keeper
regardless of the verdict (permanent ref-V coverage). This section is
the pre-experiment record; phase-2 comparisons read their before
columns HERE. No engine change, no pkg change, one expected.json line.

### The row's shape, and why

`HashMap<i64, Pt>` (`Pt = { x: i32, y: i32 }`) — **K = i64
deliberately**: the key lane (`map_entry_i`/`map_find_i`, one mix64 per
op, the nmapset-int precedent) is held constant so the row isolates the
VALUE-side costs; str keys would confound every term with the
encode()-per-hash cost the strings-round1 rows already price. The churn
runs the shapes that matter, in order:

1. **insert/overwrite churn** (2 × 100 k puts, a fresh `Pt` cell minted
   per put — the overwrite pass re-stores every key, so the sidecar
   releases the replaced cell and stores the new one);
2. **hit-heavy gets** (100 k hit gets reading both fields + 20 k
   misses, 5:1);
3. **read-modify-write through the alias** — the one-cell law in the
   hot loop: `p.x += 1` through the get-returned `?Pt`, no put, no set;
   a fresh-get read-back pass (`hits2`/`sum_after`) then observes every
   write-through. **The checksum depends on the write-through**:
   `sum_after = Σ(i+7) + n` only if the writes landed in the stored
   cells — a copy-on-get engine answers a different number (by exactly
   2 n) and fails the row loudly.
4. **grow-heavy sweep** — a fresh 200 k-entry map from cap 8 through
   every load-factor boundary (15-16 grows, the relocation drains
   moving ~460 k cells, largest last), then an overwrite pass and a
   full read-back.

Total: 1,320,002 map ops (one crossing each), ~650 k record mints.
Checksum discipline is the nmap-primmap precedent, adapted: i64
throughout, printed once, never narrowed — every term an exact integer
below 2^53, so the JS twin's doubles compute the same number
bit-exactly. **Twin gate: `140052990000` equal on rut / qjs / node**
(the `.js` twin uses JS objects as values — property mutation through a
retrieved reference IS JS's own semantics, the natural twin shape).
The pin is new work (independent construction; it reconciles bit-exact
against the counter formula: `sum_before` 10,014,000,000 = Σ(2i+15),
`sum_g` 39,999,800,000 = Σ2i, counters 100 k/100 k/20 k/200 k/200 k/
50 k/50 k/50 k at weights 7…59).

One property is disclosed IN-SOURCE because the stub A/B below relies
on it: **every value the checksum reads is written after its map's
last grow** (both overwrite passes), so deleting the relocation drain
in a scratch wrapper stub leaves the checksum bit-identical — the stub
provably moves only drain traffic.

### The numbers of record (house method, one checkout, release build)

Cross-runtime (net medians, wall − startup; two same-day runs, the
guard run second): rut net **376.8 / 373.4 ms**, qjs **304.5 / 302.8**
(rut/qjs ≈ **1.24×**), node **105.0 / 96.0** (~3.6×). Probe: compile
~7 ms, **exec median 337-341 ms**, **fuel 56,899,541**,
**VM-heap peak 28,801,340 B (27.47 MiB)**. Per map op: ~258 ns exec,
43.1 fuel-ops.

Per-phase fuel (exact, additive — single-phase clones of the churn,
same probe binary, interleaved rounds; nets are clone-minus-build
control):

| phase (ops each)                      | fuel/op | exec/op (noisy) |
|---------------------------------------|---------|-----------------|
| fresh put, m build (100 k, 15 grows)  | 72.0    | ~490 ns         |
| fresh put, g build (200 k, 16 grows)  | 74.0    | ~605 ns         |
| overwrite put (flat table)            | 40.0    | ~215 ns         |
| hit-get, 2 fields read                | 32.0    | ~156 ns         |
| hit-get, 1 field read                 | 29.0    | ~143 ns         |
| miss-get                              | 12.0    | (noise floor)   |
| rmw (get + field write)               | 30.0    | ~120 ns         |
| remove / has / re-add (avg over 200 k)| 24.8    | ~131 ns         |

(The exec column carries ±ms clone noise — the fuel column is the exact
instrument; time attribution below uses the row-level stub pairs.)

### The sidecar terms isolated — the numbers phase 2 is judged against

House method: throwaway stubs of `rut/nmapset/nmapset.rut` (NEVER
committed — one fixed probe binary with the wrapper source flipped
between interleaved rounds, the round2/round3 phase-0 method; pkg
sources are runtime-mounted; `git checkout --` restore + md5 verified
before/after every pass; tree verified clean). Two stub shapes:

- **NOVALS** — `put` drops the vals stores AND the drain walk (the
  `HashSet` shape: one crossing + grow + retry); `get` returns `nil`
  after the `nfind` (hit/miss control flow kept); `remove` drops the
  nil store. Checksum becomes deterministic **`37790000`** and
  reconciles BIT-EXACT: every value-reading counter is zero (`hits`,
  `rmw`, `hits2`, `hits_g` = 0; all sums 0), every key-only counter
  unchanged (100 k/100 k/20 k/200 k/200 k/50 k/50 k/50 k/100 k/200 k
  at weights 7…59) — 37,790,000 exactly, on every run.
- **DRAIN** — ONLY the relocation walk deleted (`next` still
  allocated, walk gone). Checksum **bit-identical `140052990000`** —
  the in-source invariant above, verified.

Full row, interleaved 5 rounds × 5 fresh-VM iters per side, order
rotated (round ranges: A 334.5-345.9, B 139.8-149.0, C 323.8-332.4 —
A/B non-overlapping by 185 ms):

| side        | median of medians | fuel       | heap peak  |
|-------------|-------------------|------------|------------|
| A — the row | **341.13 ms**     | 56,899,541 | 28,801,340 B |
| B — NOVALS  | **147.18 ms**     | 38,040,570 | **892 B**  |
| C — DRAIN   | **329.48 ms**     | 46,990,694 | 27,892,124 B |

The split:

| term | isolated by | time | share |
|------|-------------|------|-------|
| **the whole value-sidecar package** (store + load + drain + alias-write) | A−B | **193.95 ms** | **56.9%** |
| **store + load + alias-write** | C−B | 182.30 ms | 53.4% |
| **the relocation drain** | A−C | 3.6-11.7 ms (two passes; see note) | 1.1-3.4% |
| the floor (host table body + ex-sidecar VM stream + record mints) | B | 147.18 ms | 43.1% |

Drain honesty: the first pass read A−C = **11.65 ms** (ranges
non-overlapping, barely); a dedicated powered 7-round × 5-iter pass
read **3.63 ms** with overlapping round ranges (A 332.0-344.6 vs C
324.8-340.8). Recorded as a band, not a point: the drain is
fuel-heavy but time-light — its fuel is EXACT
(**9,908,847 ops = 17.4% of the row's fuel**, ~460 k moved slots at
~21 ops/slot: the `map_take_reloc` crossing + unpack + the scattered
`next[new] = vals[old]` moves), but the walk is sequential
cache-friendly copying, cheap per op next to the random-access probe
path.

Store vs load (per-phase clone pairs, both wrapper sides, interleaved
3 rounds × 3 iters — deltas are clone-minus-clone, setup cancels):
the **store** term is ~**107 ns** per overwrite-put (fuel +5/put:
the `MakeOpt` one-slot cell mint + the sidecar slot store; the
replaced cell's release is host-side, fuel-free) and the **load** term
~**65 ns** per hit-get (fuel +8-11/get: the `arrget` + cell deref the
field reads go through). Scaled over the row's 650 k puts and 500 k
value-reading gets, store ≈ 3.25 M and load ≈ 3.5 M fuel of the exact
18,858,971-op A−B delta; the remainder is the get-escape/nil-test
stream — no single hidden term.

Per-op budget, stated the way phase 2 should read it (A = 341.13 ms,
~258 ns over 1.32 M map ops): **value-sidecar package 56.9%** (measured
above), **VM interpreter stream ~10%** (43.1 fuel-ops/op × the
0.77 ns/op crossing-nop calibration ≈ 33 ms), **the crossing ≤ ~1%**
(exactly one direct host call per map op; ≤ 2.9 ns bound), and the
**~32% residual is the NOVALS floor's host side** — the key probe over
a ~28 MB working set (DRAM-miss bound, the round3 0b ladder's
signature) plus the 650 k record mints and their rc, which the stub
keeps (the put argument is still minted; only its storage is stubbed).

Heap, and a pre-registration for phase 2: the 27.47 MiB peak is **the
stored record cells themselves** (~300 k live records at peak,
~96 B/record) — the `[?V]` sidecar arrays are plain host memory,
invisible to cell accounting, and the NOVALS control collapses the cell
heap to **892 B** while answering its checksum. `refcolumn` will hold
the SAME records (retained in the val column), so **heap parity is the
expected outcome**; the experiment's delta should show up in
time/fuel (store/load spellings, the drain deleted by construction),
not in the cell heap. Any existing row moving is a bug (§0.5).

Gates at commit: all 27 probed rows fuel/heap bit-identical to their
committed records (16 pinned rows checked exactly, incl. the four
nmapset pins' checksums `734932704` / `1264308351` / `21500055` /
`2198604`); full suite rut/qjs/node exit 0, every checksum equal
`expected.json`; `expected.json` gained exactly one line
(`refvals`); workspace 525 tests, 0 failures. Files:
`benches/workloads/refvals/{rut.toml,main.rut}`,
`benches/workloads/refvals.js`, the `expected.json` line, this
section. Scratch (stubs, phase clones, pair JSONLs, guard runs):
`/tmp/opencode/batch-refval-exp/p0/`.

## Performance log — refcolumn phase 1: the experimental surface (Sep 2026)

The experiment itself, additive only (plan §0.1): ONE class, TWO
crossings, ONE row — revert is one commit deleting the three.

- **The crossings** `map_val_set_o(m, slot, v: opaque)` /
  `map_val_get_o(m, slot) -> opaque` (decls in `nmap.d.rut` + the CLI
  fixture mirror, update-BOTH): the val column through reference
  values. `NativeTable` gains a LAZY second column (`meta`) holding one
  self-releasing `OpaqueRef` owner per stored val box — allocated at
  the first opaque store, cap-aligned, moved with the keys inside
  `grow`'s existing walk (no drain for this path, round3's law),
  dropped slot-wise by the tombstone and wholesale by the table's
  `Drop`. The rc discipline (plan §0.4, the phase-6 playbook,
  documented in-source): retain on insert (one map-owned handle),
  release on overwrite (the replaced owner drops) and on remove
  (tombstone drops the owner), full release at table teardown incl.
  arena-teardown-with-live-cells (the owners carry the arena), no
  double-release path. Traps: out-of-range slots (the val lanes' shared
  law) and FOREIGN opaques — a host payload box crossed as the val is
  rejected at the store, loud.
- **The layout deviation, disclosed** (the only touch to existing
  machinery, fuel/heap/behavior identical): fitting the owner-column
  pointer inside `size_of::<NativeTable>() = 144` — the shallow size
  `OpaqueBox::alloc` charges at every `map_new`, so ANY growth moves
  every map row's heap pin (the batch's bit-identical gate) — required
  shrinking the relocation queue from a `VecDeque<(i32, i32)>` (32 B)
  to a packed `Vec<u64>` (24 B), drained from the end. The contract is
  the drained SET of independent pairs, never an order; same pairs,
  same `-1` drain end, same op count. A new unit test pins
  `size_of::<NativeTable>() == 144` as the tripwire.
- **The class** `nmapset::RefMap<K requires <the closed key union>, V>`
  — the PrimMap precedent's third family, keys on the same typed lanes
  (per-instantiation monomorphization), values in the column. Header
  documents: for REFERENCE V; prim V belongs on PrimMap (V is
  unboundable — no ref-type union exists, so admission cannot branch,
  round3 0c; a prim V would run but pay the box for nothing). The
  LEANER crossing shape, taken and documented: the wrap happens ONCE at
  put (`opaque(v)` — the box IS the stored unit), a get re-hands the
  SAME box — no per-get wrap — and unwraps with `opaque.downcast<V>`
  to the LIVE inner cell (the one-cell law). Known terms recorded for
  phase-2 attribution: the per-put box mint, the per-get downcast, and
  the +32 B accounted box cell per live value — which is why raw heap
  parity with the sidecar row is NOT the honest expectation (the
  phase-0 pre-registration assumed the column would retain the records
  directly; the crossing shape cannot take a record as `opaque`
  without the box, and an engine change to allow that is out of the
  experiment's scope).
- **The row** `refcolumn`: refvals' exact op stream over `RefMap<i64,
  Pt>`, checksum `140052990000` on rut/qjs/node bit-exactly — the
  one-cell law carries (the RMW-through-alias phase depends on it), and
  `expected.json` gained exactly one line. The `.js` twin is refvals'
  computation unchanged (JS has no sidecar to swap).
- **Tests** (9 new driver tests, `crates/rut-driver/tests/
  nmap_refmap.rs`): the one-cell law both directions + multiple
  holders; the replace law (held aliases keep the pre-replace cell);
  remove with a held alias surviving; 500-key sentinel-growth sweep
  with no drain; the rc discipline BYTE-ACCOUNTED at checkpoints
  (overwrite: 45 mints − 45 released big pairs = −720 B, distinguishable
  from a skipped release's +2880; remove: 15 small + 15 big; teardown
  with 60 live cells: exactly 30·64 + 30·80 + 24 + 144; everything
  refunded to the pre-build mark); the arena-teardown smoke (the table
  box crosses into Rust with 50 live cells, Vm dropped); the trap
  matrix (both bounds directions, foreign host box, unstored slot); the
  checksum law (RefMap == HashMap<i64, Pt> bit for bit at n = 2000,
  pinned `57059800`, hand-reconciled from the exact counters); a
  str-keyed instantiation on the s lane.
- **Gates at commit**: all 12 pinned probe rows fuel/heap
  bit-identical to their committed records (incl. refvals 56 899 541 /
  28 801 340 and the four nmapset rows); full suite rut/qjs/node exit
  0, every checksum equal `expected.json` (82 cross rows); workspace
  535 tests, 0 failures; no VERSION bump; expected.json new lines only.
  First-read probe numbers for the record, NOT a verdict (phase 2 runs
  the matched pairs): refcolumn 55 390 560 fuel / 21 600 999 B heap /
  443.9 ms cold median vs refvals 56 899 541 / 28 801 340 / 338.0 ms
  in the same run — fuel −1.5 M (the drain and sidecar spells gone),
  heap −7.2 MB ≈ the box cells replacing the sidecar arrays, time
  reads slower cold and is phase 2's question.

## Performance log — refcolumn phase 2: the verdict — the column REVERTED (Sep 2026)

The pre-registered decision (plan §0.6), now applied: **refcolumn is
SLOWER than refvals beyond noise, so the experimental surface is
REVERTED** — one commit deleting `RefMap` + the two `map_val_set_o`/
`map_val_get_o` crossings + the `refcolumn` row, keeping `refvals` and
the analysis (phases 0-1's sections above stay as the experiment's
record). This section records the verdict and WHERE the cost landed.

### The matched pair (house method)

Both rows of ONE checkout, one release build, interleaved 7 rounds ×
7 fresh-VM probe iters per side, order alternated per round, median of
the round medians (fuel and heap are pure counts — single-valued in
every round of every side, deterministic):

| side        | exec median of medians | round-median range | per map op | fuel        | VM heap peak |
|-------------|------------------------|--------------------|------------|-------------|--------------|
| refvals     | **337.68 ms**          | 330.7 – 341.7      | 255.8 ns   | 56,899,541  | 28,801,340 B |
| refcolumn   | **383.93 ms**          | 380.3 – 398.3      | 290.9 ns   | 55,390,560  | 21,600,999 B |
| **delta**   | **+46.26 ms (+13.7%)** | ranges NON-overlapping | **+35.1 ns/op** | −1,508,981 (−2.7%) | −7,200,341 B |

refvals reproduces its phase-0 record (334.5-345.9 round range then,
330.7-341.7 now; 255.8 vs ~258 ns/op) — the baseline did not move.
The verdict signal is unambiguous: every refcolumn round-median is at
least 38.6 ms ABOVE every refvals round-median. The phase-1 cold
first-read (443.9 ms) was the same verdict, cold-box inflated.

The qjs scoreboard (same session, 5 reps + 1 warmup, net medians; qjs
is stable day to day — the two rows' JS twins are the same program):

| row        | rut net   | qjs net  | node net | rut/qjs |
|------------|-----------|----------|----------|---------|
| refvals    | 366.8 ms  | 302.9 ms | 112.2 ms | 1.21×   |
| refcolumn  | 420.9 ms  | 303.8 ms | 118.8 ms | 1.39×   |

The experiment would have moved rut from 1.21× to 1.39× AGAINST qjs on
the suite's only ref-V shape. (Checksums `140052990000` equal on
rut/qjs/node for both rows throughout — the one-cell law carried
perfectly; the experiment failed on SPEED only.)

### The fuel ledger — exact (per-phase clone pairs, interleaved)

Ten single-phase clones of the churn (the phase-0 ledger's method, a
`RefMap` twin of each), 5 rounds × 3 fresh-VM iters per side. Fuel is
exact, and the per-op deltas reconcile the row's −1,508,981
BIT-EXACTLY (each clone's shared build setup cancels):

| phase (ops)                              | Δ fuel                  | per-op delta | what it is |
|------------------------------------------|-------------------------|--------------|------------|
| fresh put w/ growth (100k ×15g / 200k ×16g) | −3,202,984 / −6,405,997 | **−32.03/put** | the relocation drain deleted + the store re-spelled |
| overwrite put (flat, 300k)               | +300,000                | **+1.0/put** | box mint + `set_o` crossing replace `makeopt`+`arrset` |
| hit get, 2 fields (100k)                 | +1,600,000              | **+16.0/get** | `get_o` crossing + the downcast stream |
| hit get, 1 field (300k)                  | +4,800,000              | **+16.0/get** | same (field count is irrelevant) |
| miss get (20k)                           | 0                       | **0.0**      | the val lanes never touched |
| rmw through the alias (100k)             | +1,600,000              | **+16.0**    | the same downcast package |
| remove (50k) / has (100k) / re-add (50k) | −200,000                | **−5.0/remove**, 0/has, +1/re-add | the nil store gone; the owner drop is host-side |
| **row total**                            | **−1,508,981**          |              | **= measured exactly** |

The drain story, verified: the deleted growth-path fuel is
−3,302,984 (m) − 6,605,997 (g) = 9,908,981 ops — phase-0's DRAIN stub
measured 9,908,847; the column relocates vals with the keys host-side
and the wrapper's `map_take_reloc` stream is GONE (−33.03 ops/put on
every growth build vs +1.0 on flat puts — the difference IS the
drain). The one predicted win is real. It is just not big enough.

### Where the TIME went — the honest attribution

Fuel fell 2.7% while time ROSE 13.7% — the deleted ops were the
cheapest in the stream (the drain's sequential moves, ~9.9 M ×
~0.77 ns ≈ 7.6 ms) and the added work is the most expensive kind
(crossings + per-get cell traffic). The row-level composition that
closes, both halves measured same-session:

- **The build/put side WINS −40.4 ms**: a diagnostic clone running
  BOTH map builds in one VM (the row's actual structure) reads
  refcolumn 125.53 ms vs refvals 165.92 ms (ranges non-overlapping;
  fuel 10,800,453 vs 20,409,434). This is the drain deletion realized
  in time, plus the cheaper growth build.
- **The get side LOSES ≈ +87 ms**: the four value-reading phases'
  op-only deltas (clone delta minus that clone's build delta) read
  +172 / +182 / +187 / +166 ns per value-get — call it ~+175 ns over
  the row's 500 k value-reading gets. −40.4 + 87 ≈ +46.6 vs the
  measured +46.26. ✓
- The flat-table overwrite put reads +59…+100 ns/put in the phase
  clones (box mint + `set_o` frame + owner release vs the sidecar's
  `makeopt`+`arrset`) — inside the build/put aggregate above; not
  separately resolvable at row level.

The ~+175 ns/get is the downcast package, term by term: the second
crossing (`map_val_get_o` — a host frame the sidecar row never pays),
`TidOf` + const-compare + branch + `Unbox` (+16 fuel ops ≈ 12-16 ns at
the crossing-nop calibration), and the dominant unpriced term —
**`opaque.downcast<V>` yields a `(V, bool)` TUPLE, and that tuple is a
real record cell**: the lowering is `Op::MakeRecord` (RFC 0014 v1.1),
so every hit-get mints a tuple cell and releases it a few ops later
(RFC 0040 accounting both ways). Phase-0 priced the sidecar LOAD at
~65 ns/get; the column's answer costs ~240 ns/get. That is where the
experiment died: not the storage — the READ-BACK spelling.

Method honesty: the ten per-phase clones' raw time deltas do NOT sum
to the row (−74.6 vs +46.26) — each clone re-runs its build in a fresh
VM under different memory pressure than the row's phases ever see, so
clone-time is not additive across phases. The composition above uses
the two-builds diagnostic (the build side, measured in the row's own
shape) plus the clones' op-only get deltas (consistent across four
independent phases, +166-187 ns/get); miss-get read +4 ms on
IDENTICAL fuel and is recorded as clone noise.

### The heap terms — measured and reconciled

The phase-0 pre-registration expected heap PARITY ("the peak IS the
records; refcolumn holds the same records"). Both of its assumptions
were falsified, and the isolation probes (same binary, deterministic
RFC 0040 accounting) pin the true cell economics:

- record cell `Pt` = **40 B** (24 B `CELL_OVERHEAD` + 2 × 8 B fields —
  `mint` charges 24 + payload); a 4-field record = 56 B (probes:
  4.80 MB / 100 k bare `[Pt]`; 8.80 MB / 100 k through the column).
- the sidecar store's `MakeOpt` one-slot cell = **32 B** (24 + 8).
- the column's `Op::Box` = **32 B** (24 + 8 — `alloc_opaque` charges
  exactly this; boxed prims read exactly 32.0 B/value in isolation).
- **the `[?V]` sidecar arrays are CHARGED rut Array cells** — 8 B per
  table slot (`alloc_array`, `n × w`): m's 262,144-slot block = 2.1 MB,
  g's 524,288-slot block = 4.2 MB. The phase-0/1 note "plain host
  memory, invisible to cell accounting" was WRONG (the NOVALS control
  collapsed to 892 B because the stub deletes the array, not because
  nil blocks are free).

So: refvals = **96.0 B per live value** (record 40 + MakeOpt 32 +
sidecar block ~21 + grow-time transients ~3 → 28,801,340 B over
300 k live values, the peak catching g's last drain); refcolumn =
**72.0 B per live value** (record 40 + box 32; the column is a host
`Vec`, invisible → 21,600,999 = 300 k × 72 + base, EXACT). The
phase-1 "+32 B/box" term is measured exactly right — but it REPLACES
the sidecar's 32-B store cell one-for-one and deletes 6.3 MB of
charged blocks, so the column's heap came out 7.2 MB LOWER, not
higher. The experiment's heap outcome was a WIN (−25%), just not on
the axis the verdict needed.

### The decision, applied

§0.6's pre-registered rule reads: slower beyond noise → REVERT; the
round ranges here do not overlap at all. Applied in this commit:
`nmapset::RefMap` deleted, the two opaque-val crossings deleted (both
`nmap.d.rut` copies), the `refcolumn` row + twin + `expected.json`
line deleted, `nmap.rs` restored to its phase-0 state (the size-144
layout discipline reverts with it), the phase-1 driver suite deleted.
KEPT: the `refvals` row (permanent ref-V coverage), its checksum pin
`140052990000`, and the batch's analysis (these three sections).
Gates at commit: full suite 27 workloads × rut/qjs/node, exit 0,
every checksum equal `expected.json` (79 cross rows); the
exactly-committed fuel/heap pins hold BIT-IDENTICALLY post-revert —
refvals 56,899,541/28,801,340, nmapset-int 20,703,284/1,966,551,
nmapset-str 9,551,761/983,620, nmap-knucleotide 38,814,389/4,195,084,
nmap-hashset 13,267,176/551, nmap-primmap 17,950,301/324, kmer-view
37,614,177/4,194,916, strview 10,801,744/2,032,173, json-decode
111,322,915/34,377,027, crossing-nop 104,000,032/236, alloc
22,000,020/228, sieve 19,592,209/1,491,846, array 60,486,108/7,864,540,
fasta 200,029/16,806, binary-trees 1,048,552; workspace 525 tests,
0 failures, 82 suites (the pre-experiment counts). What the batch
keeps beyond the row: the measured knowledge that a val column for
ref-V must attack the READ-BACK (the tuple-minting downcast), not the
storage — a direct-ref lane is an engine repr change, out of the
experiment's scope by design.

## Performance log — fasthash phase 0: the fork decided NO — evidence + verdict (Sep 2026)

Phase 0 of the fasthash batch (word-bundled str/bytes hashing, 8 u8 ->
u64 per step): the profile, the path-A algebra, the order-sensitivity
census, and THE DECISION. Nothing was committed but this section — the
entire experimental surface was a ~20-line `hash_bytes` stub flipped in
`crates/rut-std/src/nmap.rs` (NEVER committed; restored +
md5-verified — `15b7a511b7a122a9564909ae67513997` — after every flip),
built into fixed scratch binaries, and interleaved. **The verdict, up
front: NEITHER fork path fires. The byte-serial FNV chain is
ILP-hidden on every row we have; the only word-bundled spelling that
keeps slots usable is 0 to +5.5% SLOWER; and path A is algebraically
closed anyway.** Phase 1's hash work is cancelled; this section is the
record (the refval-exp precedent: reversible no, analysis kept).

### Today's baselines (pristine v0, house probe, this session)

| row          | exec med (7–9 rounds) | fuel        | VM heap peak |
|--------------|-----------------------|-------------|--------------|
| nmapset-str  | 52.5–52.8 ms          | 9,551,761   | 983,620 B    |
| nmap-knuc    | 171.7–172.7 ms        | 38,814,389  | 4,195,084 B  |
| kmer-view    | 151.5–151.6 ms        | 37,614,177  | 4,194,916 B  |
| strview      | 39.0–39.8 ms          | 10,801,744  | 2,032,173 B  |
| nmap-hashset | 42.3 ms               | 13,267,176  | 551 B        |
| json-decode  | 670.9 ms              | 111,322,915 | 34,377,027 B |
| refvals      | 340.5 ms              | 56,899,541  | 28,801,340 B |
| nmapset-int  | 68.2 ms               | 20,703,284  | 1,966,551 B  |
| nmap-primmap | 60.1 ms               | 17,950,301  | 324 B        |

Fuel and heap are BIT-IDENTICAL to the committed pins on every row and
in every variant below — `hash_bytes` is host-side and off the fuel
meter, so fuel equality proves the op streams never moved and the
deltas are pure host-side time.

### The profile — the hash term is hidden, and the stubs prove it three ways

Three scratch hashes replaced the byte-serial FNV-1a body (same
function, same callers, same probing; only the value/cost changed):

- **v1 — the ceiling stub**: O(1) — first+last LE word ⊕ len through a
  splitmix finalizer. Not a real hash; the upper bound of "the hash
  term costs nothing", slots well-formed.
- **v2 — the viable path-B candidate**: word-FNV-1a over LE u64 words
  (same FNV constants), zero-extended LE tail word, PLUS a splitmix64
  finisher mixing len. Any word-bundled hash MUST look like this — v3
  shows why.
- **v3 — the naive plan-literal**: word-FNV-1a, LE words, NO finisher
  ("same structure, u64 chunks"). Measured because it had to be
  falsified.

Interleaved order-rotated rounds (run 1: 7 rounds × 9 rows, v0/v1/v2;
run 2: 9 rounds × 4 hash-active rows, v0/v2/v3; probe `--iters 3`,
fresh VMs):

| row (v0 med) | v1 delta      | v2 delta      | v3 delta        |
|--------------|---------------|---------------|-----------------|
| nmapset-str  | +1.9 (+3.7%)  | +1.8 / +1.7 (+3.4/3.3%) | +446.8 (+850%) |
| nmap-knuc    | +2.1 (+1.2%)  | −0.3 / −1.1 (−0.2/0.6%) | +104.3 (+60.8%) |
| kmer-view    | +4.2 (+2.8%)  | +2.6 / +1.7 (+1.7/1.1%) | +108.2 (+71.4%) |
| strview      | +2.2 (+5.4%)  | +1.3 / +2.1 (+3.3/5.5%) | +457.2 (+1171%) |
| nmap-hashset | +0.3 (+0.6%)  | −2.8 (−6.6%)  | not run (0 calls) |
| json-decode  | −10.0 (−1.5%) | +2.4 (+0.4%)  | not run (0 calls) |
| refvals      | +1.5 (+0.4%)  | −8.1 (−2.4%)  | not run (0 calls) |
| nmapset-int  | −1.7 (−2.5%)  | −3.8 (−5.6%)  | not run (0 calls) |
| nmap-primmap | −1.1 (−1.9%)  | −3.8 (−6.4%)  | not run (0 calls) |

Read it in three steps:

1. **The noise floor is measured, not assumed.** json-decode contains
   NO nmap table at all (its object keys live in `Vec<str>`, the fold
   is hand-written), and refvals/nmapset-int/nmap-primmap/nmap-hashset
   ride the `Bits` arm of `hash_payload` (mix64) — v1/v2 never execute
   `hash_bytes` on those rows, BY CONSTRUCTION. Their deltas: −8.1 to
   +2.4 ms (−6.6% to +2.4%). That is binary code-layout jitter and box
   drift, and it is the yardstick: every "win" below it is meaningless.
2. **The ceiling stub wins nowhere.** v1 deletes the hash term
   entirely and still cannot beat v0 beyond that floor on any row. The
   byte-multiply chain is real latency (~4 cy/byte serial) but it is
   NOT on any critical path: the out-of-order window around each op
   (string mints, crossings, probe memory traffic) covers it. The
   batch's thesis — a multiply-latency chain per byte worth ~8x —
   is falsified at the row level: the chain costs ~0 wall time here.
3. **The "viable" candidate is a net loss.** v2 recovers nothing
   (knuc −0.2/−0.6% is inside the floor) and is SLOWER on the
   short-key rows: +3.3–5.5%. The mechanism closes exactly: the
   finisher's 3 multiplies + shifts cost ~5–7 ns/op, and for ≤8-byte
   keys ("k{i}", 1-/2-mers) the byte chain it replaces is only 2–7 ×
   4 cy. nmapset-str runs 283,334 hash calls; +1.8 ms / 283k ≈ +6.4
   ns/op = the finalizer. On this suite's key lengths the fix costs
   more than the disease.

**The v3 catastrophe — the finding phase 1 needed.** v3 (word-FNV
without a finisher) is +60% to +1171% SLOWER. Slot demo on the actual
key set (`k0`..`k49999`, cap 2^17): v0 uses **41,356** slots (worst
bucket 6); v3 uses **19** (worst bucket 5,556); v2 uses 41,697 (worst
7). Why: multiplication never carries DOWNWARD — the low k bits of a
product depend only on the low k bits of the operands. Byte-FNV feeds
every byte through the low bits (each round XORs the byte in before
the multiply); word-FNV lets only each word's LOW BYTES reach the
probe-index bits. For keys ≤ 8 bytes with a constant first byte
(`'k'`) the low bits of the hash are CONSTANT across the whole key
set — 19 slots for 50k keys, linear probing into five ~5.5k-slot
runs. The RFC 0033 LE law makes this platform-independent fact, not
an accident: a finisher is not optional in any word-bundled design.

### Path A — same-result batching: closed, with the algebra pinned

The round is h' = (h ⊕ b)·P. Multiply-by-P is linear over
(Z/2^64, +), but the round ALTERNATES XOR with multiply, and XOR does
not distribute over ·P — pinned numerically: (2⊕1)·P = 3P =
3,298,534,884,633 while (2·P)⊕(1·P) = 2,199,023,256,423. So the
Horner-style parallel form that polynomial hashing admits
(Σ b_i·P^(n-i) with precomputed powers) does NOT exist here: each
byte's XOR must land BETWEEN two multiplies, and the multiply of
round i consumes all 64 bits of round i−1's state. Consequences,
each checked:

- **Table tricks cannot shorten the chain.** A T[b] entry would have
  to precompute (h ⊕ b)·P for all h — the table IS the computation.
  No FNV block-composition is published or possible under this round
  function (the state is the whole 64-bit accumulator, not a
  shift-then-OR like CRC).
- **Parallel lanes cannot recombine.** Even/odd-byte lanes each need
  as their seed the previous lane's full 64-bit output — the lanes are
  not independent; SIMD SWAR buys nothing for the same reason.
- **A same-result scheme must replay the chain** (or memoize it — see
  the menu note below), so the latency is the algorithm.

**Verdict: path A is infeasible at bit-identity — and moot**: the
unconstrained ceiling (v1, ANY values at O(1) cost) already showed
there is nothing to recover.

### The census — order-sensitivity: clean, and proven the hard way

Which checksums depend on table iteration order (path B's re-pin
risk)? **None.** Two independent proofs:

- **Structural**: every row's checksum is counters
  (added/replaced/hits/misses/removed/present) plus reads at FIXED
  keys in fixed order (the knuc readout walks ACGT nested loops; the
  str rows sum stored values by ascending i). No workload iterates a
  table into its checksum (grep-verified: no `keys()`/`entries()`/
  iteration over any nmap table in any workload).
- **Empirical**: all 9 rows ran under FOUR different hash designs —
  v0, v1, v2, v3 — i.e. every str/bytes hash value changed wholesale
  (and v3 catastrophically degraded slot assignment), and every
  checksum stayed BIT-IDENTICAL: 36/36 equal `expected.json` (kmer-view
  has no expected.json entry; it equals the knuc pin 2198604 by the
  parity law, under all four). The strongest single datapoint: the
  stale-binary accident (see Deviations) put the FULL 27-workload
  gate through v3 — a hash that collapses nmapset-str to 19 slots —
  and all 27 checksums still passed.

**The .js twins**: none replicate rut's hash or table order. Every
twin (nmapset-str/hashset/knucleotide/int/primmap, kmer-view, strview,
refvals, json-decode) computes on native JS `Map`/`Set` with JS's own
hashing, and none iterates a map into its checksum (grep: zero FNV
constants in `benches/workloads/*.js`; the lone "slot" hit is prose in
strview.js:46).

**Path B's re-pin cost list, HAD it won** (recorded because the fork
asked for it exactly): (1) `crates/rut-std/src/nmap.rs` — the
hash-pin test literals (~lines 1727–1743: `alpha`/`beta`/empty-basis +
the payload-parity asserts) and the two doc-comment sites naming FNV
as the law (`FNV_OFFSET`'s block ~485–494, `hash_bytes`'s ~496–501);
(2) prose comments naming FNV-1a in `nmapset-str/{main.rut,rut.toml}`,
`nmapset.rut`'s header, `kmer-view/main.rut`, `nmap-knucleotide`'s
docs; (3) `expected.json`: ZERO lines; (4) `.js` twins: ZERO files.
The one-time re-pin would have been six test literals + comments —
the cheapest re-pin imaginable. The census was path B's precondition,
and it PASSED; what fails path B is the benefit (the profile).

### THE DECISION — neither path; the hash does not change

The plan's own fork conditions: path A only if feasible at net win —
infeasible algebraically, moot empirically; path B only if the census
is clean AND the profile justifies a semantic-visible re-pin — the
census is clean, but the profile does not justify: the only viable
spelling is 0 to +5.5% slower, and the ceiling proves ~0 is
recoverable. **So: no hash change ships. Phase 1 is cancelled; the
constants stay the checksum law; every pin holds untouched.** The
honest no is the deliverable, per the refval-exp precedent.

What a future attempt must know (the menu, not this batch):

- Any word-bundled design MUST end in a strong avalanche finisher
  (v3's 19-slot demo), and the finisher costs ~5–7 ns/op — more than
  the byte chain on keys < ~16 bytes. The suite's longest str keys are
  12-byte k-mers; the thesis only pays on ≥16-byte keys that no
  current row has. A row with long-keys-per-op dominance would have
  to be BUILT first (e.g. a wide-record or long-URL-shaped churn).
- A same-result follow-up that does NOT touch semantics: the range
  lanes hash the same window twice on the get→put miss path (knuc/
  kmer: ~2 hashes/op, the second identical). A one-entry (ptr,len)-keyed
  memo in `typed_entry_sv`/`typed_find_sv` is bit-identical and saves
  ~1 hash of 12 bytes ≈ 1–2 ms on the k-mer rows — inside today's
  floor, recorded for honesty, not pursued.

### Method + gates

Four fixed release binaries (v0/v1/v2/v3) built from ONE checkout by
flipping only the `hash_bytes` body; binaries copied to
`/tmp/opencode/batch-fasthash/p0/bin-*/` before each rebuild so no
rebuild raced the interleave; source restored + md5 re-verified after
every flip; runs interleaved order-rotated (v0→v1→v2 rotations; run 2
v0/v2/v3) so drift lands in all columns equally. Gates at commit:
tree clean at base c72ddb9 + this section; `cargo test --workspace`
525 passed / 0 failed; full-suite rut gate exit 0, 0 checksum
mismatches, every fuel/heap pin BIT-IDENTICAL (nmapset-str
9,551,761/983,620; nmap-knuc 38,814,389/4,195,084; kmer-view
37,614,177/4,194,916; strview 10,801,744/2,032,173; nmap-hashset
13,267,176/551; nmap-primmap 17,950,301/324; nmapset-int
20,703,284/1,966,551; json-decode 111,322,915/34,377,027; refvals
56,899,541/28,801,340; crossing-nop 104,000,032/236; alloc
22,000,020/228; sieve 19,592,209/1,491,846; array 60,486,108/7,864,540;
fasta 200,029/16,806; binary-trees 1,048,552); staged by explicit
path; no parallel-session files touched (rut-lsp/vscode-extension/
rfc/wasm untouched).

**Deviations (both recorded, neither silent):** (1) the FIRST
full-suite gate ran against a stale scratch binary — v3 still sitting
in `target/release` after the last stub flip (source was restored but
not rebuilt). Caught by the exec columns (nmapset-str 502 ms vs the
52.5 ms v0 median), re-run clean on rebuilt v0 (md5-verified against
the v0 scratch copy). The accident was KEPT as evidence: it is the
full-gate form of the census proof (27/27 checksums survive even a
19-slot hash). (2) None. Scratch lives under
`/tmp/opencode/batch-fasthash/p0/` (binaries, harnesses, raw JSON);
this section is the only committed artifact.

## Known limitations / deliberate choices

- Workloads are still single files for node + qjs, but the rut side may
  be a module dir (`NAME/{rut.toml, main.rut}`): the runner spawns
  `rut run <dir>` and the dir's `[deps]` resolve through the module
   loader (RFC 0035) — currently the `nmapset`
  workloads (`nmapset-int`, `nmapset-str`, `nmap-hashset`,
  `nmap-knucleotide`, which pull the `nmap` host pkg through the
  pkg's own `[deps]`; also the dir twins `kmer-view`, `strview`, and
  the record-valued `refvals` row, the same way — the experimental
  `refcolumn` twin was reverted with its batch's verdict),
  `json-decode` (it mounts `pouch` for the
  row chunks the document generator joins), and `crossing-nop` (it
  mounts `bench-cross`, the phase-0 crossing-tax pkg). The bench
  runtimes install the host halves (math + the logger) as usual;
  `rut-bench-probe` additionally binds the nmap and bench-cross hosts
  only when the program's dep graph declares them (the
  host-surface check is exact in both directions, RFC 0025).
- The map rows' JS twins carry small adaptations: JS
  `Map`/`Set` have no insert-or-replace primitive, so the add-vs-
  replace split costs one extra `has` probe per put compared with
  rut's `put -> bool` (it biases JS against itself, which the framing
  below already expects). The adaptation predates the `mapset`
  removal — the same JS programs served the deleted `mapset` twins.
- Missing language/std features exclude `pidigits` (no bigint),
  `regex-redux` (no regex), and `reverse-complement` (stdin/bytes
  transform) from the benchmark-game set. `k-nucleotide` is no longer
  excluded — `nmapset` provides the hashmap.
- **`knucleotide`** is an *adaptation*: the sequence is generated with
  fasta's LCG (the harness takes no stdin) and built identically on
  both sides; 12-mer get-or-insert churn fills a `HashMap<str, i32>`,
  plus 1-/2-mer maps and 100 generated 12-mer fragment probes.
- `binary-trees` is an *adaptation*: it builds recursive structs
  directly (`left/right: ?Node`, `nil` when absent — RFC 0005 §8,
  RFC 0044) and counts them, rather than the benchmark-game's
  varying-depth trees. Each
  node is one RC cell, so the drop at scope exit is still the RC
  allocation/drop path this workload measures.
- **`fasta`** is likewise an adaptation: LCG-driven ACGT building into one
  accumulating `str` (`out = f"{out}{..}"`, appended in place) with a
  length/index checksum, not the benchmark-game's repeat-sequence generator.
- Scales are **reduced** from the official benchmark-game sizes: rut is
  an interpreter, so the full sizes would run for minutes to hours.
  They are still large enough that runtimes are measured, not just
  startup. Adjust the constant at the top of each `.rut`/`.js` pair
  (keep them equal) for larger runs.

## Adding a workload

1. Add `benches/workloads/NAME.rut` (`pub fn main`, log `CHECKSUM
   <value>` via the logger) and `NAME.js` (`console.log("CHECKSUM " + v)`),
   computing identical results with the same integer widths / float
   order. If the rut side needs a tree package beyond the CLI's
   single-file auto-mount (`ink`, `pouch`) — e.g. `nmapset` — ship it as
   a module dir `NAME/{rut.toml, main.rut}` with `[deps]` instead; the
   runner then spawns `rut run <dir>`.
2. Keep the scale as a named constant in both files.
3. Verify against an independent implementation and add the value to
   `workloads/expected.json`.
4. Run `node benches/run.mjs --workload NAME --repeats 1` and fix any
   compile errors or checksum mismatch.
5. Add a row to the table above.

## Sample output

```
=== cross-runtime (end-to-end process) ===

workload       runtime   checksum         wall median   startup  net median  wall min  peak RSS  ref   ok
-------------  -------  ---------------  -----------  --------  ----------  --------  --------  ---  ---
binary-trees   rut              32767      15.4 ms  3.327 ms     12.1 ms   14.0 ms    9.7 MB  yes  yes
binary-trees   node             32767      43.4 ms   18.2 ms     25.2 ms   23.2 ms   54.9 MB  yes  yes
binary-trees   qjs              32767      12.5 ms  2.974 ms     9.524 ms  12.3 ms    7.4 MB  yes  yes
fannkuch       rut             228016      15.5 ms  3.327 ms     12.2 ms   15.3 ms    5.1 MB  yes  yes
fannkuch       node            228016      22.8 ms   18.2 ms     4.592 ms  20.7 ms   52.9 MB  yes  yes
fannkuch       qjs             228016      11.3 ms  2.974 ms     8.316 ms  11.0 ms    3.5 MB  yes  yes
sieve          rut             41538      197.0 ms  3.327 ms    193.7 ms  190.8 ms   53.7 MB  yes  yes
sieve          node            41538       29.2 ms   18.2 ms    11.0 ms   25.7 ms   55.2 MB  yes  yes
sieve          qjs             41538       60.0 ms  2.974 ms    57.0 ms   59.8 ms    4.9 MB  yes  yes

=== startup floor (empty program) ===

runtime  startup wall  startup RSS
-------  ------------  -----------
rut          3.327 ms       4.3 MB
node          18.2 ms      43.6 MB
qjs          2.974 ms       3.5 MB

=== rut in-process (rut-bench-probe) ===

workload        compile    verify  exec median      fuel  VM heap peak  trap
-------------  --------  --------  -----------  --------  ------------  ----
binary-trees   1.335 ms  0.010 ms     10.2 ms    1048552       2.50 MB     —
fannkuch       2.567 ms  0.330 ms     10.6 ms    2365060        1.5 KB     —
sieve          2.483 ms  0.009 ms    181.1 ms   22201598      20.84 MB     —
```

## Performance log — refcolumn round 2: the mint deleted, the remainder measured — REVERTED again (Sep 2026)

The refval-round2 batch's verdict, and the second half of the two-round
story. **Round 1** (the refcolumn sections above): the val column lost
+13.7% beyond noise, and the autopsy pinned the loss on the READ-BACK —
`opaque.downcast<V>` minted a `(V, bool)` record cell per hit-get
(~+175 ns/get, ~+87 ms on the row). **Round 2** (this batch): phase 0
changed the surface IN PLACE — `downcast` yields `?T`, the alias
handoff, NO allocation on either branch (VERSION 6 -> 7) — then phase 1
revived the experiment re-spelled to the new recovery. The prediction
was that the mint term collapses to ~+5-10 ms and the column flips to a
net win on the strength of its build side. **The measurement: the mint
was only ~half of the get-side term. The remainder (+~87 ns/get of
crossing + box-chase + rc) still exceeds the build side's win, the row
read SLOWER in all three powered passes (inside this box's noise), and
both prongs of the pre-registered rule fire — the experiment is
REVERTED again, one commit, downcast ?T kept.**

### The matched pairs — three powered passes, one checkout, one build

Both rows of one checkout, one release build, interleaved fresh-VM
probe iters per side, order alternated per round, median of the round
medians; fuel and heap are pure counts and were single-valued in every
round of every pass (deterministic):

| pass | refvals med-of-med (round range) | refcolumn med-of-med (round range) | Δ | ranges |
|------|----------------------------------|------------------------------------|---|--------|
| 1 — 7 rounds × 7 iters | 357.88 ms (349.9 – 392.6) | 381.10 ms (361.4 – 388.0) | **+23.22 ms (+6.5%)** | overlap |
| 2 — 7 rounds × 7 iters | 354.50 ms (348.9 – 364.5) | 369.55 ms (360.8 – 397.1) | **+15.05 ms (+4.2%)** | overlap |
| 3 — 9 rounds × 9 iters | 356.70 ms (351.2 – 382.3) | 366.58 ms (358.8 – 383.2) | **+9.89 ms (+2.8%)** | overlap |

Box honesty, recorded because round 1's ranges need context: this VM is
both slower and much noisier than round 1's box (refvals median
354-358 ms here vs 337.68 there; round-median spread up to ±30 ms here
vs ±5 there — pass 1 round 1 read 392.6 while its neighbours read
~353). The direction is consistent — **refcolumn slower in all three
passes** — but no pass achieves round 1's non-overlap; the delta lives
INSIDE the noise every time. The pre-registered rule reads "SLOWER or
inside noise -> REVERT"; here both prongs fire at once, so the verdict
does not hinge on which one you weigh.

The counts: fuel 53,390,560 vs 56,899,541 (**−3,508,981, −6.2%**;
round 1: −1,508,981), VM heap peak 21,600,720 vs 28,801,340 B
(**−7,200,620 B, −25%**) — 72.0 vs 96.0 B per live value, round 1's
cell economics reproduced exactly. Per map op (pass 3): 277.7 vs
270.2 ns, **+7.5 ns/op**.

The qjs scoreboard CANNOT separate the rows on this box: two
order-bracketed runs (5 reps + 1 warmup each, net medians) read
refcolumn rut/qjs 1.16× then 1.25× and refvals 1.23× then 1.20× — the
qjs nets moved 310-328 ms between the rows' turns on the IDENTICAL
twin program (both rows' `.js` are the same computation), so the
inter-row drift exceeds the row gap and the sign flips with run order.
Round 1's scoreboard separated (1.21× vs 1.39×) because that box was
quiet. What survives: no scoreboard regression was hidden — the
powered probe pairs are the instrument, and they read "slower, inside
noise". Checksums 140052990000 equal on rut/qjs/node for both rows
throughout — the one-cell law carried perfectly, again.

### The fuel ledger — exact, both rounds reconciled to the op

Ten per-phase ladder clones (cumulative cut points of the row's churn,
one ladder per map family, this tree, this session; each family pair's
fold arithmetic is identical so it cancels in the delta; checksums
equal across the families at every step):

| phase (ops)                              | Δ fuel (rm − hm)         | per-op delta | round 1 | what changed |
|------------------------------------------|--------------------------|--------------|---------|--------------|
| fresh put w/ growth, m (100k, 15 grows)  | **−3,202,984**           | −32.03/put   | same    | nothing — round 1's term to the op |
| fresh put w/ growth, g (200k, 16 grows)  | **−6,405,997**           | −32.03/put   | same    | nothing |
| overwrite put, m (100k)                  | +100,000                 | +1.0/put     | +1.0    | nothing |
| overwrite put, g (200k)                  | +200,000                 | +1.0/put     | +1.0    | nothing |
| hit get, 2 fields (100k)                 | +1,200,000               | **+12.0/get**| +16.0   | **the mint: −4.0** |
| miss get (20k)                           | 0                        | 0.0          | 0.0     | nothing |
| rmw through the alias (100k)             | +1,200,000               | **+12.0/get**| +16.0   | **the mint: −4.0** |
| read-back gets, 1 field (100k)           | +1,200,000               | **+12.0/get**| +16.0   | **the mint: −4.0** |
| g read-back gets (200k)                  | +2,400,000               | **+12.0/get**| +16.0   | **the mint: −4.0** |
| remove (50k) / has (100k) / re-add (50k) | −200,000                 | −5.0/0/+1.0  | same    | nothing |
| **row total**                            | **−3,508,981**           |              |         | **= the measured row delta exactly** |

Both rounds reconcile against each other BIT-EXACTLY: round 1 read
−1,508,981; phase 0 deleted the `(V, bool)` mint at exactly
**4 ops × 500k value-reading gets = −2,000,000**; −1,508,981 − 2,000,000
= −3,508,981 = this round's measured delta. The get-side op stream is
now the downcast's `tidof + icmp + br + MovRef` (+12 ops over the
sidecar's arrget+deref spelling) — allocation-free exactly as phase 0
claimed; the put/build paths never moved (their deltas match round 1
to the op).

### Where the TIME went — the remainder measured

Fuel fell 6.2% while time rose 2.8-6.5%: the deleted ops were cheap,
and the remaining added work is memory, not ops. Two instruments, one
VM, interleaved:

- **The build/put side still WINS**: the one-VM diagnostic running the
  row's build structure (both map builds + both overwrite passes) reads
  refcolumn 222.7 ms (214.1-228.2) vs refvals 252.3 ms (246.5-259.0) —
  **−29.6 ms, non-overlapping** (fuel −9,308,981). Builds only, no
  overwrite passes: 131.9 (123.9-135.0) vs 180.3 (171.6-188.3) —
  **−48.5 ms, non-overlapping** (fuel −9,608,981). The drain deletion
  is only ~7.6 ms of this at the crossing-nop calibration; the rest is
  the leaner build shape itself — no `MakeOpt` cells, no charged sidecar
  arrays, 7.2 MB live instead of 9.75 MB on the m-build (clone heap),
  less memory traffic through every grow.
- **The get side still LOSES — half of round 1**: the ladder's get-side
  step deltas read +6.7 / +5.7 / +10.5 / +21.0 ms (100k 2-field / 100k
  rmw / 100k 1-field / 200k 1-field) = **+43.9 ms over 500k
  value-reading gets ≈ +88 ns/get**, vs round 1's +175. **The mint was
  ~88 ns/get (~44 ms) — phase 0 bought back exactly that half.** The
  residual ~+88 ns/get is the box re-hand package, term by term: the
  second crossing frame (~a few ns), `TidOf` + cmp + br + `MovRef`
  (+12 fuel ops ≈ 9-12 ns at the 0.77 ns/op calibration), and the
  DOMINANT unpriced memory terms — the box->record double indirection
  (every field read chases the box, then the payload slot: two cells
  that landed far apart in the heap, where the sidecar row's slot and
  cell are one cache-local load), and the re-handed handle's
  retain/release rc pair per get. None of that is on the fuel meter,
  and no SPELLING change removes it — it is the shape of storing a box
  where the baseline stores a cell reference in a rut-owned array.
- **The composition closes**: −29.6 (build side, one-VM) + 43.9
  (get-side clone deltas) + 1.0 (miss gets, on a ZERO fuel delta —
  clone noise, round 1's honesty note again) = **+15.3 ms**, inside the
  measured row band (+9.9 to +23.2 across the three passes). The same
  clone-time caveat as round 1 applies — the per-phase time sums are
  not linearly additive; this composition is the one that closes, and
  the row band is the claim.

The heap story is unchanged from round 1's accounting (both sections
above stand): refcolumn = 72.0 B/live value EXACT (record 40 + box 32;
21,600,720 = 300k × 72 + base), refvals = 96.0 (record 40 + MakeOpt 32
+ charged sidecar blocks ~21 + transients ~3). The column wins heap by
25% — the one axis it wins — and the get-side rc stayed neutral
(heap single-valued across every pass round).

### The decision, applied

§0.6's pre-registered rule: SLOWER or inside noise -> REVERT the
EXPERIMENT ONLY. Both prongs fire (slower in all three passes, inside
noise in all three). Applied in this commit: `nmapset::RefMap` deleted,
the `map_val_set_o`/`map_val_get_o` crossings deleted (both
`nmap.d.rut` copies), the `refcolumn` row + `.js` twin + its
`expected.json` line deleted, `nmap.rs` restored to its pre-revival
state (the owner column, the packed reloc queue, and the size-144
discipline revert together — they existed to make room for each
other), the driver suite deleted. Every restored file is bit-identical
to its phase-0-of-round-2 state. KEPT: the **downcast -> ?T change**
(stands on its own — strictly less allocation for every opaque user,
VERSION 7; json-decode's checksum unmoved, its fuel/heap re-pin
111,322,915 / 34,377,027 now carried by the prose pins throughout this
file, old values verbatim in this commit's body), the `refvals` row
(pin 140052990000, fuel/heap pins reproduced bit-identically on this
box), and this two-round record.

What the two rounds bought, stated once so nobody re-runs the
experiment a third time on a spelling: a ref-V val column's storage
side is a real win (drain-free growth, −25% heap, −30..−48 ms on the
build side) — and its read-back has a floor no crossing spelling
reaches, because the stored unit is a BOX: ~+88 ns/get of crossing +
double indirection + rc that survived the deletion of the mint that
round 1 blamed. Round 1 removed the tuple; round 2 measured what was
left; what is left is the box itself. A direct-ref lane (storing the
record cell behind a rut-owned handle with no wrapper) is an engine
representation change, out of the experiment's scope by design, and is
where any third attempt would have to start. Phase 3 (close-out) does
not run — the rule's fork sends the batch to close-out only on KEEP.
