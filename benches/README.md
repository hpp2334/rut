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
│   │                       # (the `mapset`/`nmapset` workloads); runs as
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
| `hashmap-int` | `HashMap<i32, i32>` churn (mapset): put/replace/hit-and-miss get/remove/re-scan | n = 100 000 | checksum `734932704` |
| `hashmap-str` | str-keyed `HashMap<str, i32>` (mapset): generated keys, removals, re-adds | n = 50 000 | checksum `1264308351` |
| `hashset` | `HashSet<i32>` (mapset): adds, dup adds, probes, removals, intersection count | n = 100 000 | checksum `21500055` |
| `knucleotide` | k-mer counting over `HashMap<str, i32>` (mapset): 12-mer fill + fragment probes | seq = 200 000 | checksum `2198604` |
| `nmapset-int` | `HashMap<i32, i32>` churn (**nmapset**, the host-implemented map experiment): line-for-line clone of `hashmap-int` | n = 100 000 | checksum `734932704` (=`hashmap-int`) |
| `nmapset-str` | str-keyed `HashMap<str, i32>` (**nmapset**): line-for-line clone of `hashmap-str` | n = 50 000 | checksum `1264308351` (=`hashmap-str`) |
| `nmap-hashset` | `HashSet<i32>` (**nmapset**): line-for-line clone of `hashset` | n = 100 000 | checksum `21500055` (=`hashset`) |
| `nmap-knucleotide` | k-mer counting over `HashMap<str, i32>` (**nmapset**): line-for-line clone of `knucleotide` | seq = 200 000 | checksum `2198604` (=`knucleotide`) |
| `json-decode` | the digest JSON decode: char-split + parser minting one `opaque` box per JSON value (and per object key), plus a downcast fold over the tree — REPS reps of a large generated document (1 200 rows; ~34k boxes minted per rep, ~100k total) | doc ~204 KB, reps 3 | checksum `4502015958359127277` |
| `crossing-nop` | the rut→host **crossing tax**, isolated: loop A calls the host `nop` (identity), loop B an inline rut fn with the same body; the `4` pair repeats both over a 4-arg sum — every body is deliberately empty, so (A−B) is the crossing and (nop4−nop) the per-param slope | 2M iterations × 4 loops | checksum `20000014000000` |

`sieve`, `quicksort`, `matrix-mul`, `mandelbrot`, `fannkuch`, `nbody` and
`spectral-norm` follow the standard algorithms (fannkuch and nbody to the
benchmarks-game definitions); `binary-trees` and `fasta` are
**adaptations**. `json-decode` is too — see below. The four `mapset` workloads (`hashmap-int`, `hashmap-str`, `hashset`, `knucleotide`) stress the
`rut/mapset` package: the rut sides ship as module dirs
(`NAME/{rut.toml, main.rut}` with `[deps] mapset = …`) and run as
`rut run <dir>` — the CLI's single-file auto-mount list stays untouched.
Honest framing: V8's `Map`/`Set` are inline-cache-optimized and QuickJS
has its own fast paths — these rows are not expected to be a win. The
point is rut's number for the RC-heap slots its keyed collections pay.
Allocation profile (mapset v1.1, byref spelling — RFC 0044): one `?K`
handle per key — a share of the key's cell, never a copy — plus one
`?V` handle per value and the `hashes`/`states` scalar arrays; there
are **no** `Hashable` boxes and no vtable dispatch on the map path
(`RawHashTable<K>` monomorphizes per key type, so `hash`/`hash_eq` are
static, origin-pinned calls — inlined for tiny bodies). Bindings never
copy (RFC 0044): `let b = a` is an O(1) share of the array cell, so
`rehash`'s old-array bindings make the swap O(1) by construction — the
old move-elision pass and its copy law are gone. mapset's hot loops
still index fields directly (`self.states[at]`): it reads clearly and
skips the handle traffic. See the performance log below.

