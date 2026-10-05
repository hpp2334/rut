# Values and variables

Every value in rut has a type, and every type is known at compile time.
This chapter covers the primitive types, how literals infer their types,
how bindings work, and the one rule that organizes everything else:
**primitives copy, every other value is a shared cell**.

For the full grammar of literals and inference rules, see
[the reference on literals and inference](../reference/literals-and-inference.md).

## The primitive types

| Group | Types | Notes |
|---|---|---|
| unsigned integers | `u8` `u16` `u32` `u64` | fixed width |
| signed integers | `i8` `i16` `i32` `i64` | two's complement |
| floats | `f32` `f64` | IEEE 754 |
| boolean | `bool` | `true` / `false` |
| text | `str` | immutable UTF-8, compared by content |
| binary | `bytes` | immutable octet buffer, compared by content |
| erased | `opaque` | a box holding any value — see [errors and optionality](errors.md) |

There is no character type. A `str`
iterates as one-codepoint `str`s, and codepoints read as `u32`
(`s.code()`, `s.code_at(i)`) — see
[string slicing and views](../reference/string-views.md).

## Variables: `let` and `let mut`

```rut
use ink::{ Logger };

entry fn main() {
    let log = Logger("vars");
    let a = 10;          // an immutable binding
    let mut b = 10;      // a mutable binding
    b = 20;              // OK — b may be reassigned
    // a = 30;           // ERROR: a is not `mut`
    log.info(f"a={a} b={b}");
}
```

```text
a=10 b=20
```

`mut` is permission to write *through that name*: reassigning the
binding, assigning to a field (`p.x = 3`), or assigning to an element
(`xs[0] = 7`). Sharing is not gated by `mut` — two bindings can name
the same value, and what `mut` controls is only who may write.

Every type has a zero value: `0` for numbers, `false` for `bool`, the
empty string, `nil` for nullables. Omit a field in a struct literal and
it takes the field's initializer if there is one, else the type's zero
value (see [structs, enums, and classes](types.md)).

## Numbers

An unsuffixed integer literal defaults to `i32`; an unsuffixed float
defaults to `f32`. The default is also a *ceiling*: a literal adapts to
the expected type only while it fits the default.

```rut
use ink::{ Logger };

entry fn main() {
    let log = Logger("numbers");
    let a = 10;                        // i32
    let b = 10u8;                      // u8 via suffix
    let c: u64 = 10;                   // u64 via annotation — 10 fits
    let big = 18446744073709551615u64; // past the i32 default: suffix required
    let d = 1.5;                       // f32
    let d64: f64 = 1.5;                // f64 via annotation
    let hex = 0xFF_u32;                // 0x / 0b / 0o bases, _ separators
    log.info(f"a={a} d={d} d64={d64} hex={hex} big={big}");
}
```

```text
a=10 d=1.5 d64=1.5 hex=255 big=18446744073709551615
```

Conversions are explicit casts: `expr as T`. Casts truncate like C —
they never trap.

```rut
use ink::{ Logger };

entry fn main() {
    let log = Logger("casts");
    let cast = 300 as u8;   // 44 — keeps the low 8 bits
    log.info(f"cast={cast}");
}
```

```text
cast=44
```

Arithmetic has two sharp edges:

- **Mixed widths do not mix.** Both operands must have the same width —
  convert one side first. `let mut total = 0.0;` makes an `f32`, and
  adding an `f64` to it is a type error; annotate `let mut total: f64 = 0.0;`
  when you mean double precision.
- **Overflow, division by zero, and out-of-bounds indexing trap.** For
  the explicit non-trapping ladder (`wrapping_add`, `saturating_mul`,
  `checked_add`), see [the standard library](stdlib.md).

## Text: plain, raw, and format strings

Three literal forms. A plain string is always inert — no interpolation
ever happens implicitly.

```rut
use ink::{ Logger };

entry fn main() {
    let log = Logger("text");
    let name = "rut";
    let s = "hi\tname";              // plain: escapes processed
    let raw = r"C:\temp\log.txt";    // raw: every byte is literal
    let t = f"hi {name}!";           // format: placeholders evaluated
    log.info(f"{s} | {raw} | {t}");
}
```

```text
hi	name | C:\temp\log.txt | hi rut!
```

Plain and format strings share the same escapes (`\t \n \r \\ \"`
and `\u{...}`). In an f-string, `{ expr }` splices any expression —
identifiers, calls, arithmetic — except nested string literals
(bind one to a variable first). `{{` and `}}` are literal braces:

```rut
use ink::{ Logger };

entry fn main() {
    let log = Logger("text");
    log.info(f"open{{close}} braces");   // prints: open{close} braces
}
```

```text
open{close} braces
```

What an f-string can render: integers (decimal), floats (shortest
round-trip decimal: `3.5`, `0.1`), `bool` (`true`/`false`), `str`
(contents), and enum members (their name: `Color.Green` renders
`Green`). Composite values — structs, classes, arrays, vecs — are a
compile error inside an f-string; write a `to_string()`-style method
and call it, or use a debug dump for development.

## Sequences and buffers

The fixed array `[T]` is built from a literal or a repeat:

