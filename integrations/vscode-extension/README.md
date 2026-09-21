# rut-vscode

VS Code support for the [rut](../../README.md) language: grammar
highlighting (TextMate + semantic tokens), diagnostics, the
document-symbol outline, hover, and completions — powered by the
language core running **as an in-process wasm module**. No server
process, no per-platform binaries: one `rut-lsp.wasm` serves every
platform.

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
diagnostics, symbols, hover, and completion providers directly against
`vscode.languages`. There is no JSON-RPC and no language-client
dependency — the module's results are already LSP values, serialized by
serde_json. The workspace is indexed with `workspace.findFiles` and
pushed into the module (`rut_add_def`), mirroring the native server's
fs walk.

The **native `rut-lsp` binary remains the face for other editors**
(Neovim / Helix / Zed / Emacs / Sublime — see
[`../README.md`](../README.md)); both faces run the same queries from
`crates/rut-lsp`, so they cannot drift.

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
npm test               # grammar corpus gate + Extension Host run
npm run test:grammar   # standalone TextMate gate: the grammar asserted over
                       # the repo corpus (rut/ + examples/ + demo/ +
                       # benches/workloads/) via vscode-textmate/oniguruma —
                       # no wasm, no VS Code needed (M1-M8 lock,
                       # docs/lsp-survey-extension.md §7)
npm run test:host      # the Extension Host run (needs `code` on PATH and
                       # bin/rut-lsp.wasm built)
```

The wasm module itself is gated by `crates/rut-lsp-wasm/smoke.js`
(`node crates/rut-lsp-wasm/smoke.js` from the repo root after
`npm run build:wasm`).
