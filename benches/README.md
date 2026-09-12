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
│   ├── NAME.rut            # the rut program (no imports; builtin print)
│   └── NAME.js             # the identical program for node + qjs
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
`--timeout SEC`, `--no-probe`, `--no-build`. A full run of the suite
takes a few minutes because rut executes the workloads (`--workload`
and `--repeats 1` are useful while iterating).

## Workloads

Every pair computes the same thing and prints one `CHECKSUM …` line; the
runner fails the run if the runtimes disagree.

| Workload | Stresses | Scale | Checksum |
|---|---|---|---|
| `empty` | process/compile startup floor | — | none |
| `sieve` | `Vec<i32>` + tight integer loops | limit 500 000 | exact `41538` |
| `quicksort` | recursion, in-place vec mutation | n = 10 000 | exact |
| `matrix-mul` | flat `Vec<f64>` multiply-add | n = 64 | exact (see below) |
| `mandelbrot` | `f64` control flow | 100 × 75, 200 iters | exact (integer) |
| `fannkuch` | permutations, array churn | n = 7 | exact |
| `nbody` | `f64` integration + `sqrt` | 1000 steps | exact |
| `spectral-norm` | `f64` power iteration + `sqrt` | n = 150 | exact |
| `binary-trees` | RC allocation/drop churn | depth 14 | exact `32767` |
| `fasta` | immutable-string building | n = 10 000 | exact |

## Method (and what "fair" means here)

- Each runtime is invoked the way it is normally used: `rut run
  file.rut`, `node file.js`, `qjs file.js`. Wall time therefore
  **includes parse/compile/verification** for all three.
- Wall time is the median over `--repeats` timed runs after `--warmup`
  discarded runs; `wall min` is also reported.
- **Peak RSS** is the whole-process high-water from GNU `time -f %M`.
  It includes each runtime's baseline, so compare against the `empty`
  row for the same runtime.
- **rut's VM heap peak** is its own byte accounting (RFC 0039), not
  process memory: the high-water of live cells. It is reported
  separately by the probe.
- The **rut probe** (`benches/probe`) compiles once, then runs `main`
  on a fresh `Vm` per iteration, splitting `compile` / `decode+verify` /
  `exec` and reading `fuel_used` (RFC 0040) and `heap_peak_bytes`.
- Checksums must agree across runtimes. `f64` arithmetic is IEEE and
  evaluated in the same order in rut and JS, and where the language has
  no builtin `sqrt` both sides carry the **same fixed-iteration Newton
  routine**, so even the float checksums match bit-for-bit. The runner
  still tolerates a 1e-9 relative difference when comparing numeric
  checksums.

## Known limitations / deliberate choices

- **This build has no module loading** (RFC 0035, M2), so workloads are
  single files and use the builtin `print`; the `std:log`-style imports
  in `examples/` would not run.
- Missing language/std features exclude `pidigits` (no bigint),
  `regex-redux` (no regex), and `k-nucleotide` / `reverse-complement`
  (bytes/hashmap/stdin) from the benchmark-game set.
- **Recursive dataclasses are not in the type system** (RFC 0009), so
  `binary-trees` builds its nodes as `Opaque`-boxed dataclasses — which
  is exactly the RC allocation path the benchmark is about.
- Scales are **reduced** from the official benchmark-game sizes: rut is
  an interpreter, so the full sizes would run for minutes to hours.
  They are still large enough that runtimes are measured, not just
  startup. Adjust the constant at the top of each `.rut`/`.js` pair
  (keep them equal) for larger runs.

## Adding a workload

1. Add `benches/workloads/NAME.rut` (no imports, `pub fn main`, print
   `CHECKSUM <value>`) and `NAME.js` (`console.log("CHECKSUM " + v)`),
   computing identical results with the same integer widths / float
   order.
2. Keep the scale as a named constant in both files.
3. Run `node benches/run.mjs --workload NAME --repeats 1` and fix any
   compile errors or checksum mismatch.
4. Add a row to the table above.

## Sample output

```
=== cross-runtime (end-to-end process) ===

workload      runtime     checksum  wall median  wall min  peak RSS   ok
------------  -------  -----------  -----------  --------  --------  ---
binary-trees  rut            32767     445.5 ms  445.5 ms   21.0 MB  yes
binary-trees  node           32767      44.4 ms   44.4 ms   54.5 MB  yes
binary-trees  qjs            32767      12.8 ms   12.8 ms    7.3 MB  yes
fasta         rut      15246:10000     585.1 ms  585.1 ms    4.9 MB  yes
fasta         node     15246:10000      20.7 ms   20.7 ms   46.9 MB  yes
fasta         qjs      15246:10000     5.298 ms  5.298 ms    4.2 MB  yes

=== rut in-process (rut-bench-probe) ===

workload      compile    verify  exec median     fuel  VM heap peak  trap
------------  --------  --------  -----------  -------  ------------  ----
binary-trees  0.356 ms  0.010 ms     422.5 ms  1605577       4.50 MB     —
fasta         0.264 ms  0.009 ms     582.3 ms  1220033      351.9 KB     —
```
