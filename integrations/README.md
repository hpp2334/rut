# rut editor support

One language server — `crates/rut-lsp` — serves every editor that speaks
LSP. Build it once:

```sh
cargo build -p rut-lsp --release
# binary: target/release/rut-lsp[.exe]
```

VS Code skips the server process entirely: its extension runs the same
language core as an in-process wasm module (`crates/rut-lsp-wasm` — see
[`vscode-extension/README.md`](vscode-extension/README.md)). Both faces
share one implementation, so features match.

What it provides (M1, the RFC 0001 M6 LSP slice landed early; the
`lsp-features` batch added the navigation + inference layer):

- **semantic tokens** — grammar highlighting: keywords, literals,
  primitives, and identifier classes (functions, methods, types, params,
  fields, enum members), including inside `f"…"` holes
- **diagnostics** — lexer + parser errors as you type (`.d.rut` files
  parse in declaration mode)
- **document symbols** — the outline: fns, enums, dataclasses, classes,
  traits, impl blocks
- **hover** — types on identifiers (written annotation or inferred),
  fields, methods, enum members, and primitives, at decl and use sites
- **completions** — member items through receiver inference; bare
  context = keywords + the std surface
- **go-to-definition / go-to-type-definition** — locals, params, fields,
  fns, types, and use-imported std names; across files through the use
  graph (ambiguity = a candidate list, never a wrong silent jump)
- **references** — a declaration's uses, shadow-aware within the file,
  cross-file through the use graph; member and local targets stay
  in-file (locals can't escape, imports carry type/fn names)
- **inlay hints** — inferred types on unannotated bindings and param
  names at exact-arity call sites (a mismatch shows nothing, never a
  wrong hint)
- **signature help** — the callee's verbatim signature with the active
  parameter highlighted, triggered at `(` and `,`

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
- Checker-level diagnostics (M5) and formatting are the remaining
  tooling-milestone items; navigation and inference are parse-level by
  design (the `lsp-features` batch report,
  `../docs/lsp-features-report.md`, records the honest limits).