The four `nmapset` workloads (`nmapset-int`, `nmapset-str`,
`nmap-hashset`, `nmap-knucleotide`) are line-for-line clones of the
mapset workloads over `rut/nmapset` — the **host-implemented** key
table experiment (the "C builtin" architecture qjs itself uses): keys
live as owned Rust data behind one `opaque` box per table, values stay
rut-side in a parallel `[?V]` array, and every map op crosses the host
boundary once through a typed crossing (`map_{entry,find,remove}_{i,u,b,s,y}`
— the key crosses directly as its own type, hashed host-side by
`hash_payload`, which ports the same mix64/FNV-1a constants; `map_entry_*`
answers `i32::MIN` grow-first, so put is one crossing even on growth). The nmapset pkg
ships mapset's exact mix64/FNV-1a constants, so every key hashes to the
same bits and the checksums MUST equal the mapset rows' — a mismatch is
a bug, and `expected.json` pins it. See the performance log below.

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
| json-decode | 675.2 ms| 62.3 ms | 24.3 ms  | 662.4 ms | 111.33 M  | 32.78 MB     |

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
the 3 reps; ~37 M fuel per rep (`111.33 M` per main incl. gen+split).
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
| json-decode      | 675.2 → 676.2 ms (~0)   | 662.4 → 670.0 ms (+1.1%)| 111.33 M (=) | 32.78 MB (=) |
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
| json-decode      | 675.2 → 676.2 ms        | ~0 (parity)| 62.3 ms            | 111.33 M (=)        |

(`=` bit-identical. Exec medians for the typed lanes: nmapset-str
54.9 ms −74%, nmap-knucleotide 216.1 ms −71%, nmap-hashset 40.4 ms
−31%, nmapset-int 81.7 ms −17% — the phase-4 log has the full
decomposition. Close-out verification re-run: full suite, all three
runtimes, exit 0 — every row equals `expected.json`; the probe's fuel
and VM-heap peaks are bit-identical on every nmap/json row (json
665.0 ms / 111.33 M / 32.78 MB; nmapset-str 57.1 ms; nmap-knucleotide
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
676.3-688.2, overlapping; fuel 111,330,118 and heap bit-identical both
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
(13,267,176 / 21,103,284 / 9,735,095 / 39,612,955 / 111,330,118 ops;
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
| json-decode      | 727.9 ms         | 111,330,118   | 34,377,147 B (32.78 MiB) |
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
| json-decode      | 671.5 ms    | 677.9 ms   | +0.9%      | 111,330,118 (identical) | 34,377,147 B (identical)  |

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

## Known limitations / deliberate choices

- Workloads are still single files for node + qjs, but the rut side may
  be a module dir (`NAME/{rut.toml, main.rut}`): the runner spawns
  `rut run <dir>` and the dir's `[deps]` resolve through the module
  loader (RFC 0035) — currently the `mapset` workloads
  (`hashmap-int`, `hashmap-str`, `hashset`, `knucleotide`) and their
  `nmapset` twins (`nmapset-int`, `nmapset-str`, `nmap-hashset`,
  `nmap-knucleotide`, which pull the `nmap` host pkg through the
  pkg's own `[deps]`), `json-decode` (it mounts `pouch` for the
  row chunks the document generator joins), and `crossing-nop` (it
  mounts `bench-cross`, the phase-0 crossing-tax pkg). The bench
  runtimes install the host halves (math + the logger) as usual;
  `rut-bench-probe` additionally binds the nmap and bench-cross hosts
  only when the program's dep graph declares them (the
  host-surface check is exact in both directions, RFC 0025).
- The `mapset` workloads' JS twins carry small adaptations: JS
  `Map`/`Set` have no insert-or-replace primitive, so the add-vs-
  replace split costs one extra `has` probe per put compared with
  rut's `put -> bool` (it biases JS against itself, which the framing
  below already expects). The `nmapset` workloads' JS twins are the
  same programs (the JS side has no notion of which rut pkg backs the
  map — the comparison rows measure the identical JS work).
- Missing language/std features exclude `pidigits` (no bigint),
  `regex-redux` (no regex), and `reverse-complement` (stdin/bytes
  transform) from the benchmark-game set. `k-nucleotide` is no longer
  excluded — the `mapset` package provides the hashmap.
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
   single-file auto-mount (`ink`, `pouch`) — e.g. `mapset` — ship it as
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
