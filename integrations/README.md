# rut editor support

One language server — `crates/rut-lsp` — serves every editor that speaks
LSP. Build it once:

```sh
cargo build -p rut-lsp --release
# binary: target/release/rut-lsp[.exe]
```

What it provides (M1, the RFC 0001 M6 LSP slice landed early):

- **semantic tokens** — grammar highlighting: keywords, literals,
  primitives, and identifier classes (functions, methods, types, params,
  fields, enum members), including inside `f"…"` holes
- **diagnostics** — lexer + parser errors as you type (`.d.rut` files
  parse in declaration mode)
- **document symbols** — the outline: fns, enums, dataclasses, classes,
  traits, impl blocks

## VS Code

See [`vscode-extension/`](vscode-extension/) — TextMate grammar + bundled
server + semantic token scope mappings.

## Neovim (0.11+)

```lua
vim.lsp.config['rut'] = {
  cmd = { '/path/to/rut/target/release/rut-lsp' },
  filetypes = { 'rut' },
  root_markers = { '.git' },
}
```

(older releases: `require('lspconfig').rut_lsp.setup { cmd = { … } }`
via a custom server definition.)

## Helix

`~/.config/helix/languages.toml`:

```toml
[[language]]
name = "rut"
scope = "source.rut"
file-types = ["rut"]
language-servers = ["rut-lsp"]

[language-server.rut-lsp]
command = "/path/to/rut/target/release/rut-lsp"
```

## Zed

Add a `rut` Zed extension wrapping the server binary (its
`language-server` setting points at `rut-lsp`), or use the LSP-pass-through
settings until one ships.

## Emacs (eglot)

```elisp
(with-eval-after-load 'eglot
  (add-to-list 'eglot-server-programs
               '(rut-mode . ("/path/to/rut/target/release/rut-lsp"))))
```

## Sublime Text (LSP package)

`.sublime-project` settings:

```json
{
  "settings": {
    "LSP": {
      "rut": {
        "enabled": true,
        "command": ["/path/to/rut/target/release/rut-lsp"]
      }
    }
  }
}
```

## Notes

- TextMate grammar for basic coloring (comments, strings, numbers,
  keywords) lives in `vscode-extension/syntaxes/rut.tmLanguage.json` —
  editors that load TextMate grammars can reuse it directly.
- Hover, completion, go-to and formatting arrive with the M6 tooling
  milestone; semantic-token classes are token-level + light-AST until M2
  module resolution lands.
