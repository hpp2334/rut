# rut-vm-threaded

Tail-call threaded dispatch for the rut VM — the **only** crate that enables
the nightly `become` / `rust-preserve-none` features, so nothing else in the
workspace has to know about them.

It is wired into `rut-vm` and is the production dispatch path on native
x86_64/aarch64. wasm (and any target without verified `become` lowering) runs
the same op methods through a portable loop instead.

## Layout

```
build.rs          emits cfg(rut_threaded) = not wasm && (x86_64 || aarch64)
src/api.rs        Machine trait, Flow/ThreadOut/ThreadState, Table, tag_of/NTAGS
src/native/mod.rs become-threaded handlers (cfg rut_threaded)
src/wasm/mod.rs   portable loop over the same Machine methods (otherwise)
src/dispatch_bench.rs  isolated match-vs-threaded microbenchmark
```

## How it works

Handlers carry the hot frame state in **arguments** — code base, tag stream,
register base, pc, and fuel — instead of reloading it from `self`. Each
handler executes one op through a shared `Machine::op_*` method, then
`become`s the next handler (an indirect tail jump). This is the reference
tail-call-interpreter shape; `dispatch_bench` measures it ~25% faster than an
equivalent `match` loop.

Every `Op` variant has a tag and a handler, so hot code never bails. Calls and
`ret` change frames and return `Flow::Redispatch`; the engine re-reads the new
frame's code/tags/regs/pc through `Machine::frame_ptrs` (fuel stays in the
threaded arguments, so redispatch is a handful of loads). `T_SLOW` remains as
a safety net: a future op with no handler still maps to it and runs through
`rut-vm`'s match interpreter (`step_one`), with the machine state synced.

`Machine::op_*` bodies destructure the current op with `unreachable_op!`,
which is a **runtime-checked** `#[cold] #[inline(never)]` panic. The tag
derivation makes a mismatch impossible, but a bug is a clean panic rather
than UB, and the cold marker keeps the check off the hot path.

## Status & results

Wired and default on native. Benchmarks (in-process `exec_min_ms`, A/B on the
same machine, lower is better):

| workload | match | threaded | speedup |
|---|--:|--:|--:|
| floatloop | 31.2 | 15.4 | **2.0×** |
| intloop | 88.2 | 44.2 | **2.0×** |
| alloc | 32.3 | 17.3 | **1.9×** |
| nbody | 9.2 | 5.5 | **1.7×** |
| matrix-mul | 10.1 | 6.5 | **1.6×** |
| spectral-norm | 54.7 | 36.7 | **1.5×** |
| fasta | 4.02 | 2.71 | **1.5×** |
| mandelbrot | 11.3 | 7.8 | **1.4×** |
| sieve | 30.3 | 21.2 | **1.4×** |
| fannkuch | 4.57 | 3.28 | **1.4×** |
| call | 35.2 | 25.4 | **1.4×** |
| quicksort | 8.66 | 6.43 | **1.4×** |
| binary-trees | 6.75 | 5.53 | **1.2×** |

All bench checksums match; the full test suite passes; no workload regresses.

## Running

```
cargo test -p rut-vm-threaded            # mechanism: flat stack + codegen
cargo test -p rut-vm-threaded --release --test dispatch -- --nocapture
```
