# rut-vscode

VS Code support for the [rut](../../README.md) language: grammar
highlighting (TextMate + `rut-lsp` semantic tokens), diagnostics, and the
document-symbol outline.

## Try it

```sh
cd integrations/vscode-extension
npm install
npm run build:server   # cargo build -p rut-lsp --release + copy into bin/
npm run compile        # esbuild bundle -> out/extension.js
```

Then open this folder in VS Code and press **F5** (*Run Extension*) — or
copy the folder into `~/.vscode/extensions`. Open any `.rut` file (try
`examples/basic/grammar-tour.rut`).

- Without the server binary the TextMate grammar still colors comments,
  strings, numbers, and keywords; an info message explains how to build
  the server.
- `rut.serverPath` overrides the bundled binary; `rut.trace.server`
  traces LSP traffic into the `rut` output channel.

## Packaging

```sh
npm run package        # vsce package -> rut-vscode-<version>.vsix
code --install-extension rut-vscode-<version>.vsix
```

## How highlighting splits

| Layer | What | Where |
|---|---|---|
| TextMate | comments, strings (incl. `r"…"`/`f"…"`), numbers + suffixes, keywords, operators — instant, no server | `syntaxes/rut.tmLanguage.json` |
| Semantic tokens | identifier classes: functions, methods, types, primitives, params, fields, enum members — exact, incl. f-string holes | `crates/rut-lsp` (`semantic.rs`) |

VS Code merges both: semantic tokens override the grammar inside their
ranges. The `semanticTokenScopes` contribution maps the legend onto theme
scopes so stock themes color everything out of the box.

## Other editors

The same server serves Neovim, Helix, Zed, Emacs, Sublime — config
snippets in [`../README.md`](../README.md).
