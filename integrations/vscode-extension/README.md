# rut-vscode

VS Code support for the [rut](../../README.md) language: grammar
highlighting (TextMate + semantic tokens), diagnostics, the
document-symbol outline, hover, completions (with auto-import),
go-to-definition (plus type definition), references, inlay hints, and
signature help — powered by the language core running **as an
in-process wasm module**. No server process, no per-platform binaries:
one `rut-lsp.wasm` serves every platform. The extension also reads your
project's `rut.jsonc`: path deps mount from anywhere on disk, pinned
url deps fetch with their `sha256` verified, and the manifest gets
schema-validated hovers.

## Try it

```sh
cd integrations/vscode-extension
npm install
npm run build:wasm    # cargo build -p rut-lsp-wasm --target wasm32-unknown-unknown --release
npm run compile       # esbuild bundle -> out/extension.js
```

Then open this folder in VS Code and press **F5** (*Run Extension*) — or
copy the folder into `~/.vscode/extensions`. Open any `.rut` file.

- Without the wasm module the TextMate grammar still colors comments,
  strings, numbers, and keywords; an info message explains how to build
  it (`npm run build:wasm`).

## How it runs

`crates/rut-lsp-wasm` exposes the language core over a raw wasm ABI (the
same envelope pattern as `rut-wasm` — no wasm-bindgen). `src/wasm.ts`
binds that ABI; `src/extension.ts` registers semantic tokens,
diagnostics, symbols, hover, completion, definition, type-definition,
references, inlay-hint, and signature-help providers directly against
`vscode.languages`. There is no JSON-RPC and no language-client
dependency — the module's results are already LSP values, serialized by
serde_json. The workspace is indexed with `workspace.findFiles` and
pushed into the module (`rut_add_def`), mirroring the native server's
fs walk.

The **native `rut-lsp` binary remains the face for other editors**
(Neovim / Helix / Zed / Emacs / Sublime — see
[`../README.md`](../README.md)); both faces run the same queries from
`crates/rut-lsp`, so they cannot drift.

## File modules — mod.rut

A rut package is a module tree: `mod.rut` beside the manifest is the
root module, and each `mod NAME;` it declares mounts `NAME/mod.rut`
beside it (`pub mod` marks the edge visible across packages). The
extension rides the same model:

- The **workspace scan** stamps every `mod.rut` under a folder-level
  package with its mod path (the wasm indexes it via `rut_add_def_mod`)
  — a nested package's own manifest keeps it out of the parent's tree.
- The **dep walk** mounts each path dep's tree from its root module's
  declarations (the wasm parses; the walk only reads the files the
  decls name), so hover/completion see mod-carrying deps.
