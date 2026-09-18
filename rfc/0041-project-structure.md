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
├── examples/                      # the runnable example projects
│   ├── README.md                  # what each project demonstrates
│   ├── 00-todolist/               # entry-fn surface over a rut class (CRUD)
│   ├── 01-sort/                   # sorting library behind one dispatcher entry
│   ├── 02-digest/                 # byte-level codecs + hashes, host is the oracle
│   ├── 03-plugin/                 # module directory + .rutbundle chat moderator
│   └── 04-custom-async/           # parse-only corpus: a user impl of the
│                                   #   builtin `Task<T>` trait + a user launcher
│                                   #   (runnable when the async plan lands)
├── benches/                       # rut vs QuickJS-ng vs V8 (README)
│   ├── README.md                  #   method, fairness rules, workloads
│   ├── run.mjs                    #   cross-runtime runner (wall + peak RSS)
│   ├── probe/                     #   rut-bench-probe: compile/verify/exec,
│   │                              #     fuel + VM-heap high-water (0039/0040)
│   ├── tools/
│   │   ├── quickjs-ng/            #   vendored engine (git submodule, pinned)
│   │   └── build-quickjs.sh       #   gcc build -> .tools/qjs
│   └── workloads/                 #   NAME.rut + NAME.js pairs, one checksum
├── integrations/                   # editor integrations — vscode first
│   ├── README.md                  #   rut-lsp configs: nvim/helix/zed/
│   │                              #   emacs/sublime
│   └── vscode-extension/          # `rut-vscode`: package.json (client +
│                                  #   semanticTokenScopes + settings),
│                                  #   language-configuration.json,
│                                  #   syntaxes/rut.tmLanguage.json,
│                                  #   src/extension.ts, esbuild.mjs,
│                                  #   scripts/copy-server.mjs
└── demo/                          # wasm playground — §3
    ├── package.json  rspack.config.ts  tsconfig.json
    ├── README.md  .gitignore
    ├── public/                    #   rut.wasm drops here (gitignored for now)
    └── src/
        ├── index.html  main.tsx  App.tsx  cases.ts  runner.ts  styles.css
        ├── examples/              #   the classics — real .rut files + .expected
        │   ├── index.ts           #     metadata; raw-imports both (asset/source)
        │   ├── sieve.rut …        #     gate: crates/rut-cli/tests/playground.rs
        │   └── sieve.expected …   #     actual pipeline output, asserted
        ├── wasm/rut-api.d.ts      #   the compile/run contract
        └── components/            #   CaseList Editor Panes StatusBar (.tsx)
