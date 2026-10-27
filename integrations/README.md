# rut editor support

One language server serves every editor that speaks LSP: the queries
are `crates/rut-lsp` (the core), the stdio face is
`crates/rut-lsp-server`. Build the binary once:

```sh
cargo build -p rut-lsp-server --release
# binary: target/release/rut-lsp[.exe]  (package rut-lsp-server, binary rut-lsp)
```

VS Code skips the server process entirely: its extension runs the same
language core as an in-process wasm module (`crates/rut-lsp-wasm` — see
[`vscode-extension/README.md`](vscode-extension/README.md)). Both faces
share one implementation, so features match.

What it provides (the LSP slice landed early; the
`lsp-features` batch added the navigation + inference layer):

- **semantic tokens** — grammar highlighting: keywords, literals,
  primitives, and identifier classes (functions, methods, types, params,
  fields, enum members), including inside `f"…"` holes
- **diagnostics** — lexer + parser errors as you type (`.d.rut` files
  parse in declaration mode); when the document's package graph
  assembles (a `mod.rut` under a folder-level package, or an opened
  dep file), the compiler's checker joins the lane and type errors —
  assignment mismatches, missing record fields — publish at their
  sites, mapped per file through the graph's stamps (parse-only for
  stray files: never worse)
- **document symbols** — the outline: fns, enums, structs, classes,
  traits, impl blocks
- **hover** — types on identifiers (written annotation or inferred),
  fields, methods, enum members, and primitives, at decl and use sites
- **completions** — tiered by context, each honest about what the
  compiler would bind:
  - after `recv.` — the receiver type's fields and methods (through
    receiver inference: field reads, chained calls, constructor
    bindings, loop variables)
  - at a bare position — the fn-local bindings (shadow- and
    scope-correct, LSP variable/parameter kinds) ahead of the keyword
    table, then the document's own decls, the ambient `prelude
    builtin` rows, and the names the document already `use`s; every
    other foreign name rides the auto-import tier below, never the
    plain list
  - at an annotation boundary (`let x: ⏐`, `id: u⏐`, `-> T⏐`) — the
    type tier: the primitive table plus the document's types and the
    imported foreign types; no keywords, fns, or locals
  - in a `use` — the chain's module names (`use ⏐`), a walked
    package's public exports (`use pkg::mod::⏐`, the `{ A, B }` brace
    form included, trigger key `{`)
  - after `modname::` in a package's own file — the module's members
    by visibility tier; after `Type::` — enum members, namespace
    statics, a type's associated fns
  - auto-import — foreign public names from the document's DIRECT
    deps (and the std surface) matching the bare prefix, each
    carrying the `use mod::Name;` insert (`additionalTextEdits`) and
    sorted after the locals (`sortText`); a transitive-only dep never
    offers (the compiler refuses `use` of a mounted-but-undeclared
    package), already-imported names are never re-offered, and one
    name reachable two ways offers exactly one item
- **go-to-definition / go-to-type-definition** — locals, params, fields,
  fns, types, and use-imported names through the use graph, into dep
  sources included: every segment of a use path is clickable — the pkg
  segment opens the dep's root module, a middle segment its `pub mod`
  edge, the leaf its declaration (a dep with both a `.d.rut` surface
  and a `mod.rut` body answers from the body; cached url deps answer
  into their `bundle:` origin — the VS Code extension is the face that
  extracts them to real files). Ambiguity = a candidate list, never a
  wrong silent jump
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

## highlight.js (browsers & the book)

See [`rut-highlightjs/`](rut-highlightjs/) — a zero-dependency
highlight.js language definition. One grammar file (`src/grammar.js`)
drives code coloring on any highlight.js-based site and in the book
(wired through `docs/book.toml`, with a static-span bake for the built
HTML). Same derivation law as everywhere else: the lexer/parser tables
are the truth.

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
- `rut-highlightjs/register.js` is generated from the package's
  `src/grammar.js` (`node scripts/build.mjs`) and committed dist-style;
  the book consumes a byte-copy shim at `docs/theme/rut-highlight.js`.
- Checker-level diagnostics now ride the editor's check lane for
  packaged documents (the compiler's checker behind
  `crates/rut-lsp/src/check.rs`); hover/completion stay index-level by
  design. Formatting is the remaining tooling-milestone item.
