# Project structure and rut.jsonc

One directory is one module; its `rut.jsonc` names the exact package it
answers to and how to reach its surface. The body is a **file module
tree**: the root module is `mod.rut` beside the manifest, and `mod
NAME;` declarations in it mount child modules (`NAME/mod.rut` beside
the declaring file). The manifest is what makes the directory **the
runnable unit**: `rut run <dir>` (or its packed `.rutbundle`) is the
only run lane — a loose `.rut` file is not a program ([the rut
CLI](cli.md)). This page is the manifest grammar, the resolution laws,
and where everything lives in the toolchain tree.

## The manifest

```jsonc
// rut/pouch/rut.jsonc — a body package (the body is mod.rut beside it)
{
  "name": "pouch"
}

// rut/calc/rut.jsonc — a pure declaration surface (a host pkg)
{
  "name": "calc",
  "type": "host",
  "entry": { "type": "./calc.d.rut" }
}

// a consumer (an app directory): a `path` row for a sibling package
// of your own, pinned url rows for the toolchain's packages
{
  "name": "app",

  "deps": {
    "greet": { "path": "../greet" },
    "pouch": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v8/dist/std/pouch.rutbundle", "sha256": "128ffcf7daa2c43ed3ebad00a6a91ff7c4c494f2442c0a80d6f2c0daa455cc34" },
    "ink":   { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v8/dist/std/ink.rutbundle", "sha256": "04d069cdf71922e7cd41224ee813b78f568c66b878d909e92b026be2530e57c0" }
  }
}
```

The keys, all of them:

| key | meaning |
|---|---|
| `name` | the package's use-path name: bare `[a-zA-Z0-9_]+` only. A scoped or quoted spelling is a manifest error. |
| `entry.type` | the declaration surface: one `.d.rut` file — the ONLY entry key left. On a lib pkg it is documentation surface (a surface-only dev state mounts as a declaration unit; `host fn` text there is refused) |
| `type` | the declared kind: `type = "host"` for a **host pkg** — a pure declaration surface whose `host fn`s the embedder binds at load ([Host fns and declaration files](host-fns.md)); `type = "lib"` (or absent) is the ordinary source package. The kind is never inferred — an `entry.type`-only manifest with no kind and no `mod.rut` is an error naming both fixes |
| `deps` | the transitively mounted dependencies — one source per descriptor: `pkg = { path = "..." }` (relative to this manifest) or `pkg = { url = "https://…", sha256 = "<64-hex>" }` (a remote `.rutbundle`, the pin optional but recommended) ([Dependency kinds](dependency-kinds.md)). `optional` is rejected here. |
| `peer-deps` | presence-gated peers — descriptors accept `path`, `optional`, `lib` ([Dependency kinds](dependency-kinds.md)) |
| `dev-deps` | mounted only while building/testing this pkg itself |
| `inline` | `true` forces source-inlining into every consumer instead of linking (packages whose class methods must resolve at the call site — inherent impls cross no surface yet; generic exports link on their own, their instantiations owned by the declaring package) |
| `format`, `format_version` | bundle keys — ignored by directory loading, required by `rut pack` ([Module bundles](bundles.md)) |
| `style` | formatter knobs: `indent_width` (1–8, default 4), `max_width` (≥ 20, default 100). Schema-free at the manifest layer — unknown keys ride; malformed values are formatter errors, never compile errors. Resolution: the nearest ancestor manifest of the formatted file; no manifest → defaults. |

The **body keys are repealed**: `entry.lib` and `entry.libs` refuse
loudly, each error naming the fix — the root module is `mod.rut` beside
the manifest, structure lives in `mod` directories. (Inside a
*published* bundle the old keys still load — the reader's compat lane;
see [Module bundles](bundles.md).)

The manifest text is **JSONC** — `//` line comments, `/* */` block
comments, and trailing commas are all legal — parsed by `serde_json`
behind a syntax-stripping front stage: the comment and comma bytes
become spaces before the parser sees them, so a syntax error keeps the
parser's own wording under a `line N:` prefix that names the ORIGINAL
file's line. The value laws are unchanged. Value errors are
path-targeted (`deps.pouch: unknown key 'feats'`). Descriptors are
key/value objects; unknown descriptor keys are path-targeted manifest
errors. Duplicate keys are last-wins. An `_`-key refuses loudly naming
the fix — comments are the prose now.

## Resolution laws

- **Exact, single-step.** A use path resolves only if a module with that
  `name` is mounted. Nothing is derived from directories or file
  layouts beyond the `mod` declarations the source itself spells.