```

## 2. The planned workspace

Partially built — the crate split below (rut-lexer / rut-ast /
rut-parser, rut-lir / rut-driver, rut-core / rut-vm) exists as of v1;
`rut-lsp` + `integrations/` land the M6 LSP slice early (RFC 0001 M6:
semantic tokens, diagnostics, document symbols — hover and
completion have landed; formatting stays with M6). It is also the workspace's first external
dependency (tower-lsp-server + tokio). Leaves marked "planned" (hir/, regalloc, decl/, session, pack,
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
│   │       │   ├── coerce.rs       #       trait widening, own/Weak sites — planned
│   │       │   └── admission.rs    #       generic bounds, requires closure — planned
│   │       ├── hir/                #     SSA-ish typed IR — RFC 0031 §3 — planned
│   │       │   ├── lower.rs        #       ast -> hir
│   │       │   ├── fold.rs         #       const folds, `is` folds, dead code
│   │       │   └── mono.rs         #       (folded into inst.rs for M1)
│   │       ├── lir/                #     bytecode — RFC 0032
│   │       │   ├── mod.rs          #       FnCompiler, typed registers
│   │       │   ├── expr.rs  stmt.rs  ops.rs  lit.rs  call.rs  generic.rs
│   │       │   ├── regalloc.rs     #       typed register assignment — planned
│   │       │   └── async.rs        #       state splitting at await (0032 §3) — planned
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
│   ├── rut-lsp/                    # the language server — the M6 LSP
│   │   │                           #   slice, landed early (0001 M6)
│   │   ├── Cargo.toml              #     tower-lsp-server + tokio — the
│   │   │                           #     workspace's first external deps
│   │   └── src/
│   │       ├── lib.rs              #     module map
│   │       ├── line_index.rs       #     byte spans (normalized src) ⇄
│   │       │                       #       LSP UTF-16 positions
│   │       ├── analysis.rs         #     analyze(): normalize → lex →
│   │       │                       #       parse → tokens/diags/symbols
│   │       ├── semantic/           #     the classifier (pure, tested)
│   │       │   ├── legend.rs       #       token-type legend, keyword
│   │       │   │                   #       set
│   │       │   ├── tokens.rs       #       token-pass classes, f-string
│   │       │   │                   #       tiling
│   │       │   ├── names.rs        #       AST-pass name classes
│   │       │   ├── recover.rs      #       name-token recovery
│   │       │   └── symbols.rs      #       the outline builder
│   │       ├── hover/              #     the definition index behind
│   │       │   ├── types.rs        #       hover: verbatim signatures,
│   │       │   │                   #       doc comments, the unified
│   │       │   │                   #       method rule (own surface ∪
│   │       │   │                   #       inherent impl methods ∪
│   │       │   │                   #       use-gated trait impls)
│   │       │   ├── build.rs        #       the index pass
│   │       │   ├── lookup.rs       #       what's under the cursor
│   │       │   ├── infer.rs        #       local receiver inference
│   │       │   └── render.rs       #       markdown rendering
│   │       ├── completion.rs        #     member + bare completion over the
│   │       │                       #       same index (impl-block methods,
│   │       │                       #       the use-both gate, keywords)
│   │       ├── server.rs           #     Backend — full sync, semantic
│   │       │                       #       tokens, documentSymbol, hover,
│   │       │                       #       completion, publishDiagnostics;
│   │       │                       #       embeds the std surface, scans
│   │       │                       #       the workspace for cross-file defs
│   │       └── main.rs             #     the `rut-lsp` stdio binary
│   └── rut-cli/                    # the `rut` binary
│       └── src/main.rs
├── rut/                            # the in-tree packages (RFC 0028) —
│   │                               #   `core` is the only STANDARD; pouch/
│   │                               #   calc/ink are swappable defaults a
│   │                               #   host may replace wholesale
│   ├── core/
│   │   ├── core.d.rut              #   prelude: uniformly `builtin class`/
│   │   │                           #   `builtin trait`/`builtin fn` — fns
│   │   │                           #   (own/downcast/assert/panic/str/bytes
│   │   │                           #   natives), types (Option/Result/
│   │   │                           #   Opaque/[T]), the engine-woven
│   │   │                           #   traits (Iterator; the async plan
│   │   │                           #   adds Task + contexts + launch_task/
│   │   │                           #   LaunchedTask as core builtin decls)
│   │   │                           #   (0028; RFC 0025 revised) — used,
│   │   │                           #   never ambient
│   │   └── rut.toml                #   core needs NO [deps] — the driver
│   │                               #   mounts it unconditionally
│   ├── pouch/
│   │   ├── pouch.rut               #   one .rut per package — the entry
│   │   └── rut.toml                #   the manifest names: [deps] + entry
│   ├── calc/
│   │   ├── calc.d.rut              #   host fns + builtin intrinsics (0028)
│   │   └── rut.toml
│   └── ink/
│       ├── ink.rut                 #   the Logger (0028), over the host's
│       │                           #   `rt` module
│       └── rut.toml
├── integrations/                   # editor integrations
│   ├── README.md                   #   nvim/helix/zed/emacs/sublime
│   │                               #   configs over the rut-lsp binary
│   └── vscode-extension/           # `rut-vscode` — TM grammar + LSP
│       ├── package.json            #   client; semanticTokenScopes,
│       │                           #   rut.serverPath, build:server
│       ├── language-configuration.json
│       ├── syntaxes/rut.tmLanguage.json  # fallback coloring (0002/0007)
│       ├── src/extension.ts        #   vscode-languageclient wiring
│       ├── esbuild.mjs  scripts/copy-server.mjs
│       └── README.md
├── demo/                           # ships in-repo — §3
├── benches/                        # cross-runtime benchmarks — rut/QuickJS/V8
├── examples/                       # the 00–03 runnable projects + 04
│                                  #   parse-only corpus (§1); the parse
│                                  #   corpus is those + the demo classics
│                                  #   (0030 §7)
└── rfc/                            # unchanged
```

