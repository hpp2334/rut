# Project structure and rut.jsonc

One directory is one module; its `rut.jsonc` names the exact package it
answers to and how to reach its surface. The body is a **file module
tree** under the package's `src/` shell: the root module is
`src/mod.rut`, and `mod NAME;` declarations in it mount child modules
(`src/NAME/mod.rut` beside the declaring file, or the single-file
`NAME.rut` next to it) — recursively, so the whole module tree lives
under `src/`, and only the module tree does: the manifest, `rut.lock`,
and `rut.scopes` stay at the package root. The shell is law, not
convention — a `.rut` outside `src/` refuses at the pack door and in
the source-zip grammar alike ([Module bundles](bundles.md)). The manifest is
what makes the directory **the runnable unit**: `rut run <dir>` (or its
packed artifact — the source `.zip` or the compiled `.rutbundle`) is
the only run lane — a loose `.rut` file is not
a program ([the rut CLI](cli.md)). This page is the manifest grammar,
the resolution laws, and where everything lives in the toolchain tree.

## The manifest

```jsonc
// rut/pouch/rut.jsonc — a body package (the body is src/mod.rut)
{
  "name": "pouch"
}

// rut/calc_host/rut.jsonc — a pure declaration surface (a host pkg);
// the spelled surface resolves under src/ (the file is src/calc.d.rut)
{
  "name": "calc_host",
  "type": "host",
  "entry": { "type": "./calc.d.rut" }
}

// the same surface, the default-root spelling: src/mod.d.rut IS the
// root — entry.type is optional on a host pkg
{
  "name": "calc_host",
  "type": "host"
}

// a consumer (an app directory): a `path` row for a sibling package
// of your own, url rows for the toolchain's packages (source zips;
// a `.rutbundle` url names the compiled wire) — the pins live
// in the app's rut.lock (`rut install` writes it)
{
  "name": "app",

  "deps": {
    "greet": { "path": "../greet" },
    "pouch": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v15/dist/std/pouch.zip" },
    "ink":   { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v15/dist/std/ink.zip" }
  }
}
```

The keys, all of them:

