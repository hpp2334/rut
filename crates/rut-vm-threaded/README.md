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

`rut-vm` drives it from `run_loop`: a threaded stretch runs until it hits an
op the crate does not thread, which returns `ThreadOut::Bail` with the
machine state synced; `step_one` then runs that one op with the match
interpreter and re-enters threading. `Flow::Redispatch` handles calls/ret
(frame changes) by re-reading `thread_state`.

`Machine::op_*` is the single source of truth for op behavior — both the
native handlers and the wasm loop call it.

## Status & results

Wired and default on native. Benchmarks (in-process `exec_min_ms`, A/B on the
same machine, lower is better):

| workload | match | threaded | |
|---|--:|--:|--:|
| intloop | 88.2 | 45.0 | **2.0×** |
| floatloop | 30.6 | 15.3 | **2.0×** |
| alloc | 31.9 | 17.8 | **1.8×** |
| mandelbrot | 11.3 | 7.9 | **1.4×** |
| nbody | 9.6 | 7.1 | **1.4×** |
| call | 41.5 | 29.6 | **1.4×** |
| sieve | 29.8 | 28.5 | 1.05× |
| fasta | 3.84 | 3.63 | 1.06× |
| matrix-mul | 9.48 | 9.31 | 1.02× |
| spectral-norm | 57.2 | 52.1 | 1.10× |
| binary-trees | 6.61 | 7.09 | 0.93× |
| quicksort | 8.53 | 9.71 | 0.88× |
| fannkuch | 4.59 | 5.35 | 0.86× |

All bench checksums match; the full test suite passes.

Remaining work to make the last four net-positive: thread the record/sum
allocators (`NewCell`, `MakeRecord`, `OptSome/OptNone`, `ResOk/ResErr`,
`SumIs`, `Unwrap*`, `EnumNew`, `ArrNew`, `ArrLit`, `Own`, `Conv`) so they stop
bailing, and trim the `Redispatch` overhead on deep recursion.

## Running

```
cargo test -p rut-vm-threaded            # mechanism: flat stack + codegen
cargo test -p rut-vm-threaded --release --test dispatch -- --nocapture
```