Dependency graph, one line: `rut-lexer ← rut-ast ← rut-parser ←
rut-driver → rut-lir ─emits binaries via→ rut-core ← rut-vm ←hosts─
rut-host-std / rut-wasm`; `rut-cli` and `demo/` sit on top, and
`rut-lsp` sits on `rut-lexer`/`rut-ast`/`rut-parser` (no driver dep —
tokens and diags come from the frontend); `integrations/vscode-extension`
launches it.

## 3. The demo page

A playground for the language surface and the budget story — **React +
rspack + TypeScript**, no editor dependency (a textarea component with
line numbers; a CodeMirror upgrade is noted but not taken).

- **Layout**: case selector (left) | editor (center) | tabs — Output /
  AST / IR (right) | status bar (fuel used, heap bytes, trap). The
  selector has two groups: **cases** (the spec snippets in `cases.ts`)
  and **classics** — real, runnable programs loaded from
  `src/examples/*.rut` (raw-imported; the playground edits live files,
  not string copies). Each classic carries a `NAME.expected` sidecar
  that is the program's *actual* pipeline output, gated by
  `crates/rut-cli/tests/playground.rs` — the static-preview text is
  never a hand-written guess.
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

## 5. Package manifests

Every in-tree or user package carries a `rut.toml` at its root (§2's
tree). The rules are small on purpose:

- **`name`** — the package's use-path name: bare `[a-zA-Z0-9_]+` only
  (RFC 0002 §4). A scoped or quoted spelling is a manifest error whose
  diagnostic points here.
- **`[deps]`** — the packages this one uses: `pkg = { path = "..." }`,
  relative to this manifest. `core` needs **no** `[deps]`
  entry: the driver mounts it unconditionally (resolution-ambient,
  name-explicit — every name still requires `use core::{ .. };`, RFC
  0028). Every other specifier — `pouch`, `ink`, app packages,
  host pkgs — is declared here or mounted by the embedder
  (`mount_dir`); an undeclared specifier's diagnostic names the
  manifest. Resolution walks the graph **recursively**, with a cycle
  guard, and **first mount wins**: an already-mounted name (the
  embedder's, the root's, or an earlier dep's) is never overwritten; a
  dep whose manifest `name` disagrees with its key is an error naming
  both.
- **entry** — the package's file: one `.rut` (or `.d.rut`) per package;
  the manifest names it (`entry.lib` for a rut-source package,
  `entry.type` alone for a **host pkg** — a pure declaration surface
  whose `host fn`s lower into the mounted surface at load time,
  RFC 0025).
- **`host_scope`** — optional: the host-fn registration prefix when it
  must differ from the package name (`rt` keeps its historical `rt:log`
  scope, RFC 0022).
- **`inline`** — optional `true`: force source-inlining into every
  consumer (`ink` — a module whose class methods must resolve at the
  call site cannot be linked).

The engine's own mounts are `core` and `calc` (`mount_std`); `pouch`
and `ink` are third-party libraries in the toolchain tree — nothing in
the engine knows their names.
