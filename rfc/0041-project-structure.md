# RFC 0041: Project Structure & the Demo Page

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0001 (overview), RFC 0030–0038 (toolchain parts),
  RFC 0039 (the self-managed heap), RFC 0040 (resource limits)
- **Part:** F — Toolchain & artifacts

## Summary

Where everything lives: the repository as it stands (§1), the Cargo
workspace it grows into (§2), and the wasm demo page that ships in-repo
(§3–§4). The trees are the spec.

## 1. The repository today

Complete tree — every file on disk, annotated with the RFC it serves:

```
rut/
├── README.md                      # root readme — pitch, repo map, demo quickstart
├── rfc/                           # the design corpus — one topic per RFC (0001 §3)
│   ├── 0001-rut-overview.md       # goals, pillars, decisions, RFC index
│   ├── 0002-lexical-structure.md
│   ├── 0003-modules-and-visibility.md
│   ├── 0004-primitive-types.md    # 0004–0008: surface kernel
│   ├── ...                        # (0005–0037 as numbered — see 0001's index)
│   ├── 0038-module-bundles.md
│   ├── 0039-vm-heap.md            # self-managed VM heap
│   ├── 0040-resource-limits.md    # heap budget, fuel, hang detection
│   └── 0041-project-structure.md  # this file
├── examples/                      # target-syntax corpus; RFCs cite files by path
│   ├── README.md                  # example → RFC index table
│   ├── basic/                     # one feature at a time
│   │   ├── grammar-tour.rut       #   0009–0012 tour
│   │   ├── dataclasses.rut        #   0009  aliasing + own
│   │   ├── classes.rut            #   0010  class-method construction, static fields
│   │   ├── traits.rut             #   0012  dyn, impl, requires, is
│   │   ├── closures-generics.rut  #   0013
│   │   ├── when.rut               #   0008
│   │   ├── option-result.rut      #   0005  + `?` (0034 §2)
│   │   ├── literals.rut           #   0007
│   │   ├── layout.rut             #   0015  size_of/align_of/Opaque
│   │   ├── type-tests.rut         #   0012 §3  `is`
│   │   ├── opaque.rut             #   0014  erasure + downcast
│   │   ├── rc-and-dispose.rut     #   0011/0016  cells, Disposal
│   │   ├── error-context.rut      #   0036
│   │   ├── module-structure.rut   #   0003
│   │   └── module-visibility.rut  #   0003
│   ├── algorithms/                # corpus code, no host deps
│   │   ├── sieve.rut  quicksort.rut  matrix-mul.rut
│   ├── concurrency/               # 0018–0019
│   │   ├── fetch-page.rut  spawn-cancel.rut  select.rut  countdown.rut
│   ├── network/                   # 0021-shaped echo/http pairs
│   │   ├── echo-server.rut  echo-worker.rut  http-fetch.rut
│   ├── workers/                   # isolate pipelines (0021)
│   │   ├── image-pipeline.rut  image-worker.rut
│   ├── memory/                    # 0016–0017
│   │   ├── temp-file.rut  node-cycle.rut  weak-cache.rut
│   ├── host/                      # 0022–0026: rut + Rust + .d.rut together
│   │   ├── my-map.rut             #   consumer
│   │   ├── my_map.rs              #   Rust implementation
│   │   ├── plugin/my_map.d.rut    #   declaration file
│   │   └── interop.rut
│   ├── json/                      # 0037: userland serde
│   │   ├── json.rut               #   the engine
│   │   └── app.rut                #   the consumer
│   └── gui/
│       └── dashboard/             # the app-shaped project (tur-style)
│           ├── main.rut           #   entry, deterministic shutdown
│           ├── models.rut         #   domain dataclasses/enums
│           ├── state.rut          #   the reactive setup
│           ├── reactive.rut       #   state/source/derive/mutation/watch/Store
│           ├── theme.rut
│           ├── components/        #   header.rut sidebar.rut common.rut
│           ├── services/          #   api.rut stream.rut
│           └── workers/           #   stats-worker.rut
└── demo/                          # wasm playground — §3
    ├── package.json  rspack.config.ts  tsconfig.json
    ├── README.md  .gitignore
    ├── public/                    #   rut.wasm drops here (gitignored for now)
    └── src/
        ├── index.html  main.tsx  App.tsx  cases.ts  runner.ts  styles.css
        ├── wasm/rut-api.d.ts      #   the compile/run contract
        └── components/            #   CaseList Editor Panes StatusBar (.tsx)
```

