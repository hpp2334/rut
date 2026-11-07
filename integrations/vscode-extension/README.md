# rut-vscode

VS Code support for the [rut](../../README.md) language: grammar
highlighting (TextMate + semantic tokens), diagnostics, the
document-symbol outline, hover, completions (with auto-import),
go-to-definition (plus type definition), references, inlay hints, and
signature help — powered by the language core running **as an
in-process wasm module**. No server process, no per-platform binaries:
one `rut-lsp.wasm` serves every platform. The extension also injects a
read-only filesystem capability so the crate's own dep walk can read
your project's `rut.jsonc`: path deps mount from anywhere on disk and
url deps mount from the global dep store (`rut install`'s) — the
editor itself NEVER fetches and NEVER writes — and the manifest gets
schema-validated hovers.

## Try it

```sh
cd integrations/vscode-extension
npm install
npm run build:wasm    # wasm-pack build crates/rut-lsp-wasm, then assemble
                      # the package (JS face + .d.ts) into crates/rut-lsp-wasm/pkg/
npm run compile       # esbuild bundle -> out/extension.js
```

Then open this folder in VS Code and press **F5** (_Run Extension_) — or
copy the folder into `~/.vscode/extensions`. Open any `.rut` file.

- Without the wasm module the TextMate grammar still colors comments,
  strings, numbers, and keywords; an info message explains how to build
  it (`npm run build:wasm`).

## How it runs

`crates/rut-lsp-wasm` exposes the language core as a wasm-bindgen
module (the `RutLsp` class — the `rut-wasm` engine pattern). The crate
owns the whole ABI face: `js/index.js` parses the envelopes and
`js/index.d.ts` declares every shape, assembled into the wasm-pack
package (`pkg/index.js` + `pkg/index.d.ts`) by `build:wasm`. The
extension imports that package and declares **no** LSP shapes of its
own — `src/wasm.ts` is a re-export, and `src/extension.ts` only maps
the values onto `vscode.*` while registering semantic tokens,
diagnostics, symbols, hover, completion, definition, type-definition,
references, inlay-hint, and signature-help providers against
`vscode.languages`. There is
no JSON-RPC and no language-client dependency — the module's results
are already LSP values, serialized by serde_json. The workspace is
indexed with `workspace.findFiles` and pushed into the module
(`add_def`) — the same scan the CLI's own lanes bound.

The dep walk lives in the crate too, behind the actor ABI: the
extension injects a **read-only host environment** (`src/env.ts` —
`read`/`readDir`/`isDir`/`notify` over `node:fs` sync calls; no write,
no delete, no fetch exists on the capability) and stirs the
crate-shipped **dep engine** (`depEngine` — debounce + one-run-in-
flight + rerun-once over `resolveDeps`). The engine walks every
workspace root per run: the root `rut.lock` feeds the pin ledger and
the root `rut.jsonc`'s rows mount — path deps from their directories,
url deps from the global dep store (`$RUT_HOME/deps/<name>@<sha256[:16]>`,
the same address `rut install` publishes to) when its `meta.json`
vouches the lock pin. A cold or drifted store is ONE hint per dep
naming `rut install` (the rust-analyzer contract); the extension's
whole dep lane is watchers + `stir()`.

The module IS the language core: the queries (`analysis`, `hover`,
`completion`, `definition`, `check`, …) live in the same crate as this
ABI (`crates/rut-lsp-wasm`), so a feature lands on the editor face as
soon as it lands on the engine — there is no second face to keep in
step. Other editors have no first-party face today (see
[`../README.md`](../README.md)).

## File modules — src/mod.rut

A rut package is a module tree: `src/mod.rut` (under the package's
`src/` shell) is the root module, and each `mod NAME;` it declares
mounts `NAME/mod.rut`
beside it — or the single-file `NAME.rut` when there is no such
directory (both spellings is the loader's loud ambiguity, mirrored as
a one-time hint) (`pub mod` marks the edge visible across packages).
The extension rides the same model:

- The **workspace scan** stamps every `mod.rut` under a folder-level
  package's `src/` shell with its mod path (the wasm indexes it via
  `rut_add_def_mod`)
  — a nested package's own manifest keeps it out of the parent's tree —
  then follows each stamped file's `mod` decls to stamp the
  single-file `NAME.rut` children at their child paths.
