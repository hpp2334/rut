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
- **Output / AST / IR panes** — every run shows the program's real
  output plus the two compile views: the parsed AST and the typed IR
  the compiler lowers to. `rut dump <file>` prints the same structures
  from the CLI.
- **Budget control** — fuel is **off by default**: an explicit ▶ Run is
  uncapped, and a non-terminating program freezes the tab — that is
  what uncapped means. The checkbox arms the fuel box (seeded at
  10,000,000; heap defaults to 4 MiB): a run that exhausts its fuel
  parks on the out-of-fuel trap, and the Resume control adds fuel and
  continues the *same* frame — structured-concurrency-grade
  resumption, not a restart. Auto-runs (case selection, edits) keep an
  internal 1M watchdog slice so a stray loop parks instead of freezing
  the page.

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
npm run smoke        # headless gate: every case really runs
npm run dev:channel  # dev server + a throwaway public https URL for demos
```

The corpus itself is documented in
[the playground corpus](../examples/playground-corpus.md), with the
bigger end-to-end projects under [Examples](index.md).