## 2. The planned workspace

Partially built — the crate split below (rut-lexer / rut-ast /
rut-parser, rut-lir / rut-driver, rut-core / rut-vm) exists as of v1;
leaves marked "planned" (hir/, regalloc, decl/, session, pack,
rut-host-std) are not yet written. Every leaf names the RFC that
specifies it:

```
rut/
├── Cargo.toml                      # [workspace] members = crates/*
├── crates/
│   ├── rut-lexer/                  # frontend base — std, wasm-compatible
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── span.rs             #     Span + the shared NEST_MAX budget
│   │       │                       #       (0030 OQ-3)
│   │       ├── diag.rs             #     Diag + renderer (0030 §2/§6)
│   │       ├── token.rs            #     kinds, reserved words (0002 §4)
│   │       └── lexer.rs            #     iterative, bracket-budgeted (0030 §1)
│   ├── rut-ast/                    # the AST — RFC 0030 §5
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── ast.rs              #     flat arena: NodeId records
│   │       └── dump.rs             #     astDump pretty-printer (demo page)
│   ├── rut-parser/                 # the parser — RFC 0030 §4
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs              #     parse(), Mode — monotone cursor,
│   │       │                       #       no backtracking
│   │       └── expr.rs  item.rs  stmt.rs  ty.rs
│   ├── rut-core/                   # core vocabulary — std (0015/0032/0033)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── types.rs            #     RutType table, TypeId (0015)
│   │       ├── ops.rs              #     typed bytecode ops (0032)
│   │       └── binary.rs           #     binary encode/decode (0033)
│   │                               #       + reloc.rs — type_id rebasing
│   │                               #       at link (0035 §1) — planned
│   ├── rut-vm/                     # the VM — std, self-managed heap (0039)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs              #     Slot/Value/Trap surface (0015 §5, 0023)
│   │       ├── heap.rs             #     the RC heap: alloc, accounting,
│   │       │                       #       teardown (0016 §6, 0039)
│   │       ├── verify.rs           #     load-time verifier (0033 §2)
│   │       ├── interp.rs           #     the loop, fuel checks (0034, 0040 §2)
│   │       └── task.rs             #     coroutines, ready ring (0018–0019)
│   ├── rut-lir/                    # the fused middle-end — std,
│   │   │                           #   wasm-compatible: resolve + typecheck
│   │   │                           #   WITH body compilation (RFC 0031 M1)
│   │   │                           #   → LIR. Layout rule: one directory per
│   │   │                           #   pipeline STAGE; each stage is
│   │   │                           #   independently testable over its IR.
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── check/              #     resolve/typecheck — RFC 0031 §1–2
│   │       │   ├── mod.rs          #       Ctx — the fused driver (M1), `==` law
│   │       │   ├── resolve.rs      #       name/type resolution
│   │       │   ├── collect.rs      #       type/trait/impl collection
│   │       │   ├── inst.rs         #       monomorphization queue (0013 §2)
│   │       │   ├── infer.rs        #       bidirectional literals (0007 §1) — planned
│   │       │   ├── coerce.rs       #       dyn widening, own/Weak sites — planned
│   │       │   └── admission.rs    #       generic bounds, requires closure — planned
│   │       ├── hir/                #     SSA-ish typed IR — RFC 0031 §3 — planned
│   │       │   ├── lower.rs        #       ast -> hir
│   │       │   ├── fold.rs         #       const folds, `is` folds, dead code
│   │       │   └── mono.rs         #       (folded into inst.rs for M1)
│   │       ├── lir/                #     bytecode — RFC 0032
│   │       │   ├── mod.rs          #       FnCompiler, typed registers
│   │       │   ├── expr.rs  stmt.rs  ops.rs  lit.rs  call.rs  generic.rs
│   │       │   ├── regalloc.rs     #       typed register assignment — planned
│   │       │   └── suspend.rs      #       state splitting at await (0032 §3) — planned
│   │       └── decl/               #     declaration surfaces — RFC 0029 — planned
│   │           ├── parse.rs        #       .d.rut parsing (in rut-parser today)
│   │           └── declir.rs       #       .d.ir emit + decl digest
│   ├── rut-driver/                 # pipeline driver — std, wasm-compatible
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs              #     compile_module / CompileOutput — the
│   │       │                       #       library face (the wasm demo's
│   │       │                       #       compile() targets this — §3)
│   │       ├── session.rs          #     source map, fs trait, caching — planned
│   │       ├── report.rs           #     diag collection + rendering (0030 §2) — planned
│   │       └── pack.rs             #     .rutbundle — RFC 0038 — planned
│   ├── rut-host-std/               # desktop host — std allowed
│   │   └── src/
│   │       ├── lib.rs              #     Vm::new convenience (0035 §3)
│   │       ├── value.rs            #     the Value boundary (0023)
│   │       ├── futures.rs          #     the futures bridge (0020)
│   │       ├── workers.rs          #     threads + channels (0021)
│   │       └── natives_fs.rs  natives_net.rs      #  0028 rt:* backings
│   ├── rut-wasm/                   # wasm-bindgen shim
│   │   └── src/lib.rs              #     compile/run with budgets (0040 §3)
│   └── rut-cli/                    # the `rut` binary
│       └── src/main.rs
├── demo/                           # ships in-repo — §3
├── examples/                       # unchanged — the parser corpus (0030 §4)
└── rfc/                            # unchanged
```