- The **dep walk** (in the crate, over the injected env) mounts each
  path dep's tree from its root module's declarations (the wasm
  parses; the walk only reads the files the decls name, probing
  `NAME/mod.rut` then `NAME.rut` per edge, inside the package's
  `src/` shell), so
  hover/completion see mod-carrying deps.
- **Completion** walks the tree: after `pkg::` the dep's `pub mod`
  children ride along (never a bare `mod`), deeper `::` segments walk
  the pub edges, and — inside a package's own file — `modname::` /
  `modname.` offers that module's members by visibility tier
  (`pub`/`pub(pkg)`/`pub(super)`/private, the compiler's predicate).
- **Hover** renders `pub? mod NAME;` decls (kind, mounted file, child
  count) and resolves use-path segments to their mounted modules.

## Semantic diagnostics — the check lane

Diagnostics are two lanes merged, one engine
(`crates/rut-lsp-wasm/src/query/check.rs`):

- **Parse lane** (always): the lexer + parser diags for the open
  document — a syntax error squiggles the moment it is written.
- **Check lane** (when a package graph assembles): the compiler's own
  checker runs over the document's package and its dep closure, so
  type errors surface in the editor — `checked = true;` (bool into a
  `str` binding), a record literal missing a field, an arity mistake —
  at the same site `rut run` reports them. The document's package is
  known from its stamps: a `mod.rut` under a folder-level package's
  `src/` shell (the
  workspace scan) or a dep file opened for editing (the dep walk).
  Diags map back through the graph's file stamps — each squiggle
  lands in the file that owns it, and a span that names no open
  document never lights up anywhere. The merge with the parse set
  dedupes true duplicates only; a distinct site is never collapsed.

No package context — a stray file under no manifest, a non-`mod.rut`
file outside a package's `src/` shell — keeps today's parse-only
diagnostics. The check lane never
makes an editor worse than the compiler-less lane it extends.

## Deps — the crate's read-only walk, the editor's store

The dep lane is the crate's, not the extension's: after the workspace
scan the extension stirs the crate-shipped dep engine, and one
`resolveDeps(roots)` run walks every workspace folder — the wasm
parses the root `rut.jsonc` and its whole dep closure over the
injected read-only env, mounting everything into the same defs store
the scan fills. The extension NEVER fetches and NEVER writes: the
capability it injects (`src/env.ts` — sync `read`/`readDir`/`isDir`
plus a `notify` hint lane) has no write, no delete, and no fetch on
it, by construction.

- **Path deps** (`"gadgets": { "path": "../gadgets" }`) resolve
  against the manifest's directory — **a dep outside the workspace
  folder mounts like any other** (the real gap a plain workspace scan
  can never close). The walk recurses transitively, cycle-safe, under
  the crate's depth/file budgets; every indexed source keeps its real
  on-disk path, so F12 jumps land in the dep's file.
- **Url deps** (`"pouch": { "url": "…" }`) mount from the **global
  dep store** — `$RUT_HOME/deps/<name>@<sha256[:16]>/`, the same
  address `rut install` publishes to (the wasm sandbox cannot read
  env vars, so its store spelling arrives relative and the env adapter
  anchors it at the real `$RUT_HOME`/`~/.rut`). A dir whose
  `meta.json` vouches the `rut.lock` pin mounts through the same dir
  lanes path deps use — its `unpack/` tree, embedded
  `deps/<identity>/` groups included, so `ink` brings its host surface
  `ink_host` along with zero manifest rows of your own. First mount
  wins: a name your manifest (or an earlier group) already mounted
  never re-mounts from a ride-along.
- **The pin is law at the mount door**: the pin's only source is the
  walk root's `rut.lock`, and a reader never re-hashes the artifact —
  the store dir's `meta.json` (written at install time, the only hash
  pass) must vouch the pin. Absent, unpinned, or drifted is ONE hint
  per dep naming `rut install` (the rust-analyzer contract: the
  editor points at the tool that owns the fix), plus a `missing` row
  in the walk's answer. No silent wrong mount, no network, ever.
- **File modules mount like the loader runs**: a dep package's root
  module (`src/mod.rut`; the entry lib while the
  transitional key stands) mounts its `mod NAME;` tree recursively —
  `NAME/mod.rut` beside the declaring file, inside the package's
  `src/` shell — so `use
  gadgets::layout::{ Column }` completes, hovers its segments
  (`layout` shows the mounted file and child count), and jumps; a
  `pub? mod NAME;` decl hovers its kind, mounted file, and child
  count; and after `modname::` / `modname.` in a package's own file
  the module's members complete by visibility tier. The wasm parses
  every file (`rut_add_def_mod` answers the declared children) — the
  env only moves bytes. `**/mod.rut` watchers re-mount a moved tree.
