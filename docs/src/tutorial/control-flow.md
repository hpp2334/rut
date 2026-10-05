# Control flow and when

rut has the classic structured statements — `if`/`else`, `while`,
`for` — and exactly one match construct: `when`, an exhaustive pattern
*expression*. The full
rules live in [the reference on control flow](../reference/control-flow.md).

## `if` and `else`

Braces are always required, whatever the body length:

```rut
use ink::{ Logger };

entry fn main() {
    let log = Logger("flow");
    for (let hits of [0, 5, 12]) {
        let mut first = false;
        let mut warm = false;
        let mut hot = false;
        if (hits == 0) {
            first = true;
        } else if (hits < 10) {
            warm = true;
        } else {
            hot = true;
        }
        log.info(f"hits={hits} first={first} warm={warm} hot={hot}");
    }
}
```

```text
hits=0 first=true warm=false hot=false
hits=5 first=false warm=true hot=false
hits=12 first=false warm=false hot=true
```

The condition must be a `bool` — there is no truthiness coercion, and
no parenthesized-assignment footgun.

## Loops

**`while`** repeats until its condition is false:

```rut
use ink::{ Logger };

entry fn main() {
    let log = Logger("sieve");
    let i = 2;
    let limit = 12;
    let mut marks = [0; 16];
    let mut m = i * i;
    while (m <= limit) {
        marks[m] = 1;
        m += i;
    }
    log.info(f"marks[4]={marks[4]} marks[5]={marks[5]} stopped at m={m}");
}
```

```text
marks[4]=1 marks[5]=0 stopped at m=14
```

**`for..of`** iterates anything with elements: fixed arrays, `Vec`s,
`str` (yielding one-codepoint `str`s), `bytes` (yielding `u8`s), and
any type that marks an `[iterable]` member (a user class or an enum —
the loop calls that ONE designated member; see
[interfaces](../reference/interfaces.md)):

```rut
use ink::{ Logger };

entry fn main() {
    let log = Logger("loops");
    let mut n = 0;
    for (let w of ["rut", "runs", "rut"]) {
        // w is a str
    }
    for (let c of "héllo") {
        n += 1;             // 5 iterations — codepoints, not the 6 bytes
    }
    log.info(f"n={n}");
}
```

```text
n=5
```

The loop variable is always spelled `let` and is fresh each iteration.

**Indexed `for`** is the C shape. The induction variable is *loop-owned*:
the update clause (and the body) may assign it without `mut`.

```rut
use ink::{ Logger };

entry fn main() {
    let log = Logger("loops");
    let xs = [3, 1, 4];
    let n = xs.len();
    let mut total = 0;
    for (let i = 0; i < n; i += 1) {
        total += xs[i];
    }
    log.info(f"total={total}");
}
```

```text
total=8
```

**`break` and `continue`** work in every loop form; there are no labels
and no `do..while`.

## `return`

`return expr;` exits the function. If the function returns nothing, the
arrow is omitted and `return;` (or falling off the end) returns:

```rut
use pouch::{ Vec };
use ink::{ Logger };

fn swap(mut xs: Vec<i32>, a: i32, b: i32) {
    let t = xs[a];
    xs[a] = xs[b];
    xs[b] = t;
    // no return — the function's type is nil
}

entry fn main() {
    let log = Logger("swap");
    let xs = Vec<i32>.from([1, 2, 3]);
    swap(xs, 0, 2);
    log.info(f"first={xs[0]} last={xs[2]}");
}
```

```text
first=3 last=1
```

## `when` — the one match

`when (value) { pattern -> body, ... }` is an expression: the first
matching arm produces the value. No fallthrough — exactly one arm runs.

```rut
use ink::{ Logger };

fn classify(n: i32) -> str {
    return when (n) {
        0       -> "zero",
        1, 2, 3 -> "small",     // comma-separated alternatives
        else    -> "big",       // the wildcard
    };
}

entry fn main() {
    let log = Logger("when");
    for (let n of [0, 2, 9]) {
        log.info(f"{n}: {classify(n)}");
    }
}
```

```text
0: zero
2: small
9: big
```

The arm body before the `,` is an expression; arms can also be blocks:

```rut
use ink::{ Logger };

enum Light { Green, Yellow, Red }

fn go()    { Logger("light").info("go"); }
fn brake() { Logger("light").info("brake"); }
fn stop()  { Logger("light").info("stop"); }

entry fn main() {
    let l = Light.Yellow;
    when (l) {
        Light.Green  -> { go(); },
        Light.Yellow -> { brake(); },
        Light.Red    -> { stop(); },
    }
}
```

```text
brake
```

Used as a statement, a `when` must produce nothing — the block arms
above are statement arms.

### Patterns

The patterns are: enum members (`Light.Red`), literals (integers,
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
use ink::{ Logger };

enum Light { Green, Yellow, Red }

fn go()    { Logger("light").info("go"); }
fn brake() { Logger("light").info("brake"); }
fn stop()  { Logger("light").info("stop"); }

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

entry fn main() {
    drive(Light.Red);
    let log = Logger("light");
    log.info(describe(0));
    log.info(describe(7));
}
```

```text
stop
zero
not zero
```

`when` over a `bool` reads nicely as a two-arm check:

```rut
use ink::{ Logger };

struct Point { x: i32; y: i32 }

entry fn main() {
    let log = Logger("nullable");
    let p: ?Point = Point { x: 1, y: 2 };
    when (p != nil) {
        true -> { log.info(f"point {p.x} {p.y}"); },
        else -> { log.info("point: nil"); },
    }
}
```

```text
point 1 2
```

Duplicate patterns — and arms made unreachable by an earlier one — are
compile errors.

### One type across the arms

All arm expressions must agree: that shared type is the `when`'s type.
A `when` that builds a value usually reads best wrapped in a function —
the function's return type then pins every arm:

```rut
use ink::{ Logger };

enum Light { Green, Yellow, Red }

fn light_for(i: i32) -> Light {
    return when (i) {
        0    -> Light.Green,
        1    -> Light.Yellow,
        else -> Light.Red,
    };
}

entry fn main() {
    let log = Logger("light");
    log.info(f"{light_for(0)} {light_for(1)} {light_for(5)}");
}
```

```text
Green Yellow Red
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

entry fn main() {
    let log = Logger("lights");

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
