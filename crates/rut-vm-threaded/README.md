# rut-vm-threaded

Tail-call threaded dispatch for the rut VM — the **only** crate that enables
the nightly `become` / `rust-preserve-none` features, so nothing else in the
workspace has to know about them.

**Status: isolated experiment, not wired into `rut-vm`.** A full Phase 1
wiring was implemented and measured; it regressed every workload, so it was
reverted (see "Results"). The crate is kept because the mechanism is proven
and the trait/backends are reusable once the design changes described below
are made.

## Layout

```
build.rs          emits cfg(rut_threaded) = not wasm && (x86_64 || aarch64)
src/api.rs        Machine trait, Flow/ThreadOut, Table, tag_of/NTAGS
src/native/mod.rs become-threaded handlers (cfg rut_threaded)
src/wasm/mod.rs   portable loop over the same Machine methods (otherwise)
```

wasm (and any target without verified `become` lowering) uses the portable
loop; only `native/` contains `become`. Both backends call the same
`Machine::op_*` methods, so op behavior is shared.

## Phase 0 — mechanism validated

`tests/phase0.rs` runs a synthetic `Machine` for 5M steps on a **256 KB**
stack: it survives, so the calls really are tail calls. Release codegen for a
handler is a clean indirect jump with no prologue/spills:

```
h_inc:
    incq   0x18(%r12)            ; op body
    inc    %r14d                 ; pc += 1
    movzbl (%rax,%r14,1),%eax    ; next tag
    mov    0x0(%r13,%rax,8),%rax ; table[tag]
    jmp    *%rax                 ; indirect tail jump
```

## Phase 1 — measured, then reverted

Phase 1 wired the engine into `Vm::run_loop`: native threads a scalar stretch
and returns `Bail` on any unthreaded op, at which point `step_one` runs that
one op through the match interpreter and re-enters. Precomputed per-op tags
and a cached handler table were added.

In-process `exec_median_ms` (release, 5 iters):

| workload | match (baseline) | threaded | Δ |
|---|--:|--:|--:|
| intloop | 88.3 | 110 | −25% |
| floatloop | 31.3 | 39 | −25% |
| call | 40.6 | 46 | −13% |
| sieve | 30.7 | 61 | −99% |
| quicksort | 8.9 | 16 | −80% |

All checksums were correct; it was purely a performance loss, so the
`rut-vm` wiring was reverted and the match loop restored.

## Why it lost, and what a win needs

The handler dispatch is fast (`jmp *`), but the **op bodies were not**: every
handler reloads `Vm` state from memory (`m`-relative loads/stores) and calls
back into `tick`/`op_*`, whereas the match fast path keeps that state live in
registers across the loop. Cold-heavy code (sieve/quicksort) additionally
paid the `Bail` → `step_one` round-trip per array/object/call op.

To beat the match loop this crate needs the reference-interpreter shape:

1. **Thread the hot state as arguments** — `pc`, the register slice, the
   current function/tag stream, and fuel — so the handlers keep them in
   registers instead of `m`-relative memory. (`extern "rust-preserve-none"`
   exists precisely for this.)
2. **Full op coverage** so the hot paths never `Bail`; the `Bail` loop is a
   large per-op tax on array/object-heavy code.
3. **No per-op `tick` call**: fold fuel into the threaded argument set.

Until then the match interpreter is faster, and `rut-vm` uses it.

## Running the test

```
cargo test -p rut-vm-threaded            # flat-stack + codegen mechanism
```
