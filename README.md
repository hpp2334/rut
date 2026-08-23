# rut

A small, statically-typed language for embedding — reified runtime
types, reference semantics with `own` copies, deterministic
destructors, poll-based coroutines, and a no-JIT register VM designed
around budgets (fuel + heap) so third-party code can never hang or
explode an embedder. Start at **[`rfc/0001-rut-overview.md`](rfc/0001-rut-overview.md)**.

> Status: the design corpus is implemented through **M1** — the compiler
> frontend (RFC 0030), the fused resolve/typecheck/monomorphization pass
> (RFC 0031, M1 shape), typed register bytecode (RFC 0032), the versioned
> module binary + verifier (RFC 0033), and the interpreter with
> fuel/heap budgets (RFC 0034/0040) — enough of the static core to run
> the demo page **live**. The RFCs remain the full product; see
> *What runs today* below for the exact slice.

```
rut/
├── rfc/        41 single-topic RFCs — the spec (0001 has the index)
├── examples/   the target-syntax corpus; RFCs cite files by path
├── crates/     the Cargo workspace — RFC 0041 §2 layout
│   ├── rut-core/    VM: RutType table (0015), RC heap (0016), binary +
│   │                verifier (0033), interpreter + budgets (0034/0040)
│   ├── rutc/        compiler: lexer/parser/AST/diags (0030), check +
│   │                monomorphization (0031), LIR (0032), emit (0033)
│   ├── rut-wasm/    the demo's compile/run surface over a raw wasm ABI
│   └── rut-cli/     the `rut` binary (run / dump) + the e2e test suite
└── demo/       wasm playground (React + rspack + TS) — RFC 0041 §3
```

## Quick start

```sh
cargo test --workspace     # 18 tests: corpus gate + 8 demo cases end-to-end
cargo run -q -p rut-cli -- run examples/algorithms/sieve.rut
cargo run -q -p rut-cli -- dump path/to/file.rut    # AST + IR dumps
```

## The demo page

```sh
cd demo
npm install
npm run build:wasm     # cargo build rut-wasm + copy to public/rut.wasm
npm run dev            # http://localhost:8080
```

Pick a prepared case (or edit one), press **Run**, and inspect
Output / AST / IR — all three tabs are live compiler output. Every run
is bounded by fuel + heap budgets (RFC 0040); the fuel demo case traps
`OutOfFuel` with the frame parked, and **Resume** adds fuel and reruns.

## What runs today (the M1 slice)

Executable — primitives, `string`, `Vec<T>`/`Array<T,N>`, `Option<T>`,
`Result<T,E>` surface, simple enums, dataclasses **and** classes
(class-method construction, `Self {}`, field initializers), traits +
`impl` blocks with vtable dispatch (`dyn I`, calli — no
devirtualization), `is` (exact + capability probe), `Opaque.new`/
`downcast<T>`, `own()`, closures (by-value captures, fn types),
monomorphized generic free functions, `when` expressions with
exhaustiveness, all assignment operators incl. the wrapping `&+` family,
conversions `i32(x)`…, `f"..."` with the RFC 0007 rendering table,
deterministic destruction at rc 0, `Trap::OutOfFuel/OutOfMemory/
Overflow/...` — all enforced by the verifier at load.

Parse-only — the whole `examples/` corpus (39 `.rut` + `.d.rut`,
zero diags; RFC 0030 §7 gate in `rutc/tests/corpus.rs`), including
`suspend`/`await`/`select` and declaration-mode surfaces; the compiler
rejects those features with targeted M2/M3 messages.

Not yet — module loading/imports (M2, RFC 0035), coroutines (M3,
RFC 0018–0020), workers (M4), weak refs (M5).

### Documented deviations (M1 pragmatism; RFCs stay authoritative)

- **C2**: the parser is depth-budgeted recursive descent, not the
  explicit-frame machine of RFC 0030 §4 — the *contract* (untrusted
  source cannot overflow the host stack; deep input is one clean
  `Diag`) holds via budgets (expressions 64, brackets/blocks 1024).
- The SSA-ish HIR of RFC 0031 §3 is fused into a typed-AST → LIR walk;
  folding is limited to the layout builtins and `is` folds.
- Closures capture **by value** (RFC 0013 §1 says by reference — lands
  with coroutine frames, RFC 0018 §4).
- The heap is `Rc`-based with byte accounting and pre-allocation budget
  checks; RFC 0039's self-managed arena is a later milestone. The
  observable contract (deterministic destructors, identity, `OutOfMemory`
  before any write) holds.
- RFC 0002 §4: `super`/`as`/`default` are contextual, not hard-reserved
  (`export(super)` RFC 0003 §2, `as` select-arm binding RFC 0019 §3,
  `.default()` members); f-string holes allow string literals (the
  lexer's hole termination is brace-based — RFC 0007 §2's note stays a
  style rule). Both decided by the corpus, which RFC 0030 §7 makes the
  conformance suite.

## The plan (RFC 0001)

- **M2** — module loading + host embedding API: natives registry, `Value`
  boundary, repr-C struct interop (RFC 0022–0028, 0035).
- **M3** — coroutines: `suspend`/`await` state machines, host-driven
  executor, cancellation (RFC 0018–0020).
- **M4** — workers, channels, transferables (RFC 0021).
- **M5** — weak refs, shutdown leak reporting (RFC 0017).
- **M6** — tooling: formatter, LSP, debugger protocol; `.rutbundle`
  (RFC 0038); pilot integration in tur.
