# The standard library

The standard library splits in two. **`core`** is the only true
standard — a prelude of builtin names that are *ambient*: in scope in
every compilation unit, no `use` needed. Everything else is a set of
**swappable packages** shipped in the toolchain tree — a program that
wants one says so (a `use` line; the CLI mounts the tree packages for
loose files automatically, and a package program lists them in its
manifest). Any of them can be replaced wholesale; the engine knows none
of their names.

The reference page is [core and the swappable packages](../reference/stdlib.md).

## What's always there (`core`)

### Reporting and bugs

```rut
panic("Rect: negative extents");       // abort with a message
assert(total == expected, "checksum"); // abort when false (message optional)
```

### Integer safety ladder

Every integer width carries compiler-lowered methods. Plain `+`/`-`/`*`
trap on overflow; these never do:

```rut
let mut v: u8 = 250;
v = v.wrapping_add(10);            // 4 — two's-complement wrap
let over = 200u8.checked_add(100); // (44, false) — .1 false = escaped
let sat = 200u8.saturating_add(100); // 255 — clamp at the bounds
```

`checked_*` answers `(T, bool)`; `.0` holds the wrapped bits either
way. Pick per call site: a checksum wraps, a length checks.

### `str` members

```rut
s.len()                 // codepoints (not bytes)
s.code()                // the FIRST codepoint as u32 (traps on empty)
s.code_at(i)            // the codepoint at codepoint index i
s.encode()              // the UTF-8 octets as bytes
s.slice(from, to)       // an O(1) view — a slice IS a str
s.starts_with(from, head)
string_join(parts)      // join a [str] in one pass
```

`s.slice` deserves a second look: no octets move; the view records a
window over the parent, prints, compares by content, iterates, and can
re-slice. Codepoint access is spelled with integers — `str.from_code(n)`
builds the 1-codepoint `str` for a `u32`. There is no `split` primitive;
tokenizing rides `s.scan(from, set)` over a caller-owned `[u8]` class
table. See [string slicing and views](../reference/string-views.md).

### `bytes` members

```rut
b.len()          // octets
b.decode()       // UTF-8 (lossy)
b.clone()        // the ONLY copy escape hatch in the language
bytes.zeroed(n)  // n zeroed octets
bytes.from(a)    // from a [u8]
```

### Erasure and cleanup

`opaque(v)` seals any value for recovery with
`opaque.downcast<T>(o) -> ?T` — see [errors and
optionality](errors.md). `on_drop(p, cleanup)` runs a callback when a
cell's refcount reaches zero, and `Weak(v)` holds a non-keeping
reference (`upgrade() -> ?T`, `nil` once the referent died) — the
memory stories live in
[the Rc heap and destructors](../reference/rc-heap.md) and
[weak references](../reference/weak-refs.md).

### `StrBuf` — the raw growable builder

Prefer the package face below; the engine cell under it is `StrBuf(cap)`
with `push`/`push_code`/`len`/`finish`.

### One gated name

`NAN` is core's single constant, deliberately name-explicit:
`use core::{ NAN };`. The float constants live in `calc`.

## `pouch` — the growable sequence

```rut
use pouch::{ Vec };

let mut xs: Vec<i32> = Vec.new();         // or Vec.with_capacity(64)
xs.push(10);                              // amortized O(1)
let v = Vec<f32>.filled(0.0, 1024);       // n slots of one value
let w = Vec<i32>.from([1, 2, 3]);         // from a fixed array (copies)
let x = xs[0];                            // indexing
xs[0] = 42;                               // needs a `mut` binding
let last = xs.pop();                      // removes + returns; traps if empty
let n = xs.len();
for (let x of xs) { .. }                  // iteration
```

`Vec<u8>` is the mutable byte builder: `push` bytes, then `freeze()`
into the immutable `bytes`. `xs.as_array()` copies the live elements
into a fixed `[T]`. A `slice(from, to)` window is compiler-lowered —
an O(1) view that writes through to the parent vector.

## `nmapset` — keyed collections

```rut
use nmapset::{ HashMap, HashSet };

let mut counts: HashMap<str, i32> = HashMap.new();
counts.put("rut", 1);          // answers true when the key was NEWLY added
let hit = counts.get("rut");   // ?i32 — nil means absent
if (hit != nil) { n = hit + 1; }
counts.has("rut");             // membership
counts.remove("runs");         // answers whether it was there
counts.len();

let mut seen: HashSet<str> = HashSet.new();
let first = seen.put("x");     // true — newly added
let again = seen.put("x");     // false
```

Keys come from a fixed set — integers, `bool`, `str`, `bytes` (no
floats: they have no stable equality contract) — hashed by the host; a
user-defined key escapes by encoding canonically to `bytes`. A hit
returns the stored cell, not a copy. There is no iteration surface:
maps and sets answer questions, they don't walk.

## `strbuild` — the string builder

```rut
use strbuild::{ StringBuilder };

let mut b = StringBuilder.new();     // or StringBuilder.with_cap(1024)
b.append("count: ");
b.append_code(33);                   // one codepoint
b.append(f" up to {n}");             // appends are amortized O(1)
let s = b.build();                   // the ONE materialization; builder keeps its buffer
```

Every `out = f"{out}{chunk}"` loop copies the whole prefix each time;
the builder appends into one growable cell and copies once, at
`build()`.

## `calc` — float math

```rut
use calc::{ Math };

let d = Math.sqrt(2.0);        // f64 host fns: sin, pow, atan2, fma, ...
let f = Math.sqrt_f(2.0f32);   // f32 twins under a `_f` suffix
Math.abs(x); Math.min(a, b); Math.max(a, b); Math.signum(x);
Math.PI; Math.E; Math.INFINITY; Math.TAU;   // f64 constants
```

rut has no overloading, so the width lives in the name. The integer
ladder is *not* here — those are core's, always available.

## `json` — encode and decode

```rut
use json::{ decodeJson, decodeJsonBytes, encodeJson, JsonSerialize, JsonDeserialize };

let (n, e) = decodeJson<i64>("42");            // (?T, ?E) — see errors
let (s, ee) = encodeJson<[i64]>([1, 2, 3]);    // (?str, ?EncodeJsonError)
```

Decode is direct and schema-driven: your type's
`impl JsonDeserialize` reads exactly the fields it expects, no
intermediate tree. Your types opt in with two small impls; the
container impls (`Vec<T>`, the map/set family) mount automatically when
those packages are in your program. Error values are a kind enum plus a
struct with `.at`/`.got`/`.expected` — see
[errors and optionality](errors.md).

## `ink` — logging

There is no `console`, no `print` — all output goes through a logger:

```rut
use ink::{ Logger };

let log = Logger.new("app");
log.info(f"started with {n} items");
log.debug("..."); log.warn("..."); log.error("...");
```

`Logger` is a plain rut class over a host-provided handle; the
embedder chooses the sink (the `rut` CLI prints to stdout). The same
pattern — declare a host surface in a `.d.rut`, wrap it in a rut class —
is how *any* embedder package reaches rut code; see
[embedding and native modules](../reference/embedding.md).

## `http`

The async HTTP client lives with the concurrency chapter —
builder construction, `send(cx)` resolving at headers, body drains and
byte streams — in [async: tasks, workers, and channels](async.md).

## Put it together

```rut
use pouch::{ Vec };
use strbuild::{ StringBuilder };
use calc::{ Math };
use ink::{ Logger };

pub fn main() {
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

    // assert: the builtin bug-catcher
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
