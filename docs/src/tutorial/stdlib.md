# The standard library

The standard library splits in two. **`core`** is the only true
standard — a prelude of builtin names that are *ambient*: in scope in
every compilation unit, no `use` needed. Everything else is a set of
**swappable packages** shipped in the toolchain tree — a program that
wants one says so: a `use` line in the source and the package's
`deps` row in its `rut.jsonc` ([project
structure](../reference/project-structure.md)). Any of them can be
replaced wholesale; the engine knows none of their names.

The reference page is [core and the swappable packages](../reference/stdlib.md).

## What's always there (`core`)

### Reporting and bugs

```rut
panic("Rect: negative extents");       // abort with a message
```

`assert` is not a builtin — a package that wants one writes the helper
over `panic` ([bugs, not flow](errors.md#panic-for-bugs-not-for-flow)):

```rut
fn assert(c: bool, m: str) { if (!c) { panic(m); } }
assert(total == expected, "checksum"); // abort when false
```

### Integer safety ladder

Every integer width carries compiler-lowered methods. Plain `+`/`-`/`*`
trap on overflow; these never do:

```rut
use ink::{ Logger };

entry fn main() {
    let log = Logger.new("ladder");
    let mut v: u8 = 250;
    v = v.wrapping_add(10);            // 4 — two's-complement wrap
    let over = 200u8.checked_add(100); // (44, false) — .1 false = escaped
    let sat = 200u8.saturating_add(100); // 255 — clamp at the bounds
    log.info(f"v={v} over=({over.0}, {over.1}) sat={sat}");
}
```

```text
v=4 over=(44, false) sat=255
```

`checked_*` answers `(T, bool)`; `.0` holds the wrapped bits either
way. Pick per call site: a checksum wraps, a length checks.

### `str` members

```rut
use ink::{ Logger };

entry fn main() {
    let log = Logger.new("str");
    let s = "héllo rut";
    log.info(f"len={s.len()} first={s.code()}");
    log.info(f"at1={s.code_at(1)} octets={s.encode().len()}");
    log.info(f"view={s.slice(6, 9)}");
    let head: str = "héllo";
    log.info(f"starts={s.starts_with(0, head)}");
    let joined = string_join(["rut", "runs"]);
    log.info(f"joined={joined}");
    let h = str.from_code(72);
    log.info(f"from_code={h}");
}
```

```text
len=9 first=104
at1=233 octets=10
view=rut
starts=true
joined=rutruns
from_code=H
```

`s.slice` deserves a second look: no octets move; the view records a
window over the parent, prints, compares by content, iterates, and can
re-slice. Codepoint access is spelled with integers — `str.from_code(n)`
builds the 1-codepoint `str` for a `u32`. Tokenizing rides
`s.scan(from, set)` over a caller-owned `[u8]` class
table. See [string slicing and views](../reference/string-views.md).

### `bytes` members

```rut
use ink::{ Logger };

entry fn main() {
    let log = Logger.new("bytes");
    let b = "rut runs".encode();
    log.info(f"len={b.len()} decode={b.decode()}");
    let copy = b.clone();     // the ONLY copy escape hatch
    let z = bytes.zeroed(4);
    let from = bytes.from([1, 2, 3]);
    log.info(f"same={copy == b} zeroed={z.len()} from={from.len()}");
}
```

```text
len=8 decode=rut runs
same=true zeroed=4 from=3
```

### Erasure and cleanup

`opaque(v)` seals any value for recovery with
`opaque.downcast<T>(o) -> ?T` — see [errors and
optionality](errors.md). Implementing `Disposal` for a type gives it
cleanup code that runs when its cell's refcount reaches zero, and
`Weak.new(v)` holds a non-keeping reference (`upgrade() -> ?T`, `nil`
once the referent died) — the memory stories live in
[the Rc heap and destructors](../reference/rc-heap.md) and
[weak references](../reference/weak-refs.md).

### The builder is a package

Core has no string-building class: the growable builder lives in the
`strbuild` package ([the stdlib
reference](../reference/stdlib.md#strbuild--the-builder)) —
`use strbuild::{ StringBuilder }`. For the common accumulator shape you
need no builder at all: `out = f"{out}{t}"` appends in place, linear in
the total output.

### One gated name

`NAN` is core's single constant, deliberately name-explicit:
`use core::{ NAN };`. The float constants live in `calc`.

## `pouch` — the growable sequence

```rut
use pouch::{ Vec };
use ink::{ Logger };

entry fn main() {
    let log = Logger.new("pouch");
    let mut xs: Vec<i32> = Vec.new();         // or Vec.with_capacity(64)
    xs.push(10);                              // amortized O(1)
    let v = Vec<f32>.filled(0.0, 1024);       // n slots of one value
    let w = Vec<i32>.from([1, 2, 3]);         // from a fixed array (copies)
    let x = xs[0];                            // indexing
    xs[0] = 42;                               // needs a `mut` binding
    let last = xs.pop();                      // removes + returns; traps if empty
    let n = xs.len();
    let mut total = 0;
    for (let e of w) {                        // iteration
        total += e;
    }
    log.info(f"x={x} last={last} len-after-pop={n} filled={v.len()} total={total}");
}
```

```text
x=10 last=42 len-after-pop=0 filled=1024 total=6
```

`Vec<u8>` is the mutable byte builder: `push` bytes, then `freeze()`
into the immutable `bytes`. `xs.as_array()` copies the live elements
into a fixed `[T]`. A `slice(from, to)` window is compiler-lowered —
an O(1) view that writes through to the parent vector.

## `nmapset` — keyed collections

```rut
use nmapset::{ HashMap, HashSet };
use ink::{ Logger };

entry fn main() {
    let log = Logger.new("nmapset");
    let mut counts: HashMap<str, i32> = HashMap.new();
    let fresh = counts.put("rut", 1);          // answers true when the key was NEWLY added
    let mut n = 0;
    let hit = counts.get("rut");   // ?i32 — nil means absent
    if (hit != nil) { n = hit + 1; }
    let there = counts.has("rut");             // membership
    let gone = counts.remove("runs");         // answers whether it was there
    let size = counts.len();

    let mut seen: HashSet<str> = HashSet.new();
    let first = seen.put("x");     // true — newly added
    let again = seen.put("x");     // false
    log.info(f"fresh={fresh} n={n} has={there} removed={gone} len={size}");
    log.info(f"first={first} again={again}");
}
```

```text
fresh=true n=2 has=true removed=false len=1
first=true again=false
```

Keys come from a fixed set — integers, `bool`, `str`, `bytes` (no
floats: they have no stable equality contract) — hashed by the host; a
user-defined key escapes by encoding canonically to `bytes`. A hit
returns the stored cell, not a copy. There is no iteration surface:
maps and sets answer questions, they don't walk.

## `flow` — the push pipeline

`Flow<E>` chains the push contract. A type is iterable when it
registers `impl Iterable<E> for T` — `for (x of it)` desugars to
`it.iterate(emit)` — and a Flow wraps one drive in adapter stages: a
closure per stage, never per element. Entry is `into_flow()`, the exit
is a sink (`Vec.from_flow`), and everything between is
chaining:

```rut
use pouch::{ Vec };
use flow::{ Flow, IntoFlow, FromFlow };
use ink::{ Logger };

entry fn main() -> nil {
    let log = Logger.new("flow");
    let nums: Vec<i32> = Vec.new();
    nums.push(1); nums.push(2); nums.push(3); nums.push(4); nums.push(5);

    // entry → adapters → sink
    let picked: Vec<i32> = Vec.from_flow(nums.into_flow()
        .map(fn(x: i32) -> i32 { return x * 2; })
        .filter(fn(x: i32) -> bool { return x > 4; })
        .take(3));
    let joined: Vec<str> = Vec.new();
    for (let x of picked.into_flow()) {
        joined.push(f"{x}");
    }

    // chains feed plain for..of (Flow is an Iterable)
    let mut sum = 0;
    for (let x of nums.into_flow().skip(1)) {
        sum += x;
    }
    log.info(f"picked={picked.len} first={picked[0]} last={picked[picked.len-1]} sum={sum}");
}
```

```text
picked=3 first=6 last=10 sum=14
```

Three laws to know:

- **The entries and sinks are traits** — `IntoFlow<E>` rows exist for
  the builtin sequences (`[T]`, `str`, `bytes`), for `Vec<T>`, and for
  `Flow<E>` itself (the identity row: a chain re-enters as a source),
  so generic code bounded on `IntoFlow<E>` takes chains and sources
  alike. The sinks (`FromFlow<E>`) take the CONTRACT —
  `it: Iterable<E>` — so a user iterable (a `CountUp`-shape, no Flow
  involved) widens straight into them.
- **`map` introduces a new type variable.** Annotate the lambda, spell
  the type argument (`.map<i32>(..)`), or pass a fn path — a lambda
  that spells nothing diagnoses with the fix.
- **Stateful stages are single-shot.** `take`/`skip` hold their
  counter in a record the drive mutates; a drained stage stays
  drained. `take` answers `false` at its stop, which stops the SOURCE
  — the elements after it are never driven.

Pipelines are the clarity tier: each stage costs one indirect call per
element, and the fused builtin loops stay the perf tier. The full
member table lives in [the standard library
reference](../reference/stdlib.md#flow-the-push-pipeline).

## `strbuild` — the string builder

```rut
use strbuild::{ StringBuilder };
use ink::{ Logger };

entry fn main() {
    let log = Logger.new("strbuild");
    let n = 3;
    let mut b = StringBuilder.new();     // or StringBuilder.with_cap(1024)
    b.append("count: ");
    b.append_code(33);                   // one codepoint
    b.append(f" up to {n}");             // appends are amortized O(1)
    let s = b.build();                   // the ONE materialization; builder keeps its buffer
    log.info(s);
}
```

```text
count: ! up to 3
```

Every `out = f"{out}{chunk}"` loop copies the whole prefix each time;
the builder appends into one growable cell and copies once, at
`build()`.

## `calc` — float math

```rut
use calc::{ Math };
use ink::{ Logger };

entry fn main() {
    let log = Logger.new("calc");
    let x: f64 = -2.0;
    let a: f64 = 3.0;
    let b: f64 = 7.0;
    let d = Math.sqrt(2.0);        // f64 host fns: sin, pow, atan2, fma, ...
    let f = Math.sqrt_f(2.0f32);   // f32 twins under a `_f` suffix
    let mx = Math.max(a, b);
    let sg = Math.signum(x);
    log.info(f"sqrt={d} sqrt_f={f} pi={Math.PI}");
    log.info(f"abs={Math.abs(x)} min={Math.min(a, b)} max={mx} signum={sg}");
}
```

```text
sqrt=1.4142135623730951 sqrt_f=1.4142135 pi=3.141592653589793
abs=2 min=3 max=7 signum=-1
```

rut has no overloading, so the width lives in the name. The integer
ladder is *not* here — those are core's, always available.

## `json` — encode and decode

```rut
use json::{ decodeJson, encodeJson };
use ink::{ Logger };

entry fn main() {
    let log = Logger.new("json");
    let (n, e) = decodeJson<i64>("42");            // (?T, ?E) — see errors
    let (s, ee) = encodeJson<[i64]>([1, 2, 3]);    // (?str, ?EncodeJsonError)
    if (e == nil && ee == nil) {
        let text = s;
        log.info(f"n={n} s={text}");
    }
}
```

```text
n=42 s=[1,2,3]
```

Decode is direct and schema-driven: your type's
`impl JsonDeserialize` reads exactly the fields it expects, no
intermediate tree. Your types opt in with two small impls; the
container impls (`Vec<T>`, the map/set family) mount automatically when
those packages are in your program. Error values are a kind enum plus a
struct with `.at`/`.got`/`.expected` — see
[errors and optionality](errors.md).

## `ink` — logging

All output goes through a logger:

```rut
use ink::{ Logger };

entry fn main() {
    let n = 3;
    let log = Logger.new("app");
    log.info(f"started with {n} items");
    log.debug("..."); log.warn("..."); log.error("...");
}
```

```text
started with 3 items
...
...
...
```

`Logger` is a plain rut class over a host-provided handle; the
embedder chooses the sink (the `rut` CLI prints to stdout). The same
pattern — declare a host surface in a `.d.rut`, wrap it in a rut class —
is how *any* embedder package reaches rut code; see
[embedding and native modules](../reference/embedding.md).

## `http`

The async HTTP client lives with the concurrency chapter —
builder construction, `send(cx)` resolving at headers, body drains and
byte streams — in [async: futures, workers, and channels](async.md).

## Put it together

```rut
use pouch::{ Vec };
use strbuild::{ StringBuilder };
use calc::{ Math };
use ink::{ Logger };

// assert is plain rut code now: the helper a package writes over panic
fn assert(c: bool, m: str) { if (!c) { panic(m); } }

entry fn main() {
    let log = Logger.new("std");

    // Vec: build, pop, read
    let mut v: Vec<i32> = Vec.new();
    v.push(10);
    v.push(20);
    v.push(30);
    let last = v.pop();
    log.info(f"len={v.len()} last={last} first={v[0]}");

    // fixed arrays + string_join from core
    let parts: Vec<str> = Vec.from(["rut", "runs"]);
    log.info(f"joined={string_join(parts.as_array())}");

    // StringBuilder: amortized appends, one materialization
    let mut b = StringBuilder.new();
    b.append("count: ");
    b.append_code(33);
    b.append(f" up to {v.len()}");
    let s = b.build();
    log.info(s);

    // calc's Math namespace (f64) and its f32 twins (_f)
    let d = Math.sqrt(2.0);
    let f = Math.sqrt_f(2.0);
    log.info(f"sqrt2 f64~{d} f32~{f} pi={Math.PI}");

    // assert: the bug-catcher this file wrote itself
    assert(v.len() == 2, "vec should hold two");
    log.info("asserted");
}
```

```text
len=2 last=30 first=10
joined=rutruns
count: ! up to 2
sqrt2 f64~1.4142135623730951 f32~1.4142135 pi=3.141592653589793
asserted
```

That completes the tutorial. From here, the
[core concepts](../core-concepts/design-goals.md) explain *why* the
language is shaped the way it is, and the
[reference](../reference/lexical-structure.md) pins down every rule.