- **Ctrl+click walks the whole use path into the dep** — every
  segment of `use A::B::…::Z` lands in real files: path deps in their
  on-disk sources, url deps in the store's `unpack/` tree under
  `$RUT_HOME` — indexed through the same lane path deps use, over
  `file://` uris — no virtual `bundle:` documents. The pkg segment
  opens the dep's root module at the top (the unpacked root
  `mod.rut`, or the `.d.rut` surface when that is all there is), a
  middle segment lands on its `pub mod` edge, the leaf on its
  declaration (the body file, when a package ships both a `.d.rut`
  surface and a `mod.rut` body) — `pouch` → the unpacked root,
  `Vec` → the `pub class Vec` line; `layout` → `pub mod layout;`,
  `Column` → the child file's struct.
  Own-package paths ride the same lanes keyed on the doc's place in
  its package: `use app::…` (the package's own name) opens the root
  module, `use helpers::…` (an own `mod` edge) opens the module's
  file, interior segments land on their mounting edges, and the leaf
  jumps through the intra-package walk — with `use imp::⏐` offering
  the module's members by visibility tier. A `pub? mod NAME;` decl is
  clickable too: ctrl+click opens the mounted child (`NAME/mod.rut`,
  or the file-form `NAME.rut`) at its top.
- **No project-local scratch**: the walk never writes beside your
  project (the old `.rut/deps/` extraction trees are gone; the
  editor-side `.rut/` writer no longer exists). What `rut run` may
  leave under `.rut/unpacked/` is the CLI's own scratch — keep `.rut/`
  in your project's `.gitignore`.
- The manifest and its dep tables re-walk on change
  (`**/rut.jsonc` and `**/mod.rut` watchers, debounced by the engine),
  a workspace-folder change restarts the whole index, and the
  `rut.reindexDeps` palette command stirs the engine on demand — the
  extension's whole dep lane is watchers + `stir()`.
- **One engine, one face** — the dep tables
  (`crates/rut-lsp-wasm/src/deps.rs`) are the CLI's own parsers and
  the walk (`crates/rut-lsp-wasm/src/walk.rs`) is the crate's; the
  extension injects the read-only capability and surfaces the hints,
  nothing more.
- Auto-import completions offer foreign public names with the
  `use mod::Name;` insert (`additionalTextEdits`) and sort after the
  locals (`sortText`) — accepting an item never ungates members, the
  engine's law; the extension only maps the edits through. The tier
  is gated to the open document's DIRECT deps: the walk stamps the
  root manifest's rows into the core when it parses them, and a
  transitive-only dep never offers — the compiler's
  closure check refuses `use` of a mounted-but-undeclared package,
  so completing one would write a D5 error. The std surface rides
  every chain as the ambient dep lane. The bare list itself stays
  honest: the document's own decls, the ambient `prelude builtin`
  rows, and the names the document already `use`s — everything else
  lives on this tier only. Annotation positions (`let x: ⏐`,
  `id: u⏐`, `-> T⏐`) get the type tier instead — primitives +
  document/imported types, no keywords, fns, or locals — and `{` is
  a trigger key for the brace form of a use (`use pouch::{ ⏐`).
- `rut.jsonc` is schema-validated (`schemas/rut.manifest.schema.json`,
  contributed via `jsonValidation`): every field's `description` is the
  hover in the jsonc editor.

## Packaging

```sh
npm run release        # the one lane: preflight → build:wasm (wasm-pack)
                       # → gates (grammar corpus + through-wasm e2e + the
                       # skip-tolerant host suite) → vsce package →
                       # byte-verify the wasm inside the vsix
npm run package        # just compile + vsce package (no gates)
code --install-extension rut-vscode-<version>.vsix
```

`release` packs whatever `package.json` says — it never bumps the
version and never touches git; a bump is a deliberate act:
`npm version X.Y.Z --no-git-tag-version` first. It warns before
overwriting an existing vsix, byte-verifies that the wasm inside the
package is the gated `bin/rut-lsp.wasm`, and reports older
`rut-vscode-*.vsix` files without deleting them.
`npm run release -- --dry-run` runs the build and the gates, then stops
before packing.

## How highlighting splits

| Layer           | What                                                                                                                                                                                                                                                                                                                   | Where                          |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------ |
| TextMate        | comments, strings (incl. `r"…"`, `f"…"`, and the hash-leveled multi-line forms `r#"…"#` / `f#"…"#` — the `#` run sets the level via a begin/end backreference), numbers + suffixes, keywords (incl. contextual `type`/`builtin`), primitives (`str`/`bytes`), `opaque`, nullable `?`, operators — instant, no analysis | `syntaxes/rut.tmLanguage.json` |
| Semantic tokens | identifier classes: functions, methods, types, primitives, params, fields, enum members — exact, incl. f-string holes                                                                                                                                                                                                  | `crates/rut-lsp` (`semantic/`) |

VS Code merges both: semantic tokens override the grammar inside their
ranges. The `semanticTokenScopes` contribution maps the legend onto theme
scopes so stock themes color everything out of the box.

## Tests

```sh
npm test               # biome (format + lint) + typecheck + grammar corpus +
                       # through-wasm e2e + Extension Host run
npm run lint           # biome check: 4-space formatting + the recommended
                       # lint set (biome.json; syntaxes/ and schemas/ excluded)
npm run format         # biome check --write
npm run typecheck      # tsc --noEmit: the hosts against the crate's .d.ts
npm run test:grammar   # standalone TextMate gate: the grammar asserted over
                       # the repo corpus (rut/ + examples/ + demo/ +
                       # benches/workloads/) via vscode-textmate/oniguruma —
                       # no wasm, no VS Code needed (M1-M8 lock)
npm run test:e2e       # through-wasm e2e gate: the SHIPPED bin/rut-lsp.wasm
                       # driven through the extension's own ABI binding
                       # (the crate's js face, bundled to out/wasm.js) over the full
                       # corpus — zero false diagnostics — plus per-feature
                       # smokes through the artifact: hover/completion
                       # (?T alias, ?Circle member resolution, bare
                       # nmapset/nmap_host completion, the
                       # by-reference-and-nullable diagnostic),
                       # definition + typeDefinition
                       # (within-file / cross-file / stdlib), inlay type +
                       # param hints, references (shadow-aware + the
                       # cross-file reverse edge), signature help
                       # (verbatim signature, active slot, mismatch -> no
                       # help), the dep clicks — a url dep's
                       # store-mounted leaf + pkg head, a path dep's
                       # `pub mod` middle segment + child-file leaf — and
                       # the check lane: a stamped doc's package compiles
                       # with its dep closure and the checker's type
                       # errors publish (assignment mismatch + missing
                       # field, each on its own line), while an unstamped
                       # doc stays parse-only clean; the wire-shape block
                       # asserts every envelope's key set against the
                       # crate's index.d.ts (the runtime half of the ABI
                       # contract that tsc cannot see).
                       # Needs bin/rut-lsp.wasm (npm run
                       # build:wasm); runs headless in plain node
npm run test:host      # the Extension Host run (needs `code` on PATH and
                       # bin/rut-lsp.wasm built); SKIPS LOUDLY (exit 0) on
                       # machines without VS Code — a skip is a skip, the
                       # two other gates still ran
```

`test/fixtures/symbols/` is a current-grammar sample module dir (a
[struct](../../docs/src/reference/structs.md), a [type
alias](../../docs/src/reference/type-aliases.md) + `requires` bound,
a [nullable](../../docs/src/reference/by-reference-and-nullable.md) `?T`,
primitive `str`/`bytes`, an
[opaque](../../docs/src/reference/opaque.md)): the e2e gate analyzes it
through the shipped wasm and hovers its alias; the host suite reads the
same symbols and semantic tokens on machines with `code`.

`test/fixtures/dep-walk/` is the manifest-deps fixture the host suite
renders: a workspace whose path dep (`outside/gadgets`) lives OUTSIDE
the folder boundary, plus a url dep pinned by the rendered `rut.lock`
— the cold-store refusal (one hint naming `rut install`), the
store-hit mount (a dep dir published under a temp `$RUT_HOME`, its
meta vouching the lock pin), and the auto-import insert all assert
there (needs `code`; the lane skips loudly without it).

The wasm module itself is gated by `crates/rut-lsp-wasm/smoke.js`
(`node crates/rut-lsp-wasm/smoke.js` from the repo root after
`npm run build:wasm`).
