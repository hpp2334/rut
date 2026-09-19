# RFC 0007: Literals & Inference — Numeric Suffixes, Plain/Raw/Format Strings

- **Status:** Draft
- **Date:** 2026-08-23
- **Revised:** 2026-09 — the byref-nullable amendment (RFC 0044): `nil`
  is the nullable's null literal (the `*T` pointer spelling is gone),
  and the fixed-array literal allocates its cell under the sharing law
  (binding shares — nothing copies).
- **Author:** hpp2334
- **Depends on:** RFC 0004 (primitives), RFC 0006 (enums)
- **Supersedes:** RFC 0002 §4, §4.1 (pre-restructure)
- **Part:** B — Language surface

## Summary

`nil` is the nullable's null literal (RFC 0005 §8, RFC 0044) and the
empty type's one
value (RFC 0004 §4). Tuples are first-class values: type `(A, B)`,
value `(a, b)`, destructuring `let (a, b) = ..`, numeric field access
`.0`/`.1`. Every type has a zero value (RFC 0007 §1.1): `0`, `0.0`,
`false`, the empty string, `nil`, the zeroed record — an omitted
struct-literal field takes its zero value.

See **`demo/src/examples/literals.rut`** — numeric suffixes and annotations,
plain/raw/format strings, fixed-array and dataclass literals, and the
builtin allocation calls (`Vec<f32>(1024)`).

## 1. Inference & conversions

- Inference is bidirectional (literal ↔ expected type); a literal without
  context defaults to `i32` / `f32` (default-width OQ — RFC 0004 OQ-1).
  **The default is also a ceiling**: an unsuffixed literal adapts to the
  expected type only while it fits that default (`1` → any int width,
  `1.0` → any float width). Past it the literal must declare itself —
  `Vec<u64>.from([13503953896175478587u64, ..])`, never the bare digits —
  so a dropped or doubled digit cannot silently re-base a constant.
  Floats: magnitude is the trigger, precision is not (`0.1` is fine
  anywhere; `1.0e300` needs `f64` even in an `f64` position).
- Numeric literals: decimal and `0x`/`0b`/`0o`, `_` separators, suffixes
  `u8..u64 i8..i64 f32 f64` (`3.14159f32`, `0xFF_u32`).
- **Conversions are casts**: `expr as T`, truncating like C/Rust —
  int→int keeps the target's low bits (signed targets sign-extend),
  float→int truncates toward zero and saturates at the target bounds
  (NaN → 0), and a conversion never traps (arithmetic still does).
  `as` binds tighter than `*`, left-associative, and its RHS is a naming
  position restricted to the numeric primitives. There are no implicit
  numeric conversions at all in v1.
- **Fixed-array literal**: `[e1, .., en]` has type `[T]` — pure data
  whose literal allocates the array cell (RFC 0005 §9); binding the
  value shares the cell — nothing is copied (RFC 0044 §1). It infers
  `T`
  bidirectionally like any literal; at module scope it is a
  load-time expression when every element is (RFC 0003 §1). A growable needs
  its builtin allocation forms: `Vec<T>()`, `Vec<T>(n)` (zeroed), or
  `Vec.from([..])` (copies).

## 2. String literals: plain, raw, format

Three literal forms — a plain `"..."` string is always inert (no
interpolation ever happens implicitly, unlike JS template literals):

| Form | Example | Meaning |
|---|---|---|
| plain | `"hi\tname"` | escapes processed (`\t \n \r \b \f \\ \" \u{...}`) |
| raw | `r"C:\temp\log.txt"` | **no** escape processing; every byte is literal (C++ `R"(...)"` / Rust `r"..."` style, minus the parens) |
| format | `f"hi {name}, n={n}"` | Rust-style placeholders, evaluated at runtime |

**Format literals** `f"..."`:

- `{ expr }` splices an expression's value (Rust `format!` braces, no `$`):
  identifiers, paths, calls, arithmetic — any expression *except* nested
  string literals (bind one first); the lexer balances braces to find the
  closing `}`.
- `{{` and `}}` are literal braces. Escapes work exactly like plain strings.
- The literal desugars at compile time to `str.concat(...)` of the literal
  chunks and one `str(x)` per placeholder — a builtin per-type formatting
  call, no runtime parsing:

```text
f"a={a} b={f(b())}"   ->   concat("a=", str(a), " b=", str(f(b())))
```

(`str`/`concat` are internal-native calls, RFC 0022 §2 — there is no
`strcat` op; RFC 0032 §1.1 R2.)

- Formattable types and their `str()` output:

| type | rendering |
|---|---|
| ints | decimal, `-` for negatives |
| `f32`/`f64` | shortest round-trip decimal (`3.5`, `0.1`, `1e300`) |
| `bool` | `true` / `false` |
| `char` | the character itself |
| `str` | contents, verbatim |
| `enum` | member name (`Color.Red` → `"Red"`) |

  Everything else (dataclass/class values, vecs and fixed arrays,
  `Option`/`Result`) is a **compile error** inside `f"..."` — preventing accidental
  implementation-detail printing. Use `debug.str(x)` for developer output;
  write a `to_string() -> str` method on your class and call it explicitly.
- In a **`Template`-expected position** the same literal builds a
  structured value instead — `Template { parts, args }` with the
  placeholder values boxed (`Opaque`, RFC 0014), not rendered — for hosts,
  l10n, and structured logging (RFC 0027). `t.str()` renders identically to
  the concat path; the default `str` behavior above is unchanged.
- Raw + format don't combine in v1 (`rf"..."` is OQ-3).

## Open questions

- OQ-1: str indexing: byte-index + helpers proposal stands
  (`for (let c of s)` is the blessed iteration).
- OQ-2: a *checked* conversion builtin (`u32.checked(x)` → `Option`,
  try-into style) — `as` is the lossy form; nothing checks.
- OQ-3: `rf"..."` (raw + format combination) and precision/width specifiers
  in format placeholders (`{x:.2}`) — deferred.
