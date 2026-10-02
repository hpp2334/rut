# Primitive types

The primitive types, the one sharing regime, and integer semantics.

## The type table

| Group | Types | Notes |
|---|---|---|
| unsigned int | `u8` `u16` `u32` `u64` | fixed width |
| signed int | `i8` `i16` `i32` `i64` | two's complement |
| float | `f32` `f64` | IEEE 754 |
| misc | `bool` | |
| text | `str` | immutable UTF-8, length-prefixed; compared by content; `s.slice(a, b)` is an O(1) view (see [String slicing and views](string-views.md)) |
| binary | `bytes` | immutable, content-compared octet buffer; the engine-level `u8` array |
| seq | `Vec<T>` | mutable, growable buffer — shared; a library class over `[T]` (see [Builtin generic types](builtin-generic-types.md)) |
| seq | `[T]` | fixed array — shared; runtime length, non-growable |
| nullable | `?T` | nil-able cell; `nil` is the null (see [By-reference and nullable](by-reference-and-nullable.md)) |
| erasure | `opaque` | the erasure box (see [opaque — erasure and downcast](opaque.md)) |
| user | `struct` / `class` records | shared cell handles (see [Structs](structs.md), [Classes and constructors](classes.md)) |

There is **no character type**: `'x'` does not parse, and `str`
iteration yields one-codepoint `str`s. Codepoints are spelled with
integers:

| Member | Meaning |
|---|---|
| `s.code() -> u32` | the FIRST codepoint of `s` (traps on empty) |
| `s.code_at(i: i32) -> u32` | the codepoint at codepoint index `i` (traps out of bounds — the index is a bug, not data) |
| `str.from_code(n: u32) -> str` | the 1-codepoint `str` for `n` |

Absence is `nil` on a nullable
`?T`.

## Size and members

`.len()` is the sequence member shared by every sequence: `[T]`,
`Vec<T>`, `str` (codepoints), `bytes` (octets).

`str` members: `len()`, `code()`, `code_at(i)`, `encode() -> bytes`,
`slice(from, to) -> str`, `starts_with(from, head) -> bool`,
`scan(from, set) -> i64`.
`bytes` members: `len()`, `decode() -> str` (UTF-8, lossy),
`clone() -> bytes`; type-methods `bytes.zeroed(n) -> bytes` and
`bytes.from(a: [u8]) -> bytes`. `str.encode()` and `bytes.decode()`
convert between text and octets.

## One regime: primitives by value, everything else shared

Primitives (`u8..u64`, `i8..i64`, `f32`/`f64`, `bool`) and `fn` values
copy on assignment, passing, and return — plain slot moves. **Every
other type is a refcounted heap cell handle**: assignment shares, and
mutation through any alias is visible through all of them — struct and
class instances, `str`, `bytes`, `Vec`, `[T]`, enums, `opaque` boxes,
trait-typed values, `?T` boxes alike. Writing is gated by the
`mut`-binding law (see [Modules and visibility](modules-and-visibility.md)),
never by the sharing.

There is no eager copy: **`bytes.clone()` is the one
copy escape hatch**. There is no `&`/`*` syntax anywhere.
`==` on cells is **identity** (the raw slot compare); `str`/`bytes`
compare by content — the full table is in
[Rc, dispose, and identity](rc-dispose-identity.md).

No `box<T>`, no loans: a loan needs an exclusivity proof, and rut has
no borrow checker. The construct is rejected, not deferred. The only
borrows anywhere are host-side, call-scoped ones at the embedding
boundary.

## Tuples and the answer channel

Tuples are first-class values: type `(A, B)`, value `(a, b)`, numeric
field access `.0`, `.1`, .., destructuring:

```rut
let (lo, hi) = bounds;
fn checked_add(self, y: u8) -> (u8, bool);
```

**`(?T, err)` — concretely `(value, ok)` / `(T, str)` — is the answer
channel** for results and errors. The convention is law:

- empty err + a value = **success**
- empty err + `nil` = **"not found"** (a legitimately absent value)
- non-empty err = **failed**

`checked_add`/`checked_sub`/`checked_mul` return `(value, ok)` — `false`
exactly on overflow, `value` the wrapped bits either way.

## Integer semantics

- Overflow in `+ - * <<` **traps** in debug and release. Wrapping
  escapes are compiler-lowered methods, ambient on every integer
  primitive (no `use`):

  | Family | Members |
  |---|---|
  | wrapping (two's complement) | `wrapping_add` `wrapping_sub` `wrapping_mul` `wrapping_shl` |
  | saturating | `saturating_add` `saturating_sub` `saturating_mul` |
  | checked (`(T, bool)`) | `checked_add` `checked_sub` `checked_mul` |

- Division by zero traps; `int / int` is integer division.
- **Mixed-width arithmetic is an error** — both operands must have equal
  width; convert first with `as`.
- Conversions are the numeric cast `expr as T`, truncating like C/Rust —
  see [Literals and inference](literals-and-inference.md). There are no
  implicit numeric conversions at all.
