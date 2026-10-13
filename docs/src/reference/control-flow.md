# Control flow and when

The structured statements, the indexed and iterating `for` forms, and
`when` — the one match construct, an exhaustive pattern *expression*.
Branching on a value is `when`: arms are `pattern -> body`, and the
construct is exhaustive.

## Statements

- `if (cond) { .. } else if (..) { .. } else { .. }` — braces required;
  the same spelling is also an expression (see below).
- `while (cond) { .. }`.
- `return expr?;` — `expr` is required unless the function returns
  `nil` (or the body ends in a tail; see below).
- `break` / `continue` exist for loops; there are no labels and no
  `do..while`.

### `if` — statement and expression

The statement form runs its effect. The same spelling is also an
**expression**: wherever an expression goes — a `let` initializer, a
call argument, an operand, an f-string hole, a `when` scrutinee — the
construct branches and produces the taken arm's value:

```rut
use ink::{ Logger };

entry fn main() {
    let log = Logger("if");
    let n = 7;
    let label = if (n > 5) { "big" } else { "small" };
    let odd = if (n % 2 == 0) { "no" } else { "yes" };
    log.info(f"{n}: {label}, odd={odd}");
}
```

```text
7: big, odd=yes
```

The laws:

- **Parens are required** — `if (cond)`, never `if cond`. Braces are
  required too, and `else if` chains link arm by arm.
- **Arms are blocks, and a block's tail is the arm's value** — the
  expression before the arm's `}`, written without `;`. A block with no
  tail produces `nil`.
- **Producing arms agree on one type** — that shared type is the
  `if`'s; a disagreement is a compile error naming both types. An arm
  that diverges (`return`, `break`, `continue`) contributes no type:
  the completing arm's value is the `if`'s.
- **No `else` types the whole `if` `nil`** — the implicit else arm is
  the `nil` value, so a value-producing then-arm without an `else` is
  an agreement error. The no-`else` form is the statement shape: it
  runs the arm for effect.
- **The `;` decides.** A trailing `;` after the statement form is the
  discard form — whatever the arms produced is nobody's (the
  assignments are the effect). Without the `;`, a trailing `if`
  statement with an `else` reads as the enclosing block's tail — the
  value the block produces.

### Block tails

Every block has a value, and one rule decides it: an expression closed
by `;` is a statement (its value is discarded); an expression followed
immediately by the block's `}` is the block's **tail** — the value the
block produces. (`let` always demands its `;`.) The tail reads the
block's own bindings, and its value moves out before the scope pops.
A fn body's tail is an **implicit return**: the
[definite-return law](functions-closures-generics.md#functions) counts
it, so a function with a result arrow can answer without spelling
`return` — the same law covers lambda block bodies:

```rut
use ink::{ Logger };

fn classify(n: i32) -> str {
    let band = if (n > 90) { "hi" } else if (n > 50) { "mid" } else { "lo" };
    band
}

fn fact(n: i32) -> i32 {
    if (n <= 1) {
        1
    } else {
        n * fact(n - 1)
    }
}

entry fn main() {
    let log = Logger("tails");
    log.info(f"{classify(72)} {fact(5)}");
}
```

```text
mid 120
```

The trailing `if` in `fact` is the statement form in tail position: its
producing arms agree on `i32` and the merged value is the body's tail.
Loop bodies produce values the same way, discarded; `when` arm blocks
are value blocks too (see [the laws](#the-laws)).

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
for (let x of expr) { .. }         // plain `let`: a read-only view
for (let mut x of expr) { .. }     // `mut`: the body may assign x
```

Indexed form — the C shape:

```rut
for (let mut i = 0; i < n; i += 1) { .. }
```

Both headers declare a **normal binding**, and the mut-binding law
gates them like any `let`: assigning the loop variable — in the body,
or in the indexed form's update clause — requires `mut` in the header:

```rut
use ink::{ Logger };

entry fn main() {
    let log = Logger("for");
    let mut total = 0;
    for (let mut i = 0; i < 5; i += 1) {   // the update clause assigns i
        total += i;
    }
    // for (let i = 0; i < 5; i += 1) { }  // ERROR: assignment to `i` requires `mut`
    let rows = [[1, 2], [3, 4]];
    for (let mut row of rows) {            // the body assigns the element
        row[0] = 99;                       // the write reaches the sequence
    }
    log.info(f"total={total} rows[1][0]={rows[1][0]}");
}
```

```text
total=10 rows[1][0]=99
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
| enum member (path) | `Light::Green` |
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
- **All arms must agree on one type** — that is the `when`'s type.
  Block arms are value blocks: a block arm's tail is the arm's value,
  and expression and block arms mix freely under the same law. Used as
  a statement, the `when`'s value is discarded — the arms still agree
  (the check is the same one), and block arms written for effect stay
  tail-less.
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
    let l = Light::Yellow;
    when (l) {
        Light::Green  -> { go(); },
        Light::Yellow -> { brake(); },
        Light::Red    -> { stop(); },   // every member: else optional
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
