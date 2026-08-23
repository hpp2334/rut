# RFC 0007: Literals & Inference — Numeric Suffixes, Plain/Raw/Format Strings

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0004 (primitives), RFC 0006 (enums)
- **Supersedes:** RFC 0002 §4, §4.1 (pre-restructure)
- **Part:** B — Language surface

## Summary

See **`examples/basic/literals.rut`** — numeric suffixes and annotations,
plain/raw/format strings, fixed-array and dataclass literals, and the
lowercase constructor-type calls (`Vec<f32>(1024)`).

## 1. Inference & conversions

- Inference is bidirectional (literal ↔ expected type); a literal without
  context defaults to `i32` / `f64` (default-width OQ — RFC 0004 OQ-1).
- Numeric literals: decimal and `0x`/`0b`/`0o`, `_` separators, suffixes
  `u8..u64 i8..i64 f32 f64` (`3.14159f32`, `0xFF_u32`).
- **Conversions are always explicit function calls** — `i32(x)`, `u8(x)`,
  `f32(x)`; there is no `as` operator (RFC 0012 §3). No implicit numeric
  conversions at all in v1. Narrowing (`i32`→`u8`, `f64`→`i32`) traps when
  the value doesn't fit; lossy intent is spelled out with `u8.wrap(x)` /
  `i32.trunc(x)` style builtins (OQ-2).
- **Fixed-array literal**: `[e1, .., en]` has type `Array<T, n>` — an
  inline **value**, pure data, no allocation (RFC 0005). It infers `T`
  bidirectionally like any literal; at module scope it is a
  load-time expression when every element is (RFC 0003 §1). A growable needs
  its own constructor: `Vec<T>()`, `Vec<T>(n)` (zeroed), or
  `Vec.from([..])` (copies).

## 2. String literals: plain, raw, format

Three literal forms — a plain `"..."` string is always inert (no
interpolation ever happens implicitly, unlike JS template literals):

| Form | Example | Meaning |
|---|---|---|
| plain | `"hi\tname"` | escapes processed (`\t \n \r \\ \" \u{...}`) |
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
| `string` | contents, verbatim |
| `enum` | member name (`Color.Red` → `"Red"`) |

  Everything else (dataclass/class values, vecs and fixed arrays,
  `Option`/`Result`) is a **compile error** inside `f"..."` — preventing accidental
  implementation-detail printing. Use `debug.str(x)` for developer output;
  write a `to_string(): string` method on your class and call it explicitly.
- In a **`Template`-expected position** the same literal builds a
  structured value instead — `Template { parts, args }` with the
  placeholder values boxed (`Opaque`, RFC 0014), not rendered — for hosts,
  l10n, and structured logging (RFC 0027). `t.str()` renders identically to
  the concat path; the default `string` behavior above is unchanged.
- Raw + format don't combine in v1 (`rf"..."` is OQ-3).

## Open questions

- OQ-1: string indexing: byte-index + helpers proposal stands
  (`for (let c of s)` is the blessed iteration).
- OQ-2: naming of lossy numeric conversions: `u8.wrap(x)` vs
  `u8.truncate(x)` vs `wrapTo<u8>(x)`.
- OQ-3: `rf"..."` (raw + format combination) and precision/width specifiers
  in format placeholders (`{x:.2}`) — deferred.
