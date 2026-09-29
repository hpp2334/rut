# Project structure and rut.toml

One directory is one module; its `rut.toml` names the exact package it
answers to and how to reach its surface and body. This page is the
manifest grammar, the resolution laws, and where everything lives in the
toolchain tree.

## The manifest

```toml
# rut/pouch/rut.toml — a body package
name = "pouch"
entry.lib = "./pouch.rut"

# rut/calc/rut.toml — a pure declaration surface (a host pkg)
name = "calc"
entry.type = "./calc.d.rut"

# a consumer (an app directory)
name = "app"
entry.lib = "./app.rut"

[deps]
pouch = { path = "../pouch" }
ink   = { path = "../ink" }
```

The keys, all of them:

| key | meaning |
|---|---|
| `name` | the package's use-path name: bare `[a-zA-Z0-9_]+` only. A scoped or quoted spelling is a manifest error. |
| `entry.lib` | the body: one `.rut` file (or `.rutc`-style artifacts where supported) |
| `entry.libs` | ordered extra `.rut` files — the **multi-lib entry** (below) |
| `entry.type` | the declaration surface: one `.d.rut` file. `entry.type` alone makes a **host pkg** — a pure signature surface whose `host fn`s the embedder binds at load ([Host fns and declaration files](host-fns.md)) |
| `[deps]` | the transitively mounted dependencies — string-valued descriptors: `pkg = { path = "..." }`, relative to this manifest. `optional` is rejected here. |
| `[peer-deps]` | presence-gated peers — descriptors accept `path`, `optional`, `lib` ([Dependency kinds](dependency-kinds.md)) |
| `[dev-deps]` | mounted only while building/testing this pkg itself |
| `host_scope` | the host-fn registration prefix when it must differ from the package name (`rt` keeps its historical `rt:log` scope) |
| `inline` | `true` forces source-inlining into every consumer instead of linking (packages whose class methods must resolve at the call site — inherent impls cross no surface yet; generic exports link on their own, their instantiations owned by the declaring package) |
| `format`, `format_version` | bundle keys — ignored by directory loading, required by `rut pack` ([Module bundles](bundles.md)) |
| `[style]` | formatter knobs: `indent_width` (1–8, default 4), `max_width` (≥ 20, default 100). Schema-free at the manifest layer — unknown keys ride; malformed values are formatter errors, never compile errors. Resolution: the nearest ancestor manifest of the formatted file; no manifest → defaults. |

The parser accepts only the TOML subset the format uses (comments,
`key = "string"`, dotted keys, `[section]`, inline tables), so the
driver stays dependency-free and wasm-compatible. Descriptors are
key/value maps; unknown descriptor keys are line-targeted manifest
errors.

## Resolution laws

- **Exact, single-step.** A use path resolves only if a module with that
  `name` is mounted. Nothing is derived from directories or file
  layouts.
- **The walk is recursive, with a cycle guard; first mount wins.** An
  already-mounted name (the embedder's, the root's, or an earlier dep's)
  is never overwritten — which is what makes peer presence-by-name sound:
  the consumer's own path for a package always beats the declarer's
  `path`.
- **Name mismatch is an error.** A dep whose manifest `name` disagrees
  with its key fails, naming both.
- **The cross-table law.** A name in `[deps]` beside `[peer-deps]` or
  `[dev-deps]` is an error naming both rows; peer + dev together is the
  sanctioned pairing.
- **`core` needs no `[deps]`** — the driver mounts it unconditionally;
  every name still requires `use core::{ .. };` per
  [core and the swappable packages](stdlib.md).

## The multi-lib entry

A package's body may be split across files:

```toml
name = "ui"
entry.lib  = "./ui.rut"
entry.libs = ["./store.rut", "./t1.rut"]
```

The loader splices base-first, then `libs` in listed order, newline-joined,
into **one module** — one namespace, one visibility scope. A name private
to one file is visible to every other file of the package. The manifest
is the canonical order (the splice never reads a directory listing —
same manifest ⇒ same module). A `libs` row without `lib`, a `.d.rut`
element, or a file named twice is a loud manifest error. This is
assembly, not a language include form — use paths stay inter-module.

Contrast with peer groups: group files are *presence-gated* and therefore
impl-only; multi-lib files are unconditional and full module citizens —
types, functions, and `pub` surface are legal in any file.

## The toolchain tree

```
rut/
├── crates/
│   ├── rut-lexer/        # spans, tokens, the lexer, diagnostics
│   ├── rut-ast/          # the flat arena AST + dumper
│   ├── rut-parser/       # the frame machine, Mode::Impl | Mode::Decl
│   ├── rut-lir/          # the fused middle end: check/ (resolve,
│   │                     #   typecheck, monomorphize) + lir/ (bodies,
│   │                     #   peephole, SROA, async lowering)
│   ├── rut-core/         # types, ops, binary encode/decode, link, sym
│   ├── rut-vm/           # heap, interpreter, verifier, driving loop
│   ├── rut-vm-threaded/  # tail-call threaded dispatch (nightly `become`)
│   ├── rut-driver/       # session, loader, bundles, dep graph, decl
│   ├── rut-fmt/          # the formatter
│   ├── rut-std/          # host bodies: log, math, nmap, http, async
│   ├── rut-wasm/         # the wasm ABI (compile/run envelope)
│   ├── rut-lsp/          # the language server (stdio)
│   ├── rut-lsp-wasm/     # the same queries as an in-process wasm module
│   └── rut-cli/          # the `rut` binary ([The rut CLI](cli.md))
├── rut/                  # the in-tree packages (below)
├── examples/             # 00-todolist … 06-github-viewer-cli
├── demo/                 # the wasm playground (React + rspack)
├── benches/              # cross-runtime benchmarks
└── integrations/         # editor configs; the vscode extension
```

Dependency line: `rut-lexer ← rut-ast ← rut-parser ← rut-driver →
rut-lir → rut-core ← rut-vm` (with `rut-vm-threaded` behind it), hosts
(`rut-std`, `rut-wasm`) on top, and `rut-cli`/`demo` above those.
`rut-lsp` sits on the frontend crates only — tokens, diagnostics, and
symbols need no VM.

## The in-tree packages

`rut/` carries the standard and swappable packages the CLI mounts on
demand:

| package | shape |
|---|---|
| `core` | the only standard package — the builtin surface, mounted unconditionally |
| `calc` | host pkg: math surface (`mount_calc`) |
| `rt`, `http_host`, `nmap_host`, `async_engine`, `bench_cross` | host pkgs — pure `.d.rut` surfaces; bodies live in `rut-std` |
| `ink`, `http`, `strbuild`, `async_host` | inline rut wrappers over host rows (`inline = true`) |
| `pouch` | the sequence library (plain linked package) |
| `json` | the base pkg with `[peer-deps]`/`[dev-deps]` — the reference consumer of [Dependency kinds](dependency-kinds.md) |
| `nmapset` | native-key maps/sets over `nmap_host` |

Nothing in the engine knows the swappable packages' names — peers and
deps are resolved by name from manifests; a host may replace the
non-`core` set wholesale ([Embedding and native modules](embedding.md)).

## The playground

`demo/` is a React + rspack + TypeScript page over the wasm build. The
wasm ABI is two calls — `compile(src)` returning diagnostics, an AST
dump, an IR dump, and an optional binary; `run(binary, budget)`
returning output lines, an optional trap, and used fuel/heap — with
every run bounded (default 10M fuel / 4 MiB heap) so unbounded loops
trap and resume instead of hanging the page. The served artifact is
deployed at <https://playground.rut.hpp2334.com>.
