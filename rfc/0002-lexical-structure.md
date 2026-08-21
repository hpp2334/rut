# RFC 0002: Lexical Structure — Source, Identifiers, Naming

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0001 (pillars)
- **Supersedes:** RFC 0006 §1–2 and RFC 0002 "Naming conventions" (pre-restructure)
- **Part:** B — Language surface

## Summary

The character-level rules of rut source: UTF-8 files, line/block/doc comments,
identifiers where `$` is an ordinary character, a hard reserved-word list, and
the enforced naming conventions (PascalCase types, snake_case functions, the
trailing-`$` dispatch-inverted marker). Tokenization *machinery* (the token
enum, maximal munch, the f-string mode stack) is RFC 0029 — this file is the
language-facing contract.

## 1. Source model

- Files are UTF-8; both LF and CRLF accepted (CRLF normalized to LF in
  spans); a BOM is skipped. Extension: `.rut`.
- Comments: `// line`, `/* block (nesting NOT allowed) */`, and `/** doc */`
  attached to the following declaration (kept in the AST for future tooling;
  not semantic).
- There is no off-side rule: statements end with `;` (required), blocks brace.

## 2. Identifiers & `$`

`(letter | _ | $)(letter | digit | _ | $)*` — `$` is a fully valid identifier
character, anywhere in the name (`on_mount$`, `viewport_size$`, `$temp`,
`$x1`, even `$` alone). There is no `$` punctuation token — the lexer only
ever produces `$` inside identifiers, and f-string holes use `{ }`
(RFC 0007 §2), so `$`-identifiers create no ambiguity anywhere.

The **trailing** `$` is a *convention* (below), not a lexical category: the
compiler gives it no meaning; greppability is enforced by style.

## 3. Naming conventions (enforced)

- **Types are PascalCase** — user types and parameterized builtins:
  `Array<T>`, `Option<T>`, `Result<T,E>`, `Rc<T>`, `Weak<T>`, `Opaque`,
  `Future<T>`, `Task<T>`, `Sender<T>`, `Receiver<T>`, `Point`, `Color`,
  `Drawable`.
- Scalars and simple buffers stay lowercase, C-style: `i32`, `u8`, `f32`,
  `bool`, `char`, `string`, `bytes`.
- **Construction is a type-call** (RFC 0010): the type name in call position
  constructs — `Circle(1, 2, 3)` (user class `factory`; `await Circle(..)`
  when the factory is `suspend`), `Rc(c)`,
  `Rc<Circle>(c)`, `Weak(b)`, `Opaque(v)`, `Array<f32>(1024)`, `bytes(64)`,
  `Channel<Job>()`. Lowercase types keep lowercase calls (`bytes(64)`).
  Named variants of multi-case builtins stay statics: `Option.some`,
  `Option.none`, `Result.ok`, `Result.err`.
- **Functions and methods are lowercase snake_case** — `unwrap_or(d)`,
  `is<T>(x)`, `upcast<T>(x)`, `downcast<T>(o)`, `select_all(futs)`,
  `spawn_worker(..)`.
- **A trailing `$` marks dispatch-inverted members** (tur convention,
  extended uniformly): anything a *runtime* fires or owns, rather than you
  calling it — event props (`on_click$: Mutation<ClickEvent, void>` or a
  closure prop), lifecycle hooks (`on_mount$`, `before_destroy$`),
  subscription updates (`on_update$`), watch controls (`start$`, `stop$`),
  engine atoms (`viewport_size$`), and `Mutation` handles generally
  (`set_tab$: Mutation<Tab, void>`). Never on functions *you* call, widget
  builders (`click`, not `click$`), or types. `$` is an ordinary identifier
  character (§2), so the suffix is advisory — the style rule is what keeps it
  meaningful.

## 4. Reserved & contextual words

- Reserved (parse error with explanation): `new`, `switch`, `case`,
  `default`, `extends`, `super`, `as`, `type` (type alias — future),
  `struct`, `match`, `null`, `undefined`, `any`, `unknown`, `typeof`,
  `instanceof`, `delete`, `in` (only `for..of`), `with`, `var`.
- `this` is contextual (class members only); `Self` is a normal identifier
  bound to the enclosing class inside its body (RFC 0010 §1); `fn`,
  `let`, `const`, `if`, `else`, `while`, `for`, `of`, `return`, `when`,
  `enum`, `class`, `dataclass`, `interface`, `implements`,
  `requires`, `import`, `export`, `from`, `private`, `static`, `suspend`,
  `await`, `factory`, `dispose`, `true`, `false`, `extern` are keywords.

Each reserved word's error message names the rut replacement ("rut does not
have `switch`; use `when`") — the full diagnostic model is RFC 0029 §6.
