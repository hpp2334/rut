# Control flow and when

rut has the classic structured statements — `if`/`else`, `while`,
`for` — and exactly one match construct: `when`, an exhaustive pattern
*expression*. There is no `switch`, no `case`, no fallthrough. The full
rules live in [the reference on control flow](../reference/control-flow.md).

## `if` and `else`

Braces are always required, whatever the body length:

```rut
if (hits == 0) {
    first = true;
} else if (hits < 10) {
    warm = true;
} else {
    hot = true;
}
```

The condition must be a `bool` — there is no truthiness coercion, and
no parenthesized-assignment footgun.

## Loops

**`while`** repeats until its condition is false:

```rut
let mut m = i * i;
while (m <= limit) {
    marks[m] = 1;
    m += i;
}
```

**`for..of`** iterates anything with elements: fixed arrays, `Vec`s,
`str` (yielding one-codepoint `str`s), and `bytes` (yielding `u8`s):

```rut
for (let w of ["rut", "runs", "rut"]) {
    // w is a str
}

for (let c of "héllo") {
    n += 1;             // 6 iterations — codepoints, not bytes
}
```

The loop variable is always spelled `let` and is fresh each iteration.

**Indexed `for`** is the C shape. The induction variable is *loop-owned*:
the update clause (and the body) may assign it without `mut`.

```rut
for (let i = 0; i < n; i += 1) {
    total += xs[i];
}
```

**`break` and `continue`** work in every loop form; there are no labels
and no `do..while`.

## `return`

`return expr;` exits the function. If the function returns nothing, the
arrow is omitted and `return;` (or falling off the end) returns:

```rut
fn swap(mut xs: Vec<i32>, a: i32, b: i32) {
    let t = xs[a];
    xs[a] = xs[b];
    xs[b] = t;
    // no return — the function's type is nil
}
```

## `when` — the one match

`when (value) { pattern -> body, ... }` is an expression: the first
matching arm produces the value. No fallthrough — exactly one arm runs.

```rut
fn classify(n: i32) -> str {
    return when (n) {
        0       -> "zero",
        1, 2, 3 -> "small",     // comma-separated alternatives
        else    -> "big",       // the wildcard
    };
}
```

The arm body before the `,` is an expression; arms can also be blocks:

```rut
when (l) {
    Light.Green  -> { go(); },
    Light.Yellow -> { brake(); },
    Light.Red    -> { stop(); },
}
```

Used as a statement, a `when` must produce nothing — the block arms
above are statement arms.

### Patterns

v1 patterns are: enum members (`Light.Red`), literals (integers,
floats, `bool`, `str`), comma-separated alternatives, and `else`.
There are no ranges, no destructuring, and no guards — an `if` inside
the arm body does that job.

### Exhaustiveness is enforced

- **Enum scrutinee:** either cover every member or add `else`. A
  partial `when` over an enum is a compile error — when you add a
  member later, the compiler finds every `when` you must extend.
- **Any other scrutinee:** `else` is mandatory (integers and strings
  cannot be enumerated).

```rut
fn drive(l: Light) {
    when (l) {
        Light.Green  -> { go(); },
        Light.Yellow -> { brake(); },
        Light.Red    -> { stop(); },   // all members: no else needed
    }
}

fn describe(n: i32) -> str {
    return when (n) {
        0    -> "zero",
        else -> "not zero",            // non-enum scrutinee: else REQUIRED
    };
}
```

`when` over a `bool` reads nicely as a two-arm check:

```rut
when (p != nil) {
    true -> { log.info(f"point {p.x} {p.y}"); },
    else -> { log.info("point: nil"); },
}
```

Duplicate patterns — and arms made unreachable by an earlier one — are
compile errors.

### One type across the arms

All arm expressions must agree: that shared type is the `when`'s type.
A `when` that builds a value usually reads best wrapped in a function —
the function's return type then pins every arm:

```rut
fn light_for(i: i32) -> Light {
    return when (i) {
        0    -> Light.Green,
        1    -> Light.Yellow,
        else -> Light.Red,
    };
}
```

## Put it together

```rut
use ink::{ Logger };

enum Light { Green, Yellow, Red }

fn action(l: Light) -> str {
    return when (l) {
        Light.Green  -> "go",
        Light.Yellow -> "brake",
        Light.Red    -> "stop",
    };
}

fn light_for(i: i32) -> Light {
    return when (i) {
        0    -> Light.Green,
        1    -> Light.Yellow,
        else -> Light.Red,
    };
}

fn classify(n: i32) -> str {
    return when (n) {
        0       -> "zero",
        1, 2, 3 -> "small",
        else    -> "big",
    };
}

pub fn main() {
    let log = Logger.new("lights");

    for (let n of [0, 2, 9]) {
        log.info(f"{n}: {classify(n)}");
    }

    let mut i = 0;
    while (i < 3) {
        log.info(action(light_for(i)));
        i += 1;
    }
}
```

```text
0: zero
2: small
9: big
go
brake
stop
```

Next: [functions, closures, and generics](functions.md).
