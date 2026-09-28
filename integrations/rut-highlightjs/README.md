# rut-highlightjs

[highlight.js](https://highlight.js.org) language definition for the
[rut](../../README.md) language: keywords (reserved + contextual),
primitive types, suffixed number literals (`10u8`, `1.5f32`, `0xFFu64`),
plain strings with escapes, raw strings `r"…"`, format strings
`f"…{expr}…"` with the `{hole}` expression highlighted, `//` and
`/* … */` comments, and use-path / member access. Zero dependencies, no
build step, no devDependencies.

## Use it

ESM / bundler:

```js
import hljs from 'highlight.js';
import rut from 'rut-highlightjs';

hljs.registerLanguage('rut', rut);
```

Plain `<script>` tag — use the committed `register.js` (the grammar
spliced into a small harness, so no bundler is needed):

```html
<script src="…/highlight.min.js"></script>
<script src="…/register.js"></script>
```

`register.js` registers the grammar and highlights every
`pre code.language-rut` block. If no global `hljs` is on the page it
does not fail silently: it exposes `window.rutHljsGrammar` and warns on
the console, naming the fix (`hljs.registerLanguage('rut',
window.rutHljsGrammar)` — with highlight.js loaded before this script).

Both entries derive from ONE definition — `src/grammar.js` (pure ESM,
pure data: no imports, no hljs helpers). `register.js` is generated
from it and committed (dist-style, like the vscode extension commits
its packaged artifact):

```sh
cd integrations/rut-highlightjs
node scripts/build.mjs     # regen register.js — commit the result
```

## In the rut book

`docs/book.toml` wires the book to the grammar through
`docs/theme/rut-highlight.js` — a byte-copy of the generated
`register.js` (mdbook 0.5 does not copy cross-tree `additional-js` into
the build, so a committed shim inside `docs/` is the least machinery
that survives deployment). Token colors live in
`docs/theme/rut-book.css`, scoped to read well in both the ayu
(default) and light themes, with a `.rut-run` hook class reserved for
the upcoming in-book run controls. After editing the grammar:

```sh
cd integrations/rut-highlightjs
node scripts/build.mjs
cp register.js ../../docs/theme/rut-highlight.js
```

mdbook only highlights in the browser; the static built HTML ships
without spans. To bake static `hljs-*` spans into `docs/book/` after a
build (no-JS readers, view-source, SEO) run the zero-dependency bake —
it post-processes every `language-rut` block with the SAME grammar:

```sh
mdbook build docs
node integrations/rut-highlightjs/scripts/bake-book.mjs
```

Idempotent: already-baked blocks are skipped.

## Derivation: the compiler tables are the truth

The grammar is transcribed from the compiler's own tables, and where
prose and code disagree the code wins:

- **keywords** — `RESERVED_KW` in `crates/rut-parser/src/lib.rs` (28
  words) plus the contextual words the parser matches by interner text
  (`entry`, `builtin`, `primitive`, `break`, `continue`, `self`,
  `Self`, `super`, `as`, `type`). The keyword table in
  `docs/src/reference/lexical-structure.md` matches `RESERVED_KW`
  exactly — no conflict there.
- **primitive types** — `is_primitive_ty` in the same file (`bool`,
  `str`, `bytes`, `f32`, `f64`, `i8`–`i64`, `u8`–`u64`; `nil` colors as
  a literal). `opaque` is added on top: a real contextual builtin type
  (the erasure box, declared `builtin primitive opaque`) even though
  the table doesn't list it.
- **literal syntax** — `crates/rut-lexer/src/lexer.rs`: `r"…"`, `f"…"`
  (`{{`/`}}` escapes; string literals inside a hole are a lex error,
  so holes highlight expressions only), escapes `\t \n \r \b \f \\ \"
  \' \0 \u{…}`, `_` digit separators, `0x`/`0b`/`0o` radices (either
  case), `i8`–`i64`/`u8`–`u64`/`f32`/`f64` suffixes.
- **raw strings: docs and lexer disagree, the lexer wins.**
  `literals-and-inference.md` says raw strings process no escapes ("every
  byte is literal" — implying `\"` stays), but `lex_raw_string_body`
  closes the string at the FIRST `"`, escaped or not; the doc claim
  rests on a stale comment in the lexer source. The grammar highlights
  what the compiler actually lexes: `end: /"/`.

Cross-check notes worth keeping in mind when editing:

- There is no `loop` keyword — no `loop` anywhere in the parser.
- `in` is not a keyword; it is a reserved word the lexer rejects with
  "iteration is `for (let x of ...)`".
- `break`/`continue` are statement keywords in the parser but NOT in
  `RESERVED_KW` (they don't bare-complete) — highlighted here anyway,
  as the TextMate grammar does.
- The vscode TextMate grammar still carries `default`, which the
  parser no longer knows — deliberately not highlighted here.