| key | meaning |
|---|---|
| `name` | the package's use-path name: bare `[a-zA-Z0-9_]+` only. A scoped or quoted spelling is a manifest error. |
| `entry.type` | the declaration surface: one `.d.rut` file — the ONLY entry key left. OPTIONAL on a host pkg: when absent, `src/mod.d.rut` is the root surface (lib parity with a body's `src/mod.rut`), and when spelled it wins — the path resolves under `src/`. Neither present is the loud no-entry error naming both spellings. On a lib pkg it is documentation surface (a surface-only dev state mounts as a declaration unit; `host fn` text there is refused) |
| `type` | the declared kind: `type = "host"` for a **host pkg** — a pure declaration surface whose `host fn`s the embedder binds at load ([Host fns and declaration files](host-fns.md)); the surface organizes as a module tree of `mod.d.rut` files under `src/`, rooted at the default root or the spelled `entry.type` ([below](#host-surface-trees)); `type = "lib"` (or absent) is the ordinary source package. The kind is never inferred — an `entry.type`-only manifest with no kind and no `src/mod.rut` is an error naming both fixes |
| `deps` | the transitively mounted dependencies — and the declaration table: a pkg may `use` exactly the heads its manifest names (`deps` ∪ `core`, plus `dev-deps` at the program root), so a head riding someone else's `deps` is mounted but not nameable ([Nameability](dependency-kinds.md#nameability)). One source per descriptor: `pkg = { path = "..." }` (relative to this manifest) or `pkg = { url = "https://…" }` (a remote artifact — a `.zip` source archive or a `.rutbundle` compiled bundle; the extension is the kind) — never both, never neither. Url rows carry **no pin**: the `sha256` key is retired from the grammar, and the pins live in the walk root's `rut.lock` beside this manifest — `rut install` writes it and every load verifies against it ([Dependency kinds](dependency-kinds.md), [The rut CLI](cli.md)). `optional` is rejected here. |
| `dev-deps` | mounted only while building/testing this pkg itself — never in a consumer's world. Descriptors are **`path`-only**: a dev row is directory-time metadata for the declarer's own build, nothing to fetch ([Dependency kinds](dependency-kinds.md)) |
| `format`, `format_version` | bundle keys — ignored by directory loading; `rut pack --mode bin` writes them into the packed manifest when this one omits them, and the source-zip pack drops them ([Module bundles](bundles.md)) |
| `style` | formatter knobs: `indent_width` (1–8, default 4), `max_width` (≥ 20, default 100). Schema-free at the manifest layer — unknown keys ride; malformed values are formatter errors, never compile errors. Resolution: the nearest ancestor manifest of the formatted file; no manifest → defaults. |

The **grammar is strict at the top level**: the keys above are all of
them, and any other top-level key refuses naming the key — `manifest:
unknown key 'whatever'`. There is no forward-compat lane: a typo is a
manifest error, never a silent ignore, and the retired keys refuse the
same way (`peer-deps` and `inline` are unknown keys now); `host_scope`
keeps its dedicated retirement refusal — the registration scope is
the package name.

The **body keys are repealed**: `entry.lib` and `entry.libs` refuse
loudly, each error naming the fix — the root module is `src/mod.rut`,
structure lives in `mod` directories under `src/`. (Inside a
*published* bundle the old keys still load — the reader's compat lane;
see [Module bundles](bundles.md).)

The manifest text is **JSONC** — `//` line comments, `/* */` block
comments, and trailing commas are all legal — parsed by `serde_json`
behind a syntax-stripping front stage: the comment and comma bytes
become spaces before the parser sees them, so a syntax error keeps the
parser's own wording under a `line N:` prefix that names the ORIGINAL
file's line. The value laws are unchanged. Value errors are
path-targeted (`deps::pouch: unknown key 'feats'`). Descriptors are
key/value objects; unknown descriptor keys are path-targeted manifest
errors. Duplicate keys are last-wins. An `_`-key refuses loudly naming
the fix — comments are the prose now.

## Resolution laws

- **Exact, single-step.** A use path resolves only if a module with
  that `name` is mounted **and the consuming manifest declares it** —
  nameability is declaration-scoped, so a mounted-but-undeclared head
  is the D5 diagnostic naming the mount edge and the fix row
  ([Dependency kinds](dependency-kinds.md)). Nothing is derived from
  directories or file layouts beyond the `mod` declarations the source
  itself spells.
- **The walk is recursive, with a cycle guard; first mount wins.** An
  already-mounted name (the embedder's, the root's, or an earlier
  dep's) is never overwritten — the consumer's own path for a package
  always beats a dep's transitive row.
- **Name mismatch is an error.** A dep whose manifest `name`
  disagrees with its key fails, naming both.
- **The cross-table law.** A name in `deps` beside `dev-deps` is an
  error naming both rows — a package is either pulled transitively or
  held for development, never both.
- **`core` needs no `deps`** — `.compile()` auto-offers it
  unconditionally ([core and the swappable packages](stdlib.md));
  every name still requires `use core::{ .. };` per
  [core and the swappable packages](stdlib.md).

## Module trees

A package's body is authored as a **file module tree** — the
`tur_kit`-style layout:

```
tur_kit/
├── rut.jsonc        # no entry keys — src/mod.rut IS the lib
└── src/
    ├── mod.rut      # the root module
    ├── layout/
    │   └── mod.rut  # tur_kit::layout
    └── widget/
        └── mod.rut  # tur_kit::widget
```

The root module declares its children:

```rut
// tur_kit/src/mod.rut
pub mod layout;
pub mod widget;
```

Mounting is **declared, not discovered**: `mod NAME;` (or `pub mod
NAME;`) in a file resolves against the sibling `NAME/mod.rut`
directory — or, when there is no such directory, the single-file
spelling `NAME.rut` beside it (both spellings under `src/`; both
present is a loud ambiguity naming both spellings; a single-file child
mounts at the logical mod path `NAME` exactly as the directory would).
The loader never reads a directory listing — an undeclared directory is
invisible, a declared child that is missing under both spellings (or
whose directory carries no `mod.rut`) is a loud error naming the
spellings. Mounting is recursive and cycle-guarded by file identity; a
repeated declaration mounts once.

Visibility is per module file ([Modules and
visibility](modules-and-visibility.md)): plain declarations are visible
in their own module and its descendants, `pub(pkg)` shares inside the
package, `pub` crosses packages. Cross-file references inside one
package are qualified positions (`layout::Column`, `widget::mk(3)` —
the `::` spelling); `use` statements stay the only cross-package door
(`use tur_kit::layout::{ Column }`).

### Host surface trees

A `type = "host"` package has no body — but its `.d.rut` surface
organizes as the same kind of tree. The **root surface** follows the
same discovery law a body's `src/mod.rut` does: `src/mod.d.rut` is the
default root (lib parity), and a spelled `entry.type` still wins,
resolved under `src/` — `calc.d.rut`, `engine.d.rut` and every other
flat host pkg keep their custom root names, moved under the shell.
Neither present is the loud no-entry
error naming both spellings. `mod NAME;` in the surface file mounts
`NAME/mod.d.rut` **beside the declaring file** — or the single-file
`NAME.rut` when there is no such directory — recursively and
cycle-guarded, exactly like a body tree with `mod.d.rut` as the module
file (both spellings present is the loud ambiguity):

```
http_host/
├── rut.jsonc           # type = "host" — no entry keys: src/mod.d.rut IS the root
└── src/
    ├── mod.d.rut       # the root surface: pub mod net;  pub mod stream;
    ├── net/
    │   └── mod.d.rut   # http_host::net — the request lane + readbacks
    └── stream/
        └── mod.d.rut   # http_host::stream — the body lanes
```

The tree is **declaration-lawed like everything else**: a declared
child with no `NAME/mod.d.rut` beside it (the whole tree under `src/`)
is a loud error naming the
declaring file, a `src/mod.rut` beside a host manifest still refuses (the
pkg is a pure surface — the message points at `mod.d.rut`), and the
`pub` on both the `mod` edge and the `host fn` row is what a
consumer's `use` crosses.

What the tree changes is the **registration naming**: a row under
`net/` registers as `pkg::net::name` (async families ride the
qualified name too — `pkg::net::send__start` et al), while the root's
own rows keep the flat `pkg::name` shape. Consumers spell the tree in
their uses (`use http_host::net::{ send }`), and the embedding Rust
registers the bodies scope-relatively (`"net::send"` under
`HostPkg::new("http_host")` — [embedding and native
modules](embedding.md)). Packing rides the same additive envelope a
body tree uses: the tree travels as `rut.mods` source rows inside the
bundle, and a FLAT host pkg packs byte-identically (no rows entry) —
[module bundles](bundles.md).

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
│   ├── rut-lsp/          # the language core: tokens, diagnostics,
│   │                     #   hover, completion, definition, check
│   ├── rut-lsp-server/   # the stdio face — the `rut-lsp` binary
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
bundle parsing (the dep tables behind the editor's dep walk). It is the
queries and the LSP values, never a transport: the two shells that
serve it are separate crates. `rut-lsp-server` is the stdio face
(tower-lsp-server + tokio) and ships the `rut-lsp` binary every
editor config spawns; `rut-lsp-wasm` is the in-process module the VS
Code extension and the playground bundle. Both call the same
`rut_lsp::analysis` entry points with the same session state shape, so
a feature lands on both faces or neither. The crate's classifier is
`rut-semantic` (re-exported as `rut_lsp::semantic`), which sits on the
frontend crates for the same reason — embedders take it without an
editor at all.

## The in-tree packages

`rut/` carries the standard and swappable packages the CLI offers on
demand:

| package | shape |
|---|---|
| `core` | the only standard package — the builtin surface, auto-offered by every run's `.compile()` |
| `calc` | the math surface as a normal lib — generic fns over `calc_host`; a `deps`-carrying dir mounts through `load_dir`/`dir_pkgs`, never `tree_pkg` ([stdlib](stdlib.md)) |
| `calc_host` | host pkg: the math rows + the `Math::` constants — offer `rut_native::tree_pkg("calc_host")`, bind `rut_std::math::pkg()` ([stdlib](stdlib.md)) |
| `ink_host`, `nmap_host`, `async_host`, `bench_cross` | host pkgs — flat `.d.rut` surfaces; bodies live in `rut-std` |
| `http_host` | host pkg, the surface-tree pilot — the default `src/mod.d.rut` root over the std HTTP rows, `net/` + `stream/` mods ([project structure](#host-surface-trees)); bodies in `rut-std` |
| `ink`, `http`, `strbuild`, `futures` | rut classes over host rows (published compiled — methods cross on the surface's inherent rows) |
| `pouch` | the growable sequence `Vec<T>` (plain linked package) |
| `flow` | the push pipeline (`Flow<E>`; its `VecFlow`/`SetFlow` wrappers over `pouch`/`nmapset`) |
| `json` | the serde pkg — owns the wrapper families over `pouch`/`nmapset`, plain `deps` ([Dependency kinds](dependency-kinds.md)) |
| `nmapset` | native-key maps/sets over `nmap_host` |

Nothing in the engine knows the swappable packages' names — deps are
resolved by name from manifests; a host may replace the non-`core`
set wholesale ([Embedding and native modules](embedding.md)).

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
