# The playground corpus

The classics: seventeen short, self-contained rut programs in
`demo/src/examples/` that the web playground offers in its picker and
runs in your browser. They are not copies — the playground imports
the `.rut` files **raw**, so what you edit there is what this repo
ships. The classics are the fastest way to see the language surface
in working code: algorithms first, then the language-surface tours,
then the memory shapes. The same order is the playground's.

## Run it

The playground itself:

```sh
cd demo && npm run dev      # serve the playground locally
# or use the deployed instance: https://playground.rut.hpp2334.com
```

Pick a case and press run — and the classics shown in the tour below
run in the book too, on their ▶ buttons. The gates that keep the
corpus honest:

```sh
cargo test -p rut-cli --test playground   # from the repo root — the native gate
cd demo && npm run smoke                  # the wasm gate, headless
```

Concretely:

- `cargo test -p rut-cli --test playground` compiles **and runs**
  every classic through the full pipeline natively — a compile failure
  or a trap fails the gate.
- `cd demo && npm run smoke` builds the demo bundle (wasm included)
  and drives every case through it, headless — the same run through
  the wasm engine.

Both gates drive the same sources through the same engine — native
and wasm — so a classic that stops running cleanly fails in CI, not
in front of a reader.

## The seventeen cases

| Case | Demonstrates | First line it prints |
|---|---|---|
| `sieve` | Sieve of Eratosthenes — flat `Vec<u8>`/`Vec<i32>` primitive buffers | `25 primes up to 100, last=97` |
| `quicksort` | in-place `Vec<i32>` mutation (handles, shared with the caller), recursion | `sorted: 1 2 2 3 5 7 8 9` |
| `matrix-mul` | flat `Vec<f32>` hot loops — unboxed buffers, no per-element refcounts | `out[0]=21 out[last]=107` |
| `classes` | class-method construction (`new`/`from`), `Self {}`, member `pub` + sealing | `count=2 area=12` |
| `closures-generics` | anonymous fns (block bodies), monomorphized generics, fn types, capture | `add=3 area=3.1415927 sum=6` |
| `structs` | reference semantics (sharing by default), identity `==` | `len=6.324555320336759 color=16711935 area=6` |
| `literals` | numeric suffixes, plain/raw/format strings, fixed `[T]`, bytes buffers | `a=10 e=1.5 d64=1.5 ch=h p.x=1 zero[0]=9 len=3 bin=64` |
| `checked-arith` | `wrapping_*` wraps two's-complement, `checked_*` answers the `(T, bool)` tuple | `wrap=4 under=255` |
| `str-views` | O(1) slice views (a slice IS a str), codepoints — `s.code` / `str.from_code` | `word=world len=5 eq=true` |
| `bytes` | the binary primitive — encode/decode, clone as the ONE copy | `round=true octets=8 chars=8` |
| `opaque` | `opaque` / `opaque.downcast<T> -> ?T` / `is` — erasure and checked recovery | `point 1 2` |
| `when` | `when` pattern expressions over enums, exhaustiveness | `small` |
| `maps` | the keyed-collection lane — `HashMap`/`HashSet`, keys admitted by the compile-time union bound | `rut=3 runs=1` |
| `node-cycle` | strong cycles keep cells alive — the program's responsibility | `head.next alive: true` |
| `tree` | recursive structs (`?Node` nullable fields), composite fields as handle slots | `nodes=15` |
| `weak-cache` | the cache/observer shape, shown with today's strong refs | `held: true id=1` |
| `type-aliases` | transparent aliases, bound-only unions, inline `requires` at the call site | `trip=1500 plain=1500 ridge/trench kind=trench` |

Output goes through `ink`'s `Logger` (the `log.info` lines above) —
the same logging surface the std packages use
([core and the swappable packages](../reference/stdlib.md)).

## Code tour

### A classic, whole