```rut
use ink::{ Logger };

entry fn main() {
    let log = Logger("arrays");
    let arr = [1, 2, 3];         // [i32] — fixed length
    let zero: [u8] = [0u8; 34];  // 34 slots of 0
    log.info(f"len={arr.len()} first={arr[0]} zero.len={zero.len()}");
}
```

```text
len=3 first=1 zero.len=34
```

The growable sequence is `Vec<T>` (from the `pouch` package —
`use pouch::{ Vec };`): `Vec.new()`, `Vec.from([..])`,
`Vec<i32>.filled(0, 1024)`, `push`, `pop`, `len`. Indexing and
`for..of` work the same on both. See [the standard library](stdlib.md)
for the full surface.

`bytes` is the binary primitive. `s.encode()` turns text into octets,
`b.decode()` reads it back (lossy UTF-8), `bytes.zeroed(n)` and
`bytes.from(a)` build buffers directly:

```rut
use ink::{ Logger };

entry fn main() {
    let log = Logger("bytes");
    let b = "rut runs".encode();
    let ok = b.decode() == "rut runs";   // true — content comparison
    log.info(f"len={b.len()} roundtrip={ok}");
}
```

```text
len=8 roundtrip=true
```

## Sharing: the one rule

Primitives and `fn` values **copy** on assignment, argument passing,
and return. **Every other value is a shared cell**: `let q = p` is an
O(1) handle move, and a write through any alias is visible through all
of them.

```rut
use ink::{ Logger };

struct Point { x: i32; y: i32 }

entry fn main() {
    let log = Logger("sharing");
    let mut p = Point { x: 1, y: 2 };
    let q = p;          // q and p name ONE cell
    p.x = 4;            // q.x is 4 now
    log.info(f"q.x={q.x} same cell: {q == p}");
}
```

```text
q.x=4 same cell: true
```

Equality follows the same split:

| Operands | `==` means |
|---|---|
| numbers, `bool` | value comparison |
| `str`, `bytes` | content comparison |
| everything else | cell identity — two separately-built literals are never equal |

The one copy escape hatch is `bytes.clone()` — a fresh buffer with the
same octets. There is no generic copy for any other type; if you need a
divergent value, build a new one.

Absence is `nil` on a nullable `?T` — see
[errors and optionality](errors.md).

## Tuples

Records `(A, B)` are first-class values with numeric fields:

```rut
use ink::{ Logger };

fn divmod(a: i32, b: i32) -> (i32, i32) {
    return (a / b, a % b);
}

entry fn main() {
    let log = Logger("tuples");
    let (q, r) = divmod(17, 5);   // destructuring
    let t = (1, true);
    log.info(f"q={q} r={r} t.0={t.0}");
}
```

```text
q=3 r=2 t.0=1
```

The pair is also rut's standard error channel — the next chapters use
it constantly.

## Put it together

Put this in a module directory and run it: `literals/rut.jsonc` names
the two packages the program uses, and the program is
`literals/main.rut`:

```jsonc
// literals/rut.jsonc
{
  "name": "literals",
  "entry": { "lib": "./main.rut" },

  "deps": {
    "pouch": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v8/dist/std/pouch.rutbundle", "sha256": "128ffcf7daa2c43ed3ebad00a6a91ff7c4c494f2442c0a80d6f2c0daa455cc34" },
    "ink":   { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v8/dist/std/ink.rutbundle", "sha256": "04d069cdf71922e7cd41224ee813b78f568c66b878d909e92b026be2530e57c0" }
  }
}
```

Save the program as `literals/main.rut` and run `rut run literals`:

```rut
use pouch::{ Vec };
use ink::{ Logger };

struct Point { x: f32; y: f32 }

fn literals(name: str) {
    let log = Logger("literals");
    let a = 10;                      // i32 (default)
    let b = 10u8;                    // u8 via suffix
    let c: u64 = 10;                 // u64 via annotation — fits the default
    let big = 18446744073709551615u64; // past the `i32` default: suffix REQUIRED
    let cast = 300 as u8;            // 44 — `as` truncates
    let d = 1.5;                     // f32 (default)
    let e = 1.5f32;                  // f32 via suffix
    let d64: f64 = 1.5;              // f64 via annotation
    let s = "hi\tname";              // plain string: escapes processed
    let raw = r"C:\temp\log.txt";    // raw literal: NO escape processing
    let t = f"hi {name}!";           // format literal
    let ch = "h";                    // a 1-codepoint str
    let mut arr = [1, 2, 3];         // [i32] — fixed array
    let mut zero: Vec<f32> = Vec<f32>.filled(0.0, 1024);  // growable, flat
    let grow = Vec<i32>.from([1, 2, 3]);  // array -> growable
    arr[0] = 7;                      // element write needs `mut`
    zero[0] = 9.0f32;
    let bin = bytes(64);             // 64 zeroed octets
    let p = Point { x: 1, y: 2 };    // struct literal: no `new`

    log.info(f"a={a} e={e} d64={d64} ch={ch} p.x={p.x} zero[0]={zero[0]} len={grow.len()} bin={bin.len()}");
}

entry fn main() {
    literals("rut");
}
```

```text
a=10 e=1.5 d64=1.5 ch=h p.x=1 zero[0]=9 len=3 bin=64
```

Next: [control flow and `when`](control-flow.md).
