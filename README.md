# rut

A small, statically-typed language for embedding — reified runtime
types, reference semantics with `own` copies, deterministic
destructors, poll-based coroutines, and a no-JIT register VM designed
around budgets (fuel + heap) so third-party code can never hang or
explode an embedder. Start at **[`rfc/0001-rut-overview.md`](rfc/0001-rut-overview.md)**.

> Status: design corpus — 41 RFCs and a target-syntax example set; no
> compiler yet. The RFCs are the product.

```
rut/
├── rfc/        41 single-topic RFCs — the spec (0001 has the index)
├── examples/   the target-syntax corpus; RFCs cite files by path
└── demo/       wasm playground (React + rspack + TS) — RFC 0041 §3
```

## The demo page

```
cd demo
npm install
npm run dev        # http://localhost:8080
```

Pick a prepared case (or edit one), press **Run**, and inspect
Output / AST / IR. Every run is bounded by fuel + heap budgets
(RFC 0040). Until the wasm artifact exists the page runs in
**static-preview mode** — it shows annotated expected output with a
banner; drop a build at `demo/public/rut.wasm` and it flips to live
execution.

## The plan

The repository grows into a Cargo workspace — `rut-core` (the VM,
self-managed heap), `rutc` (compiler), `rut-host-std`, `rut-wasm`,
`rut-cli` — mapped file-by-file in
**[`rfc/0041-project-structure.md`](rfc/0041-project-structure.md)**.