`demo/src/examples/quicksort/mod.rut` is the archetype — imports, one
algorithm, one log line — and the shared-handle mutation law in
action (the sort writes through the caller's vec). Verbatim, so what
runs here is exactly what the playground edits:

```rut
use pouch::{ Vec };
// Quicksort — in-place Vec<i32> mutation (vecs are handles: the
// mutation is shared with the caller), recursion, explicit conversions.

use ink::{ Logger };

fn swap(mut xs: Vec<i32>, a: i32, b: i32) {
    let t = xs[a];
    xs[a] = xs[b];
    xs[b] = t;
}

fn partition(xs: Vec<i32>, lo: i32, hi: i32) -> i32 {
    let pivot = xs[hi];
    let mut i = lo - 1;
    for (let j = lo; j < hi; j += 1) {
        if (xs[j] <= pivot) {
            i += 1;
            swap(xs, i, j);
        }
    }
    swap(xs, i + 1, hi);
    return i + 1;
}

fn quicksort(xs: Vec<i32>, lo: i32, hi: i32) {
    if (lo >= hi) { return; }
    let p = partition(xs, lo, hi);
    quicksort(xs, lo, p - 1);
    quicksort(xs, p + 1, hi);
}

entry fn main() {
    let log = Logger("sort");
    let xs = Vec<i32>.from([5, 2, 9, 1, 7, 3, 8, 2]);   // fixed -> growable
    quicksort(xs, 0, xs.len() - 1);

    let mut out = "";
    for (let i = 0; i < xs.len(); i += 1) {
        out = f"{out}{xs[i]} ";      // `mut`: rebinding; the f-string form builds in place
    }
    log.info(f"sorted: {out}");
}
```

```text
sorted: 1 2 2 3 5 7 8 9 
```

(The line above ends with the trailing space the program builds —
one after every element — kept byte-for-byte.)

Compare with [01 — Sort](01-sort.md): same algorithm, no host — run
it here and compare with the fuel-counted host sweep.

### Erasure and checked recovery, in one screen

The `opaque` case is the reference for the erasure primitive —
`opaque(v)` forgets the static type, `opaque.downcast<T>` answers the
nullable, and `is` names the *box*, never the payload
([opaque — erasure and downcast](../reference/opaque.md)). The case's
`erase_and_recover` verbatim, with its two payload types and a `main`
so it runs in place:

```rut
use ink::{ Logger };

struct Point { x: f32; y: f32 }
enum Flavor { Sweet, Sour }

fn erase_and_recover() {
    let log = Logger("opaque");
    let box1 = opaque(Point { x: 1, y: 2 });    // erasure = type-call;
    let box2 = opaque(Flavor.Sour);             // zero-copy (shares the cell)
    let box3 = opaque("hello");

    let p = opaque.downcast<Point>(box1);       // ?Point — the nullable
    when (p != nil) {
        true -> { log.info(f"point {p.x} {p.y}"); },
        else  -> { log.info("point: nil"); },
    }
    let wrong = opaque.downcast<i32>(box2);     // wrong type: nil, no trap
    log.info(f"sour? {opaque.downcast<Flavor>(box2) != nil} wrong? {wrong == nil}");
    log.info(f"is str: {box3 is str}");   // `is` names the box — misses every payload type; downcast recovers
}

entry fn main() {
    erase_and_recover();
}
```

```text
point 1 2
sour? true wrong? true
is str: false
```

### How a case is wired

Each classic is registered in `demo/src/examples/index.ts` — an id, a
blurb, and the raw source, in the playground's order. The table above
quotes each case's first log line, and the tour's blocks run right
here — quicksort exactly as shipped. Two details worth noticing when
you browse: the corpus
is walked by the parser conformance test too (alongside `examples/`
and the std tree — every `.rut` file in the repo must parse clean),
and one quicksort log line ends with a trailing space the program
builds on purpose — run it above and look closely.

## Takeaways

- The classics are **live sources**, not string copies: the
  playground edits what the repo ships.
- Every case is exercised by **two gates** — native and wasm — over
  the same engine.
- Seventeen files cover the language surface in run-sized doses:
  algorithms, types and interfaces, strings and bytes, erasure, patterns,
  maps, and the memory shapes.
- When you change the language, these files are the first smoke test
  — and often the clearest place to demonstrate the change.

For the bigger, host-driven versions of the same ideas, start at
[00 — Todolist](00-todolist.md) and work up to
[06 — GitHub viewer CLI](06-github-viewer-cli.md).