- **Completion** walks the tree: after `pkg::` the dep's `pub mod`
  children ride along (never a bare `mod`), deeper `::` segments walk
  the pub edges, and — inside a package's own file — `modname::` /
  `modname.` offers that module's members by visibility tier
  (`pub`/`pub(pkg)`/`pub(super)`/private, the compiler's predicate).
- **Hover** renders `pub? mod NAME;` decls (kind, mounted file, child
  count) and resolves use-path segments to their mounted modules.

## Deps — the extension reads rut.jsonc

After the workspace scan the extension hands each workspace folder's
root `rut.jsonc` to the wasm (`rut_parse_manifest` — the wasm parses
manifests, JS only moves bytes) and walks its dependency rows:

- **Path deps** (`"gadgets": { "path": "../gadgets" }`) resolve
  against the manifest's directory — **a dep outside the workspace
  folder mounts like any other** (the real gap a plain workspace scan
  can never close). The walk recurses transitively, cycle-safe, under
  a depth/file budget; every indexed source keeps its real on-disk
  URI, so F12 jumps land in the dep's file.
- **Url deps** (`"pouch": { "url": "…", "sha256": "…" }`) are
  **cache-first**: the entry under `.rut/cache/` (the same directory
  the CLI fills — the cache filename is the Rust-computed
  `sha256` of the url) is read before any network, and a hit is
  verified against the pin. On a miss the extension fetches
  in-extension, hashes the bytes (`crypto.webcrypto`), and mounts them
  only when the hash matches; the entry then lands in the cache
  atomically (tmp + rename). A poisoned entry evicts and re-fetches,
  like the CLI's healing loop.
- **The pin is law at the mount door**: fetched or cached, bytes mount
  only with their `sha256` verified. An unpinned url row never mounts
  from the editor — one error hint, no silent unpinned bytes.
- **File modules mount like the loader runs**: a dep package's root
  module (`mod.rut` beside the manifest; the entry lib while the
  transitional key stands) mounts its `mod NAME;` tree recursively —
  `NAME/mod.rut` beside the declaring file — so `use
  gadgets::layout::{ Column }` completes, hovers its segments
  (`layout` shows the mounted file and child count), and jumps; a
  `pub? mod NAME;` decl hovers its kind, mounted file, and child
  count; and after `modname::` / `modname.` in a package's own file
  the module's members complete by visibility tier. The wasm parses
  every file (`rut_add_def_mod` answers the declared children) — JS
  only moves bytes. `**/mod.rut` watchers re-mount a moved tree.
- **Offline / failure** shows one info hint per dep naming the CLI
  alternative (`rut fetch` warms the cache) — never spam, never silent
  wrong results.
- The manifest and its dep tables re-walk on change
  (`**/rut.jsonc` and `.rut/cache/**` watchers, debounced), and a
  workspace-folder change restarts the whole index.
- **One engine, two faces** — the native server is deliberately
  cache-only for url deps (a miss is a `window/showMessage` hint); the
  extension is the face that fetches. Same dep tables
  (`crates/rut-lsp/src/deps.rs`), no drift.
- Auto-import completions offer foreign public names with the
  `use mod::Name;` insert (`additionalTextEdits`) and sort after the
  locals (`sortText`) — accepting an item never ungates members, the
  engine's law; the extension only maps the edits through.
- `rut.jsonc` is schema-validated (`schemas/rut.manifest.schema.json`,
  contributed via `jsonValidation`): every field's `description` is the
  hover in the jsonc editor.

## Packaging

```sh
npm run package        # vsce package -> rut-vscode-<version>.vsix
code --install-extension rut-vscode-<version>.vsix
```

## How highlighting splits

| Layer | What | Where |
|---|---|---|
| TextMate | comments, strings (incl. `r"…"`/`f"…"`), numbers + suffixes, keywords (incl. contextual `type`/`builtin`), primitives (`str`/`bytes`), `opaque`, nullable `?`, operators — instant, no analysis | `syntaxes/rut.tmLanguage.json` |
| Semantic tokens | identifier classes: functions, methods, types, primitives, params, fields, enum members — exact, incl. f-string holes | `crates/rut-lsp` (`semantic/`) |

VS Code merges both: semantic tokens override the grammar inside their
ranges. The `semanticTokenScopes` contribution maps the legend onto theme
scopes so stock themes color everything out of the box.

## Tests

```sh
npm test               # grammar corpus gate + through-wasm e2e gate + Extension Host run
npm run test:grammar   # standalone TextMate gate: the grammar asserted over
                       # the repo corpus (rut/ + examples/ + demo/ +
                       # benches/workloads/) via vscode-textmate/oniguruma —
                       # no wasm, no VS Code needed (M1-M8 lock)
npm run test:e2e       # through-wasm e2e gate: the SHIPPED bin/rut-lsp.wasm
                       # driven through the extension's own ABI binding
                       # (src/wasm.ts, bundled to out/wasm.js) over the full
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
                       # help). Needs bin/rut-lsp.wasm (npm run
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
the folder boundary, plus a url dep served by a localhost fixture
server — the pin refusal, the pinned fetch + atomic cache write, the
auto-import insert, and the offline hint all assert there (needs
`code`; the lane skips loudly without it).

The wasm module itself is gated by `crates/rut-lsp-wasm/smoke.js`
(`node crates/rut-lsp-wasm/smoke.js` from the repo root after
`npm run build:wasm`).
