# The playground

The playground is the zero-install way to run rut: the whole engine —
lexer, parser, compiler, VM — compiled to WebAssembly and served as a
static page. It is deployed at
**<https://playground.rut.hpp2334.com>**.

## The layout

- **Case list** — prepared programs, ordered the way the repo's own
  gate runs them: algorithms first (sieve, quicksort, matrix multiply),
  then language-surface tours (classes, closures & generics, structs,
  literals, checked arithmetic, str views, bytes, opaque, when, maps &
  sets, type aliases), then memory shapes (node cycle, tree, weak
  cache).
- **Editor** — the case's source, live-editable. The cases are read
  from the same `.rut` files the repository's native test gate
  compiles and runs, so what you edit is the real corpus, not a copy.
- **Output / AST / IR panes** — every run shows the program output
  plus the two compile views: the parsed AST and the typed IR the
  compiler lowers to. `rut dump <file>` prints the same structures
  from the CLI.
- **Budget control** — each run executes under an explicit fuel + heap
  budget (default 10,000,000 fuel / 4 MiB heap). A budget that bites is
  a feature, not a bug: one prepared case parks on the out-of-fuel trap
  on purpose, and the Resume control adds fuel and continues the *same*
  frame — that is structured-concurrency-grade resumption, not a
  restart.

## The ✓/✗ verify chip

Every prepared case carries its expected output inline. After each run
the output is diffed against that expectation and the status bar shows
a ✓ or ✗ chip (a failing verify renders a line-paired diff). If a run
ever disagrees with its expected lines, that is a bug in the engine —
the chip is the page's honesty contract.

## No silent fallback

The page probes the wasm artifact at boot. If the engine or the
syntax-highlighting module is missing or invalid, you get a full-page
error naming the exact build command — the panes never mount in a
degraded mode. On the deployed site this never happens; when you run a
local build (below), the preflight exists so a stale artifact cannot
masquerade as a working playground.

## Running it locally

```sh
cd demo
npm install
npm run build:wasm   # builds rut.wasm + rut-lsp.wasm, copies to public/
npm run dev          # http://localhost:8080 with live reload
```

Other lanes:

```sh
npm run build        # static build into dist/
npm run smoke        # headless gate: every case really runs and verifies
npm run dev:channel  # dev server + a throwaway public https URL for demos
```

The corpus itself is documented in
[the playground corpus](../examples/playground-corpus.md), with the
bigger end-to-end projects under [Examples](index.md).
