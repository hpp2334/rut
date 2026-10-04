# Control flow and when

The structured statements, the indexed and iterating `for` forms, and
`when` — the one match construct, an exhaustive pattern *expression*.
Branching on a value is `when`: arms are `pattern -> body`, and the
construct is exhaustive.

## Statements

- `if (cond) { .. } else if (..) { .. } else { .. }` — braces required.
- `while (cond) { .. }`.
- `return expr?;` — `expr` is required unless the function returns
  `nil`.
- `break` / `continue` exist for loops; there are no labels and no
  `do..while`.

### `for` — two forms

Iterating form — vecs, fixed arrays, slices, strings (one-codepoint
`str`s per step), `bytes` (`u8` per step), and any type carrying an
`[iterable]`-marked member — an enum value included (see
[Interfaces and dispatch](interfaces.md) and [Enums](enums.md)). Both spellings
of this form are one loop: the loop variable is a single binding
reassigned per iteration, and over a marked type the loop calls that
ONE designated member with an `emit` closure that follows the capture
law exactly like the fused form:

```rut
for (let x of expr) { .. }
```

Indexed form — the induction variable is **loop-owned**: the update
clause and the body may assign it without `mut`; it is not a normal
binding:

```rut
for (let i = 0; i < n; i += 1) { .. }
```

## `when`

`when (x) { pattern -> body, ... }` is an **expression**: the first
matching arm's body produces its value. No fallthrough; exactly one arm
runs.

```rut
use ink::{ Logger };

fn describe(n: i32) -> str {
    return when (n) {
        0         -> "zero",
        1, 2, 3   -> "small",   // comma-separated alternatives
        else      -> "big",     // non-enum scrutinee: else REQUIRED
    };
}

entry fn main() {
    let log = Logger("t");
    log.info(f"{describe(0)} {describe(2)} {describe(10)}");
}
```

```text
zero small big
```

### Patterns

| Pattern | Example |
|---|---|
| enum member (dotted path) | `Light.Green` |
| literal | `0`, `3.5`, `true`, `"text"` |
| negative literal | `-1` |
| alternatives (comma list) | `1, 2, 3` |
| wildcard `else` / `_` | `else -> ..` |

Ranges, destructuring, binding patterns, and guards (`x if cond`) do
not exist; patterns are enum members, literals, alternatives, and the
wildcards only.

### The laws

- **Exhaustiveness is always enforced.** An enum-typed scrutinee must
  cover every member (then `else` is optional) or add `else` — a partial
  `when` is a compile error. Any other scrutinee type (integers can't be
  enumerated): `else` is **mandatory**.
- **All arms must agree on one type** — that is the `when`'s type. Used
  as a statement, that type must be `nil` (arm bodies that are evaluated
  for effect are written as block arms).
- Duplicate patterns, and arms made unreachable by earlier ones, are
  compile errors.
- Arms use `->`. Expression arms are comma-separated; block arms may
  omit the trailing comma (the corpus writes it).
- `when` literal patterns match compile-time values, never runtime `==`.

```rut
use ink::{ Logger };

enum Light { Green, Yellow, Red }

fn go() { let log = Logger("t"); log.info("go"); }
fn brake() { let log = Logger("t"); log.info("brake"); }
fn stop() { let log = Logger("t"); log.info("stop"); }

entry fn main() {
    let l = Light.Yellow;
    when (l) {
        Light.Green  -> { go(); },
        Light.Yellow -> { brake(); },
        Light.Red    -> { stop(); },   // every member: else optional
    }
}
```

```text
brake
```

A `when` is also the idiomatic nullable guard, together with `!= nil`:

```rut
use ink::{ Logger };

struct Point { x: i32; y: i32; }

entry fn main() {
    let log = Logger("t");
    let p: ?Point = Point { x: 7, y: 0 };
    when (p != nil) {
        true -> { log.info(f"{p.x}"); },
        else -> { /* absent */ },
    }
}
```

```text
7
```
