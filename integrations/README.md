# rut editor support

The language core ships as ONE crate — `crates/rut-lsp-wasm`: the
queries (semantic tokens, diagnostics, hover, completion, definition,
references, inlay hints, signature help, the check lane) and the
wasm-bindgen face that serves them. The VS Code extension runs it
in-process — no server process, no per-platform binaries; the
playground bundles the same module. The extension's dep lane is the
crate's too: it injects a read-only filesystem capability (the
`HostEnv` the crate's walk reads through) and stirs the crate-shipped
engine — the editor NEVER fetches and NEVER writes; url deps mount
from the global dep store (`$RUT_HOME`, `rut install`'s address), and
a cold store is one hint naming `rut install` (the rust-analyzer
contract). See
[`vscode-extension/README.md`](vscode-extension/README.md).

There is no first-party stdio server today: an earlier native face
(`rut-lsp-server`, the `rut-lsp` binary over tower-lsp-server) was
dropped — its code lives in git history if a stdio wrapper is ever
wanted again. The queries stay embeddable without a browser: the crate
links natively too (`cargo test -p rut-lsp-wasm` runs its whole suite
natively), so any future face is a thin shell over
`rut_lsp_wasm::analysis`.

What the editor face provides (the LSP slice landed early; the
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
    by visibility tier; after `Type::` — enum members, a type's
    statics, a type's associated fns
  - auto-import — foreign public names from the document's DIRECT
    deps (the core prelude included) matching the bare prefix, each
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
  and a `mod.rut` body answers from the body; url deps answer in the
  global dep store's real files — the crate's read-only walk mounts
  them from `$RUT_HOME`, the address `rut install` publishes to).
  Ambiguity = a candidate list, never a wrong silent jump
- **references** — a declaration's uses, shadow-aware within the file,
  cross-file through the use graph; member and local targets stay
  in-file (locals can't escape, imports carry type/fn names)
- **inlay hints** — inferred types on unannotated bindings and param
  names at exact-arity call sites (a mismatch shows nothing, never a
  wrong hint)
- **signature help** — the callee's verbatim signature with the active
  parameter highlighted, triggered at `(` and `,`

## VS Code

See [`vscode-extension/`](vscode-extension/) — TextMate grammar + the
bundled wasm module + semantic token scope mappings.

## highlight.js (browsers & the book)

See [`rut-highlightjs/`](rut-highlightjs/) — a zero-dependency
highlight.js language definition. One grammar file (`src/grammar.js`)
drives code coloring on any highlight.js-based site and in the book
(wired through `docs/book.toml`, with a static-span bake for the built
HTML). Same derivation law as everywhere else: the lexer/parser tables
are the truth.

## Notes

- TextMate grammar for basic coloring (comments, strings, numbers,
  keywords) lives in `vscode-extension/syntaxes/rut.tmLanguage.json` —
  editors that load TextMate grammars can reuse it directly.
- `rut-highlightjs/register.js` is generated from the package's
  `src/grammar.js` (`node scripts/build.mjs`) and committed dist-style;
  the book consumes a byte-copy shim at `docs/theme/rut-highlight.js`.
- Checker-level diagnostics ride the editor's check lane for packaged
  documents (the compiler's checker behind
  `crates/rut-lsp-wasm/src/query/check.rs`); hover/completion stay
  index-level by design. Formatting is the remaining
  tooling-milestone item.
