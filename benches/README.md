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
│   │                       # (the `mapset` workloads); runs as `rut run <dir>`
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
| `hashmap-int` | `HashMap<i32, i32>` churn (mapset): put/replace/hit-and-miss get/remove/re-scan | n = 100 000 | checksum `734932704` |
| `hashmap-str` | str-keyed `HashMap<str, i32>` (mapset): generated keys, removals, re-adds | n = 50 000 | checksum `1264308351` |
| `hashset` | `HashSet<i32>` (mapset): adds, dup adds, probes, removals, intersection count | n = 100 000 | checksum `21500055` |
| `knucleotide` | k-mer counting over `HashMap<str, i32>` (mapset): 12-mer fill + fragment probes | seq = 200 000 | checksum `2198604` |

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
point is rut's number for the RC-heap fat-ref slots and the vtable
dispatch its keyed collections actually pay, as a before/after anchor
for future work.

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
  (`hashmap-int`, `hashmap-str`, `hashset`, `knucleotide`). The
  bench runtimes install the host halves (math + the logger) as usual.
- The `mapset` workloads' JS twins carry small adaptations: JS
  `Map`/`Set` have no insert-or-replace primitive, so the add-vs-
  replace split costs one extra `has` probe per put compared with
  rut's `put -> bool` (it biases JS against itself, which the framing
  below already expects).
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
binary-trees   rut              32767      31.9 ms  3.048 ms     28.9 ms   29.7 ms   20.9 MB  yes  yes
binary-trees   node             32767      44.5 ms   19.5 ms     25.0 ms   43.8 ms   54.8 MB  yes  yes
binary-trees   qjs              32767      13.0 ms  3.637 ms    9.357 ms   12.3 ms    7.3 MB  yes  yes
fannkuch       rut             228016      20.5 ms  3.048 ms     17.5 ms   20.0 ms    3.1 MB  yes  yes
fannkuch       node            228016      21.1 ms   19.5 ms    1.563 ms   20.8 ms   52.8 MB  yes  yes
fannkuch       qjs             228016      11.8 ms  3.637 ms    8.150 ms   11.4 ms    3.7 MB  yes  yes
sieve          rut              41538     135.3 ms  3.048 ms    132.3 ms  134.2 ms   11.4 MB  yes  yes
sieve          node             41538      25.9 ms   19.5 ms    6.386 ms   24.6 ms   55.0 MB  yes  yes
sieve          qjs              41538      57.8 ms  3.637 ms     54.2 ms   56.5 ms    4.9 MB  yes  yes

=== startup floor (empty program) ===

runtime  startup wall  startup RSS
-------  ------------  -----------
rut          3.048 ms       3.0 MB
node          19.5 ms      43.6 MB
qjs          3.637 ms       3.6 MB

=== rut in-process (rut-bench-probe) ===

workload        compile    verify  exec median      fuel  VM heap peak  trap
-------------  --------  --------  -----------  --------  ------------  ----
binary-trees   0.300 ms  0.010 ms      28.4 ms   1605577       4.50 MB     —
fannkuch       0.430 ms  0.013 ms      17.0 ms   2828979         273 B     —
sieve          0.248 ms  0.009 ms     130.4 ms  22891359       4.13 MB     —
```
