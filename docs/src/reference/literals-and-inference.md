# Literals and inference

Numeric suffixes, the default-width rule, casts, tuple literals,
string literal forms, and the format-string desugaring.

## Inference and conversions

Inference is bidirectional (literal ↔ expected type). A literal without
context defaults to `i32` (integers) / `f32` (floats) — and **the
default is also a ceiling**: an unsuffixed literal adapts to the
expected type only while it fits that default.

```rut
use ink::{ Logger };

pub fn main() {
    let log = Logger.new("t");
    let a = 10;                // i32 (default)
    let b = 10u8;              // u8 via suffix
    let c: u64 = 10;           // u64 via annotation — fits the default
    let big = 18446744073709551615u64;  // past the i32 default: suffix REQUIRED
    // let bad = 13503953896175478587;  // ERROR — exceeds i32, add `u64`
    log.info(f"{a} {b} {c} {big}");
}
```

```text
10 10 10 18446744073709551615
```

Past the ceiling the literal must declare itself with a suffix, so a
dropped or doubled digit cannot silently re-base a constant. Floats:
magnitude is the trigger, precision is not — `0.1` is fine anywhere;
`1.0e300` needs `f64` even in an `f64` position.

Numeric literals: decimal and `0x` / `0b` / `0o`, `_` separators,
suffixes `u8..u64 i8..i64 f32 f64` (`3.14159f32`, `0xFF_u32`).

## The cast — `expr as T`

Conversions are casts, truncating like C/Rust:

- int→int keeps the target's low bits (signed targets sign-extend);
- float→int truncates toward zero and saturates at the target bounds
  (`NaN` → `0`);
- a conversion never traps (arithmetic still does).

```rut
use calc::{ Math };
use ink::{ Logger };

pub fn main() {
    let log = Logger.new("t");
    let cast = 300 as u8;      // 44
    let x = 3;
    let y = 4;
    log.info(f"{cast} {Math.sqrt((x * x + y * y) as f64)}");
}
```

```text
44 5
```

`as` binds tighter than `*`, is left-associative (`x as u32 as u64`
chains), and its right-hand side is a naming position restricted to the
numeric primitives. There are no implicit numeric conversions at all.

## Fixed arrays and repeats

- `[e1, .., en]` has type `[T]` — pure data whose literal allocates the
  array cell; binding the value shares the cell. It infers `T`
  bidirectionally like any literal; at module scope it is a load-time
  expression when every element is (see
  [Modules and visibility](modules-and-visibility.md)).
- `[v; n]` is the repeat: `n` slots of the **value** `v` — see
  [Builtin generic types](builtin-generic-types.md).

## Tuples

Tuples are first-class values: type `(T0, T1, ..)`, value `(a, b, ..)`,
numeric fields `.0` / `.1` / .., and destructuring:

```rut
use ink::{ Logger };

fn divmod(a: i32, b: i32) -> (i32, i32) {
    return (a / b, a % b);
}

pub fn main() {
    let log = Logger.new("t");
    let pair = (1, "two");         // (i32, str)
    let (n, s) = pair;
    log.info(f"{n} {s} {divmod(7, 2).0}");
}
```

```text
1 two 3
```

The `(value, ok)` / `(T, err)` pair is the language's answer channel —
see [Primitive types](primitive-types.md).

## String literals

Three forms — a plain `"..."` string is always inert (no interpolation
ever happens implicitly):

| Form | Example | Meaning |
|---|---|---|
| plain | `"hi\tname"` | escapes processed: `\t \n \r \b \f \\ \" \u{...}` |
| raw | `r"C:\temp\log.txt"` | **no** escape processing; every byte is literal |
| format | `f"hi {name}, n={n}"` | Rust-style placeholders, evaluated at runtime |

Raw and format do not combine (`rf"..."` does not parse).

## Format literals — `f"..."`

- `{ expr }` splices an expression's value: identifiers, paths, calls,
  arithmetic — any expression *except* nested string literals (bind one
  to a name first). The lexer balances braces to find the closing `}`.
- `{{` and `}}` are literal braces. Escapes work exactly like plain
  strings.
- The literal desugars at compile time to a concatenation of the literal
  chunks and one `str(x)` conversion per placeholder — a builtin
  per-type formatting call, no runtime parsing:

```text
f"a={a} b={f(b())}"   ->   concat("a=", str(a), " b=", str(f(b())))
```

Formattable types and their rendering:

| Type | Rendering |
|---|---|
| ints | decimal, `-` for negatives |
| `f32`/`f64` | shortest round-trip decimal (`3.5`, `0.1`, `1e300`) |
| `bool` | `true` / `false` |
| `str` | contents, verbatim |
| enum | member name (`Color.Red` → `"Red"`) |

Everything else — struct/class values, vecs and fixed arrays, `?T`
boxes, `opaque`, tuples — is a **compile error** inside `f"..."`
(preventing accidental implementation-detail printing). Write a
`to_string() -> str` method on your type and call it explicitly, and
route developer output through your host logger (for example the `ink`
package's `Logger.debug(f"...")`).

The accumulator idiom `out = f"{out}{chunk}"` is recognized by the
compiler and appends in place — amortized O(1) instead of copying the
whole prefix per step, linear in the total output. Core ships no
string-builder class: when a reusable builder reads better (a tokenizer
or encoder threading text through many calls), reach for the `strbuild`
package's `StringBuilder` ([stdlib](stdlib.md)); when an accumulator
binding reads better, the f-string shape IS the optimized path.