- **The walk is recursive, with a cycle guard; first mount wins.** An
  already-mounted name (the embedder's, the root's, or an earlier dep's)
  is never overwritten — which is what makes peer presence-by-name sound:
  the consumer's own path for a package always beats the declarer's
  `path`.
- **Name mismatch is an error.** A dep whose manifest `name` disagrees
  with its key fails, naming both.
- **The cross-table law.** A name in `deps` beside `peer-deps` or
  `dev-deps` is an error naming both rows; peer + dev together is the
  sanctioned pairing.
- **`core` needs no `deps`** — `.compile()` auto-offers it
  unconditionally ([core and the swappable packages](stdlib.md));
  every name still requires `use core::{ .. };` per
  [core and the swappable packages](stdlib.md).

## Module trees

A package's body is authored as a **file module tree** — the
`tur_kit`-style layout:

```
tur_kit/
├── rut.jsonc        # no entry keys — mod.rut IS the lib
├── mod.rut          # the root module
├── layout/
│   └── mod.rut      # tur_kit::layout
└── widget/
    └── mod.rut      # tur_kit::widget
```

The root module declares its children:

```rut
// tur_kit/mod.rut
pub mod layout;
pub mod widget;
```

Mounting is **declared, not discovered**: `mod NAME;` (or `pub mod
NAME;`) in a file resolves against the sibling `NAME/mod.rut`
directory. The loader never reads a directory listing — an undeclared
directory is invisible, a declared child that is missing (or whose
directory carries no `mod.rut`) is a loud error naming both spellings,
and a `NAME.rut` file where a module directory is expected says so.
Mounting is recursive and cycle-guarded by file identity; a repeated
declaration mounts once.

Visibility is per module file ([Modules and
visibility](modules-and-visibility.md)): plain declarations are visible
in their own module and its descendants, `pub(pkg)` shares inside the
package, `pub` crosses packages. Cross-file references inside one
package are qualified positions (`layout.Column`, `widget.mk(3)` —
the dot spelling); `use` statements stay the only cross-package door
(`use tur_kit::layout::{ Column }`).

## The toolchain tree

```
rut/
├── crates/
│   ├── rut-lexer/        # spans, tokens, the lexer, diagnostics
│   ├── rut-ast/          # the flat arena AST + dumper
│   ├── rut-parser/       # the frame machine, Mode::Impl | Mode::Decl
│   ├── rut-semantic/     # the semantic classifier (spans + token classes)
│   ├── rut-lir/          # the fused middle end: check/ (resolve,
│   │                     #   typecheck, monomorphize) + lir/ (bodies,
│   │                     #   peephole, SROA, async lowering)
│   ├── rut-core/         # types, ops, binary encode/decode, link, sym
│   ├── rut-vm/           # heap, interpreter, verifier, driving loop
│   ├── rut-vm-threaded/  # tail-call threaded dispatch (nightly `become`)
│   ├── rut-driver/       # the run chain: Pkg, RutRun, Compiled; bundles, dep graph, decl
│   ├── rut-native/       # the walk and the world: Source/FsSource, DepRemote/HttpRemote, pack
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
rut-lir → rut-core ← rut-vm` (with `rut-vm-threaded` behind it; the
driver sits on the vm's `HostPkgContext`/`HostRegistry` for the
install law), `rut-native` beside the driver (it depends on the
driver, never the reverse — the driver is provably pure: no fs, no
net, no `std::path`), hosts
(`rut-std`, `rut-wasm`) on top, and `rut-cli`/`demo` above those.
`rut-lsp` sits on the frontend crates — tokens, diagnostics, and
symbols need no VM — plus the driver's pure core for manifest and
bundle parsing (the dep tables behind the editor's dep walk). Its
classifier is `rut-semantic` (re-exported as
`rut_lsp::semantic`), which sits on the frontend crates for the same
reason — embedders take it without the server.

## The in-tree packages

`rut/` carries the standard and swappable packages the CLI offers on
demand:

| package | shape |
|---|---|
| `core` | the only standard package — the builtin surface, auto-offered by every run's `.compile()` |
| `calc` | host pkg: the math surface — offer `rut_native::tree_pkg("calc")`, bind `rut_std::math::pkg()` ([stdlib](stdlib.md)) |
| `ink_host`, `http_host`, `nmap_host`, `async_host`, `bench_cross` | host pkgs — pure `.d.rut` surfaces; bodies live in `rut-std` |
| `ink`, `http`, `strbuild`, `futures` | inline rut wrappers over host rows (`inline = true`) |
| `pouch` | the sequence library (plain linked package) |
| `json` | the base pkg with `peer-deps`/`dev-deps` — the reference consumer of [Dependency kinds](dependency-kinds.md) |
| `nmapset` | native-key maps/sets over `nmap_host` |

Nothing in the engine knows the swappable packages' names — peers and
deps are resolved by name from manifests; a host may replace the
non-`core` set wholesale ([Embedding and native modules](embedding.md)).

## The playground

`demo/` is a React + rspack + TypeScript page over the wasm build. The
wasm ABI is two calls — `compile(src)` returning diagnostics, an AST
dump, an IR dump, and an optional binary; `run(binary, budget)`
returning output lines, an optional trap, and used fuel/heap. Fuel is
off by default (fuel 0 is the ABI's uncapped encoding) and opts in
through the page's fuel box; the heap defaults to 4 MiB, and auto-runs
keep an internal 1M fuel watchdog so unbounded loops park and resume
instead of hanging the page. The served artifact is deployed at
<https://playground.rut.hpp2334.com>.
