# RFC 0002: Lexical Structure — Source, Identifiers, Naming

- **Status:** Draft
- **Date:** 2026-08-23
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
  `Vec<T>`, `Array<T, N>` (const-generic), `Option<T>`, `Result<T,E>`,
  `Weak<T>`,
  `Opaque`
  (the erased-storage host class, RFC 0014),
  `Slice<T>` (builtin trait —
  object type `dyn Slice<T>`, RFC 0005),
  `Future<T>`, `Task<T>`, `Sender<T>`, `Receiver<T>`, `Point`, `Color`,
  `Drawable` (object type `dyn Drawable`, RFC 0012 §2).
- Scalars and simple buffers stay lowercase, C-style: `i32`, `u8`, `f32`,
  `bool`, `char`, `str`.
- **Construction is a method call, never a type-call** (RFC 0010):
  classes construct through their own class methods — `Rect.new(3, 4)`,
  `Rect.from(other)`, `Version.parse(s)` (`await Socket.connect(..)`
  when the class method is `suspend`); host classes likewise
  (`MyMap.new(cap)`, RFC 0025). Only builtin types keep call forms —
  allocation forms like `Vec<f32>(1024)`, `Weak(b)`, `Channel<Job>()`
  are builtin syntax, not class construction. **Erasure is a class
  method too**: `Opaque.new(v) -> Opaque` (RFC 0014) — the
  erased-storage host class, boxed by construction.
  Named variants of multi-case builtins stay statics: `Option.some`,
  `Option.none`, `Result.ok`, `Result.err`.
- **Functions and methods are lowercase snake_case** — `unwrap_or(d)`,
  `own(v)`, `downcast<T>(o)`, `select_all(futs)`,
  `spawn_worker(..)`. `own(v)` is the eager shallow copy (RFC 0011 §1);
  `downcast<T>` takes **concrete `T` only**
  (RFC 0014); the type *test* is not a function at all — it is the
  `is` keyword (`x is Circle`, RFC 0012 §3).
- **A trailing `$` marks dispatch-inverted members** (tur convention,
  extended uniformly): anything a *runtime* fires or owns, rather than you
  calling it — event props (`on_click$: Mutation<ClickEvent, unit>` or a
  closure prop), lifecycle hooks (`on_mount$`, `before_destroy$`),
  subscription updates (`on_update$`), watch controls (`start$`, `stop$`),
  engine atoms (`viewport_size$`), and `Mutation` handles generally
  (`set_tab$: Mutation<Tab, unit>`). Never on functions *you* call, widget
  builders (`click`, not `click$`), or types. `$` is an ordinary identifier
  character (§2), so the suffix is advisory — the style rule is what keeps it
  meaningful.

## 4. Reserved & contextual words

- Reserved (parse error with explanation): `switch`, `case`,
  `default`, `extends`, `super`, `as` (no casts at all — erasure is
  the `Opaque.new(v)` class method, RFC 0014), `type` (type alias — future),
  `struct`, `match`, `void` (the unit type is spelled `unit`), `null`,
  `undefined`, `any`, `typeof`,
  `instanceof`, `delete`, `in` (only `for..of`), `with`, `var`,
  `const` (bindings spell `let` / `let mut` — RFC 0003 §1), `private`
  (members are private by default; visibility spells `pub` — RFC 0003 §2).
- `new` is **not** reserved — it is an ordinary identifier and the
  conventional construction method name (`Rect.new(..)`, RFC 0010);
  there is no `new` expression anywhere.
- `entry` is contextual — the **host-callable marker**: `entry fn` at
  module scope publishes the function to the embedder (RFC 0035 §3);
  everywhere else `entry` is an ordinary identifier.
- `self` is contextual — the **receiver**: the first parameter of an
  instance method (`fn add(self, x, y)` — RFC 0010 §2) and the name it
  binds in the body; a method without `self` is a class method. There is
  no `this` keyword. `Self` is a normal identifier
  bound to the enclosing class inside its body (RFC 0010 §1); `fn`,
  `let`, `mut`, `if`, `else`, `while`, `for`, `of`, `return`, `when`,
  `enum`, `class`, `dataclass`, `trait`, `impl`,
  `requires`, `import`, `pub`, `from`, `static`, `suspend`,
  `await`, `true`, `false`, `extern`, `where`
  (admission-only generic-fn bounds, RFC 0013 §2), `dyn`
  (RFC 0012 §2), and `is` (the type-test operator, `expr is Type` —
  RFC 0012 §3) are keywords. `panic(msg: str)` and
  `assert(cond, msg?)` are prelude functions, not keywords (RFC 0034 §2)
  — and like every prelude name they are **imported, never ambient**
  (`import { assert, panic } from "std:core"`, RFC 0028). `dyn` prefixes any **trait path** — a
  user `I` or the builtin `Slice<T>` — one rule, no syntax branch
  (RFC 0005, RFC 0012 §2).

Each reserved word's error message names the rut replacement ("rut does not
have `switch`; use `when`") — the full diagnostic model is RFC 0030 §6.