Dependency graph, one line: `rut-lexer ← rut-ast ← rut-parser ←
rut-driver → rut-lir ─emits binaries via→ rut-core ← rut-vm ←hosts─
rut-host-std / rut-wasm`; `rut-cli` and `demo/` sit on top.

## 3. The demo page

A playground for the language surface and the budget story — **React +
rspack + TypeScript**, no editor dependency (a textarea component with
line numbers; a CodeMirror upgrade is noted but not taken).

- **Layout**: case selector (left) | editor (center) | tabs — Output /
  AST / IR (right) | status bar (fuel used, heap bytes, trap).
- **Wasm API contract** — fixed now, mirrored by
  `demo/src/wasm/rut-api.d.ts`:
  ```ts
  compile(src: string):
    { diags: Diag[]; astDump: string; irDump: string; binary?: Uint8Array }
  run(binary: Uint8Array, budget: { fuel: number; heapBytes: number }):
    { output: string[]; trap?: string; fuelUsed: number; heapBytes: number }
  ```
- **Budgets**: every run is bounded (default 10M fuel / 4 MiB heap —
  RFC 0040); case 8 intentionally `while (true)`s to demo
  `Trap::OutOfFuel` and the resume button.
- **Static-preview mode**: until `rut.wasm` exists the runner shows the
  case's annotated expected output with a banner — the page exercises
  the *spec* today and flips to real execution when the artifact lands
  at `demo/public/rut.wasm` (gitignored until then).

## 4. Workflow

- `cd demo && npm install && npm run dev` — rspack dev server on
  http://localhost:8080; `npm run build` emits `demo/dist/`.
- The runner probes `fetch('rut.wasm')` at boot: 200 → wasm mode,
  404 → static-preview mode with the banner. No config, no env vars.

## Open questions

- OQ-1: URL-hash case sharing (`#/case/fuel-demo`) and `?src=` custom
  payloads — plain base64 proposed.
- OQ-2: embedding runnable demo panes inside rendered RFC pages —
  iframe of the same app vs. a minimal `demo-lite` build.
