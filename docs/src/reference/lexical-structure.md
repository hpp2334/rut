# Lexical structure

The character-level rules of rut source: files, comments, identifiers, the
keyword set, and the enforced naming conventions.

## Source model

- Files are UTF-8, extension `.rut`. Both LF and CRLF line endings are
  accepted (CRLF is normalized); a leading BOM is skipped.
- Comments:
  - `// line comment`
  - `/* block comment */` — block comments do **not** nest
  - `/// doc comment` (also `/** ... */`) — attaches to the following
    declaration and is kept for tooling; it is not semantic
- There is no off-side rule: statements end with `;` (required), blocks
  are braced.

## Identifiers and `$`

```text
ident := (letter | "_" | "$") (letter | digit | "_" | "$")*
```

`$` is an ordinary identifier character, valid anywhere in a name:
`on_mount$`, `viewport_size$`, `$temp`, `$x1`, even `$` alone. There is no
`$` punctuation token and no `$`-interpolation (f-string holes use
`{ }` — see [Literals and inference](literals-and-inference.md)).

The **trailing** `$` is a naming convention, not a lexical category: it
marks *dispatch-inverted* members — anything a runtime fires or owns
rather than you calling it. Event props (`on_click$`), lifecycle hooks
(`on_mount$`, `before_destroy$`), subscription updates (`on_update$`),
watch controls (`start$`, `stop$`), engine atoms (`viewport_size$`), and
mutation handles generally. Never on functions *you* call, widget
builders, or types.

## Keywords

The keyword set is exactly:

| | | | | |
|---|---|---|---|---|
| `fn` | `let` | `mut` | `if` | `else` |
| `while` | `for` | `of` | `return` | `when` |
| `enum` | `struct` | `class` | `interface` | `impl` |
| `requires` | `use` | `pub` | `static` | `async` |
| `await` | `extern` | `is` | `host` | |
| `true` | `false` | `nil` | | |

Contextual words — ordinary identifiers elsewhere:

| Word | Special meaning |
|---|---|
| `type` | the alias introducer (`type Km = Meters;` — see [Type aliases and union bounds](type-aliases.md)) |
| `entry` | `entry fn` at module scope publishes the function to the embedder |
| `self` | the explicit receiver, first parameter of an instance method |
| `Self` | names the enclosing class inside its body; the class-private literal `Self { .. }` |
| `new` | not special — the conventional construction-method name (`Rect.new(..)`) |
| `as` | the numeric cast (`x as u32`) |
| `super` | only inside `pub(super)` |
| `builtin` | declaration modes of the engine's own surface — spelled `prelude builtin` (ambient) or `pub builtin` (import-gated); see [Host fns and declaration files](host-fns.md) |
| `disposal` / `iterable` / `constructor` | the bracket markers — `[disposal] fn` / `[iterable] fn` / `[constructor] fn` designate an inherent impl member as a designated surface (before visibility: `[disposal] pub fn ..`); the set is closed and engine-owned ([interfaces](interfaces.md)) |

`panic(msg)` is a prelude function, not a keyword.

## Reserved words

Each reserved word is a hard error whose message names the rut
replacement:

| Reserved | Error replacement |
|---|---|
| `void` | the empty type spells `nil` |
| `in` | iteration is `for (let x of ..)` |
| `private` | members are private by default; add `pub` |

The list is short and closed. Words that are keywords in JavaScript or
Rust — `switch`, `match`, `null`, `var`, `const`, `delete`, `new`, … —
are ordinary identifiers here: `let null = 5;` and `fn match()` are
legal rut — and so is `trait`: an ordinary identifier (name your pet
snake `trait` if you like; it carries no meaning).

## The async block

`async { .. }` — keyword before a block — is the async primitive: one
expression form whose evaluation mints the frame (pure; legal in sync
code) and whose type is `Future<T>`
([async and await](async.md)). `async` before `fn` is the declaration
sugar over it.

## Naming conventions (enforced)

- **Types are PascalCase** — user types and parameterized builtins:
  `Vec<T>`, `[T]`, `Weak<T>`, `opaque`, `LaunchedFutureHandle<T>`,
  `Point`, `Drawable`.
  Scalars and simple buffers stay lowercase: `i32`, `u8`, `f32`, `bool`,
  `str`, `bytes`.
- **Functions and methods are lower_snake_case** — `unwrap_or(d)`,
  `push(v)`, `checked_add(y)`, `string_len(s)`.
- **Construction is a method call; the call form binds by
  designation.** User classes construct through their own class
  methods: `Rect.new(3, 4)`, `Rect.from(other)`, `Version.parse(s)` —
  see [Classes and constructors](classes.md). A class that designates
  a `[constructor]` member additionally takes the call form:
  `Point(1.0, 2.0)` is sugar for the designated member call,
  byte-identical lowering. The newtype decl's own constructor needs
  no marker: `class JsonI64(i64);` constructs as the call
  `JsonI64(64)` — the compiler-provided positional mint (see
  [Newtypes](classes.md#newtypes-the-one-field-wrapper)). Only builtin
  surfaces keep the other call forms: `bytes.zeroed(n)`, `opaque(v)`,
  the repeat `[v; n]`, and `Vec<T>.from(..)` (see
  [Builtin generic types](builtin-generic-types.md)).
- The trailing-`$` marker is kept meaningful by this style rule alone;
  the compiler attaches no semantics to it.
