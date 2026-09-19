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

`sieve`, `quicksort`, `matrix-mul`, `mandelbrot`, `fannkuch`, `nbody` and
`spectral-norm` follow the standard algorithms (fannkuch and nbody to the
benchmarks-game definitions); `binary-trees` and `fasta` are **adaptations**
— see the limitations below. The four `mapset` workloads
(`hashmap-int`, `hashmap-str`, `hashset`, `knucleotide`) stress the
`rut/mapset` package: the rut sides ship as module dirs
(`NAME/{rut.toml, main.rut}` with `[deps] mapset = …`) and run as
`rut run <dir>` — the CLI's single-file auto-mount list stays untouched.
Honest framing: V8's `Map`/`Set` are inline-cache-optimized and QuickJS
has its own fast paths — these rows are not expected to be a win. The
point is rut's number for the RC-heap slots its keyed collections pay.
Allocation profile (mapset v1.1): one `*K` cell per key — shared, not
copied, for ref keys — plus one `*V` cell per value and the
`hashes`/`states` scalar arrays; there are **no** `Hashable` boxes and
no vtable dispatch on the map path (`RawHashTable<K>` monomorphizes per
key type, so `hash`/`hash_eq` are static, origin-pinned calls — inlined
for tiny bodies). The copy law still applies: binding an array value to
a local deep-copies the buffer, unless the source is provably dead, in
which case the engine move-elides the copy — mapset's hot loops index
fields directly (`self.states[at]`) and only `rehash` holds old-array
bindings, the blessed O(1) case. See the performance log below for the
before/after these profiles bought.

The four `nmapset` workloads (`nmapset-int`, `nmapset-str`,
`nmap-hashset`, `nmap-knucleotide`) are line-for-line clones of the
mapset workloads over `rut/nmapset` — the **host-implemented** key
table experiment (the "C builtin" architecture qjs itself uses): keys
live as owned Rust data behind one `Opaque` box per table, values stay
rut-side in a parallel `[*V]` array, and every map op crosses the host
boundary (one key box mint + one or two host calls). The nmapset pkg
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
the same values as mapset), mints one `Opaque` key box, and crosses the
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
one `Opaque.new` mint and one or two host calls (~17 ns/call, the
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

## Method (and what "fair" means here)

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

## Known limitations / deliberate choices

- Workloads are still single files for node + qjs, but the rut side may
  be a module dir (`NAME/{rut.toml, main.rut}`): the runner spawns
  `rut run <dir>` and the dir's `[deps]` resolve through the module
  loader (RFC 0035) — currently the `mapset` workloads
  (`hashmap-int`, `hashmap-str`, `hashset`, `knucleotide`) and their
  `nmapset` twins (`nmapset-int`, `nmapset-str`, `nmap-hashset`,
  `nmap-knucleotide`, which pull the `nmap` host pkg through the
  pkg's own `[deps]`). The bench runtimes install the host halves
  (math + the logger) as usual; `rut-bench-probe` additionally binds
  the nmap hosts only when the program's dep graph declares them (the
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
- `binary-trees` is an *adaptation*: it builds recursive dataclasses
  directly (`left/right: Option<Node>`, RFC 0009 recursive shapes) and
  counts them, rather than the benchmark-game's varying-depth trees. Each
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
