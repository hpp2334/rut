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
